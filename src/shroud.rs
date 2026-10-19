//! The shroud: the black over ground the player has not seen.
//!
//! Follows the original game. The simulation keeps which
//! cells have been seen; here the black is drawn, with the soft edge the engine's
//! own art gives it.
//!

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use ra2_formats::shp::Shp;
use ra2_sim::LEPTONS_PER_CELL;

use crate::files::GameFiles;
use crate::shell::Screen;
use crate::units::{scene_point, Sim, UnitView};

/// Where the table from the eight neighbours to a
/// picture sits in the executable, and its length.
const TABLE_AT: usize = 0x7a_c3bc;
const IMAGE_BASE: usize = 0x40_0000;

/// Above everything on the map, below what is drawn for the mouse.
const DEPTH: f32 = 850.0;

#[derive(Resource)]
struct ShroudArt {
    /// The picture for each frame of SHROUD.SHP, as black of varying strength.
    frames: Vec<Option<Handle<Image>>>,
    table: Vec<i8>,
    shown: HashMap<(i32, i32), (Entity, usize)>,
    drawn_for: Option<(u64, IVec2)>,
}

pub struct ShroudPlugin;

impl Plugin for ShroudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Screen::InGame), setup).add_systems(
            Update,
            show.run_if(in_state(Screen::InGame)).run_if(resource_exists::<ShroudArt>).run_if(resource_exists::<Sim>).run_if(resource_exists::<UnitView>).in_set(crate::schedule::GameSet::View),
        );
    }
}

fn setup(mut commands: Commands, files: Res<GameFiles>, mut images: ResMut<Assets<Image>>) {
    let Some(shp) = files.get("shroud.shp").and_then(Shp::parse) else {
        warn!("shroud art not found");
        return;
    };
    let exe = files.engine_exe();
    let table: Vec<i8> = exe
        .as_ref()
        .and_then(|exe| exe.get(TABLE_AT - IMAGE_BASE..TABLE_AT - IMAGE_BASE + 256))
        .map(|bytes| bytes.iter().map(|&b| b as i8).collect())
        .unwrap_or_default();
    if table.is_empty() {
        warn!("the shroud's edge table could not be read from game.exe; edges will be hard");
    }
    // The art's bytes are brightness, 127 for full light down to 2 for
    // black; 0xfe is left alone.
    let frames = shp
        .frames
        .iter()
        .map(|frame| {
            if frame.width == 0 || frame.height == 0 {
                return None;
            }
            let mut rgba = vec![0u8; shp.width as usize * shp.height as usize * 4];
            for y in 0..frame.height as usize {
                for x in 0..frame.width as usize {
                    let value = frame.pixels[y * frame.width as usize + x];
                    if value == 0 || value == 0xfe {
                        continue;
                    }
                    let at = ((y + frame.y as usize) * shp.width as usize + x + frame.x as usize) * 4;
                    // The engine scales the picture's stored colour values; the
                    // black here is blended in linear light, so the share that
                    // gets through is put through the display curve first.
                    let through = (value.min(127) as f32 / 127.0).powf(2.2);
                    rgba[at + 3] = (255.0 * (1.0 - through)).round() as u8;
                }
            }
            Some(images.add(Image::new(
                Extent3d { width: shp.width as u32, height: shp.height as u32, depth_or_array_layers: 1 },
                TextureDimension::D2,
                rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            )))
        })
        .collect();
    commands.insert_resource(ShroudArt { frames, table, shown: HashMap::new(), drawn_for: None });
}

/// The picture a cell shows: none when it and everything round it has been seen.
// An unseen cell is solid (frame 15). A seen one
// makes a number from which of its eight neighbours are unseen and looks it up:
// -1 is clear, -2 solid, anything else the frame.
fn frame_of(map: &ra2_sim::Map, table: &[i8], x: i32, y: i32) -> Option<usize> {
    const SOLID: usize = 15;
    if !map.revealed(x, y) {
        return Some(SOLID);
    }
    const BITS: [((i32, i32), u8); 8] = [
        ((-1, -1), 0x40),
        ((0, -1), 0x80),
        ((1, -1), 0x01),
        ((-1, 0), 0x20),
        ((1, 0), 0x02),
        ((-1, 1), 0x10),
        ((0, 1), 0x08),
        ((1, 1), 0x04),
    ];
    let mask = BITS.iter().filter(|((dx, dy), _)| !map.revealed(x + dx, y + dy)).fold(0u8, |mask, (_, bit)| mask | bit);
    match table.get(mask as usize).copied().unwrap_or(if mask == 0 { -1 } else { -2 }) {
        -1 => None,
        -2 => Some(SOLID),
        frame => Some(frame as usize),
    }
}

fn show(
    mut commands: Commands,
    sim: Res<Sim>,
    view: Res<UnitView>,
    centre: Res<crate::game::ViewCentre>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut art: ResMut<ShroudArt>,
    mut sprites: Query<&mut Sprite>,
) {
    let map = &sim.0.map;
    if !map.shroud_on {
        return;
    }
    // Redrawn when the shroud changes or the view has moved a cell's worth.
    let stamp = (map.shroud_version, (centre.0 / 30.0).round().as_ivec2());
    if art.drawn_for == Some(stamp) {
        return;
    }
    art.drawn_for = Some(stamp);
    // The cells whose picture can show in the window, with a margin.
    let half = Vec2::new(window.width(), window.height()) / 2.0 + Vec2::splat(90.0);
    let relative = centre.0 - view.origin;
    let (width, height) = map.size();
    // Along the screen: x - y steps 30 pixels across, x + y steps 15 down.
    let across = (((relative.x - half.x - 30.0) / 30.0).floor() as i32, ((relative.x + half.x - 30.0) / 30.0).ceil() as i32);
    let down = (((relative.y - half.y) / 15.0).floor() as i32, ((relative.y + half.y) / 15.0).ceil() as i32);
    let mut wanted: HashMap<(i32, i32), usize> = HashMap::new();
    for sum in down.0..=down.1 {
        for difference in across.0..=across.1 {
            if (sum + difference) % 2 != 0 {
                continue;
            }
            let (x, y) = ((sum + difference) / 2, (sum - difference) / 2);
            if x < -1 || y < -1 || x > width || y > height {
                continue;
            }
            if let Some(frame) = frame_of(map, &art.table, x, y) {
                wanted.insert((x, y), frame);
            }
        }
    }
    let art = &mut *art;
    art.shown.retain(|cell, (entity, _)| {
        let keep = wanted.contains_key(cell);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
    for (cell, frame) in wanted {
        let Some(image) = art.frames.get(frame).cloned().flatten() else {
            continue;
        };
        match art.shown.get_mut(&cell) {
            Some((_, shown)) if *shown == frame => {}
            Some((entity, shown)) => {
                if let Ok(mut sprite) = sprites.get_mut(*entity) {
                    sprite.image = image;
                    *shown = frame;
                }
            }
            None => {
                // Placed by the cell at height 0,
                // whatever the ground's height.
                let top = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL, cell.1 * LEPTONS_PER_CELL, 0);
                let entity = commands
                    .spawn((
                        DespawnOnExit(Screen::InGame),
                        Sprite::from_image(image),
                        Transform::from_xyz(top.x, -(top.y + 15.0), DEPTH),
                    ))
                    .id();
                art.shown.insert(cell, (entity, frame));
            }
        }
    }
}
