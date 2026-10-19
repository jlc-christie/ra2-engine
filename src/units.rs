//! Vehicles in the running game: the simulation that moves them, the sprites that
//! show them, and the mouse controls that order them about.
//!
//! Mouse control and what is drawn over the map for it are in `control.rs`.

mod effects;

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;

use crate::lighting::{LightRule, Lit, StormLight, WorldLight};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ra2_sim::{LEPTONS_PER_CELL, Map, Order, TICKS_PER_SECOND, World};

use crate::files::GameFiles;
use crate::depth::Depth;
use crate::game::DEPTH_PER_ROW;
use crate::shell::Screen;
use crate::audio::Cue;
use crate::terrain::{
    CELL_HEIGHT, CELL_WIDTH, LEVEL_HEIGHT, local, UnitArt,
};

/// Credits the player starts with.
const STARTING_CREDITS: i32 = 10_000;

/// What the game shows for each building type, by type index.
#[derive(Resource)]
pub struct BuildCatalog(pub Vec<CatalogEntry>);

pub struct CatalogEntry {
    /// The type's name in rules.ini.
    pub name: String,
    /// The file name of the sidebar picture.
    pub cameo: String,
    /// Key of its name in the string table.
    pub ui_name: String,
    /// How tall its selection box is, in levels.
    pub height: i32,
    pub construction_yard: bool,
}

/// What the sidebar needs to know of a unit type.
pub struct UnitEntry {
    pub name: String,
    pub cameo: String,
    pub alt_cameo: Option<String>,
    pub ui_name: String,
    pub cost: i32,
}

/// The unit types, as the simulation numbers them.
#[derive(Resource)]
pub struct UnitCatalog(pub Vec<UnitEntry>);

/// The super weapon types: name, the key of the name shown, sidebar picture.
#[derive(Resource, Default)]
pub struct SuperCatalog(pub Vec<(String, String, String)>);

/// The building type the player is choosing a spot for, if any.
#[derive(Resource, Default)]
pub struct Placing(pub Option<u16>);

/// The running simulation.
#[derive(Resource)]
// The second part is the orders given since the last tick, which are carried
// out, in the order given, as the next one starts: the one place orders enter
// the simulation (and where a network game will send and receive them). The
// third is every order carried out, with its tick.
pub struct Sim(pub World, pub Vec<Order>, pub Vec<(u64, Order)>);

impl Sim {
    /// Asks for an order to be carried out at the start of the next tick.
    pub fn give(&mut self, order: Order) {
        self.1.push(order);
    }
}

/// A drawn vehicle image and where the model's origin falls within it.
struct UnitImage {
    handle: Handle<Image>,
    origin: Vec2,
    size: Vec2,
}

/// What is needed to draw the simulation's vehicles.
#[derive(Resource)]
pub(crate) struct UnitView {
    art: UnitArt,
    /// Scene pixel of cell (0, 0)'s corner at height 0.
    pub(crate) origin: Vec2,
    /// The palettes the map gave the units it placed, for those no house owns.
    placed_palettes: HashMap<u32, Option<usize>>,
    /// Model and voices of vehicle types that can appear during play.
    kinds: Vec<Kind>,
    /// `BuildingSlam`: the sound of a building going down.
    pub(crate) slam_sound: Option<String>,
    /// `GenericClick`, `ScoldSound` and `SellSound`.
    pub(crate) click_sound: Option<String>,
    pub(crate) scold_sound: Option<String>,
    sell_sound: Option<String>,
    /// What each building type is drawn from.
    building_art: Vec<BuildingArt>,
    anim_types: HashMap<String, crate::terrain::AnimType>,
    /// The animation types' names by the numbers the sim uses.
    anim_names: Vec<String>,
    /// Owner palettes of the buildings the map started with, by building id.
    building_palettes: HashMap<u32, Option<usize>>,
    /// Health, as a fraction, at or below which a building shows damage.
    condition_yellow: f32,
    /// Art of each wall overlay type, and the wall version last drawn.
    wall_art: HashMap<u16, String>,
    /// Art of each bridge overlay type, and the bridge version last drawn.
    bridge_art: HashMap<u8, String>,
    bridges_shown: Option<u64>,
    smudge_names: Vec<String>,
    walls_shown: Option<u64>,
    /// The art each ore cell is drawn with, and the sprites showing ore now, with
    /// the amount each shows.
    ore_art: HashMap<(i32, i32), String>,
    /// The tick the walk counters and blast rocks were last stepped on.
    walk_tick: u64,
    /// Shadows, by picture, with where each one's middle lies from the draw
    /// point.
    shadow_images: HashMap<ShadowKey, Option<(Handle<Image>, Vec2)>>,
    /// Pictures of one-off animations and of shots, by art and frame.
    effect_images: HashMap<(String, usize), Option<(Handle<Image>, Vec2)>>,
    ore_shown: Option<u64>,
    ore_images: HashMap<(String, usize), Option<(Handle<Image>, Vec2)>>,
    ore_sprites: HashMap<(i32, i32), (Entity, u8)>,
    /// The art of each kind of ground mark, the marks drawn, and the map's
    /// count of changes when they were.
    smudge_sprites: HashMap<(i32, i32), Entity>,
    smudges_shown: Option<u64>,
    /// The game speed setting (0 fastest), which paces `Normalized` animations.
    game_speed: i32,
    /// Drawn building images by art, frame, palette, height level and whether it
    /// is the shadow, with where each goes relative to the building's draw point.
    building_images: HashMap<PartKey, Option<(Handle<Image>, Vec2)>>,
    /// Each building's animation clock: when it was offline, and its super
    /// weapon's stage.
    building_clocks: HashMap<u32, AnimClock>,
    /// The player's palette, and every house's by owner.
    player_palette: Option<usize>,
    house_palettes: Vec<Option<usize>>,
    /// Drawn images by model, direction (0 to 31), palette and height level.
    images: HashMap<(String, u8, Option<u8>, Option<usize>, i32), Option<UnitImage>>,
    /// The tick the drone rocks were last stepped on.
    rock_tick: u64,
    /// The crates drawn, and the shroud's count of changes when they were.
    crates_shown: Option<(Vec<((i32, i32), bool)>, u64)>,
    /// The Mirages drawn as trees: unit, tree and cell.
    mirages_shown: Option<Vec<(u32, u16, (i32, i32))>>,
    /// Crate pictures by water and height level.
    crate_images: HashMap<(bool, i32), Option<(Handle<Image>, Vec2)>>,
}

/// What is needed to show and voice a unit type.
struct Kind {
    /// Model (or, for a foot soldier, shape) file name.
    image: String,
    voice_select: Option<String>,
    voice_move: Option<String>,
    /// A foot soldier's sequences by name in lower case: first frame, frames,
    /// frames per facing.
    sequences: Option<HashMap<String, (usize, usize, usize)>>,
    considered_aircraft: bool,
    transport_sounds: (Option<String>, Option<String>),
    chrono_sounds: (Option<String>, Option<String>),
    aux_sounds: (Option<String>, Option<String>),
    roll_angle: f32,
    deploy_sounds: (Option<String>, Option<String>),
    unloading: Option<String>,
    sheet: Option<crate::terrain::Sheet>,
    shadow_index: usize,
    no_shadow: bool,
    weight: f64,
    charge_turrets: u8,
}

/// What a building type is drawn from.
struct BuildingArt {
    art: String,
    buildup: Option<String>,
    bib: Option<String>,
    deploying: Option<String>,
    under_door: Option<String>,
    turret: Option<(String, i32, i32)>,
    turret_z_adjust: i32,
    zshape_move: (i32, i32),
    normal_z_adjust: i32,
    anims: Vec<crate::terrain::AnimSlot>,
    charged_anim_time: f32,
}

/// Art name, frame, palette, height level, shadow.
type PartKey = (String, usize, Option<usize>, i32, bool);

/// A unit's entity, by the simulation's id: its sprite, and what is kept about
/// how it is shown.
#[derive(Component)]
pub(crate) struct SimUnit(pub(crate) u32);

/// The entity of each unit on the map, by id.
#[derive(Resource, Default)]
pub(crate) struct UnitEntities(pub(crate) HashMap<u32, Entity>);

/// A sprite vehicle's walk counter, or a model's animation counter.
#[derive(Component, Default)]
struct WalkCounter(u32);

/// The ramp under a vehicle, the one before it, and the tick it changed.
#[derive(Component)]
struct Ramp {
    now: u8,
    before: u8,
    since: u64,
}

/// Gives each unit on the map an entity and takes it away when the unit goes
/// (dies, deploys, or boards something).
fn mirror_units(mut commands: Commands, sim: Res<Sim>, mut entities: ResMut<UnitEntities>, shadows: Query<&ShadowSprite>) {
    entities.0.retain(|&id, &mut entity| {
        let here = sim.0.unit(id).is_some();
        if !here {
            if let Ok(shadow) = shadows.get(entity) {
                commands.entity(shadow.0).despawn();
            }
            commands.entity(entity).despawn();
        }
        here
    });
    for unit in sim.0.units() {
        entities.0.entry(unit.id).or_insert_with(|| {
            let shadow = commands.spawn((DespawnOnExit(Screen::InGame), UnitShadow, Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0), Visibility::Hidden)).id();
            commands.spawn((DespawnOnExit(Screen::InGame), SimUnit(unit.id), WalkCounter::default(), ShadowSprite(shadow), Sprite::default())).id()
        });
    }
}

/// A vehicle's rocking: its angles about its length and its width, in
/// radians, and how fast each is changing a tick.
#[derive(Component, Clone, Copy, Debug, Default)]
struct Rocking {
    roll: f32,
    pitch: f32,
    roll_rate: f32,
    pitch_rate: f32,
}

impl Rocking {
    /// A blast's push from a point, by its strength, on a vehicle at a place
    /// facing a way (16-bit) and of a `Weight`.
    // The push is 0.04 less 0.000025 a
    // lepton of distance, times the strength, over the weight; nothing under 0.01
    // (or right on the point), no more than 0.05. It is split between the
    // vehicle's length and width by the direction from the point: along its
    // length it sets the pitch's rate to half of it, across, the roll's to all of
    // it.
    fn push(&mut self, at: (i32, i32, i32), from: (i32, i32, i32), strength: f32, facing: u16, weight: f64) {
        let d = ((at.0 - from.0) as f32, (from.1 - at.1) as f32, (at.2 - from.2) as f32);
        let distance = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
        let push = ((0.04 - distance * 0.000_025) * strength) as f64 / weight;
        if distance < 2e-5 || push < 0.01 {
            return;
        }
        let push = push.min(0.05) as f32;
        let flat = (d.0 * d.0 + d.1 * d.1).sqrt();
        let (ux, uy) = if flat == 0.0 { (d.0, d.1) } else { (d.0 / flat, d.1 / flat) };
        let angle = (facing as i32 - 0x3fff) as f64 * (-std::f64::consts::TAU / 65536.0);
        let (sin, cos) = (angle.sin() as f32, angle.cos() as f32);
        let along = uy * sin + ux * cos;
        let across = uy * cos - ux * sin;
        self.pitch_rate = along * push * 0.5;
        self.roll_rate = -across * push;
    }

    /// One tick of the swing: each angle moves by its rate, no further than an
    /// eighth of a turn either way; the rate is pulled back towards the level
    /// by 0.002 a tick while it still moves away from it and 0.005 once it
    /// heads back; on crossing the level, angle and rate both stop.
    fn step(&mut self) {
        fn axis(angle: &mut f32, rate: &mut f32) {
            const EPSILON: f32 = 2e-5;
            let before = *angle;
            if *rate == 0.0 {
                *angle = 0.0;
                return;
            }
            *angle += *rate;
            if *angle > std::f32::consts::FRAC_PI_4 {
                *angle = std::f32::consts::FRAC_PI_4;
                *rate = 0.0;
            } else if *angle < -std::f32::consts::FRAC_PI_4 {
                *angle = -std::f32::consts::FRAC_PI_4;
                *rate = 0.0;
            }
            if before > EPSILON {
                *rate -= if *rate <= 0.0 { 0.005 } else { 0.002 };
                if *angle < EPSILON {
                    (*angle, *rate) = (0.0, 0.0);
                }
            } else {
                *rate += if *rate >= 0.0 { 0.005 } else { 0.002 };
                if before < -EPSILON && *angle > -EPSILON {
                    (*angle, *rate) = (0.0, 0.0);
                }
            }
        }
        axis(&mut self.roll, &mut self.roll_rate);
        axis(&mut self.pitch, &mut self.pitch_rate);
    }

    /// Whether it is still far enough off the level to be drawn rocked.
    fn showing(&self) -> bool {
        self.roll.abs() >= 0.005 || self.pitch.abs() >= 0.005
    }
}

/// The sprite of the shadow a unit casts.
#[derive(Component)]
struct UnitShadow;

/// A unit's shadow sprite, made and taken away with it.
#[derive(Component)]
struct ShadowSprite(Entity);

/// A shadow's picture: a shape's frame drawn dark ("shp:<art>:<frame>"), or a
/// model's section on a ramp at a direction.
type ShadowKey = (String, u8);

/// One of the sprites a building is drawn from: the building's id, which of its
/// parts this is, and what it is showing.
#[derive(Component)]
struct BuildingSprite(u32, usize, Option<PartKey>);

pub struct UnitsPlugin;

impl UnitView {
    /// A unit type's `DeploySound`, or its `UndeploySound`.
    pub(crate) fn deploy_sound(&self, kind: u16, undeploy: bool) -> Option<String> {
        let kind = self.kinds.get((kind as usize).checked_sub(1)?)?;
        if undeploy { kind.deploy_sounds.1.clone() } else { kind.deploy_sounds.0.clone() }
    }

    /// The palette a house's things are drawn with.
    fn palette_of(&self, owner: u8) -> Option<usize> {
        self.house_palettes.get(owner as usize).copied().flatten()
    }

    /// The palette a unit is drawn in: its house's, or for one no house owns,
    /// the one the map gave it.
    fn unit_palette(&self, id: u32, owner: u8) -> Option<usize> {
        match self.placed_palettes.get(&id) {
            Some(&palette) if owner == 0 => palette,
            _ => self.palette_of(owner),
        }
    }

    /// What a unit of a type says when selected and when sent somewhere.
    pub(crate) fn voice(&self, world: &World, id: u32, moving: bool) -> Option<String> {
        let kind = world.unit(id).and_then(|unit| (unit.stats.kind as usize).checked_sub(1)).and_then(|kind| self.kinds.get(kind))?;
        if moving { kind.voice_move.clone() } else { kind.voice_select.clone() }
    }

    /// The palette a house's things are drawn with.
    pub(crate) fn house_palette(&self, owner: u8) -> Option<&ra2_formats::pal::Palette> {
        self.art.owner_palette(self.palette_of(owner))
    }

    /// A house's colour.
    pub(crate) fn house_colour(&self, owner: u8) -> [u8; 3] {
        self.art.owner_colour(self.palette_of(owner))
    }

    /// The player's house colour.
    pub(crate) fn player_colour(&self) -> [u8; 3] {
        if local() != crate::terrain::PLAYER {
            return self.house_colour(local());
        }
        self.art.owner_colour(self.player_palette)
    }

    /// Where the map's corner is in the scene.
    pub(crate) fn origin(&self) -> Vec2 {
        self.origin
    }

    pub(crate) fn game_speed(&self) -> i32 {
        self.game_speed
    }

    /// The colour a unit or building shows in on the radar.
    pub(crate) fn radar_colour_unit(&self, id: u32, owner: u8) -> [u8; 3] {
        self.art.owner_colour(self.unit_palette(id, owner))
    }

    /// A building's: its owner's colour, or for a neutral one the colour of
    /// the house the map gave it to.
    pub(crate) fn radar_colour_building(&self, id: u32, owner: u8) -> [u8; 3] {
        let palette = match self.building_palettes.get(&id) {
            Some(palette) if owner == 0 => *palette,
            _ => self.palette_of(owner),
        };
        self.art.owner_colour(palette)
    }
}

impl Plugin for UnitsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(TICKS_PER_SECOND as f64))
            .init_resource::<Placing>()
            .add_message::<SimEvent>()
            .init_resource::<effects::EffectRng>()
            .init_resource::<UnitEntities>()
            .add_systems(Update, publish_events.run_if(in_state(Screen::InGame)).run_if(resource_exists::<Sim>).in_set(crate::schedule::GameSet::Events))
            .add_systems(
                FixedUpdate,
                tick.run_if(in_state(Screen::InGame)).run_if(resource_exists::<Sim>).run_if(crate::pause::playing).run_if(not(resource_exists::<crate::net::NetActive>)),
            )
            .add_systems(
                Update,
                (auto_build.run_if(not(resource_exists::<crate::net::NetSeat>)), announce, (mirror_units, rock_units).chain(), show_units, show_buildings, show_walls, show_crates, show_mirages, show_bridges, show_ore, (effects::effect_systems(), step_effects, show_shots, end_beams).chain(), show_markers, show_corpses, show_debris, show_sinking, show_sparks, light_world, light_units, crate::lighting::apply)
                    .chain()
                    .run_if(in_state(Screen::InGame))
                    .run_if(resource_exists::<UnitView>)
                    .in_set(crate::schedule::GameSet::View),
            )
            .add_systems(OnExit(Screen::InGame), clear);
    }
}

/// A simulation made from a map's scene, with what is needed to show its contents.
pub struct BuiltWorld {
    pub world: World,
    pub catalog: Vec<CatalogEntry>,
    pub unit_catalog: Vec<UnitEntry>,
    building_art: Vec<BuildingArt>,
    anim_types: HashMap<String, crate::terrain::AnimType>,
    /// The animation types' names by the numbers the sim uses.
    anim_names: Vec<String>,
    building_palettes: HashMap<u32, Option<usize>>,
    condition_yellow: f32,
    /// Each house's colours, by owner.
    house_palettes: Vec<Option<usize>>,
    wall_art: HashMap<u16, String>,
    bridge_art: HashMap<u8, String>,
    smudge_names: Vec<String>,
    super_catalog: Vec<(String, String, String)>,
    ore_art: HashMap<(i32, i32), String>,
    /// Model and voices of vehicle types that can appear during play.
    kinds: Vec<Kind>,
    /// The palette the map gave each unit it placed, by id.
    placed_palettes: HashMap<u32, Option<usize>>,
}

/// Makes the simulation for a freshly built scene, taking what it needs out of it.
/// Nothing here touches the display, so tests can run a real map without a window.
pub fn build_world(files: &GameFiles, scene: &mut crate::terrain::Scene) -> BuiltWorld {
    let cells = &scene.cells;
    let width = cells.iter().map(|cell| cell.x).max().unwrap_or(0) + 1;
    let height = cells.iter().map(|cell| cell.y).max().unwrap_or(0) + 1;
    let mut map = Map::new(width, height);
    for cell in cells {
        map.set_cell(cell.x, cell.y, cell.passable, cell.level);
        map.set_land(cell.x, cell.y, cell.land);
        map.set_ground(cell.x, cell.y, cell.ramp, cell.overlay);
        if cell.deck != 0 {
            map.set_bridge(cell.x, cell.y, cell.deck);
        }
        if let Some((kind, data)) = cell.bridge {
            map.set_bridge_overlay(cell.x, cell.y, kind, data, cell.base_land);
        }
        map.set_obstacle(cell.x, cell.y, cell.obstacle);
        map.set_tree_bits(cell.x, cell.y, cell.tree_bits);
        map.set_ore_ground(cell.x, cell.y, cell.ore_ground);
    }
    let mended = map.complete_bridges();
    if mended > 0 {
        info!("bridges: {mended} sections already fallen when the map starts");
    }
    let mut ore_art = HashMap::new();
    for ore in std::mem::take(&mut scene.ore) {
        map.put_ore(ore.cell.0, ore.cell.1, ore.kind, ore.amount);
        ore_art.insert(ore.cell, ore.art);
    }
    map.lands = scene.land_rules.clone();
    if let Some((width, height, local)) = scene.playfield {
        map.set_playfield(width, height, local);
    }
    // The drive tracks are engine data, read from the user's own executable.
    let tracks = files
        .engine_exe()
        .and_then(|exe| ra2_sim::TrackTables::from_exe(&exe));
    if tracks.is_none() {
        warn!("drive tracks not found in game.exe; vehicles will turn on the spot");
    }
    let mut world = World::new(map, tracks);
    world.blockage_delay = scene.blockage_delay;
    world.sight_height_step = scene.sight_height_step;
    world.build_rules = scene.build_rules;
    world.houses = vec![
        ra2_sim::House::default(),
        ra2_sim::House {
            credits: STARTING_CREDITS,
            tech_level: scene.tech_level,
            country: scene.player_country,
            side: scene.player_side,
            human: true,
            base_centre: scene.player_start.map_or((0, 0), |cell| (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128)),
            ..ra2_sim::House::default()
        },
    ];
    // The computer houses, with the same
    // credits and tech level, each base taken to be at its start cell.
    // RA2_NOENEMY=1 leaves them out, for checks.
    let mut house_palettes: Vec<Option<usize>> = scene.house_palettes.iter().take(2).copied().collect();
    if std::env::var("RA2_NOENEMY").is_err() {
        for enemy in &scene.enemies {
            house_palettes.push(enemy.palette);
            world.houses.push(ra2_sim::House {
                credits: STARTING_CREDITS,
                tech_level: scene.tech_level,
                country: enemy.country,
                side: enemy.side,
                base_centre: (enemy.start.0 * LEPTONS_PER_CELL + 128, enemy.start.1 * LEPTONS_PER_CELL + 128),
                // A row played by a person on another machine has no brain.
                ai: (enemy.level < crate::net::HUMAN_ROW as usize).then(|| ra2_sim::ai::Brain::of(enemy.level)),
                human: enemy.level >= crate::net::HUMAN_ROW as usize,
                ..ra2_sim::House::default()
            });
        }
    }
    if scene.allies_allowed {
        world.paranoid_alliances();
    }
    let mut catalog = Vec::new();
    world.mcv_repacks = scene.mcv_repacks;
    world.buildup_ticks = scene.buildup_ticks;
    world.weapons = scene.armoury.weapons.clone();
    world.warheads = scene.armoury.warheads.clone();
    world.projectiles = scene.armoury.projectiles.clone();
    (world.gravity, world.max_damage, world.missile_rot_var, world.targeting_delay) = scene.combat_rules;
    world.elevation = scene.elevation;
    world.ballistic_scatter = scene.ballistic_scatter;
    world.condition_red = scene.condition_red as f64;
    world.idle_frequency = scene.idle_frequency;
    world.map.slope_speed = scene.slope_speed;
    world.map.condition_yellow = scene.condition_yellow as f64;
    world.ai_rules = scene.ai_rules.clone();
    world.ability_rules = scene.ability_rules.clone();
    world.rockets = scene.rockets;
    world.rad_rules = scene.rad_rules;
    (world.bridges_destroyable, world.bridge_strength) = scene.bridge_rules;
    world.veterancy = scene.veterancy;
    world.team_data = scene.team_data.clone();
    world.super_types = scene.super_types.iter().map(|info| info.sim).collect();
    world.super_rules = scene.super_rules.clone();
    world.crate_rules = scene.crate_rules.clone();
    world.map.smudge_types = scene.smudge_types.iter().map(|(_, kind)| *kind).collect();
    for &(cell, kind) in &scene.map_smudges {
        world.map.place_smudge(cell, kind);
    }
    // DATA: `[MultiplayerDialogSettings] Shroud=yes`.
    world.map.shroud_on = true;
    world.chrono = scene.chrono;
    let upkeep = &scene.upkeep;
    world.refund_percent = upkeep.refund_percent;
    world.repair_percent = upkeep.repair_percent;
    world.repair_step = upkeep.repair_step;
    world.repair_interval = upkeep.repair_interval;
    world.survivor_divisors = upkeep.survivor_divisors;
    world.crew_types = upkeep.crew_types;
    let mining = &scene.mining;
    world.ore_values = mining.values.clone();
    world.set_ore_types(mining.kinds.clone());
    world.ore_near_scan = mining.near_scan;
    world.ore_far_scan = mining.far_scan;
    world.harvester_load_rate = mining.load_rate;
    world.harvester_dump_rate = mining.dump_rate;
    world.harvester_too_far = mining.too_far;
    world.chrono_too_far = mining.chrono_too_far;
    for animation in &scene.animations {
        // RA2_ANIMATE=1 keeps them playing, for checking the animation.
        let probability = if std::env::var_os("RA2_ANIMATE").is_some() { 1.0 } else { animation.probability };
        world.add_animated_terrain(animation.cell, animation.steps.len() as u32, animation.rate, probability);
    }
    // Vehicle types, by the tag in a unit's stats less one.
    let mut kinds = Vec::new();
    let mut unit_catalog = Vec::new();
    for info in std::mem::take(&mut scene.unit_types) {
        kinds.push(Kind {
            image: info.image,
            voice_select: info.voice_select,
            voice_move: info.voice_move,
            sequences: info.sequences,
            considered_aircraft: info.considered_aircraft,
            transport_sounds: info.transport_sounds,
            chrono_sounds: info.chrono_sounds,
            aux_sounds: info.aux_sounds,
            roll_angle: info.roll_angle,
            deploy_sounds: info.deploy_sounds,
            unloading: info.unloading,
            sheet: info.sheet,
            shadow_index: info.shadow_index,
            no_shadow: info.no_shadow,
            weight: info.weight,
            charge_turrets: info.charge_turrets,
        });
        unit_catalog.push(UnitEntry { name: info.name, cameo: info.cameo, alt_cameo: info.alt_cameo, ui_name: info.ui_name, cost: info.sim.cost });
        world.unit_types.push(info.sim);
    }
    let mut building_art = Vec::new();
    for mut info in std::mem::take(&mut scene.building_types) {
        if let Some(spawn) = info.undeploys.take() {
            info.sim.undeploys_into = Some(spawn.stats);
        }
        let construction_yard = info.sim.construction_yard;
        building_art.push(BuildingArt {
            art: info.art.clone(),
            buildup: info.buildup.clone(),
            bib: info.bib.take(),
            deploying: info.deploying.take(),
            turret: info.turret.take(),
            turret_z_adjust: info.turret_z_adjust,
            zshape_move: info.zshape_move,
            normal_z_adjust: info.normal_z_adjust,
            under_door: info.under_door.take(),
            anims: std::mem::take(&mut info.anims),
            charged_anim_time: info.charged_anim_time,
        });
        world.building_types.push(info.sim);
        catalog.push(CatalogEntry {
            name: info.name,
            cameo: info.cameo,
            ui_name: info.ui_name,
            height: info.height,
            construction_yard,
        });
    }
    let mut placed_palettes = HashMap::new();
    for spawn in std::mem::take(&mut scene.unit_spawns) {
        let id = world.add_unit(spawn.cell, spawn.facing, spawn.stats, spawn.owner);
        placed_palettes.insert(id, spawn.palette);
    }
    // The foot soldiers the map starts with.
    for soldier in std::mem::take(&mut scene.infantry_spawns) {
        let Some(kind) = world.unit_types.get(soldier.kind as usize) else {
            continue;
        };
        let stats = kind.stats;
        if let Some(id) = world.add_infantry(soldier.cell, soldier.spot, soldier.facing, stats, crate::terrain::NEUTRAL) {
            placed_palettes.insert(id, soldier.palette);
        }
    }
    // The buildings the map starts with.
    let mut building_palettes = HashMap::new();
    for structure in &scene.map_structures {
        let id = world.add_map_building(crate::terrain::NEUTRAL, structure.kind, structure.cell, structure.health);
        building_palettes.insert(id, structure.palette);
    }
    for &(cell, overlay, data) in &scene.walls {
        world.add_map_wall(cell, overlay, data);
    }
    if !scene.walls.is_empty() {
        world.map.build_zones();
    }
    // The animation types by number, for the sim, which chooses explosions and
    // marks the ground under them; the view plays what it is told by name.
    let mut anim_names: Vec<String> = scene.anim_types.keys().cloned().collect();
    anim_names.sort();
    let number = |name: &String| anim_names.binary_search(&name.to_uppercase()).map_or(ra2_sim::smudge::NO_ANIM, |at| at as u16);
    world.anim_marks = anim_names
        .iter()
        .map(|name| {
            let anim = &scene.anim_types[name];
            // The stage counts images from `Start`, one each `900 / Rate`
            // frames.
            let after = match anim.middle.checked_sub(anim.start) {
                _ if anim.middle == 0 => Some(0),
                Some(stages) => Some(stages as u32 * anim.delay.max(1)),
                None => None,
            };
            ra2_sim::smudge::AnimMark { scorch: anim.scorch, crater: anim.crater, force_big: anim.force_big_craters, size: anim.size, after }
        })
        .collect();
    world.splash_anims = scene.armoury.splashes.iter().map(number).collect();
    let known = |name: &str| Some(number(&name.to_owned())).filter(|&at| at != ra2_sim::smudge::NO_ANIM);
    world.super_rules.nuke_ball.1 = Some("NUKEBALL").and_then(known);
    world.super_rules.storm.bolt_explosion = scene.bolt_explosion.as_deref().and_then(known);
    for (warhead, look) in world.warheads.iter_mut().zip(&scene.armoury.warhead_looks) {
        warhead.anim_list = look.anims.iter().map(number).collect();
    }
    let explosions = |name: &str| scene.armoury.explosions.get(&name.to_lowercase()).map(|list| list.iter().map(number).collect::<Vec<u16>>()).unwrap_or_default();
    for (kind, entry) in world.unit_types.iter_mut().zip(&unit_catalog) {
        kind.explosions = explosions(&entry.name);
    }
    for (kind, entry) in world.building_types.iter_mut().zip(&catalog) {
        kind.explosions = explosions(&entry.name);
    }
    // What dying things throw and leave, by number.
    let armoury = &scene.armoury;
    let named = |list: &[String]| list.iter().map(number).collect::<Vec<u16>>();
    let maybe = |name: &Option<String>| name.as_ref().map_or(ra2_sim::smudge::NO_ANIM, number);
    let voxel = |name: &String| armoury.voxel_debris.iter().position(|v| v.name.eq_ignore_ascii_case(name)).map(|i| i as u16);
    let leavings = |name: &str| {
        armoury.leavings.get(&name.to_lowercase()).map_or_else(Default::default, |l| ra2_sim::remains::Leavings {
            min: l.min,
            max: l.max,
            types: l.types.iter().filter_map(voxel).collect(),
            maximums: l.maximums.clone(),
            anims: named(&l.anims),
        })
    };
    for (kind, entry) in world.unit_types.iter_mut().zip(&unit_catalog) {
        kind.leavings = leavings(&entry.name);
        kind.death_anims = armoury.death_anims.get(&entry.name.to_lowercase()).map(|list| named(list)).unwrap_or_default();
    }
    for (kind, entry) in world.building_types.iter_mut().zip(&catalog) {
        kind.leavings = leavings(&entry.name);
    }
    world.remains_rules = ra2_sim::remains::Rules {
        crew_escape: scene.crew_escape,
        ship_sinking_weight: scene.ship_sinking_weight,
        wake: maybe(&armoury.wake),
        metallic_debris: named(&armoury.metallic_debris),
        infantry_explode: maybe(&armoury.infantry_deaths[0]),
        bouncers: anim_names
            .iter()
            .map(|name| {
                let (motion, [expire, bounce, trailer]) = scene.anim_types[name].bouncer.clone()?;
                Some(ra2_sim::remains::Bouncer { expire: maybe(&expire), bounce: maybe(&bounce), trailer: maybe(&trailer), ..motion })
            })
            .collect(),
        voxels: armoury
            .voxel_debris
            .iter()
            .map(|v| ra2_sim::remains::VoxelDebris {
                image: v.image.clone(),
                motion: ra2_sim::remains::Bouncer { expire: maybe(&v.expire), bounce: maybe(&v.bounce), trailer: maybe(&v.trailer), ..v.motion.clone() },
            })
            .collect(),
    };
    BuiltWorld {
        anim_names,
        unit_catalog,
        wall_art: std::mem::take(&mut scene.wall_art),
        bridge_art: std::mem::take(&mut scene.bridge_art),
        smudge_names: scene.smudge_types.iter().map(|(name, _)| name.clone()).collect(),
        super_catalog: scene.super_types.iter().map(|info| (info.name.clone(), info.ui_name.clone(), info.cameo.clone())).collect(),
        ore_art,
        world,
        catalog,
        kinds,
        placed_palettes,
        building_art,
        anim_types: std::mem::take(&mut scene.anim_types),
        building_palettes,
        condition_yellow: scene.condition_yellow,
        house_palettes,
    }
}

/// Starts the game on a built simulation: a sprite per vehicle, and the test hooks.
pub fn setup(
    commands: &mut Commands,
    rules: &crate::files::Rules,
    built: BuiltWorld,
    player_palette: Option<usize>,
    art: UnitArt,
    origin: (i32, i32),
    game_speed: i32,
) {
    let BuiltWorld {
        mut world,
        catalog,
        unit_catalog,
        kinds,
        placed_palettes,
        building_art,
        anim_types,
        anim_names,
        building_palettes,
        condition_yellow,
        house_palettes,
        wall_art,
        bridge_art,
        smudge_names,
        super_catalog,
        ore_art,
    } = built;
    commands.insert_resource(BuildCatalog(catalog));
    commands.insert_resource(UnitCatalog(unit_catalog));
    commands.insert_resource(SuperCatalog(super_catalog));
    commands.insert_resource(Placing::default());

    // RA2_MOVE=unit,x,y orders a unit to a cell at the start, for checking movement
    // without a mouse. Several orders can be given, separated by semicolons.
    // RA2_BUILD=type starts a building type in production once it becomes buildable,
    // and RA2_PLACE=x,y places it there when it is ready; for checking without a
    // mouse.
    if let Some(building) = std::env::var("RA2_BUILD").ok().and_then(|n| n.trim().parse().ok()) {
        commands.insert_resource(AutoBuild(building));
    }
    // RA2_DEPLOY=unit deploys a unit at the start; RA2_DEPLOY=mcv the player's
    // construction vehicle.
    let deploy = std::env::var("RA2_DEPLOY").ok().and_then(|n| match n.trim() {
        "mcv" => world.units().iter().find(|u| u.owner == crate::terrain::PLAYER && u.stats.deploys_into.is_some()).map(|u| u.id),
        n => n.parse().ok(),
    });
    if let Some(unit) = deploy {
        info!("RA2_DEPLOY: unit {unit}, can deploy: {}", world.can_deploy(unit));
        world.order(Order::Deploy { unit });
    }
    for order in std::env::var("RA2_MOVE").unwrap_or_default().split(';') {
        let order: Vec<i32> = order.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        let [unit, x, y] = order[..] else {
            continue;
        };
        world.order(Order::Move { unit: unit as u32, x, y });
        if let Some(moved) = world.unit(unit as u32) {
            info!(
                "RA2_MOVE: unit {unit} at {:?} to ({x}, {y}) (cell open: {}): {}",
                moved.cell(),
                world.map.passable(x, y),
                if moved.is_moving() { "route found" } else { "no route" }
            );
        }
    }

    if let Some(zones) = world.map.zones() {
        let (regions, areas, subzones) = zones.counts();
        info!("pathfinding: {regions} regions, {areas} separate areas for ground vehicles, subzones {subzones:?}");
    }
    commands.insert_resource(Sim(world, Vec::new(), Vec::new()));
    commands.insert_resource(UnitView {
        art,
        origin: Vec2::new(origin.0 as f32, origin.1 as f32),
        placed_palettes,
        kinds,
        slam_sound: rules.sound("BuildingSlam"),
        click_sound: rules.sound("GenericClick"),
        scold_sound: rules.sound("ScoldSound"),
        sell_sound: rules.sound("SellSound"),
        building_art,
        anim_types,
        anim_names,
        building_palettes,
        condition_yellow,
        house_palettes,
        game_speed,
        wall_art,
        bridge_art,
        bridges_shown: None,
        smudge_names,
        walls_shown: None,
        ore_art,
        shadow_images: HashMap::new(),
        rock_tick: 0,
        crates_shown: None,
        mirages_shown: None,
        crate_images: HashMap::new(),
        walk_tick: 0,
        effect_images: HashMap::new(),
        ore_shown: None,
        ore_images: HashMap::new(),
        ore_sprites: HashMap::new(),
        smudge_sprites: HashMap::new(),
        smudges_shown: None,
        building_images: HashMap::new(),
        building_clocks: HashMap::new(),
        player_palette,
        images: HashMap::new(),
    });
}

fn clear(mut commands: Commands) {
    commands.remove_resource::<Sim>();
    commands.remove_resource::<UnitView>();
}

fn tick(sim: ResMut<Sim>) {
    // RA2_SPEED=n runs the simulation n times as fast, for getting to a later
    // state quickly when checking something.
    let speed = std::env::var("RA2_SPEED").ok().and_then(|n| n.trim().parse().ok()).unwrap_or(1u32);
    let mut sim = sim;
    for _ in 0..speed.max(1) {
        tick_once(&mut sim);
    }
}

fn tick_once(sim: &mut Sim) {
    let tick = sim.0.current_tick();
    for order in std::mem::take(&mut sim.1) {
        sim.0.order(order);
        sim.2.push((tick, order));
    }
    sim.0.tick();
    debug_orders(sim);
}

/// Orders asked for through the environment at set ticks, for trying things
/// without a mouse. They go the way the player's do.
pub fn debug_orders(sim: &mut Sim) {
    // RA2_HOLD_AT=tick puts the RA2_BUILD building on hold at a tick.
    if let (Some(at), Some(building)) = (
        std::env::var("RA2_HOLD_AT").ok().and_then(|n| n.trim().parse::<u64>().ok()),
        std::env::var("RA2_BUILD").ok().and_then(|n| n.trim().parse::<u16>().ok()),
    )
        && sim.0.current_tick() == at {
            sim.give(Order::Suspend { house: local(), building });
        }
    // RA2_UNDEPLOY_AT=building,tick,x,y packs a building up at a tick and sends its
    // vehicle to a cell.
    if let Ok(spec) = std::env::var("RA2_UNDEPLOY_AT") {
        let spec: Vec<i64> = spec.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if let [building, at, x, y] = spec[..]
            && sim.0.current_tick() as i64 == at {
                info!("RA2_UNDEPLOY_AT: building {building}, can undeploy: {}", sim.0.can_undeploy(building as u32));
                sim.give(Order::Undeploy { building: building as u32, x: x as i32, y: y as i32 });
            }
    }
    // RA2_DEPLOY_AT=unit,tick deploys a unit at a later tick, for checking deployment
    // after a move without a mouse.
    if let Ok(spec) = std::env::var("RA2_DEPLOY_AT") {
        let spec: Vec<u64> = spec.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if let [unit, at] = spec[..]
            && sim.0.current_tick() == at {
                let unit = unit as u32;
                info!(
                    "RA2_DEPLOY_AT: unit {unit} at {:?}, can deploy: {}, in the way: {:?}",
                    sim.0.unit(unit).map(|u| (u.cell(), u.is_moving())),
                    sim.0.can_deploy(unit),
                    sim.0.deploy_obstacles(unit)
                );
                for (cell, _) in sim.0.deploy_obstacles(unit) {
                    info!("  {cell:?}: passable, overlay, slope, land = {:?}", sim.0.map.describe(cell.0, cell.1));
                }
                sim.give(Order::Deploy { unit });
            }
    }
}

/// Where a point given in leptons, on ground at a height level, is drawn in the
/// scene.
pub(crate) fn scene_point(origin: Vec2, x: i32, y: i32, level: i32) -> Vec2 {
    let per_cell = LEPTONS_PER_CELL as f32;
    origin
        + Vec2::new(
            (x - y) as f32 * (CELL_WIDTH as f32 / 2.0) / per_cell + CELL_WIDTH as f32 / 2.0,
            (x + y) as f32 * (CELL_HEIGHT as f32 / 2.0) / per_cell - (level * LEVEL_HEIGHT) as f32,
        )
}

fn show_units(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<(Entity, &SimUnit, &mut Sprite, &mut Transform, Option<&mut Depth>, &mut WalkCounter, Option<&mut Rocking>, Option<&Rock>, Option<&mut Ramp>, &ShadowSprite), Without<UnitShadow>>,
    mut shadows: Query<(Entity, &mut Sprite, &mut Transform, &mut Visibility), (With<UnitShadow>, Without<SimUnit>)>,
) {
    let view = &mut *view;
    // The shadow each unit casts this frame: picture, where its draw point is,
    // and its depth.
    let mut cast: HashMap<Entity, (ShadowKey, Vec2, f32)> = HashMap::new();
    // A vehicle that has appeared since the start (a building packing up).
    let tick = sim.0.current_tick();
    if tick != view.walk_tick {
        let ticks = view.walk_tick + 1..=tick.min(view.walk_tick + 8);
        for (entity, sim_unit, _, _, _, mut counter, mut rocking, ..) in &mut sprites {
            let Some(unit) = sim.0.unit(sim_unit.0) else {
                continue;
            };
            // "moving" for a jumpjet is
            // any state but on the ground or hovering.
            let sheet = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| k.sheet);
            let moving = match unit.jet {
                Some(jet) => !matches!(jet.state, ra2_sim::flight::GROUNDED | ra2_sim::flight::HOVERING),
                None => unit.is_moving(),
            };
            let rate = match (sheet, moving) {
                _ if unit.stats.infantry => 0,
                (Some(sheet), true) => sheet.walk_rate,
                (Some(sheet), false) => sheet.idle_rate,
                (None, true) => 1,
                (None, false) => 0,
            };
            for now in ticks.clone() {
                if rate != 0 && now % rate == 0 {
                    counter.0 += 1;
                }
                if let Some(rocking) = rocking.as_mut() {
                    rocking.step();
                }
            }
            if rocking.is_some_and(|rocking| rocking.roll == 0.0 && rocking.pitch == 0.0 && rocking.roll_rate == 0.0 && rocking.pitch_rate == 0.0) {
                commands.entity(entity).remove::<Rocking>();
            }
        }
        view.walk_tick = tick;
    }
    for (entity, sim_unit, mut sprite, mut transform, depth, counter, rocking, rock, ramp_seen, shadow) in &mut sprites {
        let Some(unit) = sim.0.unit(sim_unit.0) else {
            continue;
        };
        // A unit that has changed sides (a mind taken, or given back) shows
        // its new house's colours.
        let Some(kind) = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)) else {
            continue;
        };
        let (model, palette) = (kind.image.clone(), view.unit_palette(unit.id, unit.owner));
        let (model, palette) = (&model, &palette);
        // A
        // disguised unit that does not show its true self is drawn as the type
        // it imitates, in the colours of the house it imitates.
        let masked = unit.disguise.filter(|_| !sim.0.shows_true_self(unit, local()));
        let tree = masked.is_some_and(|(_, house)| house == ra2_sim::abilities::TREE_DISGUISE);
        let masked = masked.filter(|_| !tree);
        let mask_look = masked.and_then(|(kind, house)| (kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).map(|k| (k.image.clone(), view.palette_of(house))));
        let (model, palette) = match &mask_look {
            Some((model, palette)) => (model, palette),
            None => (model, palette),
        };
        let shown_kind = masked.map_or(unit.stats.kind, |(kind, _)| kind);
        // Held by a time weapon, or still
        // arriving after a jump, it is drawn half see-through.
        let ghost = unit.frozen.is_some() || sim.0.current_tick() < unit.warp_until;
        // A
        // cloaking unit is a quarter, then half, see-through; cloaked, its owner
        // (and whoever senses it) sees it half see-through and others not at all.
        // Nothing cloaking or cloaked casts a
        // shadow, nor a `NoShadow` type.
        let casts_shadow = !tree
            && sim.0.visual_character(unit, local()) == 0
            && !(shown_kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).is_some_and(|kind| kind.no_shadow);
        let wanted = match sim.0.visual_character(unit, local()) {
            _ if tree => Color::srgba(1.0, 1.0, 1.0, 0.0),
            5 => Color::srgba(1.0, 1.0, 1.0, 0.0),
            2..=4 => Color::srgba(1.0, 1.0, 1.0, 0.5),
            1 => Color::srgba(1.0, 1.0, 1.0, 0.75),
            _ if ghost => Color::srgba(1.0, 1.0, 1.0, 0.5),
            _ => Color::WHITE,
        };
        // Under the iron curtain it flares and
        // pulses in brightness.
        let shade = sim.0.curtain_shade(unit.curtain_until);
        let wanted = if shade == 1.0 { wanted } else { Color::srgba(shade, shade, shade, wanted.alpha()) };
        if sprite.color != wanted {
            sprite.color = wanted;
        }
        let cell = unit.cell();
        // A unit on a bridge's deck stands four levels up.
        let level = sim.0.map.level(cell.0, cell.1) as i32 + if unit.on_bridge { 4 } else { 0 };
        if unit.stats.infantry {
            // The frame is the sequence's first, plus the stage within the
            // sequence, plus the facing's column times the frames per facing; the
            // art's canvas is centred on where the soldier stands. The walk steps a
            // frame every third tick.
            use ra2_sim::combat::Stance;
            let sequences = (shown_kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| k.sequences.as_ref());
            let tick = sim.0.current_tick();
            let fighter = unit.stats.fighter;
            let firing = (unit.arms.firing as usize).saturating_sub(1);
            let has = |action: &str| sequences.and_then(|s| s.get(action)).map_or(0, |seq| seq.1);
            let since = |at: u64| tick.saturating_sub(at) as usize;
            // Going down or getting up plays through once, a frame a tick,
            // and nothing breaks into it.
            let turning_over = unit.prone_at != 0 && since(unit.prone_at) < has(if unit.prone { "down" } else { "up" });
            // A fidget or a cheer plays through once, paced by
            // the game speed, unless something else is asked of the soldier.
            let fidget = unit.fidget.and_then(|(which, at)| {
                let name = match which {
                    1 => "idle1",
                    2 => "idle2",
                    _ => "cheer",
                };
                let stage = since(at) / normalized_delay(3, view.game_speed).max(1) as usize;
                (stage < has(name)).then_some((name, stage))
            });
            let (action, stage) = match unit.stance {
                Stance::Deploying(until) => ("deploy", (fighter.deploy_frames as u64).saturating_sub(until.saturating_sub(tick)) as usize),
                Stance::Undeploying(until) => ("undeploy", (fighter.undeploy_frames as u64).saturating_sub(until.saturating_sub(tick)) as usize),
                Stance::Deployed if unit.arms.firing > 0 => ("deployedfire", firing),
                Stance::Deployed => ("deployed", 0),
                // In the air a soldier shows `Fly`
                // while it moves and `Hover` when it does not.
                // Falling under a
                // parachute he holds the `Paradrop` sequence (one picture for
                // every facing) and nothing else until he is down.
                _ if unit.falling.is_some() => ("paradrop", 0),
                Stance::Standing if sim.0.altitude(unit) > 0 && unit.arms.firing > 0 => ("firefly", firing),
                Stance::Standing if sim.0.altitude(unit) > 0 && unit.is_moving() => ("fly", (tick / 3) as usize),
                Stance::Standing if sim.0.altitude(unit) > 0 => ("hover", (tick / 3) as usize),
                Stance::Standing if turning_over => (if unit.prone { "down" } else { "up" }, since(unit.prone_at)),
                Stance::Standing if unit.arms.firing > 0 && unit.prone => ("fireprone", firing),
                Stance::Standing if unit.arms.firing > 0 => ("fireup", firing),
                // Crawling steps a frame a tick; a soldier
                // with fear of 200 or more walks in `Panic`, a frame every
                // four ticks.
                Stance::Standing if unit.stepping && unit.prone => ("crawl", tick as usize),
                Stance::Standing if unit.stepping && unit.fear >= 200 && has("panic") > 0 => ("panic", (tick / 4) as usize),
                Stance::Standing if unit.stepping => ("walk", (tick / 3) as usize),
                Stance::Standing if unit.prone => ("prone", (tick / 6) as usize),
                Stance::Standing => fidget.unwrap_or(("ready", 0)),
            };
            // A soldier of the amphibious-destroyer
            // movement zone (Tanya, the SEAL) on water or beach swims: `Swim` for
            // the walk, `Tread` standing, `WetIdle` fidgeting, `WetAttack`
            // firing.
            let action = if unit.stats.movement_zone == 3 && sim.0.map.wet(cell.0, cell.1) {
                match action {
                    "walk" | "crawl" | "panic" => "swim",
                    "ready" | "prone" => "tread",
                    "idle1" => "wetidle1",
                    "idle2" => "wetidle2",
                    "fireup" | "fireprone" => "wetattack",
                    other => other,
                }
            } else {
                action
            };
            let (first, count, per_facing) = sequences.and_then(|s| s.get(action).or(s.get("ready"))).copied().unwrap_or((0, 1, 1));
            let stage = if matches!(action, "walk" | "crawl" | "panic" | "prone" | "fly" | "hover" | "swim" | "tread") { stage % count } else { stage.min(count - 1) };
            let column = crate::terrain::INFANTRY_FACINGS[((((unit.facing as usize) >> 2) + 1) >> 1) & 31];
            let frame = first + stage + column * per_facing;
            let key = (model.clone(), frame, *palette, level, false);
            let art = &mut view.art;
            let image = view.building_images.entry(key).or_insert_with(|| {
                let drawn = art.infantry(&files, model, frame, *palette, level)?;
                let size = Vec2::new(drawn.width as f32, drawn.height as f32);
                let image = Image::new(
                    Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    drawn.rgba,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::all(),
                );
                Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
            });
            let Some((handle, middle)) = image else {
                continue;
            };
            let up = Vec2::new(0.0, lift(sim.0.altitude(unit)));
            let point = scene_point(view.origin, unit.x, unit.y, level) - slope_up(&sim.0, unit);
            let centre = snapped(point - up, *middle);
            // On the ground a soldier's shadow
            // is its own frame half the file on; in the air it is the
            // shadow blob on the ground under him.
            if casts_shadow {
                let row = (cell.0 + cell.1) as f32;
                let key = if up.y == 0.0 {
                    let frames = view.art.frame_count(&files, model);
                    (format!("shp:{model}:{}", frame + frames / 2), 0)
                } else {
                    ("infshdw".to_owned(), 0)
                };
                cast.insert(shadow.0, (key, point, 1.0 + (row + 0.95) * DEPTH_PER_ROW));
            }
            // A soldier is tested against the depth
            // with an adjustment of -10, -2 and, on the ground, his own -1; all
            // less the pixels of his height.
            let height = 15 * level + (up.y + slope_up(&sim.0, unit).y) as i32;
            let adjust = if up.y > 0.0 { -12 } else { -13 } - height;
            set_depth(&mut commands, entity, depth, Depth::Upright { adjust, write: false });
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
            let row = (cell.0 + cell.1) as f32;
            let over = if up.y > 0.0 { 40.0 } else { 0.0 };
            let mut depth = 2.0 + (row + 0.9 + over) * DEPTH_PER_ROW;
            if unit.on_bridge {
                depth = 2.0 + (row + 6.0) * DEPTH_PER_ROW;
            }
            transform.translation = Vec3::new(centre.x, -centre.y, depth);
            continue;
        }
        // A vehicle drawn from a sheet of pictures. The direction column is the
        // eighth-turn facing plus one; while it moves, or if it has an `IdleRate`,
        // the picture is the walk counter within the walk frames; the counter goes
        // up on frames divisible by `WalkRate` while moving and by `IdleRate`
        // while not.
        if let Some(sheet) = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| k.sheet) {
            let column = (((((unit.facing as usize) >> 4) + 1) >> 1) + 1) & 7;
            let counter = counter.0 as usize;
            // After a shot the picture is from the firing
            // frames: a counter set to twice their number less one runs down to
            // 0, halved, so they play backwards two frames each.
            let since = tick.saturating_sub(unit.arms.fired_at) as usize;
            let frame = if sheet.firing_frames != 0 && unit.arms.fired_at != 0 && since < 2 * sheet.firing_frames {
                sheet.start_firing + sheet.firing_frames * column + (2 * sheet.firing_frames - 1 - since) / 2
            } else if unit.is_moving() || sheet.idle_rate != 0 {
                counter % sheet.walk_frames + sheet.walk_frames * column + sheet.start_walk
            } else if sheet.standing_frames != 0 {
                sheet.standing_frames * column + sheet.start_stand
            } else {
                sheet.walk_frames * column + sheet.start_walk
            };
            let key = (model.clone(), frame, *palette, level, false);
            let art = &mut view.art;
            let image = view.building_images.entry(key).or_insert_with(|| {
                let drawn = art.infantry(&files, model, frame, *palette, level)?;
                let size = Vec2::new(drawn.width as f32, drawn.height as f32);
                let image = Image::new(
                    Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    drawn.rgba,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::all(),
                );
                Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
            });
            let Some((handle, middle)) = image else {
                continue;
            };
            let point = scene_point(view.origin, unit.x, unit.y, level) - slope_up(&sim.0, unit);
            let centre = snapped(point, *middle);
            if casts_shadow {
                let frames = view.art.frame_count(&files, model);
                cast.insert(shadow.0, ((format!("shp:{model}:{}", frame + frames / 2), 0), point, 1.0 + ((cell.0 + cell.1) as f32 + 0.95) * DEPTH_PER_ROW));
            }
            // Its own adjustment and -2.
            let height = 15 * level + slope_up(&sim.0, unit).y as i32;
            set_depth(&mut commands, entity, depth, Depth::Upright { adjust: ground_adjust(&sim.0, unit) - 2 - height, write: false });
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
            let depth = 2.0 + ((cell.0 + cell.1) as f32 + 0.9) * DEPTH_PER_ROW;
            transform.translation = Vec3::new(centre.x, -centre.y, depth);
            continue;
        }
        // A miner emptying itself is drawn as its
        // `UnloadingClass`.
        let unloading = (unit.stats.harvester && unit.work.dumping())
            .then(|| (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| k.unloading.clone()))
            .flatten();
        let model = unloading.as_ref().unwrap_or(model);
        // A rocket is drawn with its nose at its pitch. DATA: `NoSpawnAlt`: a
        // launcher with no rocket aboard is drawn as the model named with WO
        // after its own.
        // An aircraft leaning into a turn is rolled about its length by its
        // type's `RollAngle`, one way turning clockwise and the other way back.
        let bank = unit.plane.map_or(0, |plane| plane.bank);
        let altered = match (&unit.rocket, &unit.spawner) {
            (Some(rocket), _) => Some(format!("{model}@p{}", (rocket.pitch / (std::f32::consts::FRAC_PI_2 / 16.0)).round() as i32)),
            _ if bank != 0 => {
                let roll = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).map_or(0.0, |k| k.roll_angle);
                Some(format!("{model}@k{}", (roll * 1000.0 * bank as f32).round() as i32))
            }
            (None, Some(spawner)) if unit.stats.no_spawn_alt && spawner.docked() == 0 => Some(format!("{model}wo")),
            _ => None,
        };
        let model = altered.as_ref().unwrap_or(model);
        // A vehicle on
        // the ground is tipped to the ramp of the cell under its middle.
        // `IsTilter` is never asked about.
        // When that ramp
        // changes, the old tilt gives way to the new over three frames.
        let ramp = {
            let cell = unit.cell();
            let ramp = if unit.on_bridge || sim.0.altitude(unit) > 0 { 0 } else { sim.0.map.ramp(cell.0, cell.1) };
            if matches!(ramp, 1..=16) { ramp } else { 0 }
        };
        let now = sim.0.current_tick();
        let seen = match ramp_seen {
            Some(mut seen) => {
                if seen.now != ramp {
                    *seen = Ramp { now: ramp, before: seen.now, since: now };
                }
                (seen.before, seen.since)
            }
            None => {
                commands.entity(entity).try_insert(Ramp { now: ramp, before: ramp, since: 0 });
                (ramp, 0)
            }
        };
        let thirds = now.saturating_sub(seen.1).min(3) as u8;
        let tipped = match (ramp, seen.0, thirds) {
            _ if unit.rocket.is_some() => None,
            (_, old, 0..=2) if old != ramp => Some(format!("{model}@r{ramp}@b{old}_{thirds}")),
            (0, ..) => None,
            _ => Some(format!("{model}@r{ramp}")),
        };
        // A rocked vehicle is drawn rocked, its
        // angles kept to hundredths of a radian.
        let tipped = match rocking.as_deref().filter(|rocking| rocking.showing() && unit.rocket.is_none()) {
            Some(rocking) => {
                let base = tipped.as_ref().unwrap_or(model);
                let (half_width, half_length) = view.art.half_extents(&files, model.split_once('@').map_or(model.as_str(), |(name, _)| name));
                let hundredths = |angle: f32| (angle * 100.0).round() as i32 * 10;
                Some(format!("{base}@w{}_{}_{half_width}_{half_length}", hundredths(rocking.roll), hundredths(rocking.pitch)))
            }
            None => tipped,
        };
        let model = tipped.as_ref().unwrap_or(model);
        let direction = (((unit.facing >> 2) + 1) >> 1) & 31;
        // A rocking vehicle is drawn rolled and pitched
        // about its own axes.
        let rocked = rock.and_then(|rock| rock.mark(direction << 3)).map(|mark| format!("{model}{mark}"));
        let model = rocked.as_ref().unwrap_or(model);
        // The turret's facing in 32 steps.
        let charge_turrets = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).map_or(0, |kind| kind.charge_turrets);
        let which = if charge_turrets > 0 { unit.arms.charge_turret(sim.0.current_tick(), charge_turrets) } else { unit.gun.map_or(0, |(_, turret)| turret) };
        let turret = unit.stats.fighter.turret.then(|| ((((unit.arms.turret_facing() >> 2) + 1) >> 1) & 31, unit.stats.fighter.turret_offset));
        // The model's animation frame is the walk
        // counter within the frames it has (the Nighthawk's two turn its rotors).
        let frames = view.art.hva_frames(&files, model);
        let frame = if frames > 1 { counter.0 as usize % frames } else { 0 };
        view.art.hva_frame = frame;
        let key = (if frame == 0 { model.clone() } else { format!("{model}#{frame}") }, direction, turret.map(|t| t.0 + 32 * which), *palette, level);
        // Rocked pictures are let go now and then.
        if view.images.len() > 2000 {
            view.images.retain(|key, _| !key.0.contains("@w"));
        }
        let image = view.images.entry(key).or_insert_with(|| {
            let drawn = view.art.vehicle(&files, model, direction << 3, turret.map(|(dir, offset)| (dir << 3, offset, which)), *palette, level)?;
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d {
                    width: drawn.width,
                    height: drawn.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::all(),
            );
            Some(UnitImage {
                handle: images.add(image),
                origin: Vec2::new(drawn.origin.0 as f32, drawn.origin.1 as f32),
                size,
            })
        });
        let Some(image) = image else {
            continue;
        };
        let up = Vec2::new(0.0, lift(sim.0.altitude(unit)));
        let point = scene_point(view.origin, unit.x, unit.y, level) - slope_up(&sim.0, unit);
        let centre = snapped(point - up, image.size / 2.0 - image.origin);
        // A model's shadow
        // is one section of it (`ShadowIndex`; the first for a plane), turned
        // to its facing and leaning with the ramp of the cell under it (never
        // blended, never rocked), at half size for `ConsideredAircraft`, on the
        // ground under it.
        if casts_shadow && unit.rocket.is_none() {
            let kind = (shown_kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k));
            let section = if unit.plane.is_some() { 0 } else { kind.map_or(0, |kind| kind.shadow_index) };
            let half = kind.is_some_and(|kind| kind.considered_aircraft);
            let ramp = if unit.on_bridge { 0 } else { sim.0.map.ramp(cell.0, cell.1) };
            let base = model.split_once('@').map_or(model.as_str(), |(base, _)| base);
            let key = (format!("{base}@s{section}{}@r{ramp}", if half { "h" } else { "" }), direction);
            cast.insert(shadow.0, (key, point, 1.0 + ((cell.0 + cell.1) as f32 + 0.95) * DEPTH_PER_ROW));
        }
        // A vehicle's picture is tested
        // against the depth with its own adjustment, less the pixels of its
        // height.
        let height = 15 * level + (up.y + slope_up(&sim.0, unit).y) as i32;
        let own = if up.y > 0.0 { -1 } else { ground_adjust(&sim.0, unit) };
        set_depth(&mut commands, entity, depth, Depth::Upright { adjust: own - height, write: false });
        if sprite.image != image.handle {
            sprite.image = image.handle.clone();
        }
        let row = (cell.0 + cell.1) as f32;
        // Units draw over everything in their own row.
        let over = if up.y > 0.0 { 40.0 } else { 0.0 };
        let mut depth = 2.0 + (row + 0.9 + over) * DEPTH_PER_ROW;
        if unit.plane.is_some() || unit.on_bridge {
            depth = 2.0 + (row + 6.0) * DEPTH_PER_ROW;
        }
        transform.translation = Vec3::new(centre.x, -centre.y, depth);
    }
    for (entity, mut sprite, mut transform, mut visibility) in &mut shadows {
        let Some((key, point, depth)) = cast.remove(&entity) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let art = &mut view.art;
        let image = view.shadow_images.entry(key).or_insert_with_key(|key| {
            let (width, height, rgba, middle) = if let Some((art_name, frame)) = key.0.strip_prefix("shp:").and_then(|rest| rest.rsplit_once(':')) {
                let drawn = art.building(&files, art_name, frame.parse().ok()?, None, 0, true)?;
                (drawn.width, drawn.height, drawn.rgba, Vec2::new(drawn.offset.0 as f32 + drawn.width as f32 / 2.0, drawn.offset.1 as f32 + drawn.height as f32 / 2.0))
            } else if key.0 == "infshdw" {
                let drawn = art.soldier_shadow(&files)?;
                (drawn.width, drawn.height, drawn.rgba, Vec2::new(drawn.offset.0 as f32 + drawn.width as f32 / 2.0, drawn.offset.1 as f32 + drawn.height as f32 / 2.0))
            } else {
                let (model, marks) = key.0.split_once('@')?;
                let (mut section, mut ramp, mut half) = (0, 0, false);
                for mark in marks.split('@') {
                    match mark.split_at_checked(1) {
                        Some(("s", rest)) => {
                            half = rest.ends_with('h');
                            section = rest.trim_end_matches('h').parse().ok()?;
                        }
                        Some(("r", rest)) => ramp = rest.parse().ok()?,
                        _ => {}
                    }
                }
                let drawn = art.voxel_shadow(&files, model, key.1 << 3, half, section, ramp)?;
                (drawn.width, drawn.height, drawn.rgba, Vec2::new(drawn.width as f32 / 2.0 - drawn.origin.0 as f32, drawn.height as f32 / 2.0 - drawn.origin.1 as f32))
            };
            let image = Image::new(Extent3d { width, height, depth_or_array_layers: 1 }, TextureDimension::D2, rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
            Some((images.add(image), middle))
        });
        let Some((handle, middle)) = image else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if sprite.image != *handle {
            sprite.image = handle.clone();
        }
        *visibility = Visibility::Inherited;
        let centre = snapped(point, *middle);
        transform.translation = Vec3::new(centre.x, -centre.y, depth);
    }
}

/// A unit's own depth adjustment on the ground: -3 while it leaves its
/// factory, -14 for a miner on a refinery, otherwise -1.
fn ground_adjust(sim: &World, unit: &ra2_sim::Unit) -> i32 {
    let cell = unit.cell();
    let on_refinery = || {
        sim.buildings().iter().any(|b| {
            (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0)
                && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1)
                && sim.building_types.get(b.kind as usize).is_some_and(|kind| kind.refinery)
        })
    };
    if unit.tethered {
        -3
    } else if unit.stats.harvester && on_refinery() {
        -14
    } else {
        -1
    }
}

/// Sets how a sprite's depth is worked out, touching it only on a change.
fn set_depth(commands: &mut Commands, entity: Entity, current: Option<Mut<Depth>>, wanted: Depth) {
    match current {
        Some(mut current) => {
            if *current != wanted {
                *current = wanted;
            }
        }
        None => {
            commands.entity(entity).insert(wanted);
        }
    }
}

/// The delay between an animation's images when it is `Normalized`: paced by the
/// game speed so that it looks the same at any setting.
fn normalized_delay(delay: u32, speed: i32) -> u32 {
    const TABLE: [[u32; 8]; 4] = [
        [2, 2, 1, 1, 1, 1, 1, 1],
        [3, 3, 3, 2, 2, 2, 1, 1],
        [5, 4, 4, 3, 3, 2, 2, 1],
        [7, 6, 5, 4, 4, 4, 3, 2],
    ];
    let speed = speed.clamp(0, 7) as u32;
    match delay {
        0 => 0,
        1..=4 => TABLE[delay as usize - 1][speed as usize],
        _ => delay * 8 / (speed + 1),
    }
}

/// Where a building comes in the drawing order, on the scale of units' rows
/// (the X + Y of the cell they stand in).
// Objects are
// sorted by the X + Y of their centre; a building's centre is the middle of
// its foundation. (The foundation's nearest cell, used before, put a large
// building in front of a small one standing beside it.)
fn building_row(building: &ra2_sim::Building) -> f32 {
    (building.cell.0 + building.cell.1) as f32 + (building.size.0 + building.size.1) as f32 / 2.0 - 1.0
}

/// The frame an animation shows after running for some ticks, or `None` once a
/// one-shot has finished.
// From the first image up to `LoopEnd`, then round
// from `LoopStart` to `LoopEnd` for each further loop.
fn anim_frame(anim: &crate::terrain::AnimType, images: usize, ticks: u64, speed: i32) -> Option<usize> {
    let end = anim.end.unwrap_or(images);
    let loop_end = anim.loop_end.unwrap_or(end);
    let delay = if anim.normalized { normalized_delay(anim.delay, speed) } else { anim.delay };
    if delay == 0 {
        return Some(anim.start);
    }
    let steps = ticks / delay as u64;
    let first = loop_end.saturating_sub(anim.start).max(1) as u64;
    if steps < first {
        return Some(anim.start + steps as usize);
    }
    let from = anim.loop_start.saturating_sub(anim.start) as u64;
    let round = first.saturating_sub(from).max(1);
    let loops = 1 + (steps - first) / round;
    if anim.loop_count >= 0 && loops >= anim.loop_count.max(1) as u64 {
        return None;
    }
    Some(anim.start + (from + (steps - first) % round) as usize)
}

#[derive(Clone, Default)]
struct AnimClock {
    /// The last few spans the building was offline, the latest perhaps still
    /// open.
    off: Vec<(u64, Option<u64>)>,
    super_stage: u8,
    super_since: u64,
}

impl AnimClock {
    /// Ticks the building was online between two moments.
    fn online_between(&self, from: u64, to: u64) -> u64 {
        let off: u64 = self.off.iter().map(|&(start, end)| end.unwrap_or(to).min(to).saturating_sub(start.max(from))).sum();
        to.saturating_sub(from).saturating_sub(off)
    }

    fn offline(&self) -> bool {
        self.off.last().is_some_and(|span| span.1.is_none())
    }

    fn back_on(&self) -> u64 {
        self.off.last().and_then(|span| span.1).unwrap_or(0)
    }
}

/// Keeps the sprites each building is drawn from: its build-up animation while
/// that runs; afterwards its body, bib, shadows and the animations it shows at
/// rest.
// What the parts are, that they share one draw point and the building's palette,
// the frame of each, and the damaged variants at yellow health.
fn show_buildings(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut shown: Query<(Entity, &mut BuildingSprite, &mut Sprite, &mut Transform, &mut Visibility, Option<&mut Depth>)>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let standing: std::collections::HashSet<u32> = sim.0.buildings().iter().map(|b| b.id).collect();
    view.building_clocks.retain(|id, _| standing.contains(id));
    // Buildings that are gone, for instance packed up into a vehicle.
    for (entity, sprite, ..) in &shown {
        if !sim.0.buildings().iter().any(|b| b.id == sprite.0) {
            commands.entity(entity).despawn();
        }
    }
    for building in sim.0.buildings() {
        let Some(art) = view.building_art.get(building.kind as usize) else {
            continue;
        };
        let palette = match view.building_palettes.get(&building.id) {
            // A building a player has taken shows in that player's colours.
            Some(palette) if building.owner == 0 => *palette,
            _ => view.palette_of(building.owner),
        };
        let level = sim.0.map.level(building.cell.0, building.cell.1) as i32;
        let row = building_row(building);
        let (body_z, shadow_z, step) = (2.0 + row * DEPTH_PER_ROW, 1.0 + row * DEPTH_PER_ROW, DEPTH_PER_ROW / 40.0);
        // Bodies, bibs and floors are drawn before every object.
        let first_z = crate::depth::FIRST + row * DEPTH_PER_ROW / 100.0;
        let height = 15 * level;
        // The body's adjustment, and whether its depth shape is moved by
        // `ZShapePointMove` (not while it goes up or comes down).
        let body_part = |moved: bool| PartDepth::Body(art.normal_z_adjust - height - 2, moved);

        // The parts wanted now: art, frame, shadow, drawing order, depth.
        let mut parts: Vec<(String, usize, bool, f32, PartDepth)> = Vec::new();
        let with_shadow = |parts: &mut Vec<(String, usize, bool, f32, PartDepth)>,
                           art: &mut UnitArt,
                           files: &GameFiles,
                           art_name: &str,
                           frame: usize,
                           z: f32,
                           own_shadow: bool,
                           depth: PartDepth| {
            let count = art.frame_count(files, art_name);
            if own_shadow && count >= 2 {
                parts.push((art_name.to_owned(), frame + count / 2, true, shadow_z, PartDepth::None));
            }
            parts.push((art_name.to_owned(), frame, false, z, depth));
        };
        match (building.buildup_frame(), &art.buildup) {
            (Some(frame), Some(buildup)) => with_shadow(&mut parts, &mut view.art, &files, buildup, frame as usize, first_z, true, body_part(false)),
            _ => {
                let full = sim.0.building_types.get(building.kind as usize).map_or(1, |kind| kind.strength).max(1);
                let damaged = building.health as f32 / full as f32 <= view.condition_yellow;
                // A building soldiers can fire from
                // shows picture 1 only at red health, 2 with anyone inside, else
                // 0.
                let occupiable = sim.0.building_types.get(building.kind as usize).is_some_and(|kind| kind.can_be_occupied);
                let frame = if !occupiable {
                    usize::from(damaged)
                } else if sim.0.red_health(building) {
                    1
                } else if !building.occupants.is_empty() {
                    2
                } else {
                    0
                };
                // The bib lies flat, 1 nearer and the usual 2.
                if let Some(bib) = &art.bib {
                    with_shadow(&mut parts, &mut view.art, &files, bib, frame, first_z - step, true, PartDepth::Flat(-3 - height, true));
                }
                // While a factory lets a vehicle out its
                // body is the open-door art, with the floor under the door drawn
                // after it; there is no door animation in the stock art.
                let body = art.deploying.as_ref().filter(|_| building.unloading()).unwrap_or(&art.art);
                // The open-door body is moved 20 nearer.
                let body_depth = if building.unloading() && art.deploying.is_some() { PartDepth::Body(-22 - height, true) } else { body_part(true) };
                with_shadow(&mut parts, &mut view.art, &files, body, frame, first_z, true, body_depth);
                if let Some(floor) = art.under_door.as_ref().filter(|_| building.unloading()) {
                    with_shadow(&mut parts, &mut view.art, &files, floor, frame, first_z + step / 2.0, false, PartDepth::Flat(-2 - height, true));
                }
                // Offline, `Powered` animations stop on their frame and
                // `PoweredLight` ones go, to start again when it comes back.
                let mut clock = view.building_clocks.get(&building.id).cloned().unwrap_or_else(|| AnimClock { super_since: building.opened, ..default() });
                let offline = !sim.0.online(building);
                match (offline, clock.offline()) {
                    (true, false) => {
                        clock.off.push((tick, None));
                        if clock.off.len() > 8 {
                            clock.off.remove(0);
                        }
                    }
                    (false, true) => clock.off.last_mut().unwrap().1 = Some(tick),
                    _ => {}
                }
                // A super weapon's building opens up once its weapon is
                // within `ChargedAnimTime` minutes of ready, and closes again
                // once it has been fired.
                let charged = art.charged_anim_time;
                let near = sim.0.building_types.get(building.kind as usize).and_then(|kind| kind.super_weapon).and_then(|index| sim.0.houses.get(building.owner as usize)?.supers.get(index as usize)).is_some_and(|state| {
                    state.granted && charged >= if state.ready { 0.0 } else { state.left as f32 / 900.0 }
                });
                if charged <= 990.0 {
                    match (clock.super_stage, near) {
                        (0, true) | (2, false) => {
                            clock.super_stage += 1;
                            clock.super_since = tick;
                        }
                        _ => {}
                    }
                }
                let mut slots: Vec<&crate::terrain::AnimSlot> = art.anims.iter().filter(|slot| slot.refinery_level == 0 && slot.super_stage.is_none_or(|stage| stage == clock.super_stage)).collect();
                slots.sort_by_key(|slot| (slot.y_sort, -slot.z_adjust));
                for (order, slot) in slots.into_iter().enumerate() {
                    // The firing animation runs from
                    // the start of each charge.
                    // A refinery's runs while
                    // a miner's load is paid for.
                    let mut since = match (slot.special, building.charge.map(|charge| charge.0).or(building.refining)) {
                        (true, Some(began)) => began,
                        (true, None) => continue,
                        (false, _) if slot.super_stage.is_some() => clock.super_since,
                        (false, _) => building.opened,
                    };
                    let mut now = tick;
                    if slot.powered {
                        now = since + clock.online_between(since, tick);
                    } else if slot.powered_light {
                        if clock.offline() {
                            continue;
                        }
                        since = since.max(clock.back_on());
                    }
                    let name = if damaged { &slot.damaged } else { &slot.name };
                    let Some(anim) = view.anim_types.get(&name.to_uppercase()) else {
                        continue;
                    };
                    let count = view.art.frame_count(&files, &anim.image);
                    let images = if anim.shadow { count / 2 } else { count };
                    let Some(frame) = anim_frame(anim, images, now.saturating_sub(since), view.game_speed) else {
                        // Opening leads to ready, closing back to charging.
                        if matches!(slot.super_stage, Some(1 | 3)) {
                            clock.super_stage = (clock.super_stage + 1) % 4;
                            clock.super_since = tick;
                        }
                        continue;
                    };
                    // Upright, by the slot's adjustment and the usual 2.
                    let depth = PartDepth::Upright(slot.z_adjust - height - 2);
                    with_shadow(&mut parts, &mut view.art, &files, &anim.image, frame, body_z + step * (order + 1) as f32, anim.shadow, depth);
                }
                view.building_clocks.insert(building.id, clock);
                // A voxel turret, at the draw point
                // moved by `TurretAnimX` and `TurretAnimY`, facing in 32 steps;
                // not while the building is going up or coming down.
                if let Some((model, x, y)) = &art.turret {
                    let facing = (((building.arms.turret_facing() >> 2) + 1) >> 1) & 31;
                    // After a shot the barrel goes back `BarrelTravel` over
                    // `BarrelCompressFrames`, stays `BarrelHoldFrames`, and
                    // comes forward over `BarrelRecoverFrames`.
                    let (travel, compress, hold, recover) = sim.0.building_types.get(building.kind as usize).map_or((0, 0, 0, 0), |kind| kind.barrel_recoil);
                    let since = tick.saturating_sub(building.arms.fired_at) as i32;
                    let recoil = if travel == 0 || building.arms.fired_at == 0 {
                        0.0
                    } else if since < compress {
                        travel as f32 * (since + 1) as f32 / compress.max(1) as f32
                    } else if since < compress + hold {
                        travel as f32
                    } else if since < compress + hold + recover {
                        travel as f32 * (1.0 - (since - compress - hold + 1) as f32 / recover.max(1) as f32)
                    } else {
                        0.0
                    };
                    parts.push((format!("vxl:{model}:{x}:{y}:{}", (recoil * 2.0).round() as i32), facing as usize, false, body_z + step * 30.0, PartDepth::Upright(art.turret_z_adjust - height)));
                }
            }
        }

        // Every part's canvas is centred on
        // the top corner of the foundation's top cell.
        let per_cell = LEPTONS_PER_CELL;
        let anchor = scene_point(view.origin, building.cell.0 * per_cell, building.cell.1 * per_cell, level);
        let mut have = 0;
        let zmove = art.zshape_move;
        let zshape = |centre: Vec2, size: Vec2, moved: bool| {
            let (w, h) = building.size;
            let shift = if moved { zmove } else { (0, 0) };
            let corner = centre - size / 2.0;
            IVec2::new(corner.x.round() as i32 + 198 + shift.0 - 30 * (w - h), corner.y.round() as i32 + 446 + shift.1 - 15 * (w + h - 2))
        };
        let wide = building.size.0 >= 8;
        let depth_of = |part: PartDepth, size: Vec2, centre: Vec2| -> Option<Depth> {
            match part {
                PartDepth::None => None,
                PartDepth::Body(adjust, _) if wide => Some(Depth::Upright { adjust, write: true }),
                PartDepth::Body(adjust, moved) => Some(Depth::Body { adjust, at: zshape(centre, size, moved) }),
                PartDepth::Flat(adjust, write) => Some(Depth::Flat { adjust, write }),
                PartDepth::Upright(adjust) => Some(Depth::Upright { adjust, write: false }),
            }
        };
        for (entity, mut sprite, mut picture, mut transform, mut visibility, depth) in &mut shown {
            if sprite.0 != building.id {
                continue;
            }
            have = have.max(sprite.1 + 1);
            let Some((name, frame, shadow, z, part)) = parts.get(sprite.1) else {
                commands.entity(entity).despawn();
                continue;
            };
            // Held by a time weapon it is
            // drawn half see-through, as a unit is.
            let alpha = if building.frozen.is_some() { 0.5 } else { 1.0 };
            // Under the iron curtain
            // it flares and pulses in brightness; not its shadow.
            let shade = if *shadow { 1.0 } else { sim.0.curtain_shade(building.curtain_until) };
            let wanted = Color::srgba(shade, shade, shade, alpha);
            if picture.color != wanted {
                picture.color = wanted;
            }
            let key: PartKey = (name.clone(), *frame, palette, level, *shadow);
            if sprite.2.as_ref() == Some(&key) && transform.translation.z == *z {
                continue;
            }
            let image = part_image(view, &files, &mut images, &key);
            match image {
                Some((handle, centre)) => {
                    let size = images.get(&handle).map_or(Vec2::ZERO, |image| image.size_f32());
                    match depth_of(*part, size, centre) {
                        Some(wanted) => set_depth(&mut commands, entity, depth, wanted),
                        None if depth.is_some() => {
                            commands.entity(entity).remove::<(Depth, bevy::sprite_render::SpriteMaterial<crate::depth::DepthMaterial>)>();
                        }
                        None => {}
                    }
                    let centre = snapped(anchor, centre);
                    picture.image = handle;
                    transform.translation = Vec3::new(centre.x, -centre.y, *z);
                    *visibility = Visibility::Inherited;
                }
                None => *visibility = Visibility::Hidden,
            }
            sprite.2 = Some(key);
        }
        for (index, (name, frame, shadow, z, part)) in parts.iter().enumerate().skip(have) {
            let key: PartKey = (name.clone(), *frame, palette, level, *shadow);
            let (handle, place, visibility, depth) = match part_image(view, &files, &mut images, &key) {
                Some((handle, centre)) => {
                    let size = images.get(&handle).map_or(Vec2::ZERO, |image| image.size_f32());
                    let depth = depth_of(*part, size, centre);
                    let centre = snapped(anchor, centre);
                    (handle, Vec3::new(centre.x, -centre.y, *z), Visibility::Inherited, depth)
                }
                None => (Handle::default(), Vec3::new(anchor.x, -anchor.y, *z), Visibility::Hidden, None),
            };
            let mut spawned = commands.spawn((
                DespawnOnExit(Screen::InGame),
                BuildingSprite(building.id, index, Some(key)),
                Sprite::from_image(handle),
                Transform::from_translation(place),
                visibility,
            ));
            if let Some(depth) = depth {
                spawned.insert(depth);
            }
        }
    }
}

/// How a building part's depth is worked out, before its picture is known.
#[derive(Clone, Copy)]
enum PartDepth {
    None,
    /// The body, by its adjustment and whether `ZShapePointMove` applies.
    Body(i32, bool),
    /// Flat, by its adjustment, and whether it writes.
    Flat(i32, bool),
    /// Upright and only tested, by its adjustment.
    Upright(i32),
}

/// A sprite of a wall piece or its shadow.
#[derive(Component)]
struct WallSprite;

/// A crate's picture.
#[derive(Component)]
struct CrateSprite;

/// A sprite of a bridge piece or its shadow.
#[derive(Component)]
struct BridgeSprite;

/// Draws the bridges as the simulation has them.
// A high bridge's
// picture is its state, with one of four looks for the whole deck by where it
// stands, raised 16 pixels and 15 more for the second direction's pictures,
// lit as four levels up; a low bridge's piece is its type's own art at the
// frame of its place across the bridge. The shadow is the frame half the
// file's count later.
fn show_bridges(mut commands: Commands, sim: Res<Sim>, files: Res<GameFiles>, mut view: ResMut<UnitView>, mut images: ResMut<Assets<Image>>, old: Query<Entity, With<BridgeSprite>>) {
    let view = &mut *view;
    if view.bridges_shown == Some(sim.0.map.bridge_version) {
        return;
    }
    view.bridges_shown = Some(sim.0.map.bridge_version);
    for entity in &old {
        commands.entity(entity).despawn();
    }
    const VARIANTS: [usize; 16] = [0, 1, 2, 3, 3, 2, 1, 0, 2, 3, 0, 1, 1, 0, 3, 2];
    let pieces: Vec<_> = sim.0.map.bridge_overlays().collect();
    for (cell, kind, data) in pieces {
        let Some(art) = view.bridge_art.get(&kind).cloned() else {
            continue;
        };
        let high = matches!(kind, 24 | 25 | 237 | 238);
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        let mut frame = data as usize;
        if high && (data == 0 || data == 9) {
            frame += VARIANTS[(((cell.1 & 3) << 2) | (cell.0 & 3)) as usize];
        }
        let lift = match (high, data) {
            (true, 9..=17) => 31.0,
            (true, _) => 16.0,
            _ => 0.0,
        };
        let half = LEPTONS_PER_CELL / 2;
        let anchor = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, level) - Vec2::new(0.0, lift);
        let row = (cell.0 + cell.1) as f32;
        let frames = view.art.frame_count(&files, &art);
        // The deck is drawn with the ground and lies flat, 2 nearer than the
        // ground under it, four levels up for a high bridge.
        let deck = Depth::Flat { adjust: -2 - 15 * (level + if high { 4 } else { 0 }), write: true };
        for (frame, shadow, z) in [(frame + frames / 2, true, 1.0 + row * DEPTH_PER_ROW), (frame, false, crate::depth::FIRST + row * DEPTH_PER_ROW / 100.0)] {
            let key: PartKey = (format!("bridge:{art}"), frame, None, level + if high { 4 } else { 0 }, shadow);
            if let Some((handle, centre)) = part_image(view, &files, &mut images, &key) {
                let centre = snapped(anchor, centre);
                let mut spawned = commands.spawn((DespawnOnExit(Screen::InGame), BridgeSprite, Sprite::from_image(handle), Transform::from_xyz(centre.x, -centre.y, z)));
                if !shadow {
                    spawned.insert(deck.clone());
                    // A high bridge is lit as four levels up.
                    spawned.insert(Lit { cell, level, rule: LightRule::Ground { raise: if high { 4 } else { 0 } } });
                }
            }
        }
    }
}

/// Keeps the ore on the map as the simulation has it.
// The frame is the amount; drawn at the
// cell's middle moved up 12 pixels.
fn show_ore(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<&mut Sprite>,
) {
    let view = &mut *view;
    if view.ore_shown == Some(sim.0.map.ore_version) && view.smudges_shown == Some(sim.0.map.smudge_version) {
        return;
    }
    view.ore_shown = Some(sim.0.map.ore_version);
    let mut gone: Vec<(i32, i32)> = view.ore_sprites.keys().copied().filter(|cell| sim.0.map.ore_at(cell.0, cell.1).is_none()).collect();
    for cell in gone.drain(..) {
        if let Some((entity, _)) = view.ore_sprites.remove(&cell) {
            commands.entity(entity).despawn();
        }
    }
    let half = LEPTONS_PER_CELL / 2;
    // A mark is the first frame of its
    // art, set about the top corner of the cell it starts at, drawn straight
    // after the ground and so under everything else.
    if view.smudges_shown != Some(sim.0.map.smudge_version) {
        view.smudges_shown = Some(sim.0.map.smudge_version);
        for (cell, kind) in sim.0.map.smudges() {
            if view.smudge_sprites.contains_key(&cell) {
                continue;
            }
            let Some(name) = view.smudge_names.get(kind as usize).cloned() else {
                continue;
            };
            let art_cache = &mut view.art;
            let level = sim.0.map.level(cell.0, cell.1) as i32;
            // Keyed apart from the ore's pictures by a frame number ore never has.
            let image = view.ore_images.entry((name, 1000 + level as usize)).or_insert_with_key(|key| {
                let drawn = art_cache.smudge(&files, &key.0, level)?;
                let size = Vec2::new(drawn.width as f32, drawn.height as f32);
                let image = Image::new(
                    Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    drawn.rgba,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
            });
            let Some((handle, centre)) = image.clone() else {
                continue;
            };
            let level = sim.0.map.level(cell.0, cell.1) as i32;
            let at = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, level) - Vec2::new(0.0, 15.0) + centre;
            let translation = Vec3::new(at.x.round(), -at.y.round(), 0.55 + (cell.0 + cell.1) as f32 * DEPTH_PER_ROW);
            let lit = Lit { cell, level, rule: LightRule::Ground { raise: 0 } };
            let entity = commands.spawn((DespawnOnExit(Screen::InGame), Sprite::from_image(handle), Transform::from_translation(translation), lit)).id();
            view.smudge_sprites.insert(cell, entity);
        }
    }
    for (cell, _, amount) in sim.0.map.ore_cells() {
        if view.ore_sprites.get(&cell).is_some_and(|(_, shown)| *shown == amount) {
            continue;
        }
        // Ore that came during play shows the
        // picture it drew then, one of its kind's twelve.
        // DATA: rules.ini `[OverlayTypes]`: TIB01 to TIB12 and GEM01 to GEM12.
        if !view.ore_art.contains_key(&cell)
            && let Some(picture) = sim.0.map.ore_picture(cell.0, cell.1) {
                let family = if sim.0.map.ore_at(cell.0, cell.1).is_some_and(|(kind, _)| kind == 1) { "gem" } else { "tib" };
                view.ore_art.insert(cell, format!("{family}{:02}", picture + 1));
            }
        let Some(art) = view.ore_art.get(&cell) else {
            continue;
        };
        let key = (art.clone(), amount as usize);
        let art_cache = &mut view.art;
        let image = view.ore_images.entry(key).or_insert_with_key(|key| {
            let drawn = art_cache.ore(&files, &key.0, key.1)?;
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        });
        let Some((handle, centre)) = image.clone() else {
            continue;
        };
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        let at = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, level)
            - Vec2::new(0.0, 12.0)
            + centre;
        // Ore lies on the ground: under every unit and shadow.
        let translation = Vec3::new(at.x.round(), -at.y.round(), 0.6 + (cell.0 + cell.1) as f32 * DEPTH_PER_ROW);
        match view.ore_sprites.get_mut(&cell) {
            Some((entity, shown)) => {
                *shown = amount;
                if let Ok(mut sprite) = sprites.get_mut(*entity) {
                    sprite.image = handle;
                }
                commands.entity(*entity).insert(Transform::from_translation(translation));
            }
            None => {
                let entity = commands
                    .spawn((DespawnOnExit(Screen::InGame), Sprite::from_image(handle), Transform::from_translation(translation)))
                    .id();
                view.ore_sprites.insert(cell, (entity, amount));
            }
        }
    }
}

/// Draws the crates again whenever one appears, moves or is picked up.
// `WoodCrateImg` (CRATE, a theatre picture) on land, `WaterCrateImg` (WCRATE)
// on water. An overlay that is not ore or a wall is drawn with the cell's lit
// palette (the ground's), its frame the overlay's state (the lift: see below).
fn show_crates(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    old: Query<Entity, With<CrateSprite>>,
) {
    // Drawn again when a crate comes or goes, or the shroud lifts off one.
    let now: Vec<((i32, i32), bool)> = sim.0.crates().iter().map(|c| (c.cell, c.water)).collect();
    let shown = (now, sim.0.map.shroud_version);
    if view.crates_shown.as_ref() == Some(&shown) {
        return;
    }
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let view = &mut *view;
    for &(cell, water) in &shown.0 {
        if sim.0.map.shrouded(cell.0, cell.1) {
            continue;
        }
        let art = if water { "wcrate" } else { "crate" }.to_owned();
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        let half = LEPTONS_PER_CELL / 2;
        // An overlay is drawn at the
        // cell's middle, 12 pixels up for crates (`Crate=yes`) as for walls and
        // ore. (Drawn without that, a crate sat half a cell low, in front of
        // the cell that picks it up.)
        let anchor = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, level) - Vec2::new(0.0, 12.0);
        let row = (cell.0 + cell.1) as f32;
        let picture = view.crate_images.entry((water, level)).or_insert_with(|| {
            let drawn = view.art.smudge(&files, &art, level)?;
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        });
        if let Some((handle, centre)) = picture.clone() {
            let centre = snapped(anchor, centre);
            let lit = Lit { cell, level, rule: LightRule::Ground { raise: 0 } };
            commands.spawn((DespawnOnExit(Screen::InGame), CrateSprite, Sprite::from_image(handle), Transform::from_xyz(centre.x, -centre.y, 2.0 + row * DEPTH_PER_ROW), lit));
        }
    }
    view.crates_shown = Some(shown);
}

/// A Mirage's tree.
#[derive(Component)]
struct MirageSprite;

/// Draws each Mirage that the player takes for a tree as that tree: one of
/// `DefaultMirageDisguises`, drawn as a terrain object at its cell's middle.
fn show_mirages(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    old: Query<Entity, With<MirageSprite>>,
    rules: Res<crate::files::Rules>,
) {
    let now: Vec<(u32, u16, (i32, i32))> = sim
        .0
        .units()
        .iter()
        .filter_map(|unit| {
            let (look, house) = unit.disguise?;
            let cell = unit.cell();
            (house == ra2_sim::abilities::TREE_DISGUISE && !sim.0.shows_true_self(unit, local()) && !sim.0.map.shrouded(cell.0, cell.1)).then_some((unit.id, look, cell))
        })
        .collect();
    if view.mirages_shown.as_ref() == Some(&now) {
        return;
    }
    view.mirages_shown = Some(now.clone());
    let names: Vec<String> = rules.0.get("General", "DefaultMirageDisguises").map(|list| list.split(',').map(|name| name.trim().to_lowercase()).filter(|name| !name.is_empty()).collect()).unwrap_or_default();
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let view = &mut *view;
    for (_, look, cell) in now {
        let Some(name) = names.get(look as usize) else {
            continue;
        };
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        let half = LEPTONS_PER_CELL / 2;
        let anchor = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, level);
        let row = (cell.0 + cell.1) as f32;
        let frames = view.art.frame_count(&files, name);
        for (frame, shadow, z) in [(frames / 2, true, 1.0 + row * DEPTH_PER_ROW), (0, false, 2.0 + (row + 0.9) * DEPTH_PER_ROW)] {
            let key: PartKey = (format!("bridge:{name}"), frame, None, level, shadow);
            if let Some((handle, centre)) = part_image(view, &files, &mut images, &key) {
                let centre = snapped(anchor, centre);
                commands.spawn((DespawnOnExit(Screen::InGame), MirageSprite, Sprite::from_image(handle), Transform::from_xyz(centre.x, -centre.y, z)));
            }
        }
    }
}

/// Draws the wall pieces again whenever one appears or changes.
// The frame is the piece's data (damage
// level times 16 plus its connections), drawn in the viewing player's colours
// whoever owns it, at the cell's middle 12 pixels up; the shadow is the frame half
// the file later.
fn show_walls(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    old: Query<Entity, With<WallSprite>>,
) {
    let view = &mut *view;
    if view.walls_shown == Some(sim.0.map.walls_version) {
        return;
    }
    view.walls_shown = Some(sim.0.map.walls_version);
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let walls: Vec<_> = sim.0.map.walls().collect();
    for (cell, overlay, data) in walls {
        let Some(art) = view.wall_art.get(&overlay).cloned() else {
            continue;
        };
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        let half = LEPTONS_PER_CELL / 2;
        let anchor = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, level)
            - Vec2::new(0.0, 12.0);
        let row = (cell.0 + cell.1) as f32;
        let frames = view.art.frame_count(&files, &art);
        let palette = view.player_palette;
        for (frame, shadow, z) in [
            (data as usize + frames / 2, true, 1.0 + row * DEPTH_PER_ROW),
            (data as usize, false, crate::depth::FIRST + row * DEPTH_PER_ROW / 100.0),
        ] {
            let key: PartKey = (art.clone(), frame, palette, level, shadow);
            if let Some((handle, centre)) = part_image(view, &files, &mut images, &key) {
                let centre = snapped(anchor, centre);
                let mut spawned = commands.spawn((
                    DespawnOnExit(Screen::InGame),
                    WallSprite,
                    Sprite::from_image(handle),
                    Transform::from_xyz(centre.x, -centre.y, z),
                ));
                // A wall is an overlay drawn with the ground, flat by default.
                if !shadow {
                    spawned.insert(Depth::Flat { adjust: -2 - 15 * level, write: true });
                    spawned.insert(Lit { cell, level, rule: LightRule::Object { extra: 0 } });
                }
            }
        }
    }
}

/// Follows the storm and gathers the lights on the map: lit buildings while
/// they are working, the terrain's own, and radiation.
fn light_world(sim: Res<Sim>, storm: Option<ResMut<StormLight>>, light: Option<ResMut<WorldLight>>) {
    let (Some(mut storm), Some(mut light)) = (storm, light) else {
        return;
    };
    let world = &sim.0;
    let tick = world.current_tick();
    // RA2_STORM=tick shows the storm's light from that tick on, for checks.
    let forced = std::env::var("RA2_STORM").ok().and_then(|t| t.trim().parse::<u64>().ok()).is_some_and(|from| tick >= from);
    let raging = forced || world.storm_active();
    if storm.0 != raging {
        storm.0 = raging;
    }
    light.set_storm(storm.0, tick);
    light.step(tick);
    let mut sources = light.setup.statics.clone();
    // A building's light is at its middle, on while it is online.
    for building in world.buildings() {
        let Some((Some(kind), _)) = light.setup.buildings.get(building.kind as usize) else {
            continue;
        };
        if building.buildup.is_none() && world.online(building) {
            sources.push(kind.at(building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128));
        }
    }
    // A radiation site's light reaches its spread and half a cell, at
    // `RadLightFactor` times its level; every `RadLightDelay` frames it dims by
    // an even step and its tint by the share of its life left.
    let rules = world.rad_rules;
    for site in &world.rad_sites {
        let duration = (site.level as i64 * rules.duration_multiple as i64).max(1);
        let delay = rules.light_delay.max(1) as i64;
        let steps = (duration / delay).max(1);
        let elapsed = tick.saturating_sub(site.born) as i64;
        let taken = elapsed / delay;
        let intensity = (site.level as f64 * light.setup.rad_light_factor).min(crate::lighting::MAX_LIGHT as f64) as i32;
        let intensity = intensity - intensity / steps as i32 * taken as i32;
        let left = (duration - taken * delay).max(0);
        let tint = light.setup.rad_tint.map(|c| (c as i64 * left / duration) as i32);
        sources.push(crate::lighting::Source {
            x: site.cell.0 * LEPTONS_PER_CELL + 128,
            y: site.cell.1 * LEPTONS_PER_CELL + 128,
            radius: site.spread * 256 + 128,
            intensity,
            tint,
        });
    }
    light.set_sources(sources);
}

/// Tints the units and buildings by the light where they stand.
fn light_units(
    sim: Res<Sim>,
    view: Res<UnitView>,
    light: Option<Res<WorldLight>>,
    mut units: Query<(&SimUnit, &mut Sprite), Without<BuildingSprite>>,
    mut buildings: Query<(&BuildingSprite, &mut Sprite), Without<SimUnit>>,
) {
    let Some(light) = light else {
        return;
    };
    for (sprite, mut picture) in &mut units {
        let Some(unit) = sim.0.unit(sprite.0) else {
            continue;
        };
        let cell = unit.cell();
        let level = sim.0.map.level(cell.0, cell.1) as i32 + if unit.on_bridge { 4 } else { 0 };
        let aircraft = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).is_some_and(|kind| kind.considered_aircraft);
        let extra = if unit.stats.infantry {
            light.setup.extra_infantry
        } else if aircraft {
            light.setup.extra_aircraft
        } else {
            light.setup.extra_unit
        };
        let wanted = curtained(light.colour(&Lit { cell, level, rule: LightRule::Object { extra } }, picture.color.alpha()), &sim.0, unit.curtain_until);
        if picture.color != wanted {
            picture.color = wanted;
        }
    }
    for (sprite, mut picture) in &mut buildings {
        let Some(building) = sim.0.building(sprite.0) else {
            continue;
        };
        if sprite.2.as_ref().is_some_and(|key| key.4) {
            continue;
        }
        let level = sim.0.map.level(building.cell.0, building.cell.1) as i32;
        // Plus the type's `ExtraLight`.
        let extra = light.setup.buildings.get(building.kind as usize).map_or(0, |b| b.1);
        let wanted = curtained(light.colour(&Lit { cell: building.cell, level, rule: LightRule::Object { extra } }, picture.color.alpha()), &sim.0, building.curtain_until);
        if picture.color != wanted {
            picture.color = wanted;
        }
    }
}

/// An object's colour under the iron curtain.
fn curtained(colour: Color, world: &ra2_sim::World, until: u64) -> Color {
    let now = world.current_tick();
    if until <= now {
        return colour;
    }
    let total = world.super_rules.curtain_duration as u64;
    let scale = curtain_intensity(total.saturating_sub(until - now), total);
    let linear = colour.to_linear();
    Color::LinearRgba(LinearRgba::new(linear.red * scale, linear.green * scale, linear.blue * scale, linear.alpha))
}

/// How bright a curtained object is drawn, `frames` into a curtain of `total`
/// frames (1 is as usual).
// A flash up to twice as bright over 6 frames, held 4, falling to a fifth
// over 20; then a pulse between a fifth and a half (8 frames up, 16 down)
// until fewer than 54 frames are left; a fifth until fewer than 31; then the
// flash in reverse: up over 6, held 4, down to as usual over 20.
fn curtain_intensity(frames: u64, total: u64) -> f32 {
    let (mut stage, mut began, mut length) = (0u8, 0u64, 0u64);
    for frame in 0..=frames {
        let left = total.saturating_sub(frame);
        let over = frame - began >= length;
        let next = match stage {
            0 => Some((1, 6)),
            1 if over => Some((2, 4)),
            2 if over => Some((3, 20)),
            3 if over => Some((4, 8)),
            4 if over => Some((5, 16)),
            5 if over && left < 54 => {
                stage = 6;
                None
            }
            5 if over => Some((4, 8)),
            6 if left < 31 => Some((7, 6)),
            7 if over => Some((8, 4)),
            8 if over => Some((9, 20)),
            9 if over => {
                stage = 10;
                None
            }
            _ => None,
        };
        if let Some((to, frames)) = next {
            (stage, began, length) = (to, frame, frames);
        }
    }
    let remaining = length.saturating_sub(frames - began) as i64;
    let scale = match stage {
        1 => (12 - remaining) * 256 / 6,
        2 | 8 => 512,
        3 => (remaining * 461 + 1020) / 20,
        4 => (1024 - remaining * 77) / 8,
        5 => (remaining * 77 + 816) / 16,
        6 => 51,
        7 => (3072 - remaining * 461) / 6,
        9 => (remaining + 20) * 256 / 20,
        _ => 256,
    };
    scale as f32 / 256.0
}

/// The image of one part of a building, and where its middle goes relative to the
/// building's draw point.
/// Where a sprite's centre goes for its pixels to land on the screen's: a
/// frame of odd width or height has its centre half a pixel off the grid.
fn snapped(point: Vec2, middle: Vec2) -> Vec2 {
    point.round() + middle
}

fn part_image(
    view: &mut UnitView,
    files: &GameFiles,
    images: &mut Assets<Image>,
    key: &PartKey,
) -> Option<(Handle<Image>, Vec2)> {
    let art = &mut view.art;
    view.building_images
        .entry(key.clone())
        .or_insert_with(|| {
            // A name starting "vxl:" is a voxel turret, its frame the facing.
            let drawn = match key.0.strip_prefix("vxl:") {
                Some(model) => {
                    let mut fields = model.split(':');
                    let name = fields.next()?;
                    let mut shift = fields.filter_map(|n| n.parse::<i32>().ok());
                    let (x, y, recoil) = (shift.next().unwrap_or(0), shift.next().unwrap_or(0), shift.next().unwrap_or(0));
                    let mut drawn = art.turret(files, name, (key.1 as u8) << 3, key.2, key.3, recoil as f32 / 2.0)?;
                    drawn.offset.0 += x;
                    drawn.offset.1 += y;
                    drawn
                }
                None => match key.0.strip_prefix("bridge:") {
                    Some(name) => art.bridge(files, name, key.1, key.3, key.4)?,
                    None => art.building(files, &key.0, key.1, key.2, key.3, key.4)?,
                },
            };
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        })
        .clone()
}

/// Has the announcer report on the player's production.
// DATA: the `eva.ini` entries.
fn announce(sim: Res<Sim>, mut cues: MessageWriter<Cue>, mut last: Local<Option<AnnounceState>>, mut low: Local<bool>) {
    let Some(house) = sim.0.houses.get(local() as usize) else {
        return;
    };
    // Said once each time power runs short, while
    // the house has a construction yard; not again until power has come back.
    let short = sim.0.low_power(local());
    let yard = sim.0.buildings().iter().any(|b| b.owner == local() && sim.0.building_types.get(b.kind as usize).is_some_and(|k| k.construction_yard));
    if short && yard && !*low {
        cues.write(Cue::Eva("EVA_LowPower"));
    }
    *low = short && (yard || *low);
    let options = sim.0.buildable(local()).len();
    let ready = [house.production, house.defence].map(|slot| slot.is_some_and(|p| p.is_ready()));
    let tick = sim.0.current_tick();
    let mut state = AnnounceState { options, ready, next_funds_warning: 0 };
    if let Some(before) = &*last {
        // Said whenever a cameo is added.
        if options > before.options && before.options > 0 {
            cues.write(Cue::Eva("EVA_NewConstructionOptions"));
        }
        // Said once when a building finishes.
        if (0..2).any(|slot| ready[slot] && !before.ready[slot]) {
            cues.write(Cue::Eva("EVA_ConstructionComplete"));
        }
        // Said when the player has under 100 credits
        // and owns a factory, again every `SpeakDelay`. It is not tied to a
        // production standing still.
        // DATA: `SpeakDelay` is 2 minutes in the stock rules; times 900.
        state.next_funds_warning = before.next_funds_warning;
        if house.credits < 100 && options > 0 && tick >= before.next_funds_warning {
            cues.write(Cue::Eva("EVA_InsufficientFunds"));
            state.next_funds_warning = tick + 1800;
        }
    }
    *last = Some(state);
}

struct AnnounceState {
    options: usize,
    ready: [bool; 2],
    next_funds_warning: u64,
}

/// A building type to build and place automatically; see `RA2_BUILD`.
#[derive(Resource)]
struct AutoBuild(u16);

fn auto_build(
    auto: Option<Res<AutoBuild>>,
    catalog: Res<BuildCatalog>,
    mut sim: ResMut<Sim>,
    mut placing: ResMut<Placing>,
    mut done: Local<bool>,
    mut based: Local<bool>,
    units: Res<UnitCatalog>,
    mut asked: Local<usize>,
    mut enemies: Local<bool>,
    mut killed: Local<bool>,
    mut fired: Local<Vec<bool>>,
    mut mode: ResMut<crate::control::Mode>,
) {
    // RA2_BASE=GAPOWR;GAREFN;GAWALL@30,92 puts those buildings down as soon as the
    // player's first building stands: at the cell given, or wherever each first fits; for looking at
    // buildings without playing through a build order.
    if !*based
        && let Ok(names) = std::env::var("RA2_BASE") {
            let yard = sim.0.buildings().iter().find(|b| b.owner == local() && b.buildup.is_none()).map(|b| b.cell);
            if let Some(yard) = yard {
                *based = true;
                for name in names.split(';') {
                    // NAME@x,y puts it at that cell.
                    let (name, at) = match name.split_once('@') {
                        Some((name, at)) => {
                            let at: Vec<i32> = at.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                            (name, (at.len() == 2).then(|| (at[0], at[1])))
                        }
                        None => (name, None),
                    };
                    let Some(kind) = catalog.0.iter().position(|entry| entry.name.eq_ignore_ascii_case(name.trim())) else {
                        warn!("RA2_BASE: no building {name}");
                        continue;
                    };
                    let mut cells: Vec<(i32, i32)> =
                        (-14..=14).flat_map(|dy| (-14..=14).map(move |dx| (yard.0 + dx, yard.1 + dy))).collect();
                    cells.sort_by_key(|cell| (cell.0 - yard.0).pow(2) + (cell.1 - yard.1).pow(2));
                    let spot = match at {
                        Some(cell) => sim.0.place_now(local(), kind as u16, cell).then_some(cell),
                        None => cells.into_iter().find(|&cell| sim.0.place_now(local(), kind as u16, cell)),
                    };
                    let id = spot.and_then(|spot| sim.0.buildings().iter().find(|b| b.cell == spot).map(|b| b.id));
                    info!("RA2_BASE: {name} at {spot:?}, building {id:?}");
                }
                // RA2_HURT=1 halves the health of the player's buildings then.
                if std::env::var_os("RA2_HURT").is_some() {
                    sim.0.hurt_buildings_for_tests(local());
                }
            }
        }
    // RA2_SUPER='0@40,90@300;3@30,95@400;4@50,95@420' charges the player's
    // super weapon of that kind (if granted) and fires it at the cell at the
    // tick, for looking at super weapons; after a kind 3 the destination is
    // being chosen.
    if let Ok(list) = std::env::var("RA2_SUPER") {
        let entries: Vec<&str> = list.split(';').collect();
        fired.resize(entries.len(), false);
        for (at, entry) in entries.iter().enumerate() {
            let parts: Vec<Vec<i64>> = entry.split('@').map(|part| part.split(',').filter_map(|n| n.trim().parse().ok()).collect()).collect();
            let (Some(&[kind]), Some(&[x, y]), Some(&[when])) = (parts.first().map(|v| &v[..]), parts.get(1).map(|v| &v[..]), parts.get(2).map(|v| &v[..])) else {
                continue;
            };
            if fired[at] || sim.0.current_tick() < when as u64 {
                continue;
            }
            fired[at] = true;
            let Some(index) = sim.0.super_types.iter().position(|t| t.kind == kind as i32) else {
                continue;
            };
            if let Some(state) = sim.0.houses[local() as usize].supers.get_mut(index).filter(|state| state.granted) {
                state.ready = true;
                state.left = 0;
            }
            sim.give(Order::Super { house: local(), index: index as u16, cell: (x as i32, y as i32) });
            *mode = match kind {
                3 => sim.0.super_types.iter().position(|t| t.kind == 4).map_or(crate::control::Mode::None, |second| crate::control::Mode::Super(second as u16)),
                _ => crate::control::Mode::None,
            };
            info!("RA2_SUPER: kind {kind} at {x},{y}");
        }
    }
    // RA2_KILL=house,tick destroys everything a house has at a tick, for
    // checking the end of the game.
    if let Ok(kill) = std::env::var("RA2_KILL") {
        let numbers: Vec<u64> = kill.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if let [house, at] = numbers[..] {
            // At or after the tick: at speed, several pass in a frame.
            if sim.0.current_tick() >= at && !*killed {
                *killed = true;
                sim.0.destroy_house(house as u8);
            }
        }
    }
    // RA2_CAPTURE=tick hands the first other house's construction yard to the
    // player at that tick (or as soon after as one stands), for checking a
    // captured yard.
    if let Some(at) = std::env::var("RA2_CAPTURE").ok().and_then(|n| n.trim().parse::<u64>().ok()) {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if sim.0.current_tick() >= at && !DONE.load(std::sync::atomic::Ordering::Relaxed) {
            let yard = sim.0.buildings().iter().find(|b| b.owner != local() && b.owner != 0 && b.buildup.is_none() && sim.0.building_types.get(b.kind as usize).is_some_and(|k| k.construction_yard)).map(|b| (b.id, b.cell));
            if let Some((id, cell)) = yard {
                DONE.store(true, std::sync::atomic::Ordering::Relaxed);
                sim.0.give_building_for_tests(id, local());
                info!("RA2_CAPTURE: yard {id} at {cell:?} is the player's");
            }
        }
    }
    // RA2_ENEMIES='HTNK@30,92;E2@31,95;MTNK@28,90,1' puts units on the map (of house 2
    // unless a third number says otherwise)
    // once, for checking fights.
    if !*enemies {
        *enemies = true;
        for entry in std::env::var("RA2_ENEMIES").unwrap_or_default().split(';') {
            let Some((name, cell)) = entry.split_once('@') else {
                continue;
            };
            // A third number is the owner (1 is the player); 2 if not given.
            let cell: Vec<i32> = cell.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            let kind = units.0.iter().position(|unit| unit.name.eq_ignore_ascii_case(name.trim()));
            let owner = cell.get(2).copied().unwrap_or(2) as u8;
            if let (Some(kind), Some(&[x, y])) = (kind, cell.get(..2)) {
                let stats = ra2_sim::UnitStats { kind: kind as u16 + 1, ..sim.0.unit_types[kind].stats };
                let id = if stats.infantry { sim.0.add_infantry((x, y), 0, 0x80, stats, owner) } else { Some(sim.0.add_unit((x, y), 0x80, stats, owner)) };
                // Two more numbers send it to a cell at once.
                if let (Some(unit), Some(&[x, y])) = (id, cell.get(3..5)) {
                    sim.give(Order::Move { unit, x, y });
                }
            }
        }
    }
    // RA2_UNITS=MTNK,MTNK,FV asks for those units once each can be built.
    if let Ok(names) = std::env::var("RA2_UNITS") {
        let wanted: Vec<&str> = names.split(',').map(str::trim).collect();
        if let Some(name) = wanted.get(*asked) {
            let offered = sim.0.buildable_units(local());
            let unit = units.0.iter().position(|entry| entry.name.eq_ignore_ascii_case(name)).map(|unit| unit as u16);
            if let Some(unit) = unit.filter(|unit| offered.contains(unit)) {
                sim.give(Order::BuildUnit { house: local(), unit });
                *asked += 1;
            }
        }
    }
    let Some(auto) = auto else {
        return;
    };
    if *done {
        return;
    }
    let combat = sim.0.is_defence(auto.0);
    let production = sim.0.houses.get(local() as usize).and_then(|house| house.slot(combat));
    match production {
        None => sim.give(Order::Build { house: local(), building: auto.0 }),
        Some(production) if production.is_ready() => {
            let place: Vec<i32> = std::env::var("RA2_PLACE")
                .unwrap_or_default()
                .split(',')
                .filter_map(|n| n.trim().parse().ok())
                .collect();
            if let [x, y] = place[..] {
                let own: Vec<_> = sim.0.buildings().iter().filter(|b| b.owner == local()).map(|b| (b.cell, b.size)).collect();
                info!("RA2_PLACE: can place at ({x}, {y}): {}; own buildings {own:?}", sim.0.can_place(local(), auto.0, (x, y)));
                sim.give(Order::Place { house: local(), building: auto.0, x, y });
            }
            // RA2_PLACING=1 picks the finished building up for placing instead.
            if std::env::var_os("RA2_PLACING").is_some() {
                placing.0 = Some(auto.0);
            }
            *done = true;
        }
        Some(_) => {}
    }
}

/// How far up the screen the slope under a unit lifts it.
// What stands on the
// ground is drawn at the ground's height under its own point, slope and all.
pub(crate) fn slope_up(sim: &World, unit: &ra2_sim::Unit) -> Vec2 {
    if unit.on_bridge {
        return Vec2::ZERO;
    }
    Vec2::new(0.0, lift(sim.map.slope_z(unit.x, unit.y)))
}

/// The cell under a scene point, allowing for the height of the ground.
// Probes run up the screen a
// pixel at a time from 180 below the point (twelve levels); each takes the
// cell there on flat ground, and the first whose own level lifts it to the
// point or above is the answer, so the cell nearest the viewer wins. Only the
// cell's level counts: no slopes, no cliff faces. A cell under a high bridge
// is tried four levels up, so the deck wins over what shows under it. With no
// hit, the cell on flat ground.
pub(crate) fn cell_at(sim: &World, origin: Vec2, point: Vec2) -> (i32, i32) {
    let flat = |y: f32| {
        let across = (point.x - origin.x - CELL_WIDTH as f32 / 2.0) / (CELL_WIDTH as f32 / 2.0);
        let down = (y - origin.y) / (CELL_HEIGHT as f32 / 2.0) - 1.0;
        (((across + down) / 2.0).round() as i32, ((down - across) / 2.0).round() as i32)
    };
    for back in (1..=180).rev() {
        let probe = point.y + back as f32;
        let cell = flat(probe);
        let mut lifted = probe - (sim.map.level(cell.0, cell.1) as i32 * LEVEL_HEIGHT) as f32;
        if sim.map.bridged(cell.0, cell.1) {
            lifted -= (4 * LEVEL_HEIGHT) as f32;
        }
        if lifted <= point.y {
            return cell;
        }
    }
    flat(point.y)
}

/// A one-off animation playing on the map: its type, the tick it began, and where.
#[derive(Component)]
struct Effect {
    anim: String,
    began: u64,
    at: Vec2,
    depth: f32,
}

/// Starts the animations the map's tiles carry (waterfalls), playing for ever.
pub fn spawn_tile_anims(commands: &mut Commands, anims: &[(String, (i32, i32), (i32, i32))]) {
    for (anim, at, cell) in anims {
        let depth = 0.57 + (cell.0 + cell.1) as f32 * DEPTH_PER_ROW;
        commands.spawn((DespawnOnExit(Screen::InGame), Effect { anim: anim.clone(), began: 0, at: Vec2::new(at.0 as f32, at.1 as f32), depth }, Sprite::default()));
    }
}

/// A soldier's death sequence being played out, and the body to leave after.
#[derive(Component)]
struct Corpse {
    image: String,
    palette: Option<usize>,
    first: usize,
    count: usize,
    began: u64,
    at: (i32, i32, i32),
    depth: f32,
    body: Option<String>,
}

/// Steps the death sequences of soldiers, and leaves their bodies.
fn show_corpses(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    light: Option<Res<WorldLight>>,
    mut corpses: Query<(Entity, &Corpse, &mut Sprite, &mut Transform, Option<&mut Depth>, Option<&Lit>)>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    for (entity, corpse, mut sprite, mut transform, depth, lit_now) in &mut corpses {
        let stage = tick.saturating_sub(corpse.began) as usize;
        let point = scene_point_3d(view.origin, corpse.at);
        if stage >= corpse.count {
            commands.entity(entity).despawn();
            if let Some(body) = corpse.body.as_ref().map(|body| body.to_uppercase()).filter(|body| view.anim_types.contains_key(body)) {
                commands.spawn((
                    DespawnOnExit(Screen::InGame),
                    Effect { anim: body, began: tick, at: point, depth: 0.7 + (corpse.depth - 2.0).max(0.0) },
                    Sprite::default(),
                    Depth::Upright { adjust: -2 - height_pixels(corpse.at.2), write: false },
                ));
            }
            continue;
        }
        let level = sim.0.map.level(corpse.at.0.div_euclid(LEPTONS_PER_CELL), corpse.at.1.div_euclid(LEPTONS_PER_CELL)) as i32;
        let frame = corpse.first + stage;
        let key = (corpse.image.clone(), frame, corpse.palette, level, false);
        let art = &mut view.art;
        let image = view.building_images.entry(key).or_insert_with(|| {
            let drawn = art.infantry(&files, &corpse.image, frame, corpse.palette, level)?;
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::all(),
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        });
        if let Some((handle, middle)) = image {
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
            let centre = snapped(point, *middle);
            transform.translation = Vec3::new(centre.x, -centre.y, corpse.depth);
            // Drawn as the soldier was.
            set_depth(&mut commands, entity, depth, Depth::Upright { adjust: -13 - height_pixels(corpse.at.2), write: false });
        }
        let cell = (corpse.at.0.div_euclid(LEPTONS_PER_CELL), corpse.at.1.div_euclid(LEPTONS_PER_CELL));
        let lit = Lit { cell, level, rule: LightRule::Object { extra: light.as_ref().map_or(0, |light| light.setup.extra_infantry) } };
        if lit_now != Some(&lit) {
            commands.entity(entity).insert(lit);
        }
    }
}

/// An animation shown for as long as something in the game holds: the
/// chronosphere's placement marker, a missile's warning marker.
#[derive(Component)]
struct Marker {
    anim: String,
    cell: (i32, i32),
    began: u64,
}

/// The markers that should be up, and keeps them playing.
// The chronosphere's
// `ChronoPlacement` stands on its first cell, for its owner only, while the
// destination is being chosen; it plays its `StartSound` when it appears. A
// missile's `PSIWARN` stands on its target for a player whose psychic sensor
// covers the cell.
fn show_markers(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mode: Res<crate::control::Mode>,
    mut images: ResMut<Assets<Image>>,
    mut cues: MessageWriter<Cue>,
    mut markers: Query<(Entity, &Marker, &mut Sprite, &mut Transform)>,
    rules: Res<crate::files::Rules>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let me = local();
    let mut wanted: Vec<(String, (i32, i32))> = Vec::new();
    let choosing = matches!(*mode, crate::control::Mode::Super(index) if sim.0.super_types.get(index as usize).is_some_and(|kind| kind.kind == 4));
    if let (true, Some(cell), Some(name)) = (choosing, sim.0.houses.get(me as usize).and_then(|house| house.chrono_source), rules.sound("ChronoPlacement")) {
        wanted.push((name.to_uppercase(), cell));
    }
    for cell in sim.0.nuke_warnings(me) {
        wanted.push(("PSIWARN".to_owned(), cell));
    }
    let mut shown = Vec::new();
    for (entity, marker, mut sprite, mut transform) in &mut markers {
        if !wanted.iter().any(|(anim, cell)| *anim == marker.anim && *cell == marker.cell) {
            commands.entity(entity).despawn();
            continue;
        }
        shown.push((marker.anim.clone(), marker.cell));
        let Some(anim) = view.anim_types.get(&marker.anim) else {
            continue;
        };
        let count = view.art.frame_count(&files, &anim.image);
        let count = if anim.shadow { count / 2 } else { count };
        let frame = anim_frame(anim, count, tick.saturating_sub(marker.began), view.game_speed).unwrap_or(anim.loop_start);
        let art = &mut view.art;
        let alt = anim.alt_palette;
        let key = if alt { (format!("{}#alt", anim.image), frame) } else { (anim.image.clone(), frame) };
        let image = view.effect_images.entry(key).or_insert_with(|| {
            let drawn = if alt { art.infantry(&files, &anim.image, frame, Some(0), 0)? } else { art.effect(&files, &anim.image, frame)? };
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        });
        if let Some((handle, middle)) = image {
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
            let (x, y) = (marker.cell.0 * LEPTONS_PER_CELL + 128, marker.cell.1 * LEPTONS_PER_CELL + 128);
            let at = scene_point_3d(view.origin, (x, y, sim.0.ground_z(x, y) + 5));
            let centre = snapped(at + Vec2::new(0.0, anim.y_draw_offset as f32), *middle);
            let depth = 2.0 + ((marker.cell.0 + marker.cell.1) as f32 + 2.5) * DEPTH_PER_ROW;
            transform.translation = Vec3::new(centre.x, -centre.y, depth);
            let colour = Color::srgba(1.0, 1.0, 1.0, anim.opacity);
            if sprite.color != colour {
                sprite.color = colour;
            }
        }
    }
    for (anim, cell) in wanted {
        if shown.contains(&(anim.clone(), cell)) {
            continue;
        }
        if let Some(sound) = view.anim_types.get(&anim).and_then(|kind| kind.report.clone()) {
            let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
            cues.write(Cue::SoundOn(sound, scene_point_3d(view.origin, (x, y, sim.0.ground_z(x, y)))));
        }
        commands.spawn((DespawnOnExit(Screen::InGame), Marker { anim, cell, began: tick }, Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0)));
    }
}

/// The pixels up the screen a height in leptons is drawn, as `scene_point_3d`
/// has it.
fn height_pixels(z: i32) -> i32 {
    (z as f32 * LEVEL_HEIGHT as f32 / ra2_sim::combat::LEVEL_HEIGHT as f32).round() as i32
}

/// A shot in flight, by its id.
#[derive(Component)]
struct ShotSprite(u32);

/// Something the simulation reports for the player to see or hear, taken from it
/// once each frame.
#[derive(Message, Clone)]
pub(crate) struct SimEvent(pub(crate) ra2_sim::combat::Event);

fn publish_events(mut sim: ResMut<Sim>, mut events: MessageWriter<SimEvent>) {
    events.write_batch(sim.0.take_events().into_iter().map(SimEvent));
}

/// A laser beam fading out: the frame count it lasts and the tick it was fired.
#[derive(Component)]
struct Beam {
    until: u64,
}

/// How many pixels up the screen something a height off the ground is drawn.
// 104 leptons to 15 pixels.
pub(crate) fn lift(altitude: i32) -> f32 {
    (altitude as f32 * 0.143504 + if altitude >= 728 { 1.0 } else { 0.0 } + 0.5).floor()
}

/// A point in the air above the map, in scene pixels.
pub(crate) fn scene_point_3d(origin: Vec2, (x, y, z): (i32, i32, i32)) -> Vec2 {
    scene_point(origin, x, y, 0) - Vec2::new(0.0, z as f32 * LEVEL_HEIGHT as f32 / ra2_sim::combat::LEVEL_HEIGHT as f32)
}


/// Animations step on and end.
fn step_effects(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut effects: Query<(Entity, &Effect, &mut Sprite, &mut Transform)>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    for (entity, effect, mut sprite, mut transform) in &mut effects {
        let Some(anim) = view.anim_types.get(&effect.anim) else {
            commands.entity(entity).despawn();
            continue;
        };
        let count = view.art.frame_count(&files, &anim.image);
        let count = if anim.shadow { count / 2 } else { count };
        let Some(frame) = anim_frame(anim, count, tick.saturating_sub(effect.began), view.game_speed) else {
            commands.entity(entity).despawn();
            continue;
        };
        let art = &mut view.art;
        let alt = anim.alt_palette;
        let key = if alt { (format!("{}#alt", anim.image), frame) } else { (anim.image.clone(), frame) };
        let image = view.effect_images.entry(key).or_insert_with(|| {
            let drawn = if alt { art.infantry(&files, &anim.image, frame, Some(0), 0)? } else { art.effect(&files, &anim.image, frame)? };
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        });
        match image {
            Some((handle, middle)) => {
                if sprite.image != *handle {
                    sprite.image = handle.clone();
                }
                let centre = snapped(effect.at + Vec2::new(0.0, anim.y_draw_offset as f32), *middle);
                transform.translation = Vec3::new(centre.x, -centre.y, effect.depth);
                let wanted = Color::srgba(1.0, 1.0, 1.0, anim.opacity);
                if sprite.color != wanted {
                    sprite.color = wanted;
                }
            }
            None => sprite.image = Handle::default(),
        }
    }
}

/// Shots that can be seen. One that `Rotates` shows the frame for the way it
/// is going, of 32.
fn show_shots(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    armoury: Option<Res<crate::armoury::Armoury>>,
    mut images: ResMut<Assets<Image>>,
    mut shots: Query<(Entity, &ShotSprite, &mut Sprite, &mut Transform, Option<&mut Depth>)>,
) {
    let Some(armoury) = armoury else {
        return;
    };
    let view = &mut *view;
    let flying: Vec<(u32, u16, (f64, f64, f64), u8, Option<u8>)> = sim
        .0
        .bullets()
        .iter()
        .map(|b| (b.id, b.weapon, b.position, b.facing(), None))
        .chain(sim.0.leaps().iter().map(|leap| (leap.id | 0x8000_0000, leap.weapon, leap.position, leap.facing, Some(leap.house))))
        .chain(sim.0.nukes().iter().filter_map(|nuke| Some((nuke.id, sim.0.nuke_weapon(nuke)?, (nuke.at.0 as f64, nuke.at.1 as f64, nuke.at.2 as f64), 0, Some(nuke.house)))))
        .collect();
    let mut seen: Vec<u32> = Vec::new();
    for (entity, shot, mut sprite, mut transform, shot_depth) in &mut shots {
        let Some(&(_, weapon, position, facing, house)) = flying.iter().find(|b| b.0 == shot.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        seen.push(shot.0);
        let Some(art_name) = armoury.weapon_looks.get(weapon as usize).and_then(|look| look.image.clone()) else {
            continue;
        };
        let count = view.art.frame_count(&files, &art_name);
        let frame = if count >= 32 { ((((facing as usize) >> 2) + 1) >> 1) & 31 } else { 0 };
        let palette = house.and_then(|house| view.palette_of(house));
        let art = &mut view.art;
        let key = match house {
            Some(house) => (format!("{art_name}#{house}"), frame),
            None => (art_name.clone(), frame),
        };
        // A shot whose art is a voxel model (the Kirov's falling bomb, ZBOMB)
        // is drawn from the model, facing its way.
        let voxel = count == 0;
        let key = if voxel { (format!("vxl:{art_name}"), ((facing as usize) >> 3) & 31) } else { key };
        let image = view.effect_images.entry(key).or_insert_with(|| {
            let drawn = match house {
                _ if voxel => art.turret(&files, &art_name, facing & !7, palette, 0, 0.0)?,
                Some(_) => art.infantry(&files, &art_name, frame, palette, 0)?,
                None => art.effect(&files, &art_name, frame)?,
            };
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(
                Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                drawn.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
        });
        if let Some((handle, middle)) = image {
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
            let p = position;
            let centre = snapped(scene_point_3d(view.origin, (p.0 as i32, p.1 as i32, p.2 as i32)), *middle);
            let depth = 2.0 + ((p.0 + p.1) as f32 / LEPTONS_PER_CELL as f32 + 2.0) * DEPTH_PER_ROW;
            transform.translation = Vec3::new(centre.x, -centre.y, depth);
            set_depth(&mut commands, entity, shot_depth, Depth::Upright { adjust: -2 - height_pixels(p.2 as i32), write: false });
        }
    }
    for &(id, weapon, ..) in &flying {
        let visible = armoury.weapon_looks.get(weapon as usize).is_some_and(|look| look.image.is_some());
        if visible && !seen.contains(&id) {
            commands.spawn((DespawnOnExit(Screen::InGame), ShotSprite(id), Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0)));
        }
    }
}

/// Laser beams that have had their time go.
fn end_beams(mut commands: Commands, sim: Res<Sim>, beams: Query<(Entity, &Beam)>) {
    let tick = sim.0.current_tick();
    for (entity, beam) in &beams {
        if tick >= beam.until {
            commands.entity(entity).despawn();
        }
    }
}

/// A vehicle's roll and pitch, and how fast each is changing, in radians.
#[derive(Component, Default, Clone, Copy)]
struct Rock {
    roll: f32,
    roll_speed: f32,
    pitch: f32,
    pitch_speed: f32,
}

impl Rock {
    /// A push from a point `side` leptons to the vehicle's left or right, of
    /// a strength, on a vehicle of a `Weight`.
    // The push is 0.04 less 0.000025 a lepton of the distance, times the
    // strength, over the weight; under 0.01 it does nothing, over 0.05 it is
    // 0.05. Of the direction from the point, the part across the vehicle
    // sets the roll's speed and half the part along it the pitch's.
    fn push(&mut self, facing: u8, side: f32, strength: f32, weight: f32) {
        let facing16 = (facing as i32) << 8;
        let turn = |f: i32| (f as i16 as i32 - 0x3fff) as f32 * (-2.0 * std::f32::consts::PI / 65536.0);
        let a = turn(facing16 - 0x3fff);
        let point = (a.cos() * side, -a.sin() * side);
        // From the point to the vehicle, with y the other way up.
        let d = (-point.0, point.1);
        let distance = (d.0 * d.0 + d.1 * d.1).sqrt();
        let mut impulse = (0.04 - distance * 0.000025) * strength / weight.max(f32::MIN_POSITIVE);
        if distance < 0.00002 || impulse < 0.01 {
            return;
        }
        impulse = impulse.min(0.05);
        let (n0, n1) = if distance == 0.0 { d } else { (d.0 / distance, d.1 / distance) };
        let b = turn(facing16);
        let (sin, cos) = b.sin_cos();
        let along = n1 * sin + n0 * cos;
        let mut across = (n0 * sin - n1 * cos).abs();
        if (cos * along - sin * across - n0).abs() > 0.0002 || (sin * along + cos * across - n1).abs() > 0.0002 {
            across = -across;
        }
        self.pitch_speed = along * impulse * 0.5;
        self.roll_speed = -across * impulse;
    }

    /// One frame: each angle moves by its speed (no further than an eighth
    /// of a turn), is pulled back by 0.005 a frame coming back and 0.002 going
    /// out, and stops when it passes level.
    fn step(&mut self) {
        fn axis(angle: &mut f32, speed: &mut f32) {
            const EPS: f32 = 0.00002;
            let limit = std::f32::consts::FRAC_PI_4;
            let old = *angle;
            if *speed == 0.0 {
                *angle = 0.0;
            } else {
                *angle += *speed;
                if *angle > limit {
                    *angle = limit;
                    *speed = 0.0;
                } else if *angle < -limit {
                    *angle = -limit;
                    *speed = 0.0;
                }
                if old > EPS {
                    *speed -= if *speed <= 0.0 { 0.005 } else { 0.002 };
                    if *angle < EPS {
                        *angle = 0.0;
                        *speed = 0.0;
                    }
                } else {
                    *speed += if *speed >= 0.0 { 0.005 } else { 0.002 };
                }
            }
            if old < -EPS && *angle > -EPS {
                *angle = 0.0;
                *speed = 0.0;
            }
        }
        axis(&mut self.roll, &mut self.roll_speed);
        axis(&mut self.pitch, &mut self.pitch_speed);
    }

    fn still(&self) -> bool {
        self.roll == 0.0 && self.roll_speed == 0.0 && self.pitch == 0.0 && self.pitch_speed == 0.0
    }

    /// The model-name mark that draws it so (see `UnitArt::sprite`), in
    /// hundredths of a radian, with the body's facing.
    fn mark(&self, facing: u8) -> Option<String> {
        let (roll, pitch) = ((self.roll * 100.0).round() as i32, (self.pitch * 100.0).round() as i32);
        (roll != 0 || pitch != 0).then(|| format!("@q{roll}_{pitch}_{facing}"))
    }
}

/// Steps the rocking vehicles once for each tick that has passed.
fn rock_units(mut commands: Commands, sim: Res<Sim>, mut view: ResMut<UnitView>, mut rocks: Query<(Entity, &mut Rock)>) {
    let tick = sim.0.current_tick();
    let steps = tick.saturating_sub(view.rock_tick).min(60);
    view.rock_tick = tick;
    for (entity, mut rock) in &mut rocks {
        for _ in 0..steps {
            rock.step();
        }
        if rock.still() {
            commands.entity(entity).remove::<Rock>();
        }
    }
}

/// A spark flying from a bitten vehicle.
#[derive(Component)]
struct Spark {
    at: Vec3,
    speed: Vec3,
    born: u64,
}

/// The `Spark` particles of `DefaultSparkSystem`.
// DATA: `[SparkSys] ParticleCap=6` and `[Spark]` `XVelocity`, `YVelocity`,
// `MinZVelocity`, `ZVelocityRange`, `ColorList`, `ColorSpeed`.
// Not original behaviour: how the particles move and fade is a stand-in; the
// particle system's own rules are not followed.
fn spawn_sparks(commands: &mut Commands, at: (i32, i32, i32), tick: u64, pick: &mut impl FnMut(usize) -> usize) {
    for _ in 0..6 {
        let speed = Vec3::new(pick(21) as f32 - 10.0, pick(21) as f32 - 10.0, 40.0 + pick(16) as f32) / 4.0;
        commands.spawn((
            DespawnOnExit(Screen::InGame),
            Spark { at: Vec3::new(at.0 as f32, at.1 as f32, at.2 as f32 + 40.0), speed, born: tick },
            Sprite { color: Color::WHITE, custom_size: Some(Vec2::splat(2.0)), ..default() },
            Transform::from_xyz(0.0, 0.0, -10.0),
        ));
    }
}

const SPARK_COLOURS: [[f32; 3]; 4] = [[255.0, 255.0, 255.0], [200.0, 200.0, 80.0], [200.0, 10.0, 10.0], [0.0, 0.0, 0.0]];

fn show_sparks(mut commands: Commands, sim: Res<Sim>, view: Res<UnitView>, mut sparks: Query<(Entity, &Spark, &mut Sprite, &mut Transform)>) {
    let tick = sim.0.current_tick();
    for (entity, spark, mut sprite, mut transform) in &mut sparks {
        let age = tick.saturating_sub(spark.born) as f32;
        let along = age * 0.13;
        if along >= (SPARK_COLOURS.len() - 1) as f32 {
            commands.entity(entity).despawn();
            continue;
        }
        let (i, t) = (along as usize, along.fract());
        let colour: Vec<f32> = (0..3).map(|c| (SPARK_COLOURS[i][c] * (1.0 - t) + SPARK_COLOURS[i + 1][c] * t) / 255.0).collect();
        sprite.color = Color::srgb(colour[0], colour[1], colour[2]);
        let gravity = 1.5;
        let p = spark.at + spark.speed * age - Vec3::new(0.0, 0.0, gravity * age * age / 2.0);
        let point = scene_point_3d(view.origin, (p.x as i32, p.y as i32, p.z.max(0.0) as i32));
        let depth = 2.0 + ((p.x + p.y) / LEPTONS_PER_CELL as f32 + 2.0) * DEPTH_PER_ROW;
        transform.translation = Vec3::new(point.x.round(), -point.y.round(), depth);
    }
}

/// A thrown piece of debris on the screen, by its id.
#[derive(Component)]
struct DebrisSprite(u32);

/// Draws thrown debris, and lays its trailer as it flies.
// An animation shows its loop, a frame every other tick; a model
// turns by its angular speed. A `TrailerAnim` is left every
// `TrailerSeperation` frames, a model's every other frame.
// Not original behaviour: a model is turned about the upright only, where
// the engine tumbles it about a random axis; the loop's rate is not the
// type's `RandomRate`.
fn show_debris(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<(Entity, &DebrisSprite, &mut Sprite, &mut Transform)>,
    mut trailed: Local<u64>,
) {
    let view = &mut *view;
    let world = &sim.0;
    let tick = world.current_tick();
    let lay = tick != *trailed;
    *trailed = tick;
    let mut seen = Vec::new();
    for (entity, shown, mut sprite, mut transform) in &mut sprites {
        let Some(piece) = world.debris().iter().find(|d| d.id == shown.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        seen.push(piece.id);
        let at = (piece.position.0 as i32, piece.position.1 as i32, piece.position.2 as i32);
        let age = tick.saturating_sub(piece.born);
        let point = scene_point_3d(view.origin, at);
        let depth = 2.0 + ((piece.position.0 + piece.position.1) as f32 / LEPTONS_PER_CELL as f32 + 2.0) * DEPTH_PER_ROW;
        let (image, trailer, every) = match piece.voxel {
            Some(voxel) => {
                let Some(rule) = world.remains_rules.voxels.get(voxel as usize) else {
                    continue;
                };
                let facing = ((piece.spin * age as f64) as i64 + piece.id as i64 * 37).rem_euclid(256) as u8 & !7;
                let key = (format!("vxl:{}", rule.image), facing as usize);
                let art = &mut view.art;
                let image = view.effect_images.entry(key).or_insert_with(|| {
                    let drawn = art.sprite(&files, &rule.image, facing, None, 0)?;
                    let size = Vec2::new(drawn.width as f32, drawn.height as f32);
                    let image = Image::new(Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 }, TextureDimension::D2, drawn.rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
                    Some((images.add(image), size / 2.0 - Vec2::new(drawn.origin.0 as f32, drawn.origin.1 as f32)))
                });
                (image.clone(), rule.motion.trailer, 2)
            }
            None => {
                let Some(name) = view.anim_names.get(piece.anim as usize).cloned() else {
                    continue;
                };
                let Some(anim) = view.anim_types.get(&name) else {
                    continue;
                };
                let count = view.art.frame_count(&files, &anim.image);
                let count = if anim.shadow { count / 2 } else { count };
                let end = anim.loop_end.unwrap_or(count).min(count).max(anim.loop_start + 1);
                let frame = anim.loop_start + (age as usize / 2) % (end - anim.loop_start);
                let art = &mut view.art;
                let image = view.effect_images.entry((anim.image.clone(), frame)).or_insert_with_key(|key| {
                    let drawn = art.effect(&files, &key.0, key.1)?;
                    let size = Vec2::new(drawn.width as f32, drawn.height as f32);
                    let image = Image::new(Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 }, TextureDimension::D2, drawn.rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
                    Some((images.add(image), Vec2::new(drawn.offset.0 as f32, drawn.offset.1 as f32) + size / 2.0))
                });
                let rule = world.remains_rules.bouncers.get(piece.anim as usize).cloned().flatten();
                (image.clone(), rule.as_ref().map_or(ra2_sim::smudge::NO_ANIM, |r| r.trailer), rule.map_or(1, |r| r.trailer_every.max(1) as u64))
            }
        };
        if let Some((handle, middle)) = image {
            if sprite.image != handle {
                sprite.image = handle;
            }
            let centre = snapped(point, middle);
            transform.translation = Vec3::new(centre.x, -centre.y, depth);
        }
        if lay && trailer != ra2_sim::smudge::NO_ANIM && tick.is_multiple_of(every)
            && let Some(name) = view.anim_names.get(trailer as usize).cloned() {
                commands.spawn((DespawnOnExit(Screen::InGame), Effect { anim: name, began: tick, at: point, depth: depth - 0.0001 }, Sprite::default()));
            }
    }
    for piece in world.debris() {
        if !seen.contains(&piece.id) {
            commands.spawn((DespawnOnExit(Screen::InGame), DebrisSprite(piece.id), Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0)));
        }
    }
}

/// A sinking ship on the screen, by its id.
#[derive(Component)]
struct SinkSprite(u32);

/// Draws ships going down.
// Not original behaviour: the hull fades as it goes down instead of being
// hidden by the water.
fn show_sinking(
    mut commands: Commands,
    sim: Res<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<(Entity, &SinkSprite, &mut Sprite, &mut Transform)>,
) {
    let view = &mut *view;
    let world = &sim.0;
    let mut seen = Vec::new();
    for (entity, shown, mut sprite, mut transform) in &mut sprites {
        let Some(ship) = world.sinking().iter().find(|s| s.id == shown.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        seen.push(ship.id);
        let Some(model) = (ship.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).map(|k| k.image.clone()) else {
            continue;
        };
        let palette = view.unit_palette(ship.id, ship.owner);
        let cell = (ship.x.div_euclid(LEPTONS_PER_CELL), ship.y.div_euclid(LEPTONS_PER_CELL));
        let level = world.map.level(cell.0, cell.1) as i32;
        let direction = (((ship.facing >> 2) + 1) >> 1) & 31;
        // It tips 0.01 a frame, up to an eighth of a turn: one way when
        // its eight-way direction is 1 to 5, the other otherwise.
        let eighth = (((ship.facing as u32) >> 4) + 1) >> 1 & 7;
        let tip = (0.01 * (ship.depth / 5) as f32).min(std::f32::consts::FRAC_PI_4) * if (1..=5).contains(&eighth) { 1.0 } else { -1.0 };
        let model = format!("{model}{}", Rock { pitch: tip, ..Rock::default() }.mark(direction << 3).unwrap_or_default());
        let key = (model.clone(), direction, None, palette, level);
        let image = view.images.entry(key).or_insert_with(|| {
            let drawn = view.art.vehicle(&files, &model, direction << 3, None, palette, level)?;
            let size = Vec2::new(drawn.width as f32, drawn.height as f32);
            let image = Image::new(Extent3d { width: drawn.width, height: drawn.height, depth_or_array_layers: 1 }, TextureDimension::D2, drawn.rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::all());
            Some(UnitImage { handle: images.add(image), origin: Vec2::new(drawn.origin.0 as f32, drawn.origin.1 as f32), size })
        });
        let Some(image) = image else {
            continue;
        };
        if sprite.image != image.handle {
            sprite.image = image.handle.clone();
        }
        sprite.color = Color::srgba(1.0, 1.0, 1.0, (1.0 - ship.depth as f32 / 400.0).clamp(0.0, 1.0));
        let centre = scene_point(view.origin, ship.x, ship.y, level) - image.origin + image.size / 2.0 + Vec2::new(0.0, lift(ship.depth));
        let depth = 2.0 + ((cell.0 + cell.1) as f32 + 0.9) * DEPTH_PER_ROW;
        transform.translation = Vec3::new(centre.x.round(), -centre.y.round(), depth);
    }
    for ship in world.sinking() {
        if !seen.contains(&ship.id) {
            commands.spawn((DespawnOnExit(Screen::InGame), SinkSprite(ship.id), Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0)));
        }
    }
}
