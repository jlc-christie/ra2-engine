//! Development aids.

use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureFormat;
use bevy::window::PrimaryWindow;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

const CAPTURE_FRAME: u32 = 60;
const EXIT_FRAME: u32 = 120;

/// With `RA2_SHOT=<file.png>` set, saves a screenshot of the window to that file
/// shortly after start-up and then exits.
pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FakePointer>();
        if let Ok(script) = std::env::var("RA2_INPUT") {
            app.insert_resource(Script::parse(&script)).add_systems(PreUpdate, play_script.after(bevy::input::InputSystems));
        }
        if std::env::var_os("RA2_SHOT").is_some() {
            app.add_systems(PostStartup, draw_off_screen).add_systems(Update, (follow_window, capture_and_exit).chain());
        }
    }
}

/// The picture the game is drawn into while taking a screenshot.
#[derive(Resource)]
struct OffScreen(Handle<Image>);

/// Points the camera at a picture the size of the window instead of the window, so
/// the screenshot does not depend on the window being visible on the desktop.
fn draw_off_screen(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<Entity, With<Camera2d>>,
) {
    let image = Image::new_target_texture(
        window.physical_width(),
        window.physical_height(),
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    let handle = images.add(image);
    commands.entity(*camera).insert((RenderTarget::Image(ImageRenderTarget { handle: handle.clone(), scale_factor: window.scale_factor() }), IsDefaultUiCamera));
    commands.insert_resource(OffScreen(handle));
}

/// Keeps the picture the size of the window when the window changes size.
fn follow_window(mut images: ResMut<Assets<Image>>, window: Single<&Window, With<PrimaryWindow>>, target: Option<Res<OffScreen>>, mut cameras: Query<(&mut RenderTarget, &mut Projection), With<Camera2d>>) {
    let Some(target) = target else {
        return;
    };
    let wanted = (window.physical_width(), window.physical_height());
    // Only touch the render target when its dimensions change.
    if images.get(&target.0).is_some_and(|image| (image.width(), image.height()) != wanted) {
        if let Some(mut image) = images.get_mut(&target.0) {
            image.resize(Extent3d { width: wanted.0, height: wanted.1, depth_or_array_layers: 1 });
        }
    }
    for (mut camera, mut projection) in &mut cameras {
        if let RenderTarget::Image(image) = &mut *camera {
            if image.handle == target.0 && image.scale_factor != window.scale_factor() {
                image.scale_factor = window.scale_factor();
                // Bevy recalculates target information when the projection
                // changes, including a DPI change without a physical resize.
                projection.set_changed();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offscreen_target_tracks_physical_size_and_dpi_independently() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>().add_systems(Update, follow_window);
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(Image::new_target_texture(800, 600, TextureFormat::Rgba8UnormSrgb, None));
        app.insert_resource(OffScreen(handle.clone()));
        let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();
        let camera = app.world_mut().spawn((Camera2d, RenderTarget::Image(handle.clone().into()))).id();
        // Include a DPI change with no physical resize, as when moving the
        // window between monitors while preserving its physical bounds.
        for (physical, dpi) in [(UVec2::new(1600, 1200), 2.0), (UVec2::new(1600, 1200), 1.25), (UVec2::new(800, 600), 1.0)] {
            {
                let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
                window.resolution.set_physical_resolution(physical.x, physical.y);
                window.resolution.set_scale_factor(dpi);
            }
            app.update();
            let target = app.world().get::<RenderTarget>(camera).unwrap();
            let RenderTarget::Image(target) = target else { panic!("expected image target") };
            assert_eq!(target.scale_factor, dpi);
            let image = app.world().resource::<Assets<Image>>().get(&handle).unwrap();
            assert_eq!(image.size(), physical);
            assert_eq!(image.size().as_vec2() / target.scale_factor, app.world().get::<Window>(window).unwrap().size());
        }
    }
}

fn capture_and_exit(
    mut commands: Commands,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    camera: Single<&RenderTarget, With<Camera2d>>,
    sim: Option<Res<crate::units::Sim>>,
    mut reached: Local<Option<u32>>,
) {
    *frame += 1;
    // RA2_SHOT_FRAME overrides how many frames to wait before the screenshot.
    // RA2_SHOT_TICK instead waits for the simulation to reach a tick (use with
    // RA2_SPEED to get there quickly), then a few frames more for the picture to
    // catch up.
    let wanted_tick: Option<u64> = std::env::var("RA2_SHOT_TICK").ok().and_then(|n| n.parse().ok());
    let capture_frame = match wanted_tick {
        Some(tick) => {
            if reached.is_none() && sim.is_some_and(|sim| sim.0.current_tick() >= tick) {
                *reached = Some(*frame + 5);
            }
            reached.unwrap_or(u32::MAX - EXIT_FRAME)
        }
        None => std::env::var("RA2_SHOT_FRAME").ok().and_then(|n| n.parse().ok()).unwrap_or(CAPTURE_FRAME),
    };
    if *frame == capture_frame {
        let path = std::env::var("RA2_SHOT").unwrap_or_default();
        // The image target's DPI participates in its identity. Screenshot::image
        // assumes 1x, so preserve the camera's complete target on HiDPI screens.
        let shot = Screenshot(camera.into_inner().clone());
        commands
            .spawn(shot)
            .observe(save_to_disk(path));
    }
    if *frame == capture_frame + EXIT_FRAME - CAPTURE_FRAME {
        exit.write(AppExit::Success);
    }
}

/// Where a script has put the mouse.
#[derive(Resource, Default)]
pub struct FakePointer(pub Option<Vec2>);

/// `RA2_INPUT`: mouse and key presses played into the game, for automated
/// checks. Steps are separated by `;` and run in order, each as
/// `<when> <what>`: `<when>` is `t<tick>` (once the simulation has reached that
/// tick) or `+<frames>` (so many frames after the step before); `<what>` is
/// `at x,y` (put the mouse there, in window pixels), `left`, `right` (a click),
/// `hold left`/`hold right` and `let left`/`let right` (press and release) or
/// `key <KeyCode name or letter>`. Mouse steps also go to Bevy's picking,
/// so menu buttons can be clicked; that works only without `RA2_SHOT` (with
/// the off-screen picture only the screen's outer nodes are hit; not looked
/// into).
#[derive(Resource, Default)]
pub(crate) struct Script {
    steps: Vec<(When, What)>,
    next: usize,
    /// Frames since the last step ran.
    waited: u32,
    /// What to let go of, and in how many frames.
    held: Vec<(What, u32)>,
}

#[derive(Clone, Copy)]
enum When {
    Tick(u64),
    Frames(u32),
}

#[derive(Clone, Copy)]
enum What {
    At(Vec2),
    Mouse(MouseButton),
    Hold(MouseButton),
    Let(MouseButton),
    Key(KeyCode),
}

impl Script {
    fn parse(text: &str) -> Self {
        let steps = text
            .split(';')
            .filter_map(|step| {
                let mut words = step.split_whitespace();
                let when = words.next()?;
                let when = match when.strip_prefix('t') {
                    Some(tick) => When::Tick(tick.parse().ok()?),
                    None => When::Frames(when.trim_start_matches('+').parse().ok()?),
                };
                let what = match (words.next()?, words.next()) {
                    ("at", Some(at)) => {
                        let (x, y) = at.split_once(',')?;
                        What::At(Vec2::new(x.parse().ok()?, y.parse().ok()?))
                    }
                    ("left", _) => What::Mouse(MouseButton::Left),
                    ("right", _) => What::Mouse(MouseButton::Right),
                    ("hold", Some("left")) => What::Hold(MouseButton::Left),
                    ("hold", Some("right")) => What::Hold(MouseButton::Right),
                    ("let", Some("left")) => What::Let(MouseButton::Left),
                    ("let", Some("right")) => What::Let(MouseButton::Right),
                    ("key", Some(name)) => What::Key(key_named(name)?),
                    _ => return None,
                };
                Some((when, what))
            })
            .collect();
        Self { steps, ..default() }
    }
}

fn key_named(name: &str) -> Option<KeyCode> {
    Some(match name {
        "Tab" => KeyCode::Tab,
        "Escape" => KeyCode::Escape,
        "Space" => KeyCode::Space,
        "Enter" => KeyCode::Enter,
        "Ctrl" => KeyCode::ControlLeft,
        "Alt" => KeyCode::AltLeft,
        "Shift" => KeyCode::ShiftLeft,
        _ => {
            let letter = name.chars().next().filter(|_| name.len() == 1)?.to_ascii_uppercase();
            const LETTERS: [KeyCode; 26] = [
                KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG, KeyCode::KeyH, KeyCode::KeyI,
                KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR,
                KeyCode::KeyS, KeyCode::KeyT, KeyCode::KeyU, KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ,
            ];
            const DIGITS: [KeyCode; 10] = [
                KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7,
                KeyCode::Digit8, KeyCode::Digit9,
            ];
            match letter {
                'A'..='Z' => LETTERS[letter as usize - 'A' as usize],
                '0'..='9' => DIGITS[letter as usize - '0' as usize],
                _ => return None,
            }
        }
    })
}

pub(crate) fn play_script(
    mut script: ResMut<Script>,
    mut pointer: ResMut<FakePointer>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    sim: Option<Res<crate::units::Sim>>,
    mut picking: MessageWriter<bevy::picking::pointer::PointerInput>,
    window: Single<Entity, With<PrimaryWindow>>,
    camera: Query<Option<&RenderTarget>, With<Camera2d>>,
) {
    use bevy::picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput};
    // The menus are buttons that Bevy's picking finds: the same steps go to it
    // as the mouse pointer, at the picture the camera draws into.
    let target = camera.iter().next().flatten().cloned().unwrap_or_default().normalize(Some(*window));
    let at = pointer.0.unwrap_or_default();
    let mut send = |action: PointerAction| {
        if let Some(target) = target.clone() {
            picking.write(PointerInput::new(PointerId::Mouse, Location { target, position: at }, action));
        }
    };
    let script = &mut *script;
    script.held.retain_mut(|(what, frames)| {
        *frames -= 1;
        if *frames == 0 {
            match *what {
                What::Mouse(button) => {
                    buttons.release(button);
                    if button == MouseButton::Left {
                        send(PointerAction::Release(PointerButton::Primary));
                    }
                }
                What::Key(key) => keys.release(key),
                What::At(_) | What::Hold(_) | What::Let(_) => {}
            }
        }
        *frames > 0
    });
    script.waited += 1;
    let Some(&(when, what)) = script.steps.get(script.next) else {
        return;
    };
    let due = match when {
        When::Tick(tick) => sim.as_ref().is_some_and(|sim| sim.0.current_tick() >= tick),
        When::Frames(frames) => script.waited > frames,
    };
    if !due {
        return;
    }
    script.next += 1;
    script.waited = 0;
    match what {
        What::At(to) => {
            pointer.0 = Some(to);
            if let Some(target) = target.clone() {
                picking.write(PointerInput::new(PointerId::Mouse, Location { target, position: to }, PointerAction::Move { delta: to - at }));
            }
        }
        What::Mouse(button) => {
            buttons.press(button);
            if button == MouseButton::Left {
                send(PointerAction::Press(PointerButton::Primary));
            }
            script.held.push((what, 2));
        }
        What::Key(key) => {
            keys.press(key);
            script.held.push((what, 2));
        }
        What::Hold(button) => buttons.press(button),
        What::Let(button) => buttons.release(button),
    }
}
