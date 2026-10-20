//! The radar in the sidebar: its opening animation, the map picture with every
//! unit and building on it, the box showing what the main view covers, and clicks.
//!
//! Follows the original game
//!

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ra2_sim::LEPTONS_PER_CELL;

use crate::audio::Cue;
use crate::control::Selection;
use crate::cursor::Pointer;
use crate::files::GameFiles;
use crate::game::{CentreOn, ViewCentre};
use crate::hud::{BOTTOM_BAR_HEIGHT, SIDEBAR_WIDTH};
use crate::shell::Screen;
use crate::terrain::{local, RadarGround};
use crate::units::{Sim, UnitView, cell_at, scene_point};

/// The sidebar node showing RADAR.SHP.
#[derive(Component)]
pub struct RadarArt;

/// The sidebar node showing the map picture.
#[derive(Component)]
pub struct RadarMap;

// The map area inside the sidebar, and RADAR.SHP's
// place in it.
const AREA: (i32, i32, i32, i32) = (16, 49, 140, 108);
const ART_Y: f32 = 48.0;
/// The art steps every four 16 ms ticks, up to frame
/// 32, which is the open backdrop.
const STEP_SECONDS: f64 = 0.064;
const OPEN_FRAME: usize = 32;

/// How the map is laid out in the picture.
// A cell is two raw pixels wide and one
// high at (x − y + x_base, x + y − y_sum0); the raw picture is scaled to fit the
// area, keeping its shape, and centred.
#[derive(Clone, Copy)]
struct Layout {
    x_base: i32,
    y_sum0: i32,
    raw: (i32, i32),
    scale: f32,
    /// The picture's corner in the sidebar, and its size.
    at: (i32, i32),
    size: (i32, i32),
}

impl Layout {
    fn new(sim: &ra2_sim::World) -> Option<Self> {
        let (width, [left, ..]) = sim.map.playfield()?;
        let x_base = width - 2 * left - 1;
        let (w, h) = sim.map.size();
        let cells = || (0..h).flat_map(move |y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| sim.map.in_playfield(x, y));
        let y_sum0 = cells().map(|(x, y)| x + y).min()?;
        let (mut raw_w, mut raw_h) = (0, 0);
        for (x, y) in cells() {
            raw_w = raw_w.max(x - y + x_base + 2);
            raw_h = raw_h.max(x + y - y_sum0 + 1);
        }
        if raw_w < 1 || raw_h < 1 {
            return None;
        }
        let mut scale = AREA.2 as f32 / raw_w as f32;
        let size = if raw_h as f32 * scale < AREA.3 as f32 {
            (AREA.2, (raw_h as f32 * scale) as i32)
        } else {
            scale = AREA.3 as f32 / raw_h as f32;
            ((raw_w as f32 * scale) as i32, AREA.3)
        };
        let size = (size.0.max(1), size.1.max(1));
        let at = (AREA.0 + (AREA.2 - size.0).max(0) / 2, AREA.1 + (AREA.3 - size.1).max(0) / 2);
        Some(Self { x_base, y_sum0, raw: (raw_w, raw_h), scale, at, size })
    }

    /// Where a cell is in the picture.
    fn cell(&self, (x, y): (i32, i32)) -> (i32, i32) {
        (((x - y + self.x_base) as f32 * self.scale) as i32, ((x + y - self.y_sum0) as f32 * self.scale) as i32)
    }

    /// Where a point in leptons is in the picture.
    fn point(&self, x: i32, y: i32) -> (i32, i32) {
        let k = self.scale / 256.0;
        (((self.x_base * 256 - y + x) as f32 * k) as i32, ((y - self.y_sum0 * 256 + x) as f32 * k) as i32)
    }

    /// The cell at a pixel of the picture.
    fn cell_at(&self, (px, py): (i32, i32)) -> (i32, i32) {
        let u = px as f32 / self.scale - self.x_base as f32;
        let v = py as f32 / self.scale + self.y_sum0 as f32;
        (((u + v) * 0.5 + 0.5) as i32, ((v - u) * 0.5 + 0.5) as i32)
    }
}

#[derive(Resource, Default)]
struct Radar {
    /// 0 off, 1 on, 2 closing, 3 opening.
    state: u8,
    frame: usize,
    stepped: f64,
    frames: Vec<Option<(Handle<Image>, Vec2)>>,
    shown_frame: Option<usize>,
    art_side: i32,
    layout: Option<Layout>,
    /// The ground, scaled, and what it was made from.
    ground: Vec<[u8; 3]>,
    ground_version: Option<u64>,
    picture: Option<Handle<Image>>,
    sounds: Option<(Option<String>, Option<String>)>,
    /// A left press on the map that will become an order when let go.
    pressed: bool,
}

pub struct RadarPlugin;

impl Plugin for RadarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RadarEvents>().add_systems(OnEnter(Screen::InGame), |mut commands: Commands, mut events: ResMut<RadarEvents>| {
            commands.insert_resource(Radar::default());
            events.0.clear();
        }).add_systems(
            Update,
            update
                .run_if(in_state(Screen::InGame))
                .run_if(crate::pause::playing)
                .run_if(resource_exists::<Radar>)
                .run_if(resource_exists::<Sim>)
                .run_if(resource_exists::<UnitView>)
                .run_if(resource_exists::<RadarGround>),
        );
    }
}

/// Scales the raw picture of the ground down by averaging.
// An area average; each raw pixel under a picture
// pixel counts by how much of it lies inside.
/// A ping on the radar: its type, cell, and the turning square that closes on it.
pub struct RadarEvent {
    kind: usize,
    cell: (i32, i32),
    /// `None` until the radar has drawn it once (the square starts from the
    /// picture's furthest edge).
    radius: Option<f32>,
    angle: f32,
    rotation: f32,
    animating: bool,
    /// The ticks it stops being drawn and stops holding off others like it.
    shown_until: u64,
    expires: u64,
}

/// The pings alive.
#[derive(Resource, Default)]
pub struct RadarEvents(pub Vec<RadarEvent>);

/// Per type: how near a like event holds a new one off (cells), frames shown,
/// frames alive, and its colour (none: never drawn).
// Base attacked is type 3, an ally's 16.
fn event_type(kind: usize) -> (i32, u64, u64, Option<[u8; 3]>) {
    match kind {
        3 => (8, 200, 600, Some([255, 0, 255])),
        4 => (8, 200, 400, Some([255, 0, 255])),
        12 => (8, 200, 400, Some([255, 255, 0])),
        _ => (8, 0, 400, None),
    }
}

impl RadarEvents {
    /// Starts an event unless one of its type is alive within its distance.
    /// Returns whether it was started.
    pub fn raise(&mut self, kind: usize, cell: (i32, i32), tick: u64) -> bool {
        self.0.retain(|event| event.animating || tick < event.expires);
        let (distance, ..) = event_type(kind);
        let near = |event: &RadarEvent| (((event.cell.0 - cell.0) as f32).hypot((event.cell.1 - cell.1) as f32) as i32) < distance;
        if self.0.iter().any(|event| event.kind == kind && near(event)) {
            return false;
        }
        // DATA: `RadarEventRotationSpeed` (.05).
        self.0.push(RadarEvent { kind, cell, radius: None, angle: std::f32::consts::FRAC_PI_4, rotation: 0.05, animating: true, shown_until: 0, expires: 0 });
        true
    }
}

fn scale_ground(layout: &Layout, raw: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let (rw, rh) = layout.raw;
    let (ow, oh) = layout.size;
    let (sx, sy) = (rw as f32 / ow as f32, rh as f32 / oh as f32);
    let span = |start: f32, step: f32, limit: i32| -> Vec<(usize, f32)> {
        let end = start + step;
        let mut parts = Vec::new();
        let mut i = start.floor() as i32;
        while (i as f32) < end {
            let weight = (end.min(i as f32 + 1.0) - start.max(i as f32)).max(0.0);
            parts.push((i.clamp(0, limit - 1) as usize, weight));
            i += 1;
        }
        parts
    };
    let columns: Vec<Vec<(usize, f32)>> = (0..ow).map(|x| span(x as f32 * sx, sx, rw)).collect();
    let mut out = Vec::with_capacity((ow * oh) as usize);
    for y in 0..oh {
        let rows = span(y as f32 * sy, sy, rh);
        for column in &columns {
            let mut sum = [0.0f32; 3];
            for &(ry, wy) in &rows {
                for &(rx, wx) in column {
                    let pixel = raw[ry * rw as usize + rx];
                    for c in 0..3 {
                        sum[c] += pixel[c] as f32 * wx * wy;
                    }
                }
            }
            out.push(sum.map(|v| (v / (sx * sy) + 0.5).min(255.0) as u8));
        }
    }
    out
}

/// The pixels a building covers, from its own pixel.
// A diamond sized from the foundation.
fn foundation_pixels(scale: f32, (w, h): (i32, i32)) -> Vec<(i32, i32)> {
    let size = |n: i32| if n == 1 { (scale + 0.5).max(1.0) as i32 } else { (n as f32 * scale + 0.5).max(2.0) as i32 };
    let (wpx, hpx) = (size(w), size(h));
    let mut pixels = Vec::new();
    for r in 0..=wpx + hpx - 2 {
        let left = if r < hpx { -r } else { r - 2 * hpx + 2 };
        let right = if r < wpx { r } else { 2 * wpx - r - 2 };
        pixels.extend((left..=right).map(|x| (x, r)));
    }
    pixels
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut radar: ResMut<Radar>,
    mut sim: ResMut<Sim>,
    view: Res<UnitView>,
    ground: Res<RadarGround>,
    files: Res<GameFiles>,
    time: Res<Time<Real>>,
    centre: Res<ViewCentre>,
    pointer: Res<Pointer>,
    buttons: Res<ButtonInput<MouseButton>>,
    picking: (Res<Selection>, Res<crate::control::Mode>, ResMut<crate::control::RadarOver>, ResMut<RadarEvents>, Query<(), Added<RadarArt>>),
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut centre_on: ResMut<CentreOn>,
    mut cues: MessageWriter<Cue>,
    mut art: Single<(&mut ImageNode, &mut Node), (With<RadarArt>, Without<RadarMap>)>,
    mut map: Single<(&mut ImageNode, &mut Node, &mut Visibility), (With<RadarMap>, Without<RadarArt>)>,
) {
    let radar = &mut *radar;
    let (selection, modes, mut radar_over, mut events, fresh) = picking;
    // A sidebar laid out again (the window changed size) starts with blank
    // pictures: draw everything afresh.
    if !fresh.is_empty() {
        radar.shown_frame = None;
    }
    let now = time.elapsed_secs_f64();
    let side = sim.0.houses.get(local() as usize).map_or(0, |house| house.side);
    if radar.art_side != side {
        radar.art_side = side;
        radar.frames.clear();
        radar.shown_frame = None;
    }
    let (shape, palette) = crate::hud::sidebar_radar(&files, side);

    // On and off.
    // Turning on only from off or closing,
    // turning off from anything; each with its sound.
    let sounds = radar.sounds.get_or_insert_with(|| {
        (crate::audio::rules_sound(&files, "RadarOn"), crate::audio::rules_sound(&files, "RadarOff"))
    });
    let available = sim.0.radar_available(local());
    let on = matches!(radar.state, 1 | 3);
    if available && !on {
        radar.state = 3;
        cues.write_batch(sounds.0.clone().map(Cue::Sound));
    } else if !available && on {
        radar.state = 2;
        cues.write_batch(sounds.1.clone().map(Cue::Sound));
    }
    if matches!(radar.state, 2 | 3) && now - radar.stepped >= STEP_SECONDS {
        radar.stepped = now;
        if radar.state == 2 {
            radar.frame = radar.frame.saturating_sub(1);
            if radar.frame == 0 {
                radar.state = 0;
            }
        } else {
            radar.frame += 1;
            if radar.frame >= OPEN_FRAME {
                radar.frame = OPEN_FRAME;
                radar.state = 1;
            }
        }
    }
    if radar.shown_frame != Some(radar.frame) {
        if radar.frames.len() <= radar.frame {
            radar.frames.resize(radar.frame + 1, None);
        }
        let frame = radar.frame;
        let loaded = radar.frames[frame]
            .get_or_insert_with(|| crate::hud::load(&files, &mut images, shape, frame, palette).unwrap_or_default())
            .clone();
        art.0.image = loaded.0;
        art.1.top = Val::Px(crate::hud::CREDITS_HEIGHT + ART_Y - 16.0);
        radar.shown_frame = Some(frame);
    }

    if radar.state != 1 {
        *map.2 = Visibility::Hidden;
        radar.pressed = false;
        return;
    }
    if radar.layout.is_none() {
        radar.layout = Layout::new(&sim.0);
    }
    let Some(layout) = radar.layout else {
        return;
    };
    let (ow, oh) = layout.size;

    // The ground, made again when the ore changes.
    // Cells under the shroud are black, and
    // nothing there that is not the player's own shows.
    let version = sim.0.map.ore_version.wrapping_add(sim.0.map.shroud_version << 32);
    if radar.ground_version != Some(version) {
        radar.ground_version = Some(version);
        let (rw, rh) = layout.raw;
        let mut raw = vec![[0u8; 3]; (rw * rh) as usize];
        for (&(x, y), &(tile, over)) in &ground.cells {
            if !sim.0.map.in_playfield(x, y) || sim.0.map.shrouded(x, y) {
                continue;
            }
            // An overlay the map began with shows while the cell still has one.
            let colour = over.filter(|_| sim.0.map.has_overlay(x, y)).unwrap_or(tile);
            let (px, py) = (x - y + layout.x_base, x + y - layout.y_sum0);
            for px in [px, px + 1] {
                if (0..rw).contains(&px) && (0..rh).contains(&py) {
                    raw[(py * rw + px) as usize] = colour;
                }
            }
        }
        radar.ground = scale_ground(&layout, &raw);
    }

    // The picture: ground, then things, then the view box, inside a frame one
    // pixel wider all round.
    let (pw, ph) = (ow + 2, oh + 2);
    let mut rgba = vec![0u8; (pw * ph * 4) as usize];
    let mut put = |rgba: &mut [u8], x: i32, y: i32, colour: [u8; 3]| {
        if (0..ow).contains(&x) && (0..oh).contains(&y) {
            let at = (((y + 1) * pw + x + 1) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
        }
    };
    for y in 0..oh {
        for x in 0..ow {
            put(&mut rgba, x, y, radar.ground[(y * ow + x) as usize]);
        }
    }
    // Buildings as diamonds and
    // everything else as one pixel, in its house's colour; where two share a
    // pixel the player's own shows.
    let world = &sim.0;
    for own in [false, true] {
        for building in world.buildings().iter().filter(|b| (b.owner == local()) == own).filter(|b| own || !world.map.shrouded(b.cell.0, b.cell.1)) {
            let colour = view.radar_colour_building(building.id, building.owner);
            let (x, y) = layout.point(building.cell.0 * LEPTONS_PER_CELL + 128, building.cell.1 * LEPTONS_PER_CELL + 128);
            for (dx, dy) in foundation_pixels(layout.scale, building.size) {
                put(&mut rgba, x + dx, y + dy, colour);
            }
        }
        for unit in world.units().iter().filter(|u| (u.owner == local()) == own).filter(|u| own || !world.map.shrouded(u.cell().0, u.cell().1)) {
            let (x, y) = layout.point(unit.x, unit.y);
            put(&mut rgba, x, y, view.radar_colour_unit(unit.id, unit.owner));
        }
    }
    // Each ping's square shrinks by
    // `RadarEventSpeed` a pass from the picture's edge to `RadarEventMinRadius`,
    // turning; at the smallest it slows and stops square to the picture, and is
    // shown for its type's frames more.
    // DATA: `RadarEventSpeed` 1.2, `RadarEventMinRadius` 8.
    {
        let tick = world.current_tick();
        for event in &mut events.0 {
            let (_, visible, duration, colour) = event_type(event.kind);
            let (px, py) = layout.point(event.cell.0 * LEPTONS_PER_CELL + 128, event.cell.1 * LEPTONS_PER_CELL + 128);
            let radius = event.radius.get_or_insert_with(|| px.max(py).max(ow - px).max(oh - py) as f32);
            *radius = (*radius - 1.2).max(8.0);
            if event.animating {
                if (*radius - 8.0).abs() < 0.01 {
                    let quarter = std::f32::consts::FRAC_PI_2;
                    let past = (event.angle + quarter / 2.0) % quarter;
                    if past < event.rotation {
                        event.angle += past;
                        event.animating = false;
                        event.shown_until = tick + visible;
                        event.expires = tick + duration;
                    } else {
                        event.angle += event.rotation;
                        event.rotation = (event.rotation - 0.001).max(0.05 / 3.0);
                    }
                } else {
                    event.angle += event.rotation;
                }
            }
            let Some(colour) = colour.filter(|_| event.animating || tick < event.shown_until) else {
                continue;
            };
            let (sin, cos) = event.angle.sin_cos();
            let (x, y) = ((cos * *radius) as i32, (sin * *radius) as i32);
            let corners = [(x, y), (-y, x), (-x, -y), (y, -x)].map(|(dx, dy)| (px + dx, py + dy));
            for i in 0..4 {
                let (from, to) = (corners[i], corners[(i + 1) % 4]);
                let steps = (to.0 - from.0).abs().max((to.1 - from.1).abs()).max(1);
                for step in 0..=steps {
                    put(&mut rgba, from.0 + (to.0 - from.0) * step / steps, from.1 + (to.1 - from.1) * step / steps, colour);
                }
            }
        }
    }
    // The box and the frame, in the sidebar's text
    // colour.
    let (view_w, view_h) = (window.width() - SIDEBAR_WIDTH, window.height() - BOTTOM_BAR_HEIGHT);
    let middle = layout.cell(cell_at(world, view.origin, centre.0));
    let box_w = (2.0 * view_w / (60.0 / layout.scale) + 1.0) as i32;
    let box_h = (2.0 * view_h / (30.0 / layout.scale)) as i32;
    let mut box_x = middle.0 - (view_w / (60.0 / layout.scale)) as i32;
    let mut box_y = middle.1 - (2.0 * view_h / (30.0 / layout.scale) * 0.5) as i32;
    if box_x < 0 {
        box_x = 0;
    } else if box_x + box_w >= ow {
        box_x = ow - box_w - 1;
    }
    if box_y < 0 {
        box_y = 0;
    } else if box_y + box_h >= oh {
        box_y = oh - box_h - 1;
    }
    let line = crate::hud::TEXT;
    let mut outline = |rgba: &mut [u8], x0: i32, y0: i32, w: i32, h: i32, inner: bool| {
        let mut dot = |x: i32, y: i32| {
            if inner {
                put(rgba, x, y, line);
            } else if (0..pw).contains(&x) && (0..ph).contains(&y) {
                let at = ((y * pw + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&[line[0], line[1], line[2], 255]);
            }
        };
        for x in x0..x0 + w {
            dot(x, y0);
            dot(x, y0 + h - 1);
        }
        for y in y0..y0 + h {
            dot(x0, y);
            dot(x0 + w - 1, y);
        }
    };
    outline(&mut rgba, box_x, box_y, box_w, box_h, true);
    outline(&mut rgba, 0, 0, pw, ph, false);

    let image = crate::hud::rgba_image(pw as u32, ph as u32, rgba);
    match &radar.picture {
        Some(handle) => {
            if let Some(mut slot) = images.get_mut(handle) {
                *slot = image;
            }
            if map.0.image != *handle {
                map.0.image = handle.clone();
            }
        }
        None => {
            let handle = images.add(image);
            map.0.image = handle.clone();
            radar.picture = Some(handle);
        }
    }
    map.1.left = Val::Px((layout.at.0 - 1) as f32);
    map.1.top = Val::Px(crate::hud::CREDITS_HEIGHT - 16.0 + (layout.at.1 - 1) as f32);
    map.1.width = Val::Px(pw as f32);
    map.1.height = Val::Px(ph as f32);
    *map.2 = Visibility::Inherited;

    // The mouse.
    // A right press centres the view on the cell; so
    // does a left press, unless the selection has an order to give there, which
    // is then given when the button is let go. Holding and dragging does nothing.
    let over = pointer.0.and_then(|at| {
        let x = (at.x - (window.width() - SIDEBAR_WIDTH)) as i32 - layout.at.0;
        let y = (at.y - (crate::hud::CREDITS_HEIGHT - 16.0)) as i32 - layout.at.1;
        ((0..ow).contains(&x) && (0..oh).contains(&y)).then(|| layout.cell_at((x, y)))
    });
    radar_over.0 = over;
    let Some(cell) = over else {
        return;
    };
    let units: Vec<u32> = selection.units().collect();
    let go = |centre_on: &mut CentreOn| {
        let level = world.map.level(cell.0, cell.1) as i32;
        centre_on.0 = Some(scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128, level));
    };
    // The order itself is given by the map view's own click code, which is
    // told the radar's cell (`control::hover`).
    let target = world.units().iter().find(|unit| unit.cell() == cell && !world.map.shrouded(cell.0, cell.1)).map(|unit| crate::control::Target::Unit(unit.id));
    let acts = crate::control::radar_acts(world, &selection, cell, target, *modes);
    if buttons.just_pressed(MouseButton::Right) || (buttons.just_pressed(MouseButton::Left) && !acts) {
        go(&mut centre_on);
    }
    let _ = units;
}
