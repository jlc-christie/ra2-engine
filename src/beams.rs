//! Lasers, electric bolts and rad beams: the lines drawn between a weapon and
//! what it hits.
//!
//! Follows the original game.

use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin, Material2dPipeline};

use crate::files::GameFiles;
use crate::shell::Screen;
use crate::units::Sim;

/// Over everything on the map, under the shroud.
const DEPTH: f32 = 840.0;

/// One line of a laser: its ends in scene pixels, how far to the side it is drawn,
/// its colour at full strength, and when it began and for how long it shows.
#[derive(Component)]
pub struct LaserLine {
    born: u64,
    duration: u32,
    colour: [u8; 3],
    length: f32,
    mesh: Option<Handle<Mesh>>,
}

/// Draws a mesh's vertex colours added to what is under it.
// A laser's lines are added to the picture, so bright parts spill to white
// where they cross.
#[derive(Asset, TypePath, AsBindGroup, Clone, Default)]
struct Additive {}

impl Material2d for Additive {
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                });
            }
        }
        Ok(())
    }
}

#[derive(Resource)]
struct AdditiveMaterial(Handle<Additive>);

/// An electric bolt: its ends, a number its shape is made from, and the drawn
/// frames it has left.
#[derive(Component)]
pub struct Bolt {
    /// The ends in world leptons (x, y, height), the source first.
    from: (i32, i32, i32),
    to: (i32, i32, i32),
    /// Where the map's corner is on the scene, to project the ends.
    origin: Vec2,
    seed: u32,
    frames: u32,
    /// `IsAlternateColor`.
    alternate: bool,
    lines: Vec<Entity>,
}

/// The colours a bolt's strands are drawn in: two strands, the third, and the
/// two strands of a weapon with `IsAlternateColor`.
#[derive(Resource)]
struct BoltColours([[u8; 3]; 3]);

pub struct BeamsPlugin;

impl Plugin for BeamsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<Additive>::default())
            .add_systems(OnEnter(Screen::InGame), setup)
            .add_systems(Update, (fade_lasers.run_if(resource_exists::<Sim>), flicker_bolts.run_if(resource_exists::<BoltColours>), wave_rad_beams, show_sonic_waves.run_if(resource_exists::<Sim>)).run_if(in_state(Screen::InGame)));
    }
}

fn setup(mut commands: Commands, files: Res<GameFiles>, mut additive: ResMut<Assets<Additive>>) {
    commands.insert_resource(AdditiveMaterial(additive.add(Additive {})));
    // Two strands in entry 10 of PALETTE.PAL (entry
    // 5 for a weapon with `IsAlternateColor`) and one in entry 15.
    let palette = files.get("palette.pal").and_then(ra2_formats::pal::Palette::parse);
    let pick = |index: usize, fallback: [u8; 3]| palette.as_ref().map_or(fallback, |palette| palette.0[index]);
    commands.insert_resource(BoltColours([pick(10, [80, 120, 255]), pick(15, [255, 255, 255]), pick(5, [255, 255, 80])]));
}

fn line(from: Vec2, to: Vec2, side: f32, colour: [u8; 3], alpha: f32) -> (Sprite, Transform) {
    let delta = to - from;
    let length = delta.length().max(1.0);
    let across = Vec2::new(-delta.y, delta.x) / length * side;
    let middle = (from + to) / 2.0 + across;
    (
        Sprite { color: Color::srgba_u8(colour[0], colour[1], colour[2], (alpha * 255.0) as u8), custom_size: Some(Vec2::new(length, 1.0)), ..default() },
        Transform::from_xyz(middle.x, -middle.y, DEPTH).with_rotation(Quat::from_rotation_z(-delta.y.atan2(delta.x))),
    )
}

/// Starts a laser between two scene points.
// The middle line in the inner
// colour and a line each side in the outer colour, for `LaserDuration` frames,
// fading evenly to nothing. A house-colour laser is the house's colour inside and
// half of it outside, in two layers (three for a prism tower).
// The lines are added to the picture (see `Additive`). RA2_LASER_FRAMES=n
// keeps them up longer, for looking at them.
pub fn laser(commands: &mut Commands, from: Vec2, to: Vec2, inner: [u8; 3], outer: [u8; 3], house: Option<([u8; 3], u32)>, tick: u64, duration: u32) {
    let mut put = |side: f32, colour: [u8; 3]| {
        let (_, transform) = line(from, to, side, colour, 1.0);
        let length = (to - from).length().max(1.0);
        commands.spawn((DespawnOnExit(Screen::InGame), LaserLine { born: tick, duration: std::env::var("RA2_LASER_FRAMES").ok().and_then(|n| n.parse().ok()).unwrap_or(duration).max(1), colour, length, mesh: None }, transform));
    };
    match house {
        // The layers go on
        // from the inside out and the middle line last; each layer half the
        // one before; with five (a tower helped by others) the middle and the
        // first layer are at twice the house colour. A layer too dim to see
        // (under 8 in every part) ends them.
        Some((colour, layers)) => {
            let double = colour.map(|c| (c as u32 * 2).min(255) as u8);
            let (middle, mut dim) = if layers >= 5 { (double, double.map(|c| c.saturating_mul(1))) } else { (colour, colour.map(|c| c / 2)) };
            if layers >= 5 {
                dim = double;
            }
            for layer in 1..=layers {
                if dim.iter().all(|&c| c < 8) {
                    break;
                }
                put(layer as f32, dim);
                put(-(layer as f32), dim);
                dim = if layers >= 5 && layer == 1 { colour } else { dim.map(|c| c / 2) };
            }
            put(0.0, middle);
        }
        None => {
            put(0.0, inner);
            if outer != [0, 0, 0] {
                put(1.0, outer);
                put(-1.0, outer);
            }
        }
    }
}

/// Starts an electric bolt between two scene points.
// The seed is a random number from 0 to 256.
// RA2_BOLT_FRAMES=n keeps bolts up longer, for looking at them.
pub fn bolt(commands: &mut Commands, from: (i32, i32, i32), to: (i32, i32, i32), origin: Vec2, seed: u32, alternate: bool) {
    commands.spawn((DespawnOnExit(Screen::InGame), Bolt { from, to, origin, seed: seed % 257, frames: std::env::var("RA2_BOLT_FRAMES").ok().and_then(|n| n.parse().ok()).unwrap_or(17), alternate, lines: Vec::new() }));
}

// A stage a game frame; gone at the
// duration; strength (duration - stage) / duration.
fn fade_lasers(
    mut commands: Commands,
    sim: Res<Sim>,
    material: Option<Res<AdditiveMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut lines: Query<(Entity, &mut LaserLine)>,
) {
    let Some(material) = material else {
        return;
    };
    let tick = sim.0.current_tick();
    for (entity, mut laser) in &mut lines {
        let stage = tick.saturating_sub(laser.born) as u32;
        if stage >= laser.duration {
            if let Some(mesh) = laser.mesh.take() {
                meshes.remove(&mesh);
            }
            commands.entity(entity).despawn();
            continue;
        }
        let strength = (laser.duration - stage) as f32 / laser.duration as f32;
        let linear = Color::srgb_u8(laser.colour[0], laser.colour[1], laser.colour[2]).to_linear();
        let colour = [linear.red * strength, linear.green * strength, linear.blue * strength, 1.0];
        match &laser.mesh {
            Some(handle) => {
                if let Some(mut mesh) = meshes.get_mut(handle) {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![colour; 4]);
                }
            }
            None => {
                let (half_x, half_y) = (laser.length / 2.0, 0.5);
                let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-half_x, -half_y, 0.0], [half_x, -half_y, 0.0], [half_x, half_y, 0.0], [-half_x, half_y, 0.0]]);
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![colour; 4]);
                mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
                let handle = meshes.add(mesh);
                commands.entity(entity).insert((Mesh2d(handle.clone()), MeshMaterial2d(material.0.clone())));
                laser.mesh = Some(handle);
            }
        }
    }
}

// Drawn afresh each rendered frame for 17 frames, the seed going up by one each
// time. Three strands from the target end to the source: a piece longer than 64
// leptons is split at its middle (eight deep at most) and each middle pushed
// aside, in world leptons; the first split also sways by a sine of the seed.
// Strand 0 is drawn in colour 15, over the other two.
fn flicker_bolts(mut commands: Commands, colours: Res<BoltColours>, mut bolts: Query<(Entity, &mut Bolt)>) {
    for (entity, mut bolt) in &mut bolts {
        for line in bolt.lines.drain(..) {
            commands.entity(line).despawn();
        }
        if bolt.frames == 0 {
            commands.entity(entity).despawn();
            continue;
        }
        bolt.frames -= 1;
        let seed = bolt.seed;
        bolt.seed = bolt.seed.wrapping_add(1);
        let mut state = (seed.wrapping_add(bolt.frames * 7919)).wrapping_mul(2_654_435_761) | 1;
        // Inclusive.
        let mut ranged = move |low: i32, high: i32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            if high <= low { low } else { low + (state % (high - low + 1) as u32) as i32 }
        };
        type P = [i32; 3];
        let (p1, p2): (P, P) = ([bolt.from.0, bolt.from.1, bolt.from.2], [bolt.to.0, bolt.to.1, bolt.to.2]);
        let length = |a: P, b: P| (((a[0] - b[0]) as f64).powi(2) + ((a[1] - b[1]) as f64).powi(2) + ((a[2] - b[2]) as f64).powi(2)).sqrt() as i32;
        let mut a: [P; 3] = [p2; 3];
        let mut bb: [P; 3] = [p1; 3];
        let mut len = length(p1, p2);
        if len == 0 {
            continue;
        }
        let mut dev = (23 * len) >> 8;
        let big = (102 * len) >> 8;
        let mut first = true;
        let mut stack: Vec<([P; 3], [P; 3], i32, i32)> = Vec::new();
        let mut pieces: Vec<(usize, P, P)> = Vec::new();
        loop {
            if len > 64 && stack.len() < 8 {
                let mut m: [P; 3] = [[0; 3]; 3];
                for k in 0..3 {
                    for c in 0..3 {
                        m[k][c] = (a[k][c] + bb[k][c]) >> 1;
                    }
                }
                if first {
                    first = false;
                    let s: Vec<i32> = (0..6).map(|i| ((seed as f64 * std::f64::consts::PI / (i as f64 + 7.0)).sin() * dev as f64) as i32).collect();
                    for k in 0..3 {
                        m[k][0] += s[0] + s[3];
                        m[k][1] += s[1] + s[5];
                        m[k][2] += (s[2] + s[4] + 2 * dev) >> 1;
                    }
                }
                if len <= 128 {
                    for c in 0..3 {
                        m[0][c] += 2 * dev * ranged(-1, 1);
                    }
                } else {
                    for c in 0..3 {
                        m[0][c] += ranged(-dev, dev);
                    }
                }
                for k in 1..3 {
                    for c in 0..3 {
                        let r = ranged(-dev, dev);
                        if len > big {
                            m[k][c] = m[0][c] + (r >> 1);
                        } else {
                            m[k][c] += r;
                        }
                    }
                }
                len >>= 1;
                dev >>= 1;
                stack.push((m, a, len, dev));
                a = m;
                continue;
            }
            for k in [1, 2, 0] {
                pieces.push((k, bb[k], a[k]));
            }
            let Some((m, second, l, d)) = stack.pop() else {
                break;
            };
            a = second;
            bb = m;
            len = l;
            dev = d;
        }
        let origin = bolt.origin;
        let project = |p: P| crate::units::scene_point_3d(origin, (p[0], p[1], p[2]));
        for (k, from, to) in pieces {
            let colour = colours.0[if k == 0 { 1 } else if bolt.alternate { 2 } else { 0 }];
            let id = commands.spawn((DespawnOnExit(Screen::InGame), line(project(from), project(to), 0.0, colour, 1.0))).id();
            bolt.lines.push(id);
        }
    }
}

/// A rad beam (the Desolator's, the Chrono Legionnaire's): its ends in world
/// leptons, its colour, and the drawn frames it has had.
#[derive(Component)]
pub struct RadBeam {
    from: (i32, i32, i32),
    to: (i32, i32, i32),
    origin: Vec2,
    colour: [u8; 3],
    stage: u32,
    lines: Vec<Entity>,
}

/// Rendered frames a rad beam lasts.
const RAD_BEAM_FRAMES: u32 = 15;
/// How far, in leptons of height, the beam swings at the end of its life.
const RAD_BEAM_AMPLITUDE: f64 = 40.0;
/// The length of each straight piece, in leptons.
// The engine sets it from the frame rate, held to 10..100.
const RAD_BEAM_STEP: f64 = 20.0;

/// Starts a rad beam between two points in world leptons.
pub fn rad_beam(commands: &mut Commands, from: (i32, i32, i32), to: (i32, i32, i32), origin: Vec2, colour: [u8; 3]) {
    commands.spawn((DespawnOnExit(Screen::InGame), RadBeam { from, to, origin, colour, stage: 0, lines: Vec::new() }));
}

// Drawn afresh each rendered frame for 15 frames: straight pieces along the
// line, each point raised by a sine of twice its distance along in degrees
// (at least 360 leptons' worth over the beam), times an amplitude that grows
// from nothing to 40 leptons over the beam's life. Each piece's colour is
// the beam's times one half plus half its share of the full swing.
fn wave_rad_beams(mut commands: Commands, mut beams: Query<(Entity, &mut RadBeam)>) {
    for (entity, mut beam) in &mut beams {
        for line in beam.lines.drain(..) {
            commands.entity(line).despawn();
        }
        if beam.stage >= RAD_BEAM_FRAMES {
            commands.entity(entity).despawn();
            continue;
        }
        let (from, to) = (beam.from, beam.to);
        let delta = [(to.0 - from.0) as f64, (to.1 - from.1) as f64, (to.2 - from.2) as f64];
        let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        let pieces = ((length / RAD_BEAM_STEP) as i32).max(1);
        let amplitude = beam.stage as f64 / RAD_BEAM_FRAMES as f64 * RAD_BEAM_AMPLITUDE;
        let span = length.max(360.0) * 2.0;
        let point = |i: i32| -> ((i32, i32, i32), f64) {
            let swing = ((i as f64 / pieces as f64) * span).to_radians().sin() * amplitude;
            let along = |c: usize, base: i32| base + (delta[c] as i32 / pieces) * i;
            ((along(0, from.0), along(1, from.1), along(2, from.2) + swing as i32), swing)
        };
        let origin = beam.origin;
        let project = |p: (i32, i32, i32)| crate::units::scene_point_3d(origin, p);
        for i in 0..pieces {
            let (a, swing) = point(i);
            let (b, _) = point(i + 1);
            let shade = if amplitude > 0.0 { (swing as i32).abs() as f64 * 0.5 / amplitude + 0.5 } else { 0.5 };
            let colour = beam.colour.map(|c| (c as f64 * shade) as u8);
            let id = commands.spawn((DespawnOnExit(Screen::InGame), line(project(a), project(b), 0.0, colour, 1.0))).id();
            beam.lines.push(id);
        }
        // RA2_RADBEAM_HOLD=n holds beams at their nth frame, for looking at them.
        match std::env::var("RA2_RADBEAM_HOLD").ok().and_then(|n| n.parse::<u32>().ok()) {
            Some(hold) if beam.stage >= hold => {}
            _ => beam.stage += 1,
        }
    }
}

/// A stripe of a sonic wave's ripple.
#[derive(Component)]
struct SonicStripe;

/// The dolphin's sonic wave between the firer and the wave's head.
// The engine bends the picture under the wave: each pixel within the wave's
// band takes a neighbour's colour by a sine of its distance from the middle
// line, its green and blue raised by a level table. Not original behaviour:
// here stripes across the band, brightest where that sine peaks, move along
// the wave in a pale blue-green.
fn show_sonic_waves(mut commands: Commands, sim: Res<Sim>, view: Option<Res<crate::units::UnitView>>, old: Query<Entity, With<SonicStripe>>, mut drawn: Local<u64>) {
    let Some(view) = view else {
        return;
    };
    *drawn += 1;
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let origin = view.origin();
    for wave in sim.0.waves() {
        let Some((from, head)) = sim.0.wave_span(wave) else {
            continue;
        };
        let (a, b) = (crate::units::scene_point_3d(origin, from), crate::units::scene_point_3d(origin, head));
        let along = b - a;
        let length = along.length();
        if length < 1.0 {
            continue;
        }
        let dir = along / length;
        let across = Vec2::new(-dir.y, dir.x);
        let mut at = (*drawn % 8) as f32;
        while at < length {
            let middle = a + dir * at;
            let swell = (at / length * std::f32::consts::PI).sin();
            let half = 6.0 + 10.0 * swell;
            let (sprite, transform) = line(middle - across * half, middle + across * half, 0.0, [120, 220, 255], 0.25 + 0.35 * swell);
            commands.spawn((DespawnOnExit(Screen::InGame), SonicStripe, sprite, transform));
            at += 8.0;
        }
    }
}
