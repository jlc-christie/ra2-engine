//! The bar under the map with its command buttons, and the list of status
//! messages at the top of the map view.
//!
//! Follows the original game: section 1 and topic B.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ra2_formats::csf::Csf;
use ra2_formats::fnt::Font;

use crate::cursor::Pointer;
use crate::files::GameFiles;
use crate::hud::{absolute, load, rgba_image, BOTTOM_BAR_HEIGHT, SIDEBAR_WIDTH};
use crate::shell::Screen;
use crate::units::UnitView;

const PALETTE: &str = "sidebar.pal";

/// What a button on the bar (or its key) asks for.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Team button n (1 or 2): make the team from the selection if it has no
    /// members, centre on it if all are selected, else select it.
    Team(usize),
    /// A right click on a team button: the team is disbanded.
    Disband(usize),
    TypeSelect,
    Deploy,
    Guard,
    Planning(bool),
}

/// Whether planning mode is on: the Z key and the bar's last button both set it.
#[derive(Resource, Default)]
pub struct PlanningMode(pub bool);

/// A line for the message list, by its key in the string table or as given.
#[derive(Message, Clone, Debug)]
pub enum Status {
    Key(&'static str),
    Text(String),
    /// A message the game posts with `MessageDelay` (8.64 s with the stock
    /// rules) in a house's colour: a player defeated, low power.
    Long(String, [u8; 3]),
}

/// How long the player's own messages last: 240 ticks of 16 ms.
const SHORT_MESSAGE: f64 = 240.0 * 0.016;
/// `MessageDelay` 0.6 x 900 ticks of 16 ms.
// DATA: rules.ini `[General] MessageDelay=.6`.
const LONG_MESSAGE: f64 = 0.6 * 900.0 * 0.016;

// The buttons of a
// skirmish, in slot order, each with its art (BUTTONnn by the button's number).
const BUTTONS: [(usize, &str); 6] = [(0, "team"), (1, "team"), (3, "type"), (4, "deploy"), (6, "guard"), (9, "plan")];

#[derive(Component)]
struct BarButton {
    slot: usize,
    frames: [Handle<Image>; 2],
}

#[derive(Component)]
struct Thumb;

#[derive(Component)]
struct OpenOnly;

#[derive(Resource)]
struct Bar {
    /// The bar's own picture, and the window width it was laid out for.
    root: Entity,
    width: f32,
    open: bool,
    /// The slot being held down, and with which mouse button.
    held: Option<(usize, MouseButton)>,
    first_x: f32,
    thumb_open_x: f32,
    thumb_shut_x: f32,
    click: Option<String>,
    font: Option<Font>,
    strings: Option<Csf>,
    /// The messages showing: text, picture, and when each goes (seconds).
    lines: Vec<(Entity, f64)>,
}

#[derive(Component)]
struct MessageRoot;

/// A message being typed out: one more character every 16 ms, each showing
/// the box as wide as the text so far.
#[derive(Component)]
struct Typing {
    began: f64,
    widths: Vec<f32>,
    shown: usize,
}

/// `IncomingMessage` as a message that is not silent arrives, and
/// `MessageCharTyped` as its characters appear.
#[derive(Default)]
struct MessageSounds {
    incoming: Option<String>,
    typed: Option<String>,
    read: bool,
}

pub struct BarPlugin;

impl Plugin for BarPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Command>()
            .add_message::<Status>()
            .init_resource::<PlanningMode>()
            .add_systems(OnEnter(Screen::InGame), setup.after(crate::game::load_map))
            .add_systems(Update, relayout.run_if(in_state(Screen::InGame)).run_if(resource_exists::<Bar>))
            .add_systems(Update, (buttons, messages).run_if(in_state(Screen::InGame)).run_if(resource_exists::<Bar>));
    }
}

/// A window that changes width mid-game gets its bar laid out again, open or
/// shut as it was, with its messages kept.
fn relayout(mut commands: Commands, bar: Res<Bar>, window: Single<&Window, With<PrimaryWindow>>, mut respawn: Local<Option<bevy::ecs::system::SystemId>>) {
    if window.width() == bar.width {
        return;
    }
    let id = *respawn.get_or_insert_with(|| commands.register_system(setup));
    commands.entity(bar.root).despawn();
    commands.run_system(id);
}

fn setup(mut commands: Commands, files: Res<GameFiles>, mut images: ResMut<Assets<Image>>, window: Single<&Window, With<PrimaryWindow>>, old: Option<Res<Bar>>) {
    // The row of button places ends at the right end
    // cap, 28 pixels from the sidebar; there are as many 52-pixel places as fit
    // between two 28-pixel caps and a 168-pixel gap; the thumb sits left of the
    // first place when the bar is open and against the end cap when it is shut.
    let width = window.width() - SIDEBAR_WIDTH;
    let count = ((window.width() - 28.0 - 168.0 - 28.0) / 52.0).floor().max(0.0);
    let end_cap = width - 28.0;
    let first_x = end_cap - 52.0 * count;
    let (thumb_open_x, thumb_shut_x) = (first_x - 28.0, end_cap - 28.0);
    let root = commands
        .spawn((
            DespawnOnExit(Screen::InGame),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(SIDEBAR_WIDTH),
                bottom: Val::Px(0.0),
                height: Val::Px(BOTTOM_BAR_HEIGHT),
                overflow: Overflow::clip(),
                ..default()
            },
            GlobalZIndex(5),
        ))
        .with_children(|parent| {
            // The spacer, the thumb, a background
            // under each button place while open, the buttons, the end cap.
            if let Some((image, size)) = load(&files, &mut images, "lspacer.shp", 0, PALETTE) {
                parent.spawn((absolute(0.0, 0.0, size), ImageNode::new(image)));
            }
            if let Some((image, size)) = load(&files, &mut images, "bttnbkgd.shp", 0, PALETTE) {
                for slot in 0..count as usize {
                    parent.spawn((absolute(first_x + 52.0 * slot as f32, 0.0, size), ImageNode::new(image.clone()), OpenOnly));
                }
            }
            for (slot, (art, _)) in BUTTONS.iter().enumerate() {
                let name = format!("button{art:02}.shp");
                let (Some((up, size)), Some((down, _))) = (load(&files, &mut images, &name, 0, PALETTE), load(&files, &mut images, &name, 1, PALETTE)) else {
                    warn!("bar button {name} not found");
                    continue;
                };
                parent.spawn((absolute(first_x + 52.0 * slot as f32, 0.0, size), ImageNode::new(up.clone()), BarButton { slot, frames: [up, down] }, OpenOnly));
            }
            if let Some((image, size)) = load(&files, &mut images, "rendcap.shp", 0, PALETTE) {
                parent.spawn((absolute(end_cap, 0.0, size), ImageNode::new(image)));
            }
            if let Some((image, size)) = load(&files, &mut images, "lendcap.shp", 0, PALETTE) {
                parent.spawn((absolute(thumb_open_x, 0.0, size), ImageNode::new(image), Thumb));
            }
        })
        .id();

    // The list starts 3 pixels in from the left of
    // the map view, at its top.
    if old.is_none() {
        commands.spawn((
            DespawnOnExit(Screen::InGame),
            Node { position_type: PositionType::Absolute, left: Val::Px(3.0), top: Val::Px(0.0), ..default() },
            GlobalZIndex(4),
            MessageRoot,
        ));
    }
    let (open, lines) = old.as_ref().map_or((true, Vec::new()), |old| (old.open, old.lines.clone()));
    commands.insert_resource(Bar {
        root,
        width: window.width(),
        open,
        held: None,
        first_x,
        thumb_open_x,
        thumb_shut_x,
        click: crate::audio::rules_sound(&files, "GUIMainButtonSound"),
        font: files.get("game.fnt").and_then(Font::parse),
        strings: files.get("ra2.csf").and_then(Csf::parse),
        lines,
    });
}

/// Works the buttons and the thumb.
// A button acts when the mouse is let go over it; the
// team buttons also take the right button; the planning button stays down while
// its mode is on; the thumb hides and shows the buttons, with the button sound.
#[allow(clippy::too_many_arguments)]
fn buttons(
    mouse: Res<ButtonInput<MouseButton>>,
    pointer: Res<Pointer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut bar: ResMut<Bar>,
    mut planning: ResMut<PlanningMode>,
    mut shown: Query<(&BarButton, &mut ImageNode)>,
    mut open_only: Query<&mut Visibility, With<OpenOnly>>,
    mut thumb: Query<&mut Node, With<Thumb>>,
    mut orders: MessageWriter<Command>,
    mut status: MessageWriter<Status>,
    mut cues: MessageWriter<crate::audio::Cue>,
) {
    let top = window.height() - BOTTOM_BAR_HEIGHT;
    let over = pointer.0.filter(|at| at.y >= top && at.x < window.width() - SIDEBAR_WIDTH);
    let slot_over = over.filter(|_| bar.open).and_then(|at| {
        let slot = ((at.x - bar.first_x) / 52.0).floor();
        (at.x >= bar.first_x && (slot as usize) < BUTTONS.len()).then_some(slot as usize)
    });
    let thumb_x = if bar.open { bar.thumb_open_x } else { bar.thumb_shut_x };
    let on_thumb = over.is_some_and(|at| at.x >= thumb_x && at.x < thumb_x + 28.0);
    for button in [MouseButton::Left, MouseButton::Right] {
        if mouse.just_pressed(button) {
            if let Some(slot) = slot_over.filter(|&slot| button == MouseButton::Left || BUTTONS[slot].1 == "team") {
                bar.held = Some((slot, button));
            }
        }
        if mouse.just_released(button) {
            if let Some((slot, held)) = bar.held.filter(|(_, held)| *held == button) {
                bar.held = None;
                if slot_over == Some(slot) {
                    match (BUTTONS[slot].1, held) {
                        ("team", MouseButton::Left) => {
                            orders.write(Command::Team(slot + 1));
                        }
                        ("team", _) => {
                            orders.write(Command::Disband(slot + 1));
                        }
                        ("type", _) => {
                            orders.write(Command::TypeSelect);
                        }
                        ("deploy", _) => {
                            orders.write(Command::Deploy);
                        }
                        ("guard", _) => {
                            orders.write(Command::Guard);
                        }
                        _ => {
                            // What the button says
                            // as the mode comes on.
                            planning.0 = !planning.0;
                            orders.write(Command::Planning(planning.0));
                            if planning.0 {
                                status.write(Status::Key("MSG:PlanningModeIntro1Button"));
                                status.write(Status::Key("MSG:PlanningModeIntro3"));
                            }
                        }
                    }
                }
            }
        }
    }
    if mouse.just_released(MouseButton::Left) && on_thumb {
        bar.open = !bar.open;
        cues.write_batch(bar.click.clone().map(crate::audio::Cue::Sound));
    }
    for (button, mut image) in &mut shown {
        let down = bar.held.is_some_and(|(slot, _)| slot == button.slot && slot_over == Some(slot)) || (BUTTONS[button.slot].1 == "plan" && planning.0);
        let wanted = &button.frames[usize::from(down)];
        if image.image != *wanted {
            image.image = wanted.clone();
        }
    }
    let wanted = if bar.open { Visibility::Inherited } else { Visibility::Hidden };
    for mut visibility in &mut open_only {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    for mut node in &mut thumb {
        if node.left != Val::Px(thumb_x) {
            node.left = Val::Px(thumb_x);
        }
    }
}

/// Adds new status messages and takes away those whose time is up.
// At most six lines 19 pixels apart, the oldest dropped to make room; a command's
// status line stays 240 sixteenths-of-a-second-ish ticks (16 ms each: 3.84 s of
// real time) and is added silently; the rest close up at once when one goes.
#[allow(clippy::too_many_arguments)]
fn messages(
    mut commands: Commands,
    mut incoming: MessageReader<Status>,
    files: Res<GameFiles>,
    mut cues: MessageWriter<crate::audio::Cue>,
    mut sounds: Local<MessageSounds>,
    mut typing: Query<(Entity, &mut Typing)>,
    mut bar: ResMut<Bar>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time<Real>>,
    view: Option<Res<UnitView>>,
    root: Query<Entity, With<MessageRoot>>,
    mut nodes: Query<&mut Node, Without<MessageRoot>>,
) {
    let now = time.elapsed_secs_f64();
    let Ok(root) = root.single() else {
        return;
    };
    if !sounds.read {
        *sounds = MessageSounds { incoming: crate::audio::rules_sound(&files, "IncomingMessage"), typed: crate::audio::rules_sound(&files, "MessageCharTyped"), read: true };
    }
    for (entity, mut typed) in &mut typing {
        let count = (((now - typed.began) / 0.016) as usize + 1).min(typed.widths.len());
        if count != typed.shown {
            typed.shown = count;
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.width = Val::Px(typed.widths[count - 1]);
            }
            if let Some(sound) = sounds.typed.clone() {
                cues.write(crate::audio::Cue::Sound(sound));
            }
        }
    }
    let mut changed = false;
    for message in incoming.read() {
        let player = view.as_ref().map_or(crate::hud::TEXT, |view| view.player_colour());
        let (text, colour, lasts) = match message {
            Status::Key(key) => (bar.strings.as_ref().and_then(|strings| strings.get(key)).unwrap_or(key).to_owned(), player, SHORT_MESSAGE),
            Status::Text(text) => (text.clone(), player, SHORT_MESSAGE),
            Status::Long(text, colour) => (text.clone(), *colour, LONG_MESSAGE),
        };
        let typed = matches!(message, Status::Long(..));
        let Some(font) = bar.font.as_ref() else {
            continue;
        };
        let (width, mask) = font.rasterize(&text);
        let widths: Vec<f32> = if typed { text.char_indices().map(|(at, c)| font.rasterize(&text[..at + c.len_utf8()]).0 as f32 + 2.0).collect() } else { Vec::new() };
        let height = font.height;
        if width == 0 || mask.len() != width * height {
            continue;
        }
        // Each line on a black box a pixel bigger than the text all round, so it
        // reads over any ground.
        let (box_w, box_h) = (width + 2, height + 2);
        let mut rgba = vec![0u8; box_w * box_h * 4];
        for pixel in rgba.chunks_mut(4) {
            pixel[3] = 255;
        }
        for row in 0..height {
            for col in 0..width {
                if mask[row * width + col] != 0 {
                    let at = ((row + 1) * box_w + col + 1) * 4;
                    rgba[at..at + 3].copy_from_slice(&colour);
                }
            }
        }
        let (width, height) = (box_w, box_h);
        let image = images.add(rgba_image(width as u32, height as u32, rgba));
        if bar.lines.len() >= 6 {
            let (oldest, _) = bar.lines.remove(0);
            commands.entity(oldest).despawn();
        }
        // Each line goes under the ones already showing.
        let size = Vec2::new(width as f32, height as f32);
        let line = if typed {
            if let Some(sound) = sounds.incoming.clone() {
                cues.write(crate::audio::Cue::Sound(sound));
            }
            let mut node = absolute(0.0, 19.0 * bar.lines.len() as f32, Vec2::new(widths.first().copied().unwrap_or(size.x), size.y));
            node.overflow = Overflow::clip();
            let line = commands.spawn((node, Typing { began: now, widths, shown: 1 }, ChildOf(root))).id();
            commands.spawn((absolute(0.0, 0.0, size), ImageNode::new(image), ChildOf(line)));
            line
        } else {
            commands.spawn((absolute(0.0, 19.0 * bar.lines.len() as f32, size), ImageNode::new(image), ChildOf(root))).id()
        };
        bar.lines.push((line, now + lasts));
        changed = true;
    }
    let before = bar.lines.len();
    bar.lines.retain(|(line, until)| {
        let keep = now < *until;
        if !keep {
            commands.entity(*line).despawn();
        }
        keep
    });
    if changed || bar.lines.len() != before {
        for (row, (line, _)) in bar.lines.iter().enumerate() {
            if let Ok(mut node) = nodes.get_mut(*line) {
                node.top = Val::Px(19.0 * row as f32);
            }
        }
    }
}
