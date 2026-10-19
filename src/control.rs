//! Mouse control of the map view, and what is drawn over the map for it: the
//! cursor to show, health bars, the box around a selected building, its text, the
//! placement grid and the lines to where units are going.
//!

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use ra2_formats::csf::Csf;
use ra2_formats::fnt::Font;
use ra2_formats::ini::Ini;
use ra2_formats::pal::Palette;
use ra2_formats::shp::Shp;
use crate::action::{self, Action, Mods};
use ra2_sim::{LEPTONS_PER_CELL, Order, World};

use crate::audio::Cue;
use crate::cursor::{self, Pointer, WantedCursor};
use crate::files::GameFiles;
use crate::game::DEPTH_PER_ROW;
use crate::hud::{BOTTOM_BAR_HEIGHT, SIDEBAR_WIDTH};
use crate::shell::Screen;
use crate::terrain::{LEVEL_HEIGHT, local};
use crate::units::{BuildCatalog, Placing, Sim, UnitView, cell_at, scene_point};

/// Something on the map the mouse can point at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Target {
    Unit(u32),
    Building(u32),
}

/// What the player has selected, in the order it was selected.
// Any number of things.
#[derive(Resource, Default)]
pub struct Selection(pub Vec<Target>);

impl Selection {
    pub(crate) fn units(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().filter_map(|target| match target {
            Target::Unit(id) => Some(*id),
            Target::Building(_) => None,
        })
    }
}

/// The repair and sell modes of the mouse.
// Turning one on drops the selection and
// the other mode, and needs the player to own a building; it stays on after a
// click; a right click turns it off.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    None,
    Repair,
    Sell,
    /// Choosing where a super weapon (by its type) is to strike.
    Super(u16),
}

impl Mode {
    /// Turns a mode on, or off if it is on already.
    pub(crate) fn toggle(&mut self, which: Mode, sim: &World, selection: &mut Selection, placing: &Placing) {
        if *self == which {
            *self = Mode::None;
        } else if placing.0.is_none() && sim.buildings().iter().any(|b| b.owner == local()) {
            *self = which;
            selection.0.clear();
        }
    }
}

/// The drag box: where the left button went down, and the box once the mouse has
/// moved far enough, both in scene pixels.
#[derive(Resource, Default)]
struct Band {
    pressed: Option<Vec2>,
    drawn: Option<(Vec2, Vec2)>,
}

/// What is under the mouse on the map.
#[derive(Resource, Default)]
struct Hover {
    /// The scene point and cell, when the mouse is over the map view.
    point: Option<Vec2>,
    cell: (i32, i32),
    target: Option<Target>,
    /// Pointed at through the radar and not on the map view itself.
    radar: bool,
}

/// The cell under the mouse on the radar's picture, when it is over it.
#[derive(Resource, Default)]
pub(crate) struct RadarOver(pub Option<(i32, i32)>);

/// Whether the radar passes a click on to the selection.
// The actions the radar allows, by number, and the super weapons that can be
// aimed through it (not the chronosphere).
pub(crate) fn radar_acts(world: &World, selection: &Selection, hover_cell: (i32, i32), target: Option<Target>, mode: Mode) -> bool {
    match mode {
        Mode::Super(index) => !matches!(world.super_types.get(index as usize).map(|kind| kind.kind), Some(3 | 4)),
        Mode::None => {
            let point = (hover_cell.0 * LEPTONS_PER_CELL + 128, hover_cell.1 * LEPTONS_PER_CELL + 128);
            let alone = selection.0.len() == 1;
            let a = match (action::best(world, &selection.0, point), target) {
                (Some(best), Some(object)) => action::for_object(world, best, object, alone, selection.0.contains(&object), Mods::default()),
                (Some(best), None) => action::for_cell(world, best, hover_cell, Mods::default()),
                _ => return false,
            };
            matches!(a as u8, 1 | 2 | 3 | 5 | 6 | 9 | 0x10 | 0x1a)
        }
        _ => false,
    }
}

/// Planning mode: the orders each unit has been given to carry out later, and
/// those now being carried out one after another.
// While the mode is on an order is not carried out
// but added to the unit's plan; leaving the mode starts every plan, each unit
// taking its next order as it finishes the last; an ordinary order throws a
// unit's plan away.
#[derive(Resource, Default)]
pub(crate) struct Plans {
    was_on: bool,
    said_key: bool,
    pub(crate) waiting: HashMap<u32, Vec<Order>>,
    running: HashMap<u32, (Vec<Order>, Option<u64>)>,
}

/// When the last order was given, for the lines that show it.
#[derive(Resource, Default)]
struct LastOrder(Option<u64>);

/// The lines show for this many frames after an order.
const ACTION_LINE_FRAMES: u64 = 25;

/// The art and colours the overlays are drawn with.
#[derive(Resource)]
struct OverlayArt {
    pips: Option<Shp>,
    pip_border: Option<Shp>,
    /// PIPS2.SHP: the small boxes for what a unit carries.
    cargo_pips: Option<Shp>,
    /// OREGATH.SHP, shown before a miner taking ore, and the palette of
    /// animations.
    ore_gather: Option<Shp>,
    anim_palette: Option<Palette>,
    /// PARACH.SHP, over a soldier coming down, and a palette of units.
    parachute: Option<Shp>,
    unit_palette: Option<Palette>,
    /// BOMBCURS.SHP, shown on something with a bomb stuck on it.
    bomb: Option<Shp>,
    /// WRENCH.SHP, shown on a building under repair, and its palette.
    wrench: Option<Shp>,
    mouse_palette: Option<Palette>,
    place: Option<Shp>,
    palette: Palette,
    font: Option<Font>,
    strings: Option<Csf>,
    /// `ConditionYellow` and `ConditionRed`, as fractions of full health.
    yellow: f32,
    red: f32,
    /// The player's colour, for text over buildings.
    house_rgb: [u8; 3],
    images: HashMap<OverlayKey, (Handle<Image>, IVec2, IVec2)>,
}

/// Everything an overlay image depends on, so each is drawn once.
#[derive(Clone, PartialEq, Eq, Hash)]
enum OverlayKey {
    Cargo { gems: i32, ore: i32 },
    /// Frames of PIPS2.SHP and where each goes.
    Pips(Vec<(usize, IVec2)>),
    Wrench(usize),
    UnitBar { lit: i32, frame: usize, border: bool },
    Group(usize, bool),
    TargetLine(IVec2),
    BuildingBar { size: (i32, i32), height: i32, lit: i32, frame: usize },
    BoxRear { size: (i32, i32), height: i32 },
    BoxFront { size: (i32, i32), height: i32 },
    Text(String),
    Place(usize),
    Line(IVec2),
}

/// Marks everything drawn over the map for the mouse; all of it is redrawn each
/// frame.
#[derive(Component)]
struct Overlay;

pub struct ControlPlugin;

impl Plugin for ControlPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<GroupOf>()
            .init_resource::<Mode>()
            .init_resource::<Band>()
            .init_resource::<Hover>()
            .init_resource::<RadarOver>()
            .init_resource::<LastOrder>()
            .init_resource::<Plans>()
            .add_systems(OnEnter(Screen::InGame), (setup, forget_last_game))
            .add_systems(OnExit(Screen::InGame), |mut wanted: ResMut<WantedCursor>| wanted.0 = cursor::NORMAL)
            .add_systems(
                Update,
                (select_later, hover, shortcuts, clicks, run_plans, choose_cursor, draw_overlays, draw_sensor_lines)
                    .chain()
                    .run_if(in_state(Screen::InGame))
                    .run_if(crate::pause::playing)
                    .run_if(resource_exists::<UnitView>)
                    .run_if(resource_exists::<OverlayArt>),
            );
    }
}

/// What was selected, grouped, planned or being placed in the last game says
/// nothing about this one, whose units have the same numbers again.
fn forget_last_game(mut selection: ResMut<Selection>, mut groups: ResMut<GroupOf>, mut mode: ResMut<Mode>, mut band: ResMut<Band>, mut last_order: ResMut<LastOrder>, mut plans: ResMut<Plans>) {
    *selection = Selection::default();
    *groups = GroupOf::default();
    *mode = Mode::default();
    *band = Band::default();
    *last_order = LastOrder::default();
    *plans = Plans::default();
}

fn setup(mut commands: Commands, files: Res<GameFiles>) {
    let rules = files.get("rules.ini").map(Ini::parse);
    // DATA: rules.ini `ConditionYellow` and `ConditionRed`, as percentages.
    let percent = |key: &str, default: f32| {
        rules
            .as_ref()
            .and_then(|rules| rules.get("AudioVisual", key).or_else(|| rules.get("General", key)))
            .and_then(|value| value.trim().trim_end_matches('%').parse::<f32>().ok())
            .map_or(default, |value| value / 100.0)
    };
    let house_rgb = rules
        .as_ref()
        .and_then(|rules| rules.get("Colors", "Gold"))
        .and_then(|value| {
            let hsv: Vec<u8> = value.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            Some(crate::colors::remap_shades([*hsv.first()?, *hsv.get(1)?, *hsv.get(2)?])[0])
        })
        .unwrap_or([255, 255, 0]);
    let Some(palette) = files.get("palette.pal").and_then(Palette::parse) else {
        warn!("palette.pal not found; nothing will be drawn over the map");
        return;
    };
    commands.insert_resource(OverlayArt {
        pips: files.get("pips.shp").and_then(Shp::parse),
        pip_border: files.get("pipbrd.shp").and_then(Shp::parse),
        cargo_pips: files.get("pips2.shp").and_then(Shp::parse),
        bomb: files.get("bombcurs.shp").and_then(Shp::parse),
        parachute: files.get("parach.shp").and_then(Shp::parse),
        unit_palette: files.get("unittem.pal").and_then(Palette::parse),
        ore_gather: files.get("oregath.shp").and_then(Shp::parse),
        anim_palette: files.get("anim.pal").and_then(Palette::parse),
        wrench: files.get("wrench.shp").and_then(Shp::parse),
        mouse_palette: files.get("mousepal.pal").and_then(Palette::parse),
        place: files.get("place.shp").and_then(Shp::parse),
        palette,
        font: files.get("game.fnt").and_then(Font::parse),
        strings: files.get("ra2.csf").and_then(Csf::parse),
        yellow: percent("ConditionYellow", 0.5),
        red: percent("ConditionRed", 0.25),
        house_rgb,
        images: HashMap::new(),
    });
    commands.insert_resource(Selection::default());
    // An order given at start-up by RA2_MOVE shows its line too.
    commands.insert_resource(LastOrder(std::env::var_os("RA2_MOVE").map(|_| 0)));
    // RA2_SELECT=u<id> or b<id> starts with a unit or building selected, for
    // automated checks; buildings are numbered from the last unit id on.
    if let Ok(pick) = std::env::var("RA2_SELECT") {
        let id = pick[1..].trim().parse().ok();
        let target = match (pick.as_bytes().first(), id) {
            (Some(b'u'), Some(id)) => Some(Target::Unit(id)),
            (Some(b'b'), Some(id)) => Some(Target::Building(id)),
            _ => None,
        };
        commands.insert_resource(Selection(target.into_iter().collect()));
        if let Some(target) = target {
            commands.insert_resource(SelectLater(target));
        }
    }
}

/// RA2_SELECT's choice, kept until what it names exists (a building put down
/// by RA2_BASE comes some frames in).
#[derive(Resource)]
struct SelectLater(Target);

fn select_later(mut commands: Commands, later: Option<Res<SelectLater>>, sim: Res<Sim>, mut selection: ResMut<Selection>) {
    if let Some(later) = later
        && exists(&sim.0, later.0) {
            selection.0 = vec![later.0];
            commands.remove_resource::<SelectLater>();
        }
}

/// Finds what the mouse is over.
// The nearest unit whose draw point is close enough, counting vertical distance at
// half weight (squared distance under 200: up to 14 pixels sideways or 19 up and
// down); failing that, what stands on the cell, which is how buildings are found.
fn hover(
    pointer: Res<Pointer>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    view: Res<UnitView>,
    sim: Res<Sim>,
    mut hover: ResMut<Hover>,
    radar_over: Res<RadarOver>,
) {
    *hover = Hover::default();
    let Some(at) = pointer.0 else {
        return;
    };
    // The radar finds the cell and what stands on
    // it, and from there on acts as the map view does.
    if let Some(cell) = radar_over.0 {
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        hover.point = Some(scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128, level));
        hover.cell = cell;
        hover.radar = true;
        if !sim.0.map.shrouded(cell.0, cell.1) {
            hover.target = sim.0.units().iter().find(|unit| unit.cell() == cell).map(|unit| Target::Unit(unit.id)).or_else(|| {
                sim.0.buildings().iter().find(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1)).map(|b| Target::Building(b.id))
            });
        }
        return;
    }
    if at.x >= window.width() - SIDEBAR_WIDTH || at.y >= window.height() - BOTTOM_BAR_HEIGHT {
        return;
    }
    let (camera, camera_transform) = *camera;
    let Ok(world_point) = camera.viewport_to_world_2d(camera_transform, at) else {
        return;
    };
    let point = Vec2::new(world_point.x, -world_point.y);
    let cell = cell_at(&sim.0, view.origin, point);
    let unit = sim
        .0
        .units()
        .iter()
        .map(|unit| {
            let offset = unit_point(&sim.0, &view, unit) - point;
            (unit.id, (offset.x * offset.x + 0.5 * offset.y * offset.y).trunc())
        })
        .filter(|(_, distance)| *distance < 200.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| Target::Unit(id));
    let building = || {
        sim.0
            .buildings()
            .iter()
            .find(|b| {
                (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0)
                    && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1)
            })
            .map(|b| Target::Building(b.id))
    };
    hover.point = Some(point);
    hover.cell = cell;
    // Nothing under the shroud can be pointed at.
    let hidden = |target: &Target| match *target {
        // Nor a cloaked unit no sensor finds.
        Target::Unit(id) => sim.0.unit(id).is_some_and(|unit| unit.owner != local() && (sim.0.map.shrouded(unit.cell().0, unit.cell().1) || sim.0.hidden_from(unit, local()))),
        Target::Building(_) => sim.0.map.shrouded(cell.0, cell.1),
    };
    hover.target = unit.filter(|target| !hidden(target)).or_else(|| building().filter(|target| !hidden(target)));
}

fn unit_point(sim: &World, view: &UnitView, unit: &ra2_sim::Unit) -> Vec2 {
    let cell = unit.cell();
    scene_point(view.origin, unit.x, unit.y, sim.map.level(cell.0, cell.1) as i32 + if unit.on_bridge { 4 } else { 0 }) - Vec2::new(0.0, crate::units::lift(sim.altitude(unit))) - crate::units::slope_up(sim, unit)
}

/// The top cell a building's foundation would have with the mouse over a cell.
// Back from the mouse cell by half of one less than the
// foundation's size on each axis, rounding down.
fn placement_origin(sim: &World, cell: (i32, i32), building: u16) -> (i32, i32) {
    let size = sim.building_types.get(building as usize).map_or((1, 1), |kind| kind.size);
    (cell.0 - (size.0 - 1) / 2, cell.1 - (size.1 - 1) / 2)
}

fn owned_by_player(sim: &World, target: Target) -> bool {
    match target {
        Target::Unit(id) => sim.unit(id).is_some_and(|unit| unit.owner == local()),
        Target::Building(id) => sim.buildings().iter().any(|b| b.id == id && b.owner == local()),
    }
}

/// The numbered group each thing is in, for the number shown on it.
#[derive(Resource, Default)]
pub struct GroupOf(pub std::collections::HashMap<Target, usize>);

// A thing is in one group at a time (the number
// is kept on the thing itself).
fn assign_group(groups: &mut [Vec<Target>; 10], group: usize, members: &[Target], shown: &mut GroupOf) {
    for other in groups.iter_mut() {
        other.retain(|target| !members.contains(target));
    }
    groups[group] = members.to_vec();
    publish_groups(groups, shown);
}

fn publish_groups(groups: &[Vec<Target>; 10], shown: &mut GroupOf) {
    shown.0.clear();
    for (group, members) in groups.iter().enumerate() {
        for &member in members {
            shown.0.insert(member, group);
        }
    }
}

/// The player's numbered groups, the view bookmarks, and the last group key.
#[derive(Default)]
struct Shortcuts {
    groups: [Vec<Target>; 10],
    bookmarks: [Option<Vec2>; 4],
    last_group: Option<(usize, u64)>,
    /// When the player's soldiers last cheered.
    cheered_at: Option<u64>,
    /// When the type-select key went down, and whether the last selection
    /// command was a type select that should next take in the whole map.
    type_key_down: Option<f64>,
    type_whole_map: Option<Vec<Target>>,
    /// The game's tick when last looked at, to tell a new game from the old.
    seen_tick: u64,
}

/// The keyboard commands, on the game's default keys.
// DATA: the key for each command, from the game's keyboard.ini.
// What each does, as far as it
// is built: S stop, X scatter, H centre on the base (a construction yard first,
// then any building, then a unit), numpad 5 centre on the selection, N next
// object, Ctrl+number make a group, number select it (twice within 25 frames to
// centre on it), Alt+number select and centre, Shift+number add it, F1 to F4 go to
// a view bookmark and Ctrl+F1 to F4 set one.
fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    view: Res<UnitView>,
    catalog: Res<BuildCatalog>,
    centre: Res<crate::game::ViewCentre>,
    mut centre_on: ResMut<crate::game::CentreOn>,
    mut sim: ResMut<Sim>,
    mut selection: ResMut<Selection>,
    mut state: Local<Shortcuts>,
    mut shown_groups: ResMut<GroupOf>,
    window: Single<&Window, With<PrimaryWindow>>,
    time: Res<Time<Real>>,
    mut bar: MessageReader<crate::bar::Command>,
    mut status: MessageWriter<crate::bar::Status>,
    mut cues: MessageWriter<Cue>,
    files: Res<GameFiles>,
) {
    let world = &sim.0;
    // A new game: the groups and bookmarks of the last one mean nothing in it.
    if world.current_tick() < state.seen_tick {
        *state = Shortcuts::default();
        shown_groups.0.clear();
    }
    state.seen_tick = world.current_tick();
    let asked: Vec<crate::bar::Command> = bar.read().copied().collect();
    // DATA: keyboard.ini `TypeSelect` (T).
    // On
    // letting the key go within half a second, everything of the player's on
    // screen of the types selected is selected; if that was all selected already,
    // or the key is tapped again straight after, everything of those types on
    // the whole map.
    if keys.just_pressed(KeyCode::KeyT) {
        state.type_key_down = Some(time.elapsed_secs_f64());
    }
    let by_button = asked.contains(&crate::bar::Command::TypeSelect);
    if keys.just_released(KeyCode::KeyT) || by_button {
        let quick = by_button || state.type_key_down.take().is_some_and(|down| time.elapsed_secs_f64() - down <= 0.5);
        let kinds: Vec<(bool, u16)> = selection
            .0
            .iter()
            .filter_map(|&target| match target {
                Target::Unit(id) => world.unit(id).filter(|u| u.owner == local()).map(|u| (false, u.stats.kind)),
                Target::Building(id) => world.buildings().iter().find(|b| b.id == id && b.owner == local()).map(|b| (true, b.kind)),
            })
            .collect();
        // The status line it leaves.
        if quick && kinds.is_empty() {
            status.write(crate::bar::Status::Key("MSG:NothingSelected"));
        }
        if quick && !kinds.is_empty() {
            let half = Vec2::new(window.width() - SIDEBAR_WIDTH, window.height() - BOTTOM_BAR_HEIGHT) / 2.0;
            let seen = |point: Vec2| (point.x - centre.0.x).abs() <= half.x && (point.y - centre.0.y).abs() <= half.y;
            let matching = |whole: bool| -> Vec<Target> {
                let units = world
                    .units()
                    .iter()
                    .filter(|u| u.owner == local() && kinds.contains(&(false, u.stats.kind)))
                    .filter(|u| whole || seen(unit_point(world, &view, u)))
                    .map(|u| Target::Unit(u.id));
                let buildings = world
                    .buildings()
                    .iter()
                    .filter(|b| b.owner == local() && kinds.contains(&(true, b.kind)))
                    .filter(|b| {
                        let level = world.map.level(b.cell.0, b.cell.1) as i32;
                        whole || seen(scene_point(view.origin, b.cell.0 * LEPTONS_PER_CELL, b.cell.1 * LEPTONS_PER_CELL, level))
                    })
                    .map(|b| Target::Building(b.id));
                units.chain(buildings).collect()
            };
            let again = state.type_whole_map.as_ref() == Some(&selection.0);
            let on_screen = matching(false);
            let whole = again || on_screen.iter().all(|target| selection.0.contains(target));
            selection.0 = if whole { matching(true) } else { on_screen };
            state.type_whole_map = Some(selection.0.clone());
            status.write(crate::bar::Status::Key(if whole { "MSG:SelAcrossMap" } else { "MSG:SelAcrossScreen" }));
        }
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let point_of = |target: Target| -> Option<Vec2> {
        match target {
            Target::Unit(id) => world.unit(id).map(|unit| unit_point(world, &view, unit)),
            Target::Building(id) => world.buildings().iter().find(|b| b.id == id).map(|b| {
                let half = LEPTONS_PER_CELL / 2;
                let level = world.map.level(b.cell.0, b.cell.1) as i32;
                scene_point(
                    view.origin,
                    b.cell.0 * LEPTONS_PER_CELL + b.size.0 * half,
                    b.cell.1 * LEPTONS_PER_CELL + b.size.1 * half,
                    level,
                )
            }),
        }
    };

    if keys.just_pressed(KeyCode::KeyH) {
        let mine = || world.buildings().iter().filter(|b| b.owner == local());
        let yard = mine().find(|b| catalog.0.get(b.kind as usize).is_some_and(|entry| entry.construction_yard));
        let target = yard
            .or_else(|| mine().next())
            .map(|b| Target::Building(b.id))
            .or_else(|| world.units().iter().find(|u| u.owner == local()).map(|u| Target::Unit(u.id)));
        centre_on.0 = target.and_then(point_of);
    }
    if keys.just_pressed(KeyCode::Numpad5) {
        centre_on.0 = selection.0.first().copied().and_then(point_of);
    }
    if keys.just_pressed(KeyCode::KeyN) {
        let all: Vec<Target> = world
            .units()
            .iter()
            .filter(|u| u.owner == local())
            .map(|u| Target::Unit(u.id))
            .chain(world.buildings().iter().filter(|b| b.owner == local()).map(|b| Target::Building(b.id)))
            .collect();
        let at = selection.0.first().and_then(|now| all.iter().position(|t| t == now));
        if let Some(&next) = all.get(at.map_or(0, |at| (at + 1) % all.len().max(1))) {
            selection.0 = vec![next];
            centre_on.0 = point_of(next);
        }
    }

    // A team button makes the team from the selection
    // when it has no members, centres on it when all of them are selected,
    // and otherwise selects it; a right click disbands it.
    for command in &asked {
        match *command {
            crate::bar::Command::Team(n) => {
                let group = n - 1;
                let members: Vec<Target> = state.groups[group].iter().copied().filter(|target| exists(world, *target)).collect();
                if members.is_empty() {
                    assign_group(&mut state.groups, group, &selection.0, &mut shown_groups);
                } else if members.iter().all(|member| selection.0.contains(member)) {
                    centre_on.0 = members.first().copied().and_then(point_of);
                } else {
                    selection.0 = members;
                    if let Some(Target::Unit(id)) = selection.0.first().copied() {
                        cues.write_batch(view.voices.get(&id).and_then(|voices| voices.0.clone()).map(Cue::Sound));
                    }
                }
            }
            crate::bar::Command::Disband(n) => {
                state.groups[n - 1].clear();
                publish_groups(&state.groups, &mut shown_groups);
            }
            _ => {}
        }
    }

    const NUMBERS: [KeyCode; 10] = [
        KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5,
        KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9, KeyCode::Digit0,
    ];
    for (group, key) in NUMBERS.into_iter().enumerate() {
        if !keys.just_pressed(key) {
            continue;
        }
        if ctrl {
            assign_group(&mut state.groups, group, &selection.0, &mut shown_groups);
            continue;
        }
        let members: Vec<Target> = state.groups[group].iter().copied().filter(|target| exists(world, *target)).collect();
        let Some(&leader) = members.first() else {
            continue;
        };
        let tick = world.current_tick();
        let again = state.last_group.is_some_and(|(last, at)| last == group && tick.checked_sub(at).is_some_and(|since| since < 25));
        if alt || (again && members.iter().all(|member| selection.0.contains(member))) {
            centre_on.0 = point_of(leader);
        }
        // With Shift the group joins the selection.
        if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            for member in members {
                if !selection.0.contains(&member) {
                    selection.0.push(member);
                }
            }
        } else {
            selection.0 = members;
        }
        // The first
        // unit selected answers as for a click.
        if let Some(Target::Unit(id)) = selection.0.first().copied() {
            cues.write_batch(view.voices.get(&id).and_then(|voices| voices.0.clone()).map(Cue::Sound));
        }
        state.last_group = Some((group, tick));
    }
    for (slot, key) in [KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4].into_iter().enumerate() {
        if keys.just_pressed(key) {
            if ctrl {
                state.bookmarks[slot] = Some(centre.0);
            } else if let Some(point) = state.bookmarks[slot] {
                centre_on.0 = Some(point);
            }
        }
    }

    // Every selected unit gets the order.
    // DATA: rules.ini `StopSound` and `ScatterSound` (and `GuardSound`), heard when
    // the selection is given the command. `DeploySound` is empty in the stock
    // rules. Played once if any of the
    // selection took the order.
    for (key, sound) in [(KeyCode::KeyS, "StopSound"), (KeyCode::KeyX, "ScatterSound")] {
        if keys.just_pressed(key) && !selection.0.is_empty() {
            cues.write_batch(crate::audio::rules_sound(&files, sound).map(Cue::Sound));
        }
    }
    if keys.just_pressed(KeyCode::KeyS) {
        for target in selection.0.clone() {
            if let Target::Building(building) = target {
                sim.give(Order::Halt { building });
            }
        }
    }
    // C has every soldier of the player's cheer, at most once
    // every `MaximumCheerRate` frames, with `CheerSound`.
    if keys.just_pressed(KeyCode::KeyC) && !ctrl {
        let rules = files.get("rules.ini").map(Ini::parse);
        let rate = rules.as_ref().and_then(|rules| rules.get("General", "MaximumCheerRate")).and_then(|v| v.split(';').next()?.trim().parse::<u64>().ok()).unwrap_or(300);
        let now = sim.0.current_tick();
        if state.cheered_at.is_none_or(|at| now < at || now >= at + rate) {
            sim.give(Order::Cheer { house: crate::terrain::local() });
            cues.write_batch(crate::audio::rules_sound(&files, "CheerSound").map(Cue::Sound));
            state.cheered_at = Some(now);
        }
    }
    let units: Vec<u32> = selection.units().collect();
    for unit in units {
        if keys.just_pressed(KeyCode::KeyS) {
            sim.give(Order::Stop { unit });
        }
        if keys.just_pressed(KeyCode::KeyX) {
            sim.give(Order::Scatter { unit });
        }
    }
}

/// The mouse on the map.
// A press that moves five pixels or more
// becomes a box; letting go of a box selects the player's units whose draw point is
// inside (adding to the selection with Shift, replacing it otherwise; a box with
// nothing at all in it counts as a plain click). A plain click toggles one of the
// player's things with Shift, selects it alone otherwise, or gives every selected
// unit the order for what was clicked, the first of them answering. A right click
// backs out of placing a building, or else deselects everything.
fn clicks(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    (hover, mut drag): (Res<Hover>, ResMut<crate::game::RightDrag>),
    view: Res<UnitView>,
    mut sim: ResMut<Sim>,
    mut selection: ResMut<Selection>,
    mut band: ResMut<Band>,
    mut mode: ResMut<Mode>,
    mut placing: ResMut<Placing>,
    mut last_order: ResMut<LastOrder>,
    mut cues: MessageWriter<Cue>,
    mut bar: MessageReader<crate::bar::Command>,
    files: Res<GameFiles>,
    mut plans: ResMut<Plans>,
    planning: Res<crate::bar::PlanningMode>,
    mut status: MessageWriter<crate::bar::Status>,
) {
    // Forget things that no longer exist. A building that has packed up hands its
    // place in the selection to the vehicle it became.
    let before = selection.0.len();
    let kept: Vec<Target> = selection
        .0
        .iter()
        .filter_map(|&target| match target {
            // A type with `Selectable=no` (the
            // paradrop plane) is never selected.
            Target::Unit(id) if sim.0.unit(id).is_some_and(|unit| unit.stats.unselectable) => None,
            _ if exists(&sim.0, target) => Some(target),
            Target::Building(id) => sim.0.unpacked_into(id).map(Target::Unit),
            Target::Unit(_) => None,
        })
        .collect();
    if kept != selection.0 {
        if kept.len() == before {
            last_order.0 = Some(sim.0.current_tick());
        }
        selection.0 = kept;
    }
    let deploy = |sim: &mut Sim, unit: u32, cues: &mut MessageWriter<Cue>| {
        if sim.0.unit(unit).is_some_and(|unit| !unit.cargo.is_empty()) {
            // A transport lets its
            // passengers out.
            sim.give(Order::Deploy { unit });
        } else if sim.0.unit(unit).is_some_and(|unit| unit.stats.infantry && unit.stats.fighter.deployer) {
            // A soldier that can dig in
            // does, or gets up again, with the type's `DeploySound` or
            // `UndeploySound`.
            if let Some(state) = sim.0.unit(unit) {
                let up = state.stance != ra2_sim::combat::Stance::Standing;
                cues.write_batch(view.deploy_sound(state.stats.kind, up).map(Cue::Sound));
            }
            sim.give(Order::Deploy { unit });
        } else if sim.0.can_deploy(unit) {
            sim.give(Order::Deploy { unit });
            cues.write_batch(view.slam_sound.clone().map(Cue::Sound));
        } else if sim.0.unit(unit).is_some_and(|unit| unit.stats.deploys_into.is_some()) {
            info!(
                "cannot deploy unit {unit} at {:?}: in the way {:?}",
                sim.0.unit(unit).map(|unit| (unit.cell(), unit.is_moving())),
                sim.0.deploy_obstacles(unit)
            );
            cues.write(Cue::Eva("EVA_CannotDeployHere"));
        }
    };
    // A right click undoes one thing: placing a
    // building, else the repair or sell mode, else the selection.
    // A right press on the radar only
    // moves the view there (`radar.rs`); nothing is cancelled or deselected.
    if buttons.just_pressed(MouseButton::Right) && hover.radar {
        return;
    }
    // On letting go, unless the press became a drag that scrolled the view.
    if buttons.just_released(MouseButton::Right) && drag.press.is_some() {
        let dragged = drag.started;
        *drag = crate::game::RightDrag::default();
        if dragged {
            return;
        }
        *band = Band::default();
        if placing.0.take().is_none() {
            if *mode != Mode::None {
                *mode = Mode::None;
            } else {
                selection.0.clear();
            }
        }
        return;
    }
    // DATA: keyboard.ini `ToggleRepair` (K) and `ToggleSell` (L).
    if keys.just_pressed(KeyCode::KeyK) {
        mode.toggle(Mode::Repair, &sim.0, &mut selection, &placing);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        mode.toggle(Mode::Sell, &sim.0, &mut selection, &placing);
    }
    if *mode != Mode::None {
        // A click on one of the
        // player's buildings turns its repairs on or off, or sells it.
        *band = Band::default();
        // A click on
        // the map sends the super weapon there; the launch ends the choosing.
        if hover.radar && !radar_acts(&sim.0, &selection, hover.cell, hover.target, *mode) {
            return;
        }
        // The launch ends
        // the choosing. The order is carried out on the next tick, so the
        // choosing ends when the weapon is seen to have gone (no longer
        // ready); the chronosphere's second choice (kind 4) ends on its click.
        if let Mode::Super(index) = *mode {
            let kind = sim.0.super_types.get(index as usize).map(|kind| kind.kind);
            let ready = sim.0.houses[local() as usize].supers.get(index as usize).is_some_and(|state| state.ready);
            if kind != Some(4) && !ready {
                *mode = Mode::None;
                return;
            }
        }
        if let Mode::Super(index) = *mode {
            if buttons.just_released(MouseButton::Left) && hover.point.is_some() {
                sim.give(Order::Super { house: local(), index, cell: hover.cell });
                // The chronosphere's first click
                // goes on to the choosing of where to.
                let kind = sim.0.super_types.get(index as usize).map(|kind| kind.kind);
                let second = sim.0.super_types.iter().position(|kind| kind.kind == 4);
                if let (Some(3), Some(second), true) = (kind, second, sim.0.houses[local() as usize].chrono_source.is_some()) {
                    *mode = Mode::Super(second as u16);
                } else if kind == Some(4) || !sim.0.houses[local() as usize].supers.get(index as usize).is_some_and(|state| state.ready) {
                    *mode = Mode::None;
                }
            }
            return;
        }
        if buttons.just_released(MouseButton::Left)
            && let Some(Target::Building(building)) = hover.target.filter(|target| owned_by_player(&sim.0, *target)) {
                match *mode {
                    Mode::Repair => {
                        let was = sim.0.buildings().iter().any(|b| b.id == building && b.repairing);
                        let damaged = sim.0.can_repair(building);
                        sim.give(Order::Repair { building });
                        let sound = if !was && !damaged { &view.scold_sound } else { &view.click_sound };
                        cues.write_batch(sound.clone().map(Cue::Sound));
                        if !was && damaged {
                            cues.write(Cue::Eva("EVA_Repairing"));
                        }
                    }
                    _ => {
                        sim.give(Order::Sell { building });
                        cues.write_batch(view.click_sound.clone().map(Cue::Sound));
                    }
                }
            }
        return;
    }
    let deploy_button = bar.read().any(|command| *command == crate::bar::Command::Deploy);
    // Not while planning.
    if (keys.just_pressed(KeyCode::KeyD) || deploy_button) && planning.0 {
        status.write(crate::bar::Status::Key("MSG:PlanningModeNoDeploy"));
        cues.write_batch(view.scold_sound.clone().map(Cue::Sound));
        return;
    }
    if keys.just_pressed(KeyCode::KeyD) || deploy_button {
        // If the first thing selected is a building
        // of the player's with soldiers in it, they come out and nothing else
        // is done.
        if let Some(Target::Building(building)) = selection.0.first().copied()
            && sim.0.buildings().iter().any(|b| b.id == building && b.owner == local() && !b.occupants.is_empty()) {
                sim.give(Order::Evacuate { building });
                return;
            }
        // Every selected thing that can deploy;
        // but if some of the `Deployer` soldiers are dug in and some are not,
        // those dug in are left as they are, so that a mixed group first all
        // digs in and only then all gets up.
        let units: Vec<u32> = selection.units().collect();
        let dug_in = |sim: &World, unit: u32| {
            sim.unit(unit).filter(|u| u.stats.infantry && u.stats.fighter.deployer).map(|u| matches!(u.stance, ra2_sim::combat::Stance::Deploying(_) | ra2_sim::combat::Stance::Deployed))
        };
        let states: Vec<bool> = units.iter().filter_map(|&unit| dug_in(&sim.0, unit)).collect();
        let mixed = states.contains(&true) && states.contains(&false);
        for unit in units {
            if mixed && dug_in(&sim.0, unit) == Some(true) {
                continue;
            }
            deploy(&mut sim, unit, &mut cues);
        }
        return;
    }

    // The drag box.
    if buttons.just_pressed(MouseButton::Left) && placing.0.is_none() && !hover.radar {
        band.pressed = hover.point;
        band.drawn = None;
    }
    if buttons.pressed(MouseButton::Left)
        && let (Some(from), Some(to)) = (band.pressed, hover.point)
            && (band.drawn.is_some() || (to - from).length().trunc() > 4.0) {
                band.drawn = Some((from, to));
            }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let boxed = band.drawn.take();
    band.pressed = None;
    if hover.point.is_none() && boxed.is_none() {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some((from, to)) = boxed {
        let (low, high) = (from.min(to), from.max(to));
        let inside = |point: Vec2| point.x >= low.x && point.x <= high.x && point.y >= low.y && point.y <= high.y;
        let caught: Vec<(u32, u8)> = sim
            .0
            .units()
            .iter()
            .filter(|unit| inside(unit_point(&sim.0, &view, unit)))
            .map(|unit| (unit.id, unit.owner))
            .collect();
        if !caught.is_empty() {
            if !shift {
                selection.0.clear();
            }
            let mut spoke = false;
            for (id, owner) in caught {
                let target = Target::Unit(id);
                if owner != local() || selection.0.contains(&target) {
                    continue;
                }
                selection.0.push(target);
                if !spoke {
                    spoke = true;
                    let voice = view.voices.get(&id).and_then(|voices| voices.0.clone());
                    cues.write_batch(voice.map(Cue::Sound));
                }
            }
            return;
        }
        // An empty box is a plain click where it ended.
    }
    if hover.point.is_none() {
        return;
    }

    if hover.radar && (placing.0.is_some() || !radar_acts(&sim.0, &selection, hover.cell, hover.target, Mode::None)) {
        return;
    }
    if let Some(building) = placing.0 {
        let (x, y) = placement_origin(&sim.0, hover.cell, building);
        if sim.0.can_place(local(), building, (x, y)) {
            sim.give(Order::Place { house: local(), building, x, y });
            placing.0 = None;
            cues.write_batch(view.slam_sound.clone().map(Cue::Sound));
        }
        return;
    }

    // The click's meaning
    // comes from the best of the selection for what is under the mouse; a select
    // or toggle acts on the selection, anything else has every selected thing
    // work out its own answer and act on it, the first one speaking.
    let mods = Mods {
        ctrl: keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        alt: keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
        shift,
    };
    let point = (hover.cell.0 * LEPTONS_PER_CELL + 128, hover.cell.1 * LEPTONS_PER_CELL + 128);
    let alone = selection.0.len() == 1;
    let answer = |world: &World, me: Target| match hover.target {
        Some(object) => action::for_object(world, me, object, alone, selection.0.contains(&object), mods),
        None => action::for_cell(world, me, hover.cell, mods),
    };
    let lead = match action::best(&sim.0, &selection.0, point) {
        Some(best) => answer(&sim.0, best),
        // Nothing selected: something of the player's is selected by a click.
        None => match hover.target {
            Some(object) if owned_by_player(&sim.0, object) => Action::Select,
            _ => Action::None,
        },
    };
    match (lead, hover.target) {
        (Action::ToggleSelect, Some(object)) => {
            if owned_by_player(&sim.0, object) {
                match selection.0.iter().position(|selected| *selected == object) {
                    Some(at) => {
                        selection.0.remove(at);
                    }
                    None => selection.0.push(object),
                }
            }
            return;
        }
        (Action::Select, Some(object)) => {
            selection.0 = vec![object];
            if let Target::Unit(id) = object {
                let voice = view.voices.get(&id).and_then(|voices| voices.0.clone());
                cues.write_batch(voice.map(Cue::Sound));
            }
            return;
        }
        _ => {}
    }
    // In planning mode an order joins the unit's plan
    // instead of being carried out; an ordinary order throws its plan away.
    fn give(sim: &mut Sim, plans: &mut Plans, planning: bool, unit: u32, order: Order) {
        if planning {
            plans.waiting.entry(unit).or_default().push(order);
        } else {
            plans.waiting.remove(&unit);
            plans.running.remove(&unit);
            sim.give(order);
        }
    }
    let planned_before: usize = plans.waiting.values().map(Vec::len).sum();
    let mut spoke = false;
    for me in selection.0.clone() {
        let own = answer(&sim.0, me);
        let (x, y) = match hover.target {
            Some(Target::Unit(id)) => sim.0.unit(id).map_or(hover.cell, |unit| unit.cell()),
            _ => hover.cell,
        };
        let mut ordered = true;
        match (me, own) {
            // "cannot move there" still
            // sends the unit to the nearest cell it can reach. A vehicle clicked on
            // itself, and a dug-in soldier, ignore it.
            (Target::Unit(unit), Action::Move | Action::NoMove) => {
                let stays = hover.target == Some(me)
                    || sim.0.unit(unit).is_some_and(|u| u.stats.infantry && u.stance != ra2_sim::combat::Stance::Standing);
                if stays {
                    ordered = false;
                } else {
                    give(&mut sim, &mut plans, planning.0, unit, Order::Move { unit, x, y });
                }
            }
            (Target::Unit(unit), Action::Harvest) => give(&mut sim, &mut plans, planning.0, unit, Order::Harvest { unit, x, y }),
            // A bomb and its disarming are attacks.
            (Target::Unit(unit), Action::Attack | Action::SabotageFire | Action::IvanBomb | Action::Disarm) => {
                let target = hover.target.map_or(ra2_sim::combat::Aim::Cell(hover.cell.0, hover.cell.1), aim_of);
                give(&mut sim, &mut plans, planning.0, unit, Order::Attack { unit, target });
            }
            (Target::Building(building), Action::Attack | Action::SabotageFire) => {
                let target = hover.target.map_or(ra2_sim::combat::Aim::Cell(hover.cell.0, hover.cell.1), aim_of);
                sim.give(Order::Aim { building, target });
            }
            (Target::Unit(unit), Action::Enter) => match hover.target {
                Some(Target::Unit(transport)) => give(&mut sim, &mut plans, planning.0, unit, Order::Board { unit, transport }),
                Some(Target::Building(building)) if sim.0.unit(unit).is_some_and(|u| u.plane.is_some()) => give(&mut sim, &mut plans, planning.0, unit, Order::Dock { unit, building }),
                Some(Target::Building(building)) if sim.0.entry_action(unit, building) == Some(ra2_sim::combat::Entry::Treat) => give(&mut sim, &mut plans, planning.0, unit, Order::Enter { unit, building }),
                Some(Target::Building(building))
                    if sim.0.unit(unit).is_some_and(|u| u.stats.harvester)
                        && sim.0.buildings().iter().any(|b| b.id == building && b.owner == local() && sim.0.building_types.get(b.kind as usize).is_some_and(|kind| kind.refinery)) =>
                {
                    give(&mut sim, &mut plans, planning.0, unit, Order::Dock { unit, building })
                }
                _ => give(&mut sim, &mut plans, planning.0, unit, Order::Move { unit, x, y }),
            },
            (Target::Unit(unit), Action::Capture | Action::EngineerRepair | Action::Sabotage) => {
                if let Some(Target::Building(building)) = hover.target {
                    give(&mut sim, &mut plans, planning.0, unit, Order::Enter { unit, building });
                }
            }
            (Target::Unit(unit), Action::Deploy | Action::NoDeploy) => {
                deploy(&mut sim, unit, &mut cues);
                ordered = false;
            }
            // On itself, a building with soldiers in
            // it puts them out; a factory becomes the primary one, with the
            // announcer.
            (Target::Building(building), Action::Deploy) => {
                let state = sim.0.buildings().iter().find(|b| b.id == building).map(|b| (b.occupants.is_empty(), b.primary));
                match state {
                    Some((false, _)) => sim.give(Order::Evacuate { building }),
                    Some((true, primary)) => {
                        sim.give(Order::Primary { building });
                        if !primary {
                            cues.write(Cue::Eva("EVA_PrimaryBuildingSelected"));
                        }
                    }
                    None => {}
                }
                ordered = false;
            }
            // A move click with a
            // building that packs up turns it into its vehicle, which drives
            // there; with a factory it sets the rally point.
            (Target::Building(building), Action::Move) => {
                if sim.0.can_undeploy(building) {
                    sim.give(Order::Undeploy { building, x, y });
                } else if sim.0.has_rally_point(building) {
                    sim.give(Order::Rally { building, x, y });
                    cues.write(Cue::Eva("EVA_NewRallyPointEstablished"));
                    ordered = false;
                }
            }
            _ => ordered = false,
        }
        if ordered {
            last_order.0 = Some(sim.0.current_tick());
            if let (false, Target::Unit(unit)) = (spoke, me) {
                spoke = true;
                let voice = view.voices.get(&unit).and_then(|voices| voices.1.clone());
                cues.write_batch(voice.map(Cue::Sound));
            }
        }
    }
    // With `AddPlanningModeCommandSound`.
    if plans.waiting.values().map(Vec::len).sum::<usize>() > planned_before {
        cues.write_batch(crate::audio::rules_sound(&files, "AddPlanningModeCommandSound").map(Cue::Sound));
    }
}

/// Turns planning mode on and off with its key, and carries plans out.
// DATA: keyboard.ini `PlanningMode` (Z): on while held. The two sounds; the
// messages as it comes on (the key's own wording the first time); on leaving,
// every unit with a plan starts on it and takes each next order as it finishes
// the one before.
fn run_plans(
    keys: Res<ButtonInput<KeyCode>>,
    files: Res<GameFiles>,
    mut planning: ResMut<crate::bar::PlanningMode>,
    mut plans: ResMut<Plans>,
    mut sim: ResMut<Sim>,
    mut status: MessageWriter<crate::bar::Status>,
    mut cues: MessageWriter<Cue>,
) {
    if keys.just_pressed(KeyCode::KeyZ) && !planning.0 {
        planning.0 = true;
        if !std::mem::replace(&mut plans.said_key, true) {
            status.write(crate::bar::Status::Key("MSG:PlanningModeIntro1Key"));
        }
        status.write(crate::bar::Status::Key("MSG:PlanningModeIntro3"));
    }
    if keys.just_released(KeyCode::KeyZ) && planning.0 {
        planning.0 = false;
    }
    if planning.0 != plans.was_on {
        plans.was_on = planning.0;
        let sound = if planning.0 { "StartPlanningModeSound" } else { "EndPlanningModeSound" };
        cues.write_batch(crate::audio::rules_sound(&files, sound).map(Cue::Sound));
        if !planning.0 {
            let waiting = std::mem::take(&mut plans.waiting);
            for (unit, orders) in waiting {
                plans.running.insert(unit, (orders, None));
            }
        }
    }
    // An order is carried out on the tick after it is given, so a unit is not
    // taken to have finished one until that tick has passed.
    let tick = sim.0.current_tick();
    let mut next_orders = Vec::new();
    plans.running.retain(|&unit, (orders, sent)| {
        let Some(state) = sim.0.unit(unit) else {
            return false;
        };
        let idle = !state.is_moving() && state.arms.target.is_none();
        if sent.is_some_and(|at| tick < at + 2 || !idle) {
            return true;
        }
        if orders.is_empty() {
            return false;
        }
        next_orders.push(orders.remove(0));
        *sent = Some(tick);
        true
    });
    for order in next_orders {
        sim.give(order);
    }
}

fn aim_of(target: Target) -> ra2_sim::combat::Aim {
    match target {
        Target::Unit(id) => ra2_sim::combat::Aim::Unit(id),
        Target::Building(id) => ra2_sim::combat::Aim::Building(id),
    }
}

fn exists(sim: &World, target: Target) -> bool {
    match target {
        Target::Unit(id) => sim.unit(id).is_some(),
        Target::Building(id) => sim.buildings().iter().any(|b| b.id == id),
    }
}

/// Picks the cursor for what the mouse is over.
// The rules followed here, as far
// as this game has the things they depend on.
fn choose_cursor(
    pointer: Res<Pointer>,
    window: Single<&Window, With<PrimaryWindow>>,
    hover: Res<Hover>,
    sim: Res<Sim>,
    selection: Res<Selection>,
    placing: Res<Placing>,
    mode: Res<Mode>,
    keys: Res<ButtonInput<KeyCode>>,
    blocked: Res<crate::game::ScrollBlocked>,
    drag: Res<crate::game::RightDrag>,
    mut wanted: ResMut<WantedCursor>,
) {
    wanted.0 = cursor::NORMAL;
    // While a right drag scrolls, the cursor shows which ways the view
    // can still go.
    if drag.started {
        wanted.0 = match drag.blocked {
            1 => 62,
            2 => 64,
            3 => 63,
            4 => 66,
            6 => 65,
            8 => 68,
            9 => 69,
            12 => 67,
            _ => 61,
        };
        return;
    }
    let Some(at) = pointer.0 else {
        return;
    };
    // Scrolling at the outermost pixel of the window: the direction is from the
    // middle of the screen to a point in the zone the mouse is in.
    let (w, h) = (window.width(), window.height());
    if at.x < 1.0 || at.y < 1.0 || at.x >= w - 1.0 || at.y >= h - 1.0 {
        let zone = |value: f32, size: f32, edge: f32| {
            if value < size * edge {
                -1.0
            } else if value > size * (1.0 - edge) {
                1.0
            } else {
                0.0
            }
        };
        let (dx, dy) = (zone(at.x, w, 0.16) * w / 2.0, zone(at.y, h, 0.21) * h / 2.0);
        if dx != 0.0 || dy != 0.0 {
            // Eight directions, north first, clockwise.
            let turn = dx.atan2(-dy) / std::f32::consts::TAU;
            let facing = ((turn * 8.0).round() as i32).rem_euclid(8) as usize;
            // The table's second set of eight is for a view that cannot
            // scroll that way.
            wanted.0 = if blocked.0 { cursor::NO_SCROLL } else { cursor::SCROLL } + facing;
        }
        return;
    }
    if hover.point.is_none() || placing.0.is_some() {
        return;
    }
    let world = &sim.0;
    // In repair mode the wrench over one of the
    // player's damaged buildings; in sell mode the sell cursor over one of the
    // player's buildings; otherwise the refusing cursor of each.
    if hover.radar && !radar_acts(world, &selection, hover.cell, hover.target, *mode) {
        return;
    }
    let small = if hover.radar { cursor::SMALL } else { 0 };
    if let Mode::Super(index) = *mode {
        wanted.0 = small | match world.super_types.get(index as usize).map(|kind| kind.kind) {
            Some(0) => 53,
            Some(1) => 57,
            Some(2) => 50,
            Some(3 | 4) => 58,
            _ => 47,
        };
        return;
    }
    if *mode != Mode::None {
        let own = match hover.target {
            Some(Target::Building(id)) if owned_by_player(world, Target::Building(id)) => Some(id),
            _ => None,
        };
        wanted.0 = match (*mode, own) {
            (Mode::Repair, Some(id)) if world.can_repair(id) => cursor::REPAIR,
            (Mode::Repair, _) => cursor::NO_REPAIR,
            (_, Some(_)) => cursor::SELL,
            _ => cursor::NO_SELL,
        };
        return;
    }
    // The best of
    // the selection answers for what is under the mouse, and its answer picks
    // the cursor.
    let mods = Mods {
        ctrl: keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        alt: keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
        shift: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
    };
    let point = (hover.cell.0 * LEPTONS_PER_CELL + 128, hover.cell.1 * LEPTONS_PER_CELL + 128);
    let alone = selection.0.len() == 1;
    wanted.0 = small | match (action::best(world, &selection.0, point), hover.target) {
        (Some(best), Some(object)) => {
            let a = action::for_object(world, best, object, alone, selection.0.contains(&object), mods);
            action::cursor_of(a, alone && action::in_range(world, best, aim_of(object), mods))
        }
        (Some(best), None) => {
            let a = action::for_cell(world, best, hover.cell, mods);
            let cell = ra2_sim::combat::Aim::Cell(hover.cell.0, hover.cell.1);
            action::cursor_of(a, alone && a == Action::Attack && action::in_range(world, best, cell, mods))
        }
        // Not original behaviour: only the player's own things can be selected.
        (None, Some(object)) if owned_by_player(world, object) => cursor::SELECT,
        _ => cursor::NORMAL,
    };
}

/// A small picture being put together, in pixels relative to a point on the map.
struct Canvas {
    min: IVec2,
    size: IVec2,
    rgba: Vec<u8>,
}

impl Canvas {
    fn new(min: IVec2, max: IVec2) -> Self {
        let size = (max - min + IVec2::ONE).max(IVec2::ONE);
        Self { min, size, rgba: vec![0; (size.x * size.y * 4) as usize] }
    }

    fn put(&mut self, at: IVec2, rgb: [u8; 3]) {
        let local = at - self.min;
        if local.x >= 0 && local.y >= 0 && local.x < self.size.x && local.y < self.size.y {
            let index = ((local.y * self.size.x + local.x) * 4) as usize;
            self.rgba[index..index + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }

    fn fill(&mut self, min: IVec2, max: IVec2, rgb: [u8; 3]) {
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                self.put(IVec2::new(x, y), rgb);
            }
        }
    }

    /// A one-pixel line.
    /// A line of five pixels on, three off.
    fn dashed(&mut self, from: IVec2, to: IVec2, rgb: [u8; 3], phase: i32) {
        let delta = (to - from).abs();
        let step = IVec2::new(if from.x < to.x { 1 } else { -1 }, if from.y < to.y { 1 } else { -1 });
        let (mut at, mut error, mut count) = (from, delta.x - delta.y, phase);
        loop {
            if count.rem_euclid(8) < 5 {
                self.put(at, rgb);
            }
            count += 1;
            if at == to {
                break;
            }
            let doubled = 2 * error;
            if doubled > -delta.y {
                error -= delta.y;
                at.x += step.x;
            }
            if doubled < delta.x {
                error += delta.x;
                at.y += step.y;
            }
        }
    }

    /// The engine's range ring: two ellipse outlines a pixel apart.
    fn ring(&mut self, rx: i32, ry: i32, rgb: [u8; 3]) {
        for (a, b) in [(rx, ry), (rx - 1, ry - 1)] {
            let steps = (rx * 8).max(64);
            let mut last = IVec2::new(a, 0);
            for i in 1..=steps {
                let angle = i as f32 / steps as f32 * std::f32::consts::TAU;
                let next = IVec2::new((angle.cos() * a as f32).round() as i32, (angle.sin() * b as f32).round() as i32);
                self.line(last, next, rgb);
                last = next;
            }
        }
    }

    /// A line seen through, `alpha` of 255.
    fn faint_line(&mut self, from: IVec2, to: IVec2, rgb: [u8; 3], alpha: u8) {
        let mut points = Vec::new();
        let delta = (to - from).abs();
        let step = IVec2::new(if from.x < to.x { 1 } else { -1 }, if from.y < to.y { 1 } else { -1 });
        let (mut at, mut error) = (from, delta.x - delta.y);
        loop {
            points.push(at);
            if at == to {
                break;
            }
            let doubled = error * 2;
            if doubled > -delta.y {
                error -= delta.y;
                at.x += step.x;
            }
            if doubled < delta.x {
                error += delta.x;
                at.y += step.y;
            }
        }
        for at in points {
            let local = at - self.min;
            if local.x >= 0 && local.y >= 0 && local.x < self.size.x && local.y < self.size.y {
                let index = ((local.y * self.size.x + local.x) * 4) as usize;
                self.rgba[index..index + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
            }
        }
    }

    fn line(&mut self, from: IVec2, to: IVec2, rgb: [u8; 3]) {
        let delta = (to - from).abs();
        let step = IVec2::new(if from.x < to.x { 1 } else { -1 }, if from.y < to.y { 1 } else { -1 });
        let (mut at, mut error) = (from, delta.x - delta.y);
        loop {
            self.put(at, rgb);
            if at == to {
                break;
            }
            let doubled = error * 2;
            if doubled > -delta.y {
                error -= delta.y;
                at.x += step.x;
            }
            if doubled < delta.x {
                error += delta.x;
                at.y += step.y;
            }
        }
    }

    /// A line of dashes: `pattern` says which of each sixteen pixels along it
    /// are drawn, starting `offset` into it.
    fn patterned(&mut self, from: IVec2, to: IVec2, rgb: [u8; 3], pattern: [bool; 16], offset: usize) {
        let delta = (to - from).abs();
        let step = IVec2::new(if from.x < to.x { 1 } else { -1 }, if from.y < to.y { 1 } else { -1 });
        let (mut at, mut error, mut count) = (from, delta.x - delta.y, offset);
        loop {
            if pattern[count & 15] {
                self.put(at, rgb);
            }
            count += 1;
            if at == to {
                break;
            }
            let doubled = error * 2;
            if doubled > -delta.y {
                error -= delta.y;
                at.x += step.x;
            }
            if doubled < delta.x {
                error += delta.x;
                at.y += step.y;
            }
        }
    }

    /// Draws a frame with the middle of the shape's canvas on a point.
    // What the engine's "centred" drawing flag does.
    fn shape(&mut self, shp: &Shp, frame: usize, palette: &Palette, centre: IVec2) {
        let Some(frame) = shp.frames.get(frame) else {
            return;
        };
        let corner = centre - IVec2::new(shp.width as i32 / 2, shp.height as i32 / 2)
            + IVec2::new(frame.x as i32, frame.y as i32);
        for row in 0..frame.height as i32 {
            for col in 0..frame.width as i32 {
                let index = frame.pixels[(row * frame.width as i32 + col) as usize];
                if index != 0 {
                    self.put(corner + IVec2::new(col, row), palette.0[index as usize]);
                }
            }
        }
    }

    fn into_image(self, images: &mut Assets<Image>) -> (Handle<Image>, IVec2, IVec2) {
        let image = Image::new(
            Extent3d {
                width: self.size.x as u32,
                height: self.size.y as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        (images.add(image), self.min, self.size)
    }
}

/// Where a point offset from a place on the map, in leptons and height levels, is
/// drawn relative to that place.
// The projection, rounding towards zero.
fn project(dx: i32, dy: i32, levels: i32) -> IVec2 {
    IVec2::new((dx - dy) * 30 / 256, (dx + dy) * 15 / 256 - levels * LEVEL_HEIGHT)
}

/// The first and last quarters of an edge of a box.
fn edge_ends(canvas: &mut Canvas, a: [i32; 3], b: [i32; 3], rgb: [u8; 3]) {
    let mix = |p: [i32; 3], q: [i32; 3]| [(3 * p[0] + q[0]) / 4, (3 * p[1] + q[1]) / 4, (3 * p[2] + q[2]) / 4];
    quarter(canvas, a, mix(a, b), rgb);
    quarter(canvas, b, mix(b, a), rgb);
}

/// A line between two points given in leptons, with height in hundredths of a
/// level.
fn quarter(canvas: &mut Canvas, a: [i32; 3], b: [i32; 3], rgb: [u8; 3]) {
    let flat = |p: [i32; 3]| project(p[0], p[1], 0) - IVec2::new(0, p[2] * LEVEL_HEIGHT / 100);
    canvas.line(flat(a), flat(b), rgb);
}

fn spawn_overlay(commands: &mut Commands, at: Vec2, image: &(Handle<Image>, IVec2, IVec2), z: f32) {
    let corner = at.round() + image.1.as_vec2();
    let centre = corner + image.2.as_vec2() / 2.0;
    commands.spawn((
        Overlay,
        DespawnOnExit(Screen::InGame),
        Sprite::from_image(image.0.clone()),
        Transform::from_xyz(centre.x, -centre.y, z),
    ));
}

/// Above everything on the map.
const FRONT: f32 = 900.0;

/// What the player's psychic sensors show: a line from each enemy unit to what
/// it is after or where it is going.
// For every foot unit the sensors reveal, each frame and for as long as they
// do: a line from the unit to its target, or with no target to its
// destination, with a three-pixel square at each end, in its house's colour.
// The line is dashes of five pixels with gaps of three that crawl a pixel
// every 32 ms of real time; its colour swells to white and back over the first
// half of every 1.024 s.
fn draw_sensor_lines(mut commands: Commands, mut images: ResMut<Assets<Image>>, sim: Res<Sim>, view: Res<UnitView>, time: Res<Time<Real>>) {
    let world = &sim.0;
    if !world.buildings().iter().any(|b| b.owner == local() && world.building_types.get(b.kind as usize).is_some_and(|kind| kind.psychic_radius > 0)) {
        return;
    }
    let clock = time.elapsed().as_millis() as u32;
    let pattern = std::array::from_fn(|n| n % 8 < 5);
    let offset = ((clock.wrapping_neg() >> 5) & 15) as usize;
    let phase = clock & 0x3ff;
    let glow = if phase >= 512 { 0 } else if phase & 0x100 != 0 { (phase ^ 0xff) & 0xff } else { phase & 0xff };
    let point = |p: (i32, i32, i32)| scene_point(view.origin, p.0, p.1, 0) - Vec2::new(0.0, p.2 as f32 * LEVEL_HEIGHT as f32 / ra2_sim::combat::LEVEL_HEIGHT as f32);
    let half = LEPTONS_PER_CELL / 2;
    for unit in world.units() {
        if unit.owner == local() || unit.health <= 0 || unit.tethered || !world.sensor_reveals(local(), unit) {
            continue;
        }
        let to = unit.arms.target.and_then(|target| world.aim_point(target)).or_else(|| {
            let cell = unit.destination()?;
            let deck = if world.map.bridged(cell.0, cell.1) { 4 } else { 0 };
            Some((cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, (world.map.level(cell.0, cell.1) as i32 + deck) * ra2_sim::combat::LEVEL_HEIGHT))
        });
        let Some(to) = to else {
            continue;
        };
        let at = point((unit.x, unit.y, world.ground_z(unit.x, unit.y) + world.altitude(unit)));
        let delta = (point(to) - at).round().as_ivec2();
        let house = view.house_colour(unit.owner);
        let rgb = house.map(|c| (c as u32 + (255 - c as u32) * glow / 256) as u8);
        let mut canvas = Canvas::new(delta.min(IVec2::ZERO) - 2, delta.max(IVec2::ZERO));
        canvas.patterned(IVec2::ZERO, delta, rgb, pattern, offset);
        for end in [IVec2::ZERO, delta] {
            canvas.fill(end - 2, end, house);
        }
        let image = canvas.into_image(&mut images);
        spawn_overlay(&mut commands, at, &image, FRONT);
    }
}

/// Redraws everything shown over the map for the mouse.
fn draw_overlays(
    mut commands: Commands,
    mut art: ResMut<OverlayArt>,
    mut images: ResMut<Assets<Image>>,
    sim: Res<Sim>,
    view: Res<UnitView>,
    catalog: Res<BuildCatalog>,
    hover: Res<Hover>,
    selection: Res<Selection>,
    band: Res<Band>,
    placing: Res<Placing>,
    (last_order, prefs): (Res<LastOrder>, Res<crate::prefs::Prefs>),
    plans: Res<Plans>,
    planning: Res<crate::bar::PlanningMode>,
    groups: Res<GroupOf>,
    clock: Res<Time<Real>>,
    old: Query<Entity, With<Overlay>>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let art = &mut *art;
    let world = &sim.0;
    // PALETTE.PAL entries. 15 for the
    // selection box, 3 for the line to a destination.
    let (box_rgb, line_rgb) = (art.palette.0[15], art.palette.0[3]);

    // The placement grid. Each foundation cell gets frame 0 if the building may
    // stand on it, or else frame 2 plus the cell's slope number, centred on the
    // cell's top corner, one pixel up. Frames past 18 are empty, so the steepest
    // slopes show nothing.
    for target in &selection.0 {
        let Target::Building(id) = *target else {
            continue;
        };
        let Some(building) = world.buildings().iter().find(|b| b.id == id) else {
            continue;
        };
        let Some(kind) = world.building_types.get(building.kind as usize).filter(|kind| kind.radial_indicator) else {
            continue;
        };
        let range = if kind.psychic_radius > 0 { kind.psychic_radius } else { kind.gap_radius };
        if range <= 0 || building.owner != local() || building.health <= 0 {
            continue;
        }
        let rx = ((range as f32 + 0.5) / std::f32::consts::SQRT_2 * 60.0) as i32;
        let ry = rx / 2;
        let half = LEPTONS_PER_CELL / 2;
        let level = world.map.level(building.cell.0, building.cell.1) as i32;
        let centre = scene_point(view.origin, building.cell.0 * LEPTONS_PER_CELL + building.size.0 * half, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * half, level);
        let house = view.house_colour(building.owner);
        let dim = |c: [u8; 3]| c.map(|c| (c as i32 - c as i32 * 50 / 256) as u8);
        let mut canvas = Canvas::new(IVec2::new(-rx - 1, -ry - 1), IVec2::new(rx + 1, ry + 1));
        if kind.concentric_indicator {
            let t = (clock.elapsed().as_millis() / 2) as i32 & 0x3ff;
            if t < 0x200 {
                let colour = if t < 0x100 {
                    let n = 255 - (t & 0xff);
                    house.map(|c| (c as i32 + (255 - c as i32) * n / 256) as u8)
                } else {
                    house
                };
                canvas.ring(rx, ry, colour);
            }
            let inner = rx * t / 1024;
            if inner > 32 {
                canvas.ring(inner, inner / 2, dim(house));
            }
        } else {
            for (i, strength) in [0.05f32, 0.2, 0.4, 1.0].into_iter().enumerate() {
                let angle = (world.current_tick() as f32 + i as f32) * 0.005;
                let end = IVec2::new((angle.cos() * rx as f32).round() as i32, (-angle.sin() * ry as f32).round() as i32);
                canvas.faint_line(IVec2::ZERO, end, house, (strength * 255.0) as u8);
            }
            canvas.ring(rx, ry, dim(house));
        }
        let image = canvas.into_image(&mut images);
        spawn_overlay(&mut commands, centre, &image, FRONT);
    }
    if let (Some(building), Some(_)) = (placing.0, hover.point) {
        let range = world
            .building_types
            .get(building as usize)
            .and_then(|kind| kind.fighter.primary)
            .and_then(|mount| world.weapons.get(mount.weapon as usize))
            .map_or(0, |weapon| weapon.range / 256);
        // A
        // gap generator's ring is its `GapRadiusInCells`.
        // Before either, a
        // `PsychicDetectionRadius` when the type has one.
        let kind = world.building_types.get(building as usize);
        let range = match kind.map_or((0, 0), |kind| (kind.psychic_radius, kind.gap_radius)) {
            (psychic, _) if psychic > 0 => psychic,
            (_, gap) if gap > 0 => gap,
            _ => range,
        };
        if range > 0 {
            let rx = ((range as f32 + 0.5) / std::f32::consts::SQRT_2 * 60.0) as i32;
            let ry = rx / 2;
            let level = world.map.level(hover.cell.0, hover.cell.1) as i32;
            let centre = scene_point(view.origin, hover.cell.0 * LEPTONS_PER_CELL + 128, hover.cell.1 * LEPTONS_PER_CELL + 128, level);
            // The pulse: a value that runs 0 to 100 and back in tens, a step a frame.
            let step = (world.current_tick() % 20) as i32;
            let v = if step <= 10 { step * 10 } else { (20 - step) * 10 };
            let colour = view.player_colour().map(|c| {
                let c = c as i32 - c as i32 * v / 256;
                (c - c * 50 / 256) as u8
            });
            let mut canvas = Canvas::new(IVec2::new(-rx - 1, -ry - 1), IVec2::new(rx + 1, ry + 1));
            canvas.ring(rx, ry, colour);
            let image = canvas.into_image(&mut images);
            spawn_overlay(&mut commands, centre, &image, FRONT);
        }
    }
    if let (Some(building), Some(_), Some(place)) = (placing.0, hover.point, &art.place) {
        let origin = placement_origin(world, hover.cell, building);
        let cells = world.placement(local(), building, origin);
        // When a wall may go where the mouse is, the
        // cells that would be filled to join it to other pieces are marked too.
        let line: Vec<_> = if cells.iter().all(|(_, ok)| *ok) {
            world.wall_line(local(), building, origin).into_iter().map(|cell| (cell, true)).collect()
        } else {
            Vec::new()
        };
        for (cell, ok) in cells.into_iter().chain(line) {
            let frame = if ok { 0 } else { 2 + world.map.ramp(cell.0, cell.1) as usize };
            let image = art.images.entry(OverlayKey::Place(frame)).or_insert_with(|| {
                let half = IVec2::new(place.width as i32, place.height as i32);
                let mut canvas = Canvas::new(-half, half);
                canvas.shape(place, frame, &art.palette, IVec2::ZERO);
                canvas.into_image(&mut images)
            });
            let level = world.map.level(cell.0, cell.1) as i32;
            let top = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL, cell.1 * LEPTONS_PER_CELL, level);
            spawn_overlay(&mut commands, top - Vec2::Y, image, FRONT);
        }
    }

    // A miner taking ore and not moving shows OREGATH.SHP 30 leptons ahead of
    // it along its facing in eighths: eight runs of 15 frames in reverse order
    // of facing, a frame a game frame, in the palette of animations, under the
    // miner.
    if let (Some(shp), Some(palette)) = (&art.ore_gather, &art.anim_palette) {
        for unit in world.units().iter().filter(|unit| unit.stats.harvester && unit.work.digging && !unit.is_moving()) {
            let d8 = ((((unit.facing as usize) >> 4) + 1) >> 1) & 7;
            let angle = (((d8 as i32) << 13) - 0x3fff) as f32 * -std::f32::consts::TAU / 65536.0;
            let (x, y) = (unit.x + (angle.cos() * 30.0) as i32, unit.y - (angle.sin() * 30.0) as i32);
            let frame = (7 - d8) * 15 + ((unit.id as u64 + world.current_tick()) % 15) as usize;
            let half = IVec2::new(shp.width as i32 / 2, shp.height as i32 / 2);
            let mut canvas = Canvas::new(-half, half);
            canvas.shape(shp, frame, palette, IVec2::ZERO);
            let image = canvas.into_image(&mut images);
            let cell = unit.cell();
            let point = scene_point(view.origin, x, y, world.map.level(cell.0, cell.1) as i32);
            spawn_overlay(&mut commands, point, &image, 2.0 + ((cell.0 + cell.1) as f32 + 0.85) * DEPTH_PER_ROW);
        }
    }

    // A veteran shows frame 13 of PIPS.SHP and an elite frame 14, centred 5
    // right and 2 down of the unit's point, and 5 and 4 more for anything but a
    // foot soldier; at full brightness. Not on something disguised to the
    // player.
    if let (Some(pips), palette) = (&art.pips, &art.palette) {
        for unit in world.units().iter().filter(|unit| unit.rank() < 2 && !world.disguised_to(unit, local())) {
            let cell = unit.cell();
            if world.map.shrouded(cell.0, cell.1) {
                continue;
            }
            let frame = if unit.rank() == 0 { 14 } else { 13 };
            let offset = if unit.stats.infantry { IVec2::new(5, 2) } else { IVec2::new(10, 6) };
            let mut canvas = Canvas::new(IVec2::new(-12, -12), IVec2::new(24, 24));
            canvas.shape(pips, frame, palette, offset);
            let image = canvas.into_image(&mut images);
            let point = unit_point(world, &view, unit);
            spawn_overlay(&mut commands, point, &image, FRONT);
        }
    }

    // A `Parachute`
    // animation 75 leptons above a falling soldier and fixed to it, in his
    // house's colours. DATA: art.ini `[PARACH]`: two game frames a picture,
    // opening over pictures 0 to 19, then round 20 to 38; at touchdown it plays
    // on through the collapse, 40 to 68, and is gone.
    if let Some(shp) = &art.parachute {
        let now = world.current_tick();
        for unit in world.units().iter().filter(|unit| !unit.doomed && (unit.falling.is_some() || (unit.landed_at > 0 && now < unit.landed_at + 58))) {
            let frame = match unit.falling {
                Some(_) => {
                    let step = (now.saturating_sub(unit.dropped_at) / 2) as usize;
                    if step < 20 { step } else { 20 + (step - 20) % 19 }
                }
                None => 40 + ((now - unit.landed_at) / 2) as usize,
            };
            let Some(palette) = view.house_palette(unit.owner).or(art.unit_palette.as_ref()) else {
                continue;
            };
            let half = IVec2::new(shp.width as i32 / 2, shp.height as i32 / 2);
            let mut canvas = Canvas::new(-half, half);
            canvas.shape(shp, frame, palette, IVec2::ZERO);
            let image = canvas.into_image(&mut images);
            let cell = unit.cell();
            let lift = (unit.falling.unwrap_or(0) + 75) as f32 * 15.0 / ra2_sim::combat::LEVEL_HEIGHT as f32;
            let point = scene_point(view.origin, unit.x, unit.y, world.map.level(cell.0, cell.1) as i32) - Vec2::new(0.0, lift);
            spawn_overlay(&mut commands, point, &image, 2.0 + ((cell.0 + cell.1) as f32 + 41.0) * DEPTH_PER_ROW);
        }
    }

    // A bomb's
    // picture sits on its carrier's draw point, in the mouse's palette: six
    // pairs of frames as the fuse burns down (a sixth of the delay each), the
    // two of a pair changing every `IvanIconFlickerRate` (8) frames.
    if let (Some(shp), Some(palette)) = (&art.bomb, &art.mouse_palette) {
        for bomb in world.bombs.iter().filter(|bomb| bomb.house == local()) {
            let Some(at) = world.aim_point(bomb.target) else {
                continue;
            };
            let tick = world.current_tick();
            let stage = (tick.saturating_sub(bomb.planted) / ((bomb.goes_off - bomb.planted).max(6) / 6)) as usize;
            let frame = (stage * 2 + usize::from(tick % 16 >= 8)).min(11);
            let mut canvas = Canvas::new(IVec2::new(-28, -22), IVec2::new(28, 22));
            canvas.shape(shp, frame, palette, IVec2::ZERO);
            let image = canvas.into_image(&mut images);
            let point = scene_point(view.origin, at.0, at.1, 0) - Vec2::new(0.0, at.2 as f32 * 15.0 / ra2_sim::combat::LEVEL_HEIGHT as f32);
            spawn_overlay(&mut commands, point, &image, FRONT);
        }
    }

    // Plans show only while planning mode is on: dashed lines from the unit to
    // each place in turn, dark underneath and in the house's colour on top, with
    // a filled box at each place.
    if planning.0 {
        for (&unit, orders) in &plans.waiting {
            let Some(state) = world.unit(unit) else {
                continue;
            };
            let point_of = |order: &Order| -> Option<Vec2> {
                let cell = match *order {
                    Order::Move { x, y, .. } | Order::Harvest { x, y, .. } => (x, y),
                    Order::Attack { target, .. } => {
                        let at = world.aim_point(target)?;
                        (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL))
                    }
                    Order::Board { transport, .. } => world.unit(transport)?.cell(),
                    Order::Enter { building, .. } | Order::Dock { building, .. } => world.buildings().iter().find(|b| b.id == building)?.cell,
                    _ => return None,
                };
                Some(scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128, world.map.level(cell.0, cell.1) as i32))
            };
            let colour = if selection.0.contains(&Target::Unit(unit)) { [255, 255, 255] } else { view.player_colour() };
            let mut from = unit_point(world, &view, state);
            for order in orders {
                let Some(to) = point_of(order) else {
                    continue;
                };
                let delta = (to - from).round().as_ivec2();
                let mut canvas = Canvas::new(delta.min(IVec2::ZERO) - 4, delta.max(IVec2::ZERO) + 5);
                for (shift, rgb) in [(1, [0, 0, 0]), (0, colour)] {
                    canvas.dashed(IVec2::new(0, shift), delta + IVec2::new(0, shift), rgb, 0);
                }
                canvas.fill(delta - 2, delta + 3, colour);
                let image = canvas.into_image(&mut images);
                spawn_overlay(&mut commands, from, &image, FRONT);
                from = to;
            }
        }
    }

    // The drag box, in the selection box's colour.
    if let Some((from, to)) = band.drawn {
        let size = (to - from).abs().round().as_ivec2();
        let mut canvas = Canvas::new(IVec2::ZERO, size);
        for (a, b) in [
            (IVec2::ZERO, IVec2::new(size.x, 0)),
            (IVec2::new(0, size.y), size),
            (IVec2::ZERO, IVec2::new(0, size.y)),
            (IVec2::new(size.x, 0), size),
        ] {
            canvas.line(a, b, box_rgb);
        }
        let image = canvas.into_image(&mut images);
        spawn_overlay(&mut commands, from.min(to), &image, FRONT);
    }

    // A building under repair shows the wrench at its draw point, running
    // through its seven frames with the mouse's palette.
    if let (Some(wrench), Some(palette)) = (&art.wrench, &art.mouse_palette) {
        let n = ((14 * 8 / (view.game_speed() + 1)) / 4).max(2) as u64;
        let frame = ((world.current_tick() % n) * 6 / (n - 1)) as usize;
        for building in world.buildings().iter().filter(|b| b.repairing) {
            let level = world.map.level(building.cell.0, building.cell.1) as i32;
            let at = scene_point(view.origin, building.cell.0 * LEPTONS_PER_CELL, building.cell.1 * LEPTONS_PER_CELL, level);
            let image = art.images.entry(OverlayKey::Wrench(frame)).or_insert_with(|| {
                let half = IVec2::new(wrench.width as i32, wrench.height as i32);
                let mut canvas = Canvas::new(-half, half);
                canvas.shape(wrench, frame, palette, IVec2::ZERO);
                canvas.into_image(&mut images)
            });
            spawn_overlay(&mut commands, at, image, FRONT);
        }
    }

    let shown: Vec<Target> =
        selection.0.iter().copied().chain(hover.target.filter(|target| !selection.0.contains(target))).collect();
    for target in shown {
        let selected = selection.0.contains(&target);
        match target {
            Target::Unit(id) => {
                let Some(unit) = world.unit(id) else {
                    continue;
                };
                let at = unit_point(world, &view, unit);
                // Seventeen pips two pixels apart, in
                // green, yellow or red, with the border art only when selected.
                let ratio = unit.health as f32 / unit.stats.strength.max(1) as f32;
                let lit = ((ratio * 17.0) as i32).clamp(1, 17);
                let frame = if ratio <= art.red {
                    17
                } else if ratio <= art.yellow {
                    16
                } else {
                    15
                };
                let key = OverlayKey::UnitBar { lit, frame, border: selected };
                if let (Some(pips), Some(border)) = (&art.pips, &art.pip_border) {
                    let image = art.images.entry(key).or_insert_with(|| {
                        let mut canvas = Canvas::new(IVec2::new(-24, -36), IVec2::new(24, -16));
                        if selected {
                            canvas.shape(border, 0, &art.palette, IVec2::new(1, -26));
                        }
                        for i in 0..lit {
                            canvas.shape(pips, frame, &art.palette, IVec2::new(-15 + 2 * i, -25));
                        }
                        canvas.into_image(&mut images)
                    });
                    spawn_overlay(&mut commands, at, image, FRONT);
                }

                // With the bar, the number of the group it is in (the tenth
                // shown as 0): at 14 left and 29 up of its point for a foot
                // soldier, 26 up for the rest, in the owner's colour on a
                // black box with an outline, the box two pixels wider than the
                // digit each side and four shorter than the font, one more on
                // the left.
                if let (Some(&group), Some(font), true) = (groups.0.get(&target), &art.font, unit.owner == local()) {
                    let rgb = art.house_rgb;
                    let foot = unit.stats.infantry;
                    let image = art.images.entry(OverlayKey::Group(group, foot)).or_insert_with(|| {
                        let text = ((group + 1) % 10).to_string();
                        let (width, mask) = font.rasterize(&text);
                        let (w, h) = (width as i32, font.height as i32);
                        let origin = IVec2::new(-14, if foot { -29 } else { -26 });
                        let corner = origin + IVec2::new(-3, 2);
                        let far = corner + IVec2::new(w + 5, h - 4);
                        let mut canvas = Canvas::new(corner - IVec2::ONE, far + IVec2::ONE);
                        canvas.fill(corner, far, [0, 0, 0]);
                        for (a, b) in [
                            (corner, IVec2::new(far.x, corner.y)),
                            (IVec2::new(far.x, corner.y), far),
                            (far, IVec2::new(corner.x, far.y)),
                            (IVec2::new(corner.x, far.y), corner),
                        ] {
                            canvas.line(a, b, rgb);
                        }
                        for row in 0..h {
                            for col in 0..w {
                                if mask[(row * w + col) as usize] != 0 {
                                    canvas.put(origin + IVec2::new(col, row), rgb);
                                }
                            }
                        }
                        canvas.into_image(&mut images)
                    });
                    spawn_overlay(&mut commands, at, image, FRONT + 0.5);
                }

                // A miner's load as five boxes under
                // it, four pixels apart: gems first (blue), then ore (gold), then
                // empty ones; each count is five times its share of `Storage`,
                // rounded. Shown with the health bar, for the player's own.
                if unit.stats.harvester && unit.owner == local() && unit.stats.storage > 0 {
                    let cargo = unit.work.cargo;
                    let share = |bales: u32| (5.0 * bales as f32 / unit.stats.storage as f32 + 0.5) as i32;
                    let (gems, ore) = (share(cargo[1]), share(cargo[0] + cargo[2] + cargo[3]));
                    if let Some(pips) = &art.cargo_pips {
                        let image = art.images.entry(OverlayKey::Cargo { gems, ore }).or_insert_with(|| {
                            let mut canvas = Canvas::new(IVec2::new(-24, 0), IVec2::new(12, 20));
                            let (mut gems, mut ore) = (gems, ore);
                            for i in 0..5 {
                                let frame = if gems > 0 {
                                    gems -= 1;
                                    5
                                } else if ore > 0 {
                                    ore -= 1;
                                    1
                                } else {
                                    0
                                };
                                canvas.shape(pips, frame, &art.palette, IVec2::new(-15 + 4 * i, 10));
                            }
                            canvas.into_image(&mut images)
                        });
                        spawn_overlay(&mut commands, at, image, FRONT);
                    }
                }

                // A transport shows a box for
                // each place aboard, filled with each passenger's own pip; a
                // gunner vehicle's places after the first sit two steps further on.
                let carrier = (unit.stats.kind as usize).checked_sub(1).and_then(|k| world.unit_types.get(k)).filter(|k| k.passengers > 0);
                if let (Some(carrier), Some(pips), true) = (carrier, &art.cargo_pips, unit.owner == local()) {
                    let slots = carrier.passengers.clamp(0, 12) as usize;
                    // The newest passenger takes the
                    // highest slot of those in use (the sizes summed); a soldier
                    // shows his type's pip, a vehicle one pip of frame 5 and a
                    // pip of frame 4 for each further point of its `Size`.
                    let kind_of = |u: &ra2_sim::Unit| (u.stats.kind as usize).checked_sub(1).and_then(|k| world.unit_types.get(k));
                    let mut frames = vec![0usize; slots + 1];
                    let total: i32 = unit.cargo.iter().filter_map(kind_of).map(|k| k.size).sum();
                    let mut k = (total - 1).min(slots as i32);
                    for passenger in unit.cargo.iter().rev() {
                        let Some(kind) = kind_of(passenger) else {
                            continue;
                        };
                        let mut put = |at: i32, frame: usize| {
                            if let Some(slot) = usize::try_from(at).ok().and_then(|at| frames.get_mut(at)) {
                                *slot = frame;
                            }
                        };
                        if passenger.stats.infantry {
                            put(k, kind.pip as usize);
                        } else {
                            let extra = kind.size - 1;
                            for step in 0..extra.max(0) {
                                put(k - step, 4);
                            }
                            k -= extra.max(0);
                            put(k, 5);
                        }
                        k -= 1;
                    }
                    frames.truncate(slots);
                    let mut canvas = Canvas::new(IVec2::new(-24, 0), IVec2::new(-10 + 4 * slots as i32 + 12, 20));
                    for (slot, frame) in frames.into_iter().enumerate() {
                        let place = if carrier.gunner && slot > 0 { slot as i32 + 2 } else { slot as i32 };
                        canvas.shape(pips, frame, &art.palette, IVec2::new(-15 + 4 * place, 10));
                    }
                    let image = canvas.into_image(&mut images);
                    spawn_overlay(&mut commands, at, &image, FRONT);
                }

                // A unit that sends out spawns has a pip for
                // each, lit (frame 1) for those aboard, ready or reloading; an
                // aircraft with `PipScale=Ammo` and no passengers a bar (frame 12)
                // three pixels higher for each point of ammunition, or with
                // `PipWrap` that many bars coloured by how many rounds of them are
                // full (frames 14 up, 13 empty). Both from the same corner as the
                // other rows, four pixels apart.
                if let (Some(pips), true) = (&art.cargo_pips, unit.owner == local()) {
                    let mut marks: Vec<(usize, IVec2)> = Vec::new();
                    if let Some(spawns) = unit.stats.spawns.filter(|spawns| spawns.number > 0) {
                        let docked = unit.spawner.as_ref().map_or(spawns.number as usize, |spawner| spawner.docked());
                        for i in 0..spawns.number.min(16) as usize {
                            marks.push((usize::from(i < docked), IVec2::new(-15 + 4 * i as i32, 10)));
                        }
                    }
                    let kind = (unit.stats.kind as usize).checked_sub(1).and_then(|k| world.unit_types.get(k));
                    if let (Some(kind), Some(plane), Some(stats)) = (kind.filter(|k| k.pip_scale == 1 && k.passengers <= 0), unit.plane, unit.stats.plane) {
                        let (most, ammo) = (stats.ammo.clamp(0, 16), plane.ammo);
                        if kind.pip_wrap == 0 {
                            let mut lit = if most > 0 { (most as f32 * ammo as f32 / most as f32 + 0.5) as i32 } else { 0 };
                            if lit == 0 && ammo > 0 {
                                lit = 1;
                            }
                            for i in 0..most.min(lit) {
                                marks.push((12, IVec2::new(-15 + 4 * i, 7)));
                            }
                        } else {
                            let wrap = kind.pip_wrap.clamp(1, 16);
                            let levels = stats.ammo / wrap;
                            for i in 0..wrap {
                                let frame = (0..levels).rev().find(|k| k * wrap + i < ammo).map_or(13, |k| 14 + k as usize);
                                marks.push((frame, IVec2::new(-15 + 4 * i, 7)));
                            }
                        }
                    }
                    if !marks.is_empty() {
                        let image = art.images.entry(OverlayKey::Pips(marks.clone())).or_insert_with(|| {
                            let far = marks.iter().map(|(_, at)| at.x).max().unwrap_or(0);
                            let mut canvas = Canvas::new(IVec2::new(-24, 0), IVec2::new(far + 12, 20));
                            for (frame, at) in &marks {
                                canvas.shape(pips, *frame, &art.palette, *at);
                            }
                            canvas.into_image(&mut images)
                        });
                        spawn_overlay(&mut commands, at, image, FRONT);
                    }
                }

                // The line to where it is going, for a moment after the order.
                // A one-pixel line with a
                // three-pixel square at each end.
                // Only with the Options screen's
                // Target Lines (`UnitActionLines`) on.
                let fresh = prefs.target_lines && last_order.0.is_some_and(|tick| world.current_tick() < tick + ACTION_LINE_FRAMES);
                // With a target, only the line to it is drawn.
                let aim = unit.arms.target.filter(|_| unit.arms.attacking).and_then(|target| world.aim_point(target));
                if let (true, true, None, Some(goal)) = (selected, fresh, aim, unit.destination()) {
                    let level = world.map.level(goal.0, goal.1) as i32;
                    let half = LEPTONS_PER_CELL / 2;
                    let end = scene_point(
                        view.origin,
                        goal.0 * LEPTONS_PER_CELL + half,
                        goal.1 * LEPTONS_PER_CELL + half,
                        level,
                    );
                    let delta = (end - at).round().as_ivec2();
                    let image = art.images.entry(OverlayKey::Line(delta)).or_insert_with(|| {
                        let mut canvas = Canvas::new(delta.min(IVec2::ZERO) - 2, delta.max(IVec2::ZERO));
                        canvas.line(IVec2::ZERO, delta, line_rgb);
                        for end in [IVec2::ZERO, delta] {
                            canvas.fill(end - 2, end, line_rgb);
                        }
                        canvas.into_image(&mut images)
                    });
                    spawn_overlay(&mut commands, at, image, FRONT);
                }
                // And to what it is attacking, in entry 8 (red).
                let target_rgb = art.palette.0[8];
                if let (true, true, Some(point)) = (selected, fresh, aim) {
                    let end = scene_point(view.origin, point.0, point.1, 0) - Vec2::new(0.0, point.2 as f32 * 15.0 / ra2_sim::combat::LEVEL_HEIGHT as f32);
                    let delta = (end - at).round().as_ivec2();
                    let image = art.images.entry(OverlayKey::TargetLine(delta)).or_insert_with(|| {
                        let mut canvas = Canvas::new(delta.min(IVec2::ZERO) - 2, delta.max(IVec2::ZERO));
                        canvas.line(IVec2::ZERO, delta, target_rgb);
                        for end in [IVec2::ZERO, delta] {
                            canvas.fill(end - 2, end, target_rgb);
                        }
                        canvas.into_image(&mut images)
                    });
                    spawn_overlay(&mut commands, at, image, FRONT);
                }
                // A selected unit that holds another's mind shows an arc to it
                // in its house's colour: 32 straight pieces between the two,
                // lifted by up to 512 leptons in the middle, a 3-pixel square
                // at each end, and a pulse of white running along it by the
                // clock. Only the controller's selection shows it.
                if let (true, Some(victim)) = (selected, unit.controls.and_then(|id| world.unit(id))) {
                    let from = (unit.x, unit.y, world.ground_z(unit.x, unit.y) + world.altitude(unit));
                    let to = (victim.x, victim.y, world.ground_z(victim.x, victim.y) + world.altitude(victim) + 0x46);
                    let spot = |p: (i32, i32, i32)| scene_point(view.origin, p.0, p.1, 0) - Vec2::new(0.0, p.2 as f32 * LEVEL_HEIGHT as f32 / ra2_sim::combat::LEVEL_HEIGHT as f32);
                    let start = spot(from);
                    let points: Vec<IVec2> = (0..=32)
                        .map(|i| {
                            let mix = |a: i32, b: i32| (a * (32 - i) + b * i) / 32;
                            let lift = ((1.0 - ((i - 16) as f32 / 16.0).powi(2)) * 512.0) as i32;
                            (spot((mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2) + lift)) - start).round().as_ivec2()
                        })
                        .collect();
                    let (low, high) = points.iter().fold((IVec2::ZERO, IVec2::ZERO), |(low, high), p| (low.min(*p), high.max(*p)));
                    let house = view.house_colour(unit.owner);
                    let phase = ((clock.elapsed().as_millis() as u32) >> 4) << 3;
                    let mut canvas = Canvas::new(low - 2, high);
                    for i in 1..=32usize {
                        let v = ((8 * i as u32).wrapping_sub(phase)) & 0x1f8;
                        let rgb = if v > 0 && v <= 0xf8 { house.map(|c| (c as u32 + (255 - c as u32) * v / 256) as u8) } else { house };
                        canvas.line(points[i - 1], points[i], rgb);
                    }
                    for end in [points[0], points[32]] {
                        canvas.fill(end - 2, end, house);
                    }
                    let image = canvas.into_image(&mut images);
                    spawn_overlay(&mut commands, start, &image, FRONT);
                }
            }
            Target::Building(id) => {
                let Some(building) = world.buildings().iter().find(|b| b.id == id) else {
                    continue;
                };
                let Some(kind) = world.building_types.get(building.kind as usize) else {
                    continue;
                };
                let entry = catalog.0.get(building.kind as usize);
                let height = entry.map_or(2, |entry| entry.height);
                let size = building.size;
                let level = world.map.level(building.cell.0, building.cell.1) as i32;
                // The middle of the foundation.
                let at = scene_point(
                    view.origin,
                    building.cell.0 * LEPTONS_PER_CELL + size.0 * LEPTONS_PER_CELL / 2,
                    building.cell.1 * LEPTONS_PER_CELL + size.1 * LEPTONS_PER_CELL / 2,
                    level,
                );
                let (hx, hy, top) = (size.0 * 128, size.1 * 128, height * 100);
                let reach = project(hx, -hy, 0).x + 8;
                let bounds = (
                    IVec2::new(-reach, project(-hx, -hy, height).y - 12),
                    IVec2::new(reach, project(hx, hy, 0).y + 4),
                );

                // The health bar runs along the top
                // left edge of the box at roof height, a pip every (-4, +2).
                let ratio = building.health as f32 / kind.strength.max(1) as f32;
                let (p1, p2) = (project(-hx, hy, height), project(-hx, -hy, height));
                let count = (p1.y - p2.y) / 2;
                let lit = ((ratio * count as f32) as i32).clamp(1, count.max(1));
                let frame = if ratio <= art.red {
                    4
                } else if ratio <= art.yellow {
                    2
                } else {
                    1
                };
                if let Some(pips) = &art.pips {
                    let key = OverlayKey::BuildingBar { size, height, lit, frame };
                    let image = art.images.entry(key).or_insert_with(|| {
                        let mut canvas = Canvas::new(bounds.0, bounds.1);
                        for i in 0..count {
                            let centre = IVec2::new(p1.x + 3 + 4 * count - 4 * i, p1.y + 4 - 2 * count + 2 * i);
                            canvas.shape(pips, if i < lit { frame } else { 0 }, &art.palette, centre);
                        }
                        canvas.into_image(&mut images)
                    });
                    spawn_overlay(&mut commands, at, image, FRONT);
                }
                // With soldiers inside, a
                // pip for each place, filled (PIPS.SHP 7) or empty (6), from the
                // foundation's left corner plus (6, -1), stepping (4, 2).
                if let (Some(pips), false) = (&art.pips, building.occupants.is_empty()) {
                    let corner = project(-hx, hy, 0) + IVec2::new(6, -1);
                    let mut canvas = Canvas::new(bounds.0 - IVec2::new(8, 8), bounds.1 + IVec2::new(4 * kind.max_occupants as i32 + 8, 2 * kind.max_occupants as i32 + 8));
                    for i in 0..kind.max_occupants as i32 {
                        let frame = if (i as usize) < building.occupants.len() { 7 } else { 6 };
                        canvas.shape(pips, frame, &art.palette, corner + IVec2::new(4 * i, 2 * i));
                    }
                    let image = canvas.into_image(&mut images);
                    spawn_overlay(&mut commands, at, &image, FRONT);
                }
                if !selected {
                    continue;
                }

                // A selected factory's rally point: a dashed line (five on,
                // three off, crawling) from the building's middle to the middle
                // of the cell, dark underneath and in the house's colour on top.
                if let Some(rally) = building.rally.filter(|_| building.owner == local()) {
                    let level = |cell: (i32, i32)| world.map.level(cell.0, cell.1) as i32;
                    let from = scene_point(
                        view.origin,
                        building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128,
                        building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128,
                        level(building.cell),
                    );
                    let to = scene_point(view.origin, rally.0 * LEPTONS_PER_CELL + 128, rally.1 * LEPTONS_PER_CELL + 128, level(rally));
                    let delta = (to - from).round().as_ivec2();
                    let phase = ((0x7fff_ffff - world.current_tick() as i64) % 15) as i32;
                    let colour = view.radar_colour_building(id, building.owner);
                    let mut canvas = Canvas::new(delta.min(IVec2::ZERO) - 1, delta.max(IVec2::ZERO) + 3);
                    for (shift, rgb) in [(2, [0, 0, 0]), (1, colour), (0, colour)] {
                        canvas.dashed(IVec2::new(0, shift), delta + IVec2::new(0, shift), rgb, phase);
                    }
                    let image = canvas.into_image(&mut images);
                    spawn_overlay(&mut commands, from, &image, FRONT);
                }

                // The box, drawn as the two
                // end quarters of each edge; the five edges at the back corner go
                // behind the building, the rest in front, without the three
                // quarters that meet at the top front corner.
                let rear = art.images.entry(OverlayKey::BoxRear { size, height }).or_insert_with(|| {
                    let mut canvas = Canvas::new(bounds.0, bounds.1);
                    let back = [-hx, -hy];
                    edge_ends(&mut canvas, [back[0], back[1], 0], [back[0], back[1], top], box_rgb);
                    for z in [0, top] {
                        edge_ends(&mut canvas, [-hx, -hy, z], [hx, -hy, z], box_rgb);
                        edge_ends(&mut canvas, [-hx, -hy, z], [-hx, hy, z], box_rgb);
                    }
                    canvas.into_image(&mut images)
                });
                // Just behind the building's own picture.
                let row = (building.cell.0 + size.0 - 1 + building.cell.1 + size.1 - 1) as f32;
                spawn_overlay(&mut commands, at, rear, 2.0 + row * DEPTH_PER_ROW - DEPTH_PER_ROW / 2.0);
                let front = art.images.entry(OverlayKey::BoxFront { size, height }).or_insert_with(|| {
                    let mut canvas = Canvas::new(bounds.0, bounds.1);
                    edge_ends(&mut canvas, [-hx, hy, 0], [hx, hy, 0], box_rgb);
                    edge_ends(&mut canvas, [hx, hy, 0], [hx, -hy, 0], box_rgb);
                    edge_ends(&mut canvas, [-hx, hy, 0], [-hx, hy, top], box_rgb);
                    edge_ends(&mut canvas, [hx, -hy, 0], [hx, -hy, top], box_rgb);
                    quarter(&mut canvas, [hx, hy, 0], [hx, hy, top / 4], box_rgb);
                    quarter(&mut canvas, [hx, -hy, top], [hx, -hy / 2, top], box_rgb);
                    quarter(&mut canvas, [-hx, hy, top], [-hx / 2, hy, top], box_rgb);
                    canvas.into_image(&mut images)
                });
                spawn_overlay(&mut commands, at, front, FRONT);

                // "Primary" on the primary factory, or the power figures on a
                // power plant, in a black box outlined in the house colour.
                let (output, drain) = world.power(local());
                let text = if building.primary {
                    art.strings.as_ref().and_then(|s| s.get("TXT_PRIMARY")).map(str::to_owned)
                } else if kind.power > 0 {
                    art.strings
                        .as_ref()
                        .and_then(|s| s.get("TXT_POWER_DRAIN2"))
                        .map(|format| format.replacen("%d", &output.to_string(), 1).replacen("%d", &drain.to_string(), 1))
                } else {
                    None
                };
                if let (Some(text), Some(font)) = (text, &art.font) {
                    let rgb = art.house_rgb;
                    let image = art.images.entry(OverlayKey::Text(text.clone())).or_insert_with(|| {
                        let (width, mask) = font.rasterize(&text);
                        let (w, h) = (width as i32, font.height as i32);
                        let corner = IVec2::new(-w / 2 - 4, -2);
                        let far = corner + IVec2::new(w + 7, h + 3);
                        let mut canvas = Canvas::new(corner, far);
                        canvas.fill(corner, far, [0, 0, 0]);
                        for (a, b) in [
                            (corner, IVec2::new(far.x, corner.y)),
                            (IVec2::new(far.x, corner.y), far),
                            (far, IVec2::new(corner.x, far.y)),
                            (IVec2::new(corner.x, far.y), corner),
                        ] {
                            canvas.line(a, b, rgb);
                        }
                        for row in 0..h {
                            for col in 0..w {
                                if mask[(row * w + col) as usize] != 0 {
                                    canvas.put(IVec2::new(-w / 2 + col, row), rgb);
                                }
                            }
                        }
                        canvas.into_image(&mut images)
                    });
                    spawn_overlay(&mut commands, at, image, FRONT + 1.0);
                }
            }
        }
    }
}
