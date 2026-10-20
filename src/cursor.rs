//! The mouse pointer, drawn from the game's own cursor art.
//!
//! The table of cursors (which frames, how fast, where the hot spot is) is engine
//! data, read from the player's own copy of the game; describes
//! it.
//!

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{CursorOptions, PrimaryWindow};
use ra2_formats::pal::Palette;
use ra2_formats::shp::Shp;

use crate::files::GameFiles;

/// Where the table sits, its length and entry size.
const TABLE_AT: usize = 0x7e_2af8;
const TABLE_ENTRIES: usize = 79;
const IMAGE_BASE: usize = 0x40_0000;

// What these entries are used for.
/// Added to a cursor's number to ask for its small form, used over the radar.
pub const SMALL: usize = 1 << 12;
pub const NORMAL: usize = 0;
/// Scrolling north; the next seven go clockwise.
pub const SCROLL: usize = 1;
/// The same eight directions when the view cannot scroll that way.
pub const NO_SCROLL: usize = 9;
pub const SELECT: usize = 17;
pub const MOVE: usize = 18;
pub const NO_MOVE: usize = 19;
/// Attack with the target in range.
pub const ATTACK: usize = 20;
/// Attack out of range; also a miner over ore.
pub const HARVEST: usize = 21;
pub const GUARD_AREA: usize = 22;
pub const ENTER: usize = 25;
pub const NO_ENTER: usize = 26;
pub const DEPLOY: usize = 27;
/// An engineer over a friendly building it could mend.
pub const ENGINEER_REPAIR: usize = 33;
pub const SELL: usize = 30;
pub const NO_SELL: usize = 32;
pub const REPAIR: usize = 34;
pub const NO_REPAIR: usize = 35;
pub const NO_DEPLOY: usize = 28;
pub const SABOTAGE: usize = 52;
pub const IVAN_BOMB: usize = 38;
pub const DISARM: usize = 59;

/// Where the mouse is in the window, if it is over it.
// RA2_CURSOR=x,y pretends the mouse is there, for automated checks.
#[derive(Resource, Default)]
pub struct Pointer(pub Option<Vec2>);

pub(crate) fn track_pointer(mut pointer: ResMut<Pointer>, window: Single<&Window, With<PrimaryWindow>>, fake: Res<crate::debug::FakePointer>) {
    let fixed: Vec<f32> = std::env::var("RA2_CURSOR")
        .unwrap_or_default()
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    pointer.0 = match fixed[..] {
        [x, y] => Some(Vec2::new(x, y)),
        _ => fake.0.or(window.cursor_position()),
    };
}

/// The cursor the game wants shown.
#[derive(Resource, Default)]
pub struct WantedCursor(pub usize);

#[derive(Clone, Copy)]
struct Entry {
    start: usize,
    /// The first frame of the small form, if it has one.
    small: Option<usize>,
    count: usize,
    /// Sixteen-millisecond ticks per frame; 0 for a still cursor.
    rate: u32,
    hot_spot: (i32, i32),
}

#[derive(Resource)]
struct CursorArt {
    shp: Shp,
    palette: Palette,
    entries: Vec<Entry>,
    frames: HashMap<usize, Handle<Image>>,
    node: Entity,
    /// The cursor on show, the frame within it, and when that frame began.
    current: usize,
    offset: usize,
    since: f64,
}

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WantedCursor>()
            .init_resource::<Pointer>()
            .add_systems(PreUpdate, track_pointer.after(bevy::input::InputSystems).after(crate::debug::play_script))
            .add_systems(Startup, setup)
            .add_systems(PostUpdate, show.run_if(resource_exists::<CursorArt>));
    }
}

/// The table stores where the hot spot is as a code:
/// 0 for the left or top, 12345 for the middle, 54321 for the right or bottom (the
/// full width or height, not one less).
fn hot_spot(code: i32, size: i32) -> i32 {
    match code {
        12345 => size / 2,
        54321 => size,
        _ => 0,
    }
}

fn setup(mut commands: Commands, files: Res<GameFiles>) {
    let (Some(shp), Some(palette)) = (
        files.get("mouse.shp").and_then(Shp::parse),
        files.get("mousepal.pal").and_then(Palette::parse),
    ) else {
        warn!("cursor art not found; using the system pointer");
        return;
    };
    let size = (shp.width as i32, shp.height as i32);
    let exe = files.engine_exe();
    let number = |at: usize| -> Option<i32> {
        let exe = exe.as_ref()?;
        Some(i32::from_le_bytes(exe.get(at..at + 4)?.try_into().ok()?))
    };
    let mut entries: Vec<Entry> = (0..TABLE_ENTRIES)
        .map_while(|index| {
            let at = TABLE_AT - IMAGE_BASE + index * 24;
            Some(Entry {
                start: number(at)? as usize,
                small: usize::try_from(number(at + 12)?).ok(),
                count: number(at + 4)?.max(1) as usize,
                rate: number(at + 8)?.max(0) as u32,
                hot_spot: (hot_spot(number(at + 16)?, size.0), hot_spot(number(at + 20)?, size.1)),
            })
        })
        .collect();
    // The table is recognisable by its first entry: frame 0, alone, still.
    let plausible = entries.len() == TABLE_ENTRIES
        && entries[0].start == 0
        && entries[0].count == 1
        && entries.iter().all(|entry| entry.start + entry.count <= shp.frames.len());
    if !plausible {
        warn!("cursor table not found in game.exe; only the plain pointer is available");
        entries = vec![Entry { start: 0, small: None, count: 1, rate: 0, hot_spot: (0, 0) }];
    }
    let node = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(size.0 as f32),
                height: Val::Px(size.1 as f32),
                ..default()
            },
            ImageNode::default(),
            GlobalZIndex(10_000),
            Pickable::IGNORE,
            Visibility::Hidden,
        ))
        .id();
    commands.insert_resource(CursorArt {
        shp,
        palette,
        entries,
        frames: HashMap::new(),
        node,
        current: usize::MAX,
        offset: 0,
        since: 0.0,
    });
}

fn show(
    mut art: ResMut<CursorArt>,
    wanted: Res<WantedCursor>,
    pointer: Res<Pointer>,
    scale: Res<UiScale>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut nodes: Query<(&mut Node, &mut ImageNode, &mut Visibility)>,
    mut entered: MessageReader<bevy::window::CursorEntered>,
    mut focused: MessageReader<bevy::window::WindowFocused>,
    mut hide_again: Local<f64>,
) {
    let art = &mut *art;
    let mut options = window.into_inner();
    let Ok((mut node, mut image, mut visibility)) = nodes.get_mut(art.node) else {
        return;
    };
    let Some(position) = pointer.0 else {
        *visibility = Visibility::Hidden;
        return;
    };
    let now = time.elapsed_secs_f64();
    // Not original behaviour: the system's own pointer is hidden so that only the
    // game's shows. A request to hide it made before the window is ready, or while
    // the mouse is outside it, can be lost (seen on macOS, where both pointers
    // showed in the menus), so it is made again when the mouse comes in, when the
    // window gains focus, and a few times just after starting: showing it for one
    // frame is what makes the next request go through.
    let came_back = entered.read().count() + focused.read().filter(|event| event.focused).count() > 0;
    if options.visible {
        options.visible = false;
    } else if came_back || (now < 3.0 && now >= *hide_again) {
        *hide_again = now + 0.5;
        options.visible = true;
    }

    let number = wanted.0 & !SMALL;
    let kind = if number < art.entries.len() { number } else { NORMAL };
    let entry = art.entries[kind];
    // The radar asks for the table's small start frame.
    let small = Some(wanted.0 & SMALL != 0).filter(|on| *on).and(entry.small).filter(|start| start + entry.count <= art.shp.frames.len());
    let kind = if small.is_some() { kind | SMALL } else { kind };
    // A new cursor starts from its first
    // frame; an animated one moves on a frame each time `rate` ticks of 16 ms have
    // passed, at most one frame per update.
    if kind != art.current {
        art.current = kind;
        art.offset = 0;
        art.since = now;
    } else if entry.rate > 0 && now - art.since >= entry.rate as f64 * 0.016 {
        art.offset = (art.offset + 1) % entry.count;
        art.since = now;
    }
    let frame = small.unwrap_or(entry.start) + art.offset;
    let handle = art.frames.entry(frame).or_insert_with(|| {
        let (w, h) = (art.shp.width as usize, art.shp.height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        if let Some(frame) = art.shp.frames.get(frame) {
            for row in 0..frame.height as usize {
                for col in 0..frame.width as usize {
                    let index = frame.pixels[row * frame.width as usize + col];
                    let at = ((frame.y as usize + row) * w + frame.x as usize + col) * 4;
                    if index != 0 && at + 3 < rgba.len() {
                        let [r, g, b] = art.palette.0[index as usize];
                        rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
                    }
                }
            }
        }
        images.add(Image::new(
            Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ))
    });
    if image.image != *handle {
        image.image = handle.clone();
    }
    // Window positions already include the window's DPI conversion. UI nodes
    // additionally use UiScale, which grows when the menu is maximised. Undo
    // that extra scale here so the pointer also follows the mouse in the blank
    // margins around the centred menu instead of moving beyond the viewport.
    let position = position / scale.0;
    node.left = Val::Px(position.x.floor() - entry.hot_spot.0 as f32);
    node.top = Val::Px(position.y.floor() - entry.hot_spot.1 as f32);
    *visibility = Visibility::Visible;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_follows_mouse_into_blank_margins_after_menu_resize() {
        let mut app = App::new();
        app.add_message::<bevy::window::CursorEntered>()
            .add_message::<bevy::window::WindowFocused>()
            .init_resource::<WantedCursor>()
            .init_resource::<Pointer>()
            .init_resource::<Time<Real>>()
            .init_resource::<Assets<Image>>()
            .insert_resource(UiScale(1.0))
            .add_systems(Update, show);
        let window = app.world_mut().spawn((Window::default(), CursorOptions::default(), PrimaryWindow)).id();
        let node = app.world_mut().spawn((Node::default(), ImageNode::default(), Visibility::Hidden)).id();
        app.insert_resource(CursorArt {
            shp: Shp { width: 1, height: 1, frames: vec![] },
            palette: Palette([[0; 3]; 256]),
            entries: vec![Entry { start: 0, small: None, count: 1, rate: 0, hot_spot: (4, 6) }],
            frames: HashMap::new(),
            node,
            current: usize::MAX,
            offset: 0,
            since: 0.0,
        });

        // Start in a window, maximise into a display with blank menu margins,
        // then restore the original size. Each position is in window pixels.
        for (size, scale, pointer) in [
            ((800, 600), 1.0, Vec2::new(700.0, 500.0)),
            ((1920, 1440), 2.0, Vec2::new(1900.0, 1400.0)),
            ((3072, 1920), 3.0, Vec2::new(3000.0, 1860.0)),
            ((800, 600), 1.0, Vec2::new(700.0, 500.0)),
        ] {
            app.world_mut().get_mut::<Window>(window).unwrap().resolution =
                bevy::window::WindowResolution::new(size.0, size.1).with_scale_factor_override(1.0);
            app.world_mut().resource_mut::<UiScale>().0 = scale;
            app.world_mut().resource_mut::<Pointer>().0 = Some(pointer);
            app.update();
            let shown = app.world().get::<Node>(node).unwrap();
            let (Val::Px(left), Val::Px(top)) = (shown.left, shown.top) else { panic!("cursor must have a position") };
            let hotspot = (Vec2::new(left, top) + Vec2::new(4.0, 6.0)) * scale;
            assert_eq!(hotspot, pointer);
            assert_eq!(*app.world().get::<Visibility>(node).unwrap(), Visibility::Visible);
        }
        app.world_mut().resource_mut::<Pointer>().0 = None;
        app.update();
        assert_eq!(*app.world().get::<Visibility>(node).unwrap(), Visibility::Hidden);
    }
}
