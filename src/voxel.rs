//! Draws voxel models (vehicles) into sprites on the CPU.

use std::collections::HashMap;

use ra2_formats::pal::Palette;
use ra2_formats::vxl::{Hva, Vpl, Vxl};

/// Unit vectors a voxel's normal index refers to, per normals type.
pub struct NormalTables(HashMap<u8, Vec<[f32; 3]>>);

impl NormalTables {
    /// Parses the table file that ships with the map editor in the game directory:
    /// a table count, then for each table its type, its entry count and that many
    /// vectors of three floats.
    // DATA: FinalAlert2/voxel_normal_tables.bin from the game directory.
    // All four tables are byte-for-byte identical to
    // the ones inside the executable.
    pub fn parse(data: &[u8]) -> Self {
        let mut tables = HashMap::new();
        let mut at = 1;
        for _ in 0..data.first().copied().unwrap_or(0) {
            let (Some(&kind), Some(&count)) = (data.get(at), data.get(at + 1)) else {
                break;
            };
            at += 2;
            let Some(raw) = data.get(at..at + count as usize * 12) else {
                break;
            };
            at += raw.len();
            let vectors = raw
                .chunks_exact(12)
                .map(|v| std::array::from_fn(|i| f32::from_le_bytes(v[i * 4..][..4].try_into().unwrap())))
                .collect();
            tables.insert(kind, vectors);
        }
        Self(tables)
    }

    pub fn empty() -> Self {
        Self(HashMap::new())
    }
}

pub struct VoxelSprite {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The pixel within the sprite where the model's origin falls.
    pub origin: (i32, i32),
}

/// Rotates a model-space vector into view space: x to the right of the screen, y up
/// the screen, z towards the viewer.
// The unit's own transform is a turn about z of `-(direction - 8) * pi / 16`,
// where direction is its facing rounded to 32 steps. The view matrix, applied
// after it, is a turn of -45 degrees about z followed by -60 degrees about x.
pub fn to_view(v: [f32; 3], facing: u8) -> [f32; 3] {
    to_view_on(v, facing, None)
}

/// The tilt of something standing on a slope: the azimuth of the axis it is
/// tipped about and the angle, by the cell's ramp number.
// Each ramp's matrix is a turn
// about z by the azimuth, a turn about x by the angle, and the turn about z
// undone. The two angles come out of the engine's arctangent table, which is
// why they are not the exact slopes. Ramps 17 to 20 are drawn level.
fn slope_tilt(ramp: u8) -> Option<(f32, f32)> {
    use std::f32::consts::PI;
    const SINGLE: f32 = 0.372_388_54;
    const DOUBLE: f32 = 0.511_634_77;
    let quarter = |n: u8| [5.0, 3.0, 1.0, 7.0][n as usize % 4] * PI / 4.0;
    match ramp {
        1..=4 => Some(([3.0, 2.0, 1.0, 0.0][ramp as usize - 1] * PI / 2.0, DOUBLE)),
        5..=12 => Some((quarter(ramp - 5), SINGLE)),
        13..=16 => Some((quarter(ramp - 13), DOUBLE)),
        _ => None,
    }
}

/// The turn a slope gives what stands on it, as a quaternion (w, x, y, z):
/// by the ramp's angle about the level axis at its azimuth.
fn slope_turn(ramp: u8) -> [f32; 4] {
    match slope_tilt(ramp) {
        Some((azimuth, angle)) => {
            let (sin, cos) = (angle / 2.0).sin_cos();
            [cos, sin * azimuth.cos(), sin * azimuth.sin(), 0.0]
        }
        None => [1.0, 0.0, 0.0, 0.0],
    }
}

/// The tilt of a model on a ramp, or part of the way from one ramp's tilt to
/// another's.
// When
// the ramp under a vehicle's middle changes, the old and new tilts are blended
// along the shortest turn between them over three frames: none of the new on
// the frame of the change, then a third, two thirds, and all of it.
pub fn slope_matrix(ramp: u8, from: Option<(u8, u8)>) -> Option<[[f32; 3]; 3]> {
    let to = slope_turn(ramp);
    let q = match from {
        Some((old, thirds)) if thirds < 3 => {
            let a = slope_turn(old);
            let t = thirds as f32 / 3.0;
            let mut dot = a[0] * to[0] + a[1] * to[1] + a[2] * to[2] + a[3] * to[3];
            let to = if dot < 0.0 { to.map(|c| -c) } else { to };
            dot = dot.abs().min(1.0);
            let theta = dot.acos();
            let (wa, wb) = if theta < 1e-4 { (1.0 - t, t) } else { (((1.0 - t) * theta).sin() / theta.sin(), (t * theta).sin() / theta.sin()) };
            std::array::from_fn(|i| a[i] * wa + to[i] * wb)
        }
        _ => to,
    };
    if q[0] >= 0.999_999 {
        return None;
    }
    let [w, x, y, z] = q;
    Some([
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ])
}

/// The turn of a model rolled and pitched about the axes of a body facing a
/// compass direction, as a turn of the whole scene.
pub fn rock_matrix(roll: f32, pitch: f32, facing: u8) -> [[f32; 3]; 3] {
    let direction = (((facing as i32 >> 2) + 1) >> 1) & 31;
    let turn = -(direction - 8) as f32 * std::f32::consts::PI / 16.0;
    let z = |a: f32| {
        let (sin, cos) = a.sin_cos();
        [[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]]
    };
    let (rs, rc) = roll.sin_cos();
    let (ps, pc) = pitch.sin_cos();
    let x = [[1.0, 0.0, 0.0], [0.0, rc, -rs], [0.0, rs, rc]];
    let y = [[pc, 0.0, ps], [0.0, 1.0, 0.0], [-ps, 0.0, pc]];
    let mul = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] { std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum())) };
    mul(mul(mul(z(turn), x), y), z(-turn))
}

/// The same for a model tilted by a slope.
// The model is turned to its
// facing and the slope's matrix applied to that, before the view's.
pub fn to_view_on(v: [f32; 3], facing: u8, tilt: Option<&[[f32; 3]; 3]>) -> [f32; 3] {
    world_to_view(to_world(v, facing, tilt))
}

fn rotate_z(v: [f32; 3], turn: f32) -> [f32; 3] {
    let (sin, cos) = turn.sin_cos();
    [v[0] * cos - v[1] * sin, v[0] * sin + v[1] * cos, v[2]]
}

/// A model-space vector turned to a facing and tipped by a slope.
fn to_world(v: [f32; 3], facing: u8, tilt: Option<&[[f32; 3]; 3]>) -> [f32; 3] {
    let direction = (((facing as i32 >> 2) + 1) >> 1) & 31;
    let v = rotate_z(v, -(direction - 8) as f32 * std::f32::consts::PI / 16.0);
    match tilt {
        Some(m) => std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]),
        None => v,
    }
}

fn world_to_view(v: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = rotate_z(v, -std::f32::consts::FRAC_PI_4);
    // Tilt: cos(-60 degrees) = 0.5, sin(-60 degrees) = -0.866.
    [x, 0.5 * y + 0.866_025_4 * z, -0.866_025_4 * y + 0.5 * z]
}

/// The direction of the light in view space.
// The vector (-0.7071, -0.7071, 0) turned 45 degrees
// about the y axis.
const LIGHT: [f32; 3] = [-0.5, -0.707_106_8, 0.5];
/// Sharpness of the highlight.
// Passed as 3.0.
const SHININESS: f32 = 3.0;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Which section of the shading table a surface uses, from its normal and the
/// light direction, both in the limb's own space.
// The light is rotated into the limb's space
// by the inverse of the limb's full transform. The "view" vector used for the
// highlight is (0, 0, 1) rotated by the inverse of a second matrix, which
// is the identity, so it is the limb's own up axis. Diffuse is the normal dotted
// with the light, floored at zero; the highlight is `s / (k - s * k + s)` with `s`
// the normal dotted with the normalised sum of light and up, floored at zero; the
// section is sixteen times their sum, truncated.
fn shade_section(normal: [f32; 3], light: [f32; 3]) -> usize {
    let half = [light[0], light[1], light[2] + 1.0];
    let length = dot(half, half).sqrt().max(f32::EPSILON);
    let half = half.map(|c| c / length);
    let diffuse = dot(normal, light).max(0.0);
    let s = dot(normal, half);
    let highlight = (s / (SHININESS - s * SHININESS + s)).max(0.0);
    (16.0 * (diffuse + highlight)) as usize
}

/// How a vehicle is rocked by a blast: angles about its length (roll) and
/// its width (pitch), in radians, and the half-extents of its body that keep
/// the rocked hull clear of the ground.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rock {
    pub roll: f32,
    pub pitch: f32,
    pub half_width: f32,
    pub half_length: f32,
}

impl Rock {
    /// The model turned about its own axes, then moved back so that the edge
    /// it rocks onto stays where it was.
    // The roll turns about x,
    // the pitch about y; the move along x is the length less the length times
    // the pitch's cosine, against the pitch's sign, and along y the same for
    // the width and the roll, with the roll's sign; each truncated.
    fn turn(&self, v: [f32; 3]) -> [f32; 3] {
        let (sy, cy) = self.pitch.sin_cos();
        let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
        let (sx, cx) = self.roll.sin_cos();
        [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx]
    }

    fn shift(&self) -> [f32; 3] {
        let (a, b) = (self.half_width, self.half_length);
        let dx = (b - (b * self.pitch.cos()).trunc()).trunc();
        let dy = (a - (a * self.roll.cos()).trunc()).trunc();
        [if self.pitch < 0.0 { -dx } else { dx }, if self.roll > 0.0 { -dy } else { dy }, 0.0]
    }

    /// How far it is lifted, after the slope.
    fn lift(&self) -> f32 {
        (self.roll.sin().abs() * self.half_width + self.pitch.sin().abs() * self.half_length).trunc()
    }
}

/// Everything that places a model besides its facing: the nose raised by a
/// pitch (a rocket), the slope it stands on, and its rocking.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pose {
    pub pitch: f32,
    /// Rolled this many radians about its length, before the pitch.
    pub roll: f32,
    pub tilt: Option<[[f32; 3]; 3]>,
    pub rock: Option<Rock>,
}

impl Pose {
    fn rolled(&self, v: [f32; 3]) -> [f32; 3] {
        let (rs, rc) = self.roll.sin_cos();
        [v[0], v[1] * rc - v[2] * rs, v[1] * rs + v[2] * rc]
    }

    /// A model-space direction in view space.
    fn turn(&self, v: [f32; 3], facing: u8) -> [f32; 3] {
        let v = self.rolled(v);
        let (ps, pc) = self.pitch.sin_cos();
        let mut v = [v[0] * pc - v[2] * ps, v[1], v[0] * ps + v[2] * pc];
        if let Some(rock) = &self.rock {
            v = rock.turn(v);
        }
        to_view_on(v, facing, self.tilt.as_ref())
    }

    /// A model-space point in view space.
    pub fn place(&self, v: [f32; 3], facing: u8) -> [f32; 3] {
        let v = self.rolled(v);
        let (ps, pc) = self.pitch.sin_cos();
        let mut v = [v[0] * pc - v[2] * ps, v[1], v[0] * ps + v[2] * pc];
        let mut lift = 0.0;
        if let Some(rock) = &self.rock {
            let shift = rock.shift();
            v = rock.turn(v);
            v = std::array::from_fn(|i| v[i] + shift[i]);
            lift = rock.lift();
        }
        let mut world = to_world(v, facing, self.tilt.as_ref());
        world[2] += lift;
        world_to_view(world)
    }
}

/// Draws a model facing a compass direction (0 to 255, clockwise from north).
/// `light` is the brightness multiplier per channel in thousandths.
#[allow(clippy::too_many_arguments)]
pub fn render(
    vxl: &Vxl,
    hva: Option<&Hva>,
    vpl: Option<&Vpl>,
    normals: &NormalTables,
    palette: &Palette,
    facing: u8,
    light: [u32; 3],
    frame: usize,
    pose: Pose,
) -> Option<VoxelSprite> {
    // Every voxel as (screen x, screen y, nearness, colour).
    let mut points: Vec<(f32, f32, f32, [u8; 3])> = Vec::new();
    for (index, limb) in vxl.limbs.iter().enumerate() {
        let extent: [f32; 3] =
            std::array::from_fn(|i| (limb.max[i] - limb.min[i]) / limb.size[i].max(1) as f32);
        let matrix = hva.and_then(|hva| hva.frames.get(frame % hva.frames.len().max(1))?.get(index)).copied();
        let limb_to_model = |v: [f32; 3]| -> [f32; 3] {
            match matrix {
                Some(m) => std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]),
                None => v,
            }
        };
        // The light in the limb's space: its components along the limb's axes.
        let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let limb_light = axes.map(|axis| dot(pose.turn(limb_to_model(axis), facing), LIGHT));

        let table = normals.0.get(&limb.normals_type);
        for voxel in &limb.voxels {
            let cell = [voxel.x as f32, voxel.y as f32, voxel.z as f32];
            let local: [f32; 3] = std::array::from_fn(|i| limb.min[i] + (cell[i] + 0.5) * extent[i]);
            let mut position = limb_to_model(local);
            if let Some(m) = matrix {
                for i in 0..3 {
                    position[i] += m[i][3] * limb.scale * extent[i];
                }
            }
            let normal = table
                .and_then(|table| table.get(voxel.normal as usize))
                .copied()
                .unwrap_or([0.0, 0.0, 1.0]);
            let color = match vpl {
                Some(vpl) => {
                    let section = shade_section(normal, limb_light).min(vpl.sections.len() - 1);
                    vpl.sections[section][voxel.color as usize]
                }
                None => voxel.color,
            };
            let rgb = palette.0[color as usize];
            let rgb = std::array::from_fn(|i| (rgb[i] as u32 * light[i] / 1000).min(255) as u8);

            let view = pose.place(position, facing);
            points.push((view[0], -view[1], view[2], rgb));
        }
    }
    if points.is_empty() {
        return None;
    }

    let min_x = points.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor() as i32 - 1;
    let min_y = points.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32 - 1;
    let max_x = points.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32 + 2;
    let max_y = points.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32 + 2;
    let (width, height) = ((max_x - min_x) as usize, (max_y - min_y) as usize);
    let mut rgba = vec![0u8; width * height * 4];
    let mut depth = vec![f32::MIN; width * height];
    // Each voxel is written to two horizontally adjacent pixels, its colour looked
    // up by (shade, colour) straight from the VPL sections.
    for (x, y, d, rgb) in points {
        let (px, py) = ((x.floor() as i32 - min_x) as usize, (y.floor() as i32 - min_y) as usize);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let at = (py + dy) * width + px + dx;
            if at < depth.len() && d > depth[at] {
                depth[at] = d;
                rgba[at * 4..at * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }
    Some(VoxelSprite {
        width: width as u32,
        height: height as u32,
        rgba,
        origin: (-min_x, -min_y),
    })
}

/// A model's shadow on the ground: the footprint of one of its sections seen
/// from above, turned to a compass direction and laid on the slope under it, as
/// translucent black.
// The section is turned by its facing and the slope, flattened to no height, and drawn
// at half size for a `ConsideredAircraft` type. Its place in the model is that
// of the animation's first frame. A section the model does not have leaves the
// whole model to cast it.
pub fn shadow(vxl: &Vxl, hva: Option<&Hva>, facing: u8, half: bool, section: usize, tilt: Option<[[f32; 3]; 3]>) -> Option<VoxelSprite> {
    let scale = if half { 0.5 } else { 1.0 };
    let mut points: Vec<(f32, f32)> = Vec::new();
    let only = (section < vxl.limbs.len()).then_some(section);
    for (index, limb) in vxl.limbs.iter().enumerate().filter(|(index, _)| only.is_none_or(|only| only == *index)) {
        let extent: [f32; 3] = std::array::from_fn(|i| (limb.max[i] - limb.min[i]) / limb.size[i].max(1) as f32);
        let matrix = hva.and_then(|hva| hva.frames.first()?.get(index)).copied();
        for voxel in &limb.voxels {
            let cell = [voxel.x as f32, voxel.y as f32, voxel.z as f32];
            let local: [f32; 3] = std::array::from_fn(|i| limb.min[i] + (cell[i] + 0.5) * extent[i]);
            let position: [f32; 3] = match matrix {
                Some(m) => std::array::from_fn(|i| m[i][0] * local[0] + m[i][1] * local[1] + m[i][2] * local[2] + m[i][3] * limb.scale * extent[i]),
                None => local,
            };
            let mut ground = to_world(position.map(|c: f32| c * scale), facing, tilt.as_ref());
            ground[2] = 0.0;
            let view = world_to_view(ground);
            points.push((view[0], -view[1]));
        }
    }
    if points.is_empty() {
        return None;
    }
    let min_x = points.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor() as i32 - 1;
    let min_y = points.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32 - 1;
    let max_x = points.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32 + 2;
    let max_y = points.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32 + 2;
    let (width, height) = ((max_x - min_x) as usize, (max_y - min_y) as usize);
    let mut rgba = vec![0u8; width * height * 4];
    for (x, y) in points {
        let (px, py) = ((x.floor() as i32 - min_x) as usize, (y.floor() as i32 - min_y) as usize);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let at = (py + dy) * width + px + dx;
            if at * 4 + 3 < rgba.len() {
                rgba[at * 4..at * 4 + 4].copy_from_slice(&[0, 0, 0, 128]);
            }
        }
    }
    Some(VoxelSprite { width: width as u32, height: height as u32, rgba, origin: (-min_x, -min_y) })
}
