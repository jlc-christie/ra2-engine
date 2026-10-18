//! Vehicles in the running game: the simulation that moves them, the sprites that
//! show them, and the mouse controls that order them about.
//!
//! Mouse control and what is drawn over the map for it are in `control.rs`.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ra2_sim::{LEPTONS_PER_CELL, Map, Order, TICKS_PER_SECOND, World};

use crate::files::GameFiles;
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
    /// Art of its build-up animation.
    pub buildup: Option<String>,
    /// Art name, and the file name of the sidebar picture.
    pub art: String,
    pub cameo: String,
    /// Key of its name in the string table.
    pub ui_name: String,
    /// How tall its selection box is, in levels.
    pub height: i32,
    /// Whether it produces things (`Factory`), so can be made the primary one.
    pub factory: bool,
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

/// How close to a vehicle's centre a click must be to select it, in pixels.
pub(crate) const PICK_RADIUS: f32 = 22.0;

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
    /// Model and owner palette of each unit, by unit id.
    looks: HashMap<u32, (String, Option<usize>)>,
    /// What each unit says when selected and when sent somewhere, by unit id.
    pub(crate) voices: HashMap<u32, (Option<String>, Option<String>)>,
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
    /// The walk counters of sprite vehicles, by unit id, and the tick they were
    /// last stepped on.
    walk_counters: HashMap<u32, u32>,
    /// Each vehicle's ramp, the one before it, and the tick it changed.
    ramps: HashMap<u32, (u8, u8, u64)>,
    walk_tick: u64,
    /// Unit pictures with the pixels the ground hides taken out, by picture and
    /// place.
    masked: HashMap<(bevy::asset::AssetId<Image>, i32, i32), Handle<Image>>,
    /// Shadows of flyers, by model and direction.
    shadow_images: HashMap<(String, u8), Option<(Handle<Image>, Vec2)>>,
    /// Where each building type's body art lies from its draw point, and its size.
    body_boxes: HashMap<u16, Option<((i32, i32), u32, u32)>>,
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
}

/// What a building type is drawn from.
struct BuildingArt {
    art: String,
    buildup: Option<String>,
    bib: Option<String>,
    deploying: Option<String>,
    under_door: Option<String>,
    turret: Option<(String, i32, i32)>,
    zshape_move: (i32, i32),
    normal_z_adjust: i32,
    anims: Vec<crate::terrain::AnimSlot>,
    charged_anim_time: f32,
}

/// Art name, frame, palette, height level, shadow.
type PartKey = (String, usize, Option<usize>, i32, bool);

#[derive(Component)]
struct UnitSprite(u32);

/// The shadow under a unit in the air, by the unit's id.
#[derive(Component)]
struct FlyerShadow(u32);

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

    pub(crate) fn game_speed(&self) -> i32 {
        self.game_speed
    }

    /// The colour a unit or building shows in on the radar.
    pub(crate) fn radar_colour_unit(&self, id: u32, owner: u8) -> [u8; 3] {
        self.art.owner_colour(if owner == 0 { self.looks.get(&id).and_then(|look| look.1) } else { self.palette_of(owner) })
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
            .add_systems(
                FixedUpdate,
                tick.run_if(in_state(Screen::InGame)).run_if(resource_exists::<Sim>).run_if(crate::pause::playing).run_if(not(resource_exists::<crate::net::NetActive>)),
            )
            .add_systems(
                Update,
                (auto_build, announce, show_units, show_buildings, show_walls, show_crates, show_mirages, show_bridges, show_ore, show_effects, show_markers, show_corpses, show_radiation)
                    .chain()
                    .run_if(in_state(Screen::InGame))
                    .run_if(resource_exists::<UnitView>),
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
    /// Model and palette, and voices, of each starting unit by id.
    looks: HashMap<u32, (String, Option<usize>)>,
    voices: HashMap<u32, (Option<String>, Option<String>)>,
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
    // DATA: `[MultiplayerDialogSettings] Shroud=yes`. RA2_NOSHROUD=1 turns it off
    // for checks.
    world.map.shroud_on = std::env::var("RA2_NOSHROUD").is_err();
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
            zshape_move: info.zshape_move,
            normal_z_adjust: info.normal_z_adjust,
            under_door: info.under_door.take(),
            anims: std::mem::take(&mut info.anims),
            charged_anim_time: info.charged_anim_time,
        });
        world.building_types.push(info.sim);
        catalog.push(CatalogEntry {
            name: info.name,
            buildup: info.buildup,
            art: info.art,
            cameo: info.cameo,
            ui_name: info.ui_name,
            height: info.height,
            factory: info.factory,
            construction_yard,
        });
    }
    let mut looks = HashMap::new();
    let mut voices = HashMap::new();
    for spawn in std::mem::take(&mut scene.unit_spawns) {
        let id = world.add_unit(spawn.cell, spawn.facing, spawn.stats, spawn.owner);
        voices.insert(id, (spawn.voice_select, spawn.voice_move));
        looks.insert(id, (spawn.image, spawn.palette));
    }
    // The foot soldiers the map starts with.
    for soldier in std::mem::take(&mut scene.infantry_spawns) {
        let Some(kind) = world.unit_types.get(soldier.kind as usize) else {
            continue;
        };
        let stats = kind.stats;
        if let Some(id) = world.add_infantry(soldier.cell, soldier.spot, soldier.facing, stats, crate::terrain::NEUTRAL) {
            let kind = &kinds[soldier.kind as usize];
            voices.insert(id, (kind.voice_select.clone(), kind.voice_move.clone()));
            looks.insert(id, (kind.image.clone(), soldier.palette));
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
            ra2_sim::smudge::AnimMark { scorch: anim.scorch, crater: anim.crater, force_big: anim.force_big_craters, size: anim.size }
        })
        .collect();
    world.splash_anims = scene.armoury.splashes.iter().map(number).collect();
    let known = |name: &str| Some(number(&name.to_owned())).filter(|&at| at != ra2_sim::smudge::NO_ANIM);
    world.super_rules.nuke_ball.1 = known("NUKEBALL");
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
        looks,
        voices,
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
    files: &GameFiles,
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
        looks,
        voices,
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
    for &id in looks.keys() {
        commands.spawn((DespawnOnExit(Screen::InGame), UnitSprite(id), Sprite::default()));
    }

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
        looks,
        voices,
        kinds,
        slam_sound: crate::audio::rules_sound(&files, "BuildingSlam"),
        click_sound: crate::audio::rules_sound(&files, "GenericClick"),
        scold_sound: crate::audio::rules_sound(&files, "ScoldSound"),
        sell_sound: crate::audio::rules_sound(&files, "SellSound"),
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
        masked: HashMap::new(),
        walk_counters: HashMap::new(),
        ramps: HashMap::new(),
        walk_tick: 0,
        body_boxes: HashMap::new(),
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
    ) {
        if sim.0.current_tick() == at {
            sim.give(Order::Suspend { house: local(), building });
        }
    }
    // RA2_UNDEPLOY_AT=building,tick,x,y packs a building up at a tick and sends its
    // vehicle to a cell.
    if let Ok(spec) = std::env::var("RA2_UNDEPLOY_AT") {
        let spec: Vec<i64> = spec.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if let [building, at, x, y] = spec[..] {
            if sim.0.current_tick() as i64 == at {
                info!("RA2_UNDEPLOY_AT: building {building}, can undeploy: {}", sim.0.can_undeploy(building as u32));
                sim.give(Order::Undeploy { building: building as u32, x: x as i32, y: y as i32 });
            }
        }
    }
    // RA2_DEPLOY_AT=unit,tick deploys a unit at a later tick, for checking deployment
    // after a move without a mouse.
    if let Ok(spec) = std::env::var("RA2_DEPLOY_AT") {
        let spec: Vec<u64> = spec.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if let [unit, at] = spec[..] {
            if sim.0.current_tick() == at {
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
    terrain_depth: Option<Res<crate::depth::TerrainDepth>>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<(Entity, &UnitSprite, &mut Sprite, &mut Transform), Without<FlyerShadow>>,
    mut shadows: Query<(Entity, &FlyerShadow, &mut Sprite, &mut Transform), Without<UnitSprite>>,
) {
    let view = &mut *view;
    // What flies
    // casts a shadow on the ground straight under it.
    let mut shadowed: Vec<u32> = Vec::new();
    for (entity, shadow, mut sprite, mut transform) in &mut shadows {
        let Some(unit) = sim.0.unit(shadow.0).filter(|unit| sim.0.altitude(unit) > 0) else {
            commands.entity(entity).despawn();
            continue;
        };
        shadowed.push(unit.id);
        let cell = unit.cell();
        let level = sim.0.map.level(cell.0, cell.1) as i32;
        let direction = (((unit.facing >> 2) + 1) >> 1) & 31;
        let kind = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k));
        let (Some(kind), Some((model, _))) = (kind, view.looks.get(&unit.id)) else {
            continue;
        };
        let key = (model.clone(), if unit.stats.infantry { 0 } else { direction });
        let art = &mut view.art;
        let half = kind.considered_aircraft;
        let image = view.shadow_images.entry(key).or_insert_with_key(|key| {
            let (width, height, rgba, middle) = if unit.stats.infantry {
                let drawn = art.soldier_shadow(&files)?;
                let middle = Vec2::new(drawn.offset.0 as f32 + drawn.width as f32 / 2.0, drawn.offset.1 as f32 + drawn.height as f32 / 2.0);
                (drawn.width, drawn.height, drawn.rgba, middle)
            } else {
                let drawn = art.flyer_shadow(&files, &key.0, key.1 << 3, half)?;
                let middle = Vec2::new(drawn.width as f32 / 2.0 - drawn.origin.0 as f32, drawn.height as f32 / 2.0 - drawn.origin.1 as f32);
                (drawn.width, drawn.height, drawn.rgba, middle)
            };
            let image = Image::new(
                Extent3d { width, height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), middle))
        });
        if let Some((handle, middle)) = image {
            if sprite.image != *handle {
                sprite.image = handle.clone();
            }
            let centre = scene_point(view.origin, unit.x, unit.y, level) + *middle - slope_up(&sim.0, unit);
            transform.translation = Vec3::new(centre.x.round(), -centre.y.round(), 1.0 + ((cell.0 + cell.1) as f32 + 0.95) * DEPTH_PER_ROW);
        }
    }
    for unit in sim.0.units() {
        if sim.0.altitude(unit) > 0 && !shadowed.contains(&unit.id) {
            commands.spawn((DespawnOnExit(Screen::InGame), FlyerShadow(unit.id), Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0)));
        }
    }
    // Where each building stands in depth, for putting units in front of it or
    // behind it.
    // Against a
    // building's body, a unit whose picture ends at row yu is in front where
    // yu > 0.75 (yb + 10.7 − NormalZAdjust + a) + 0.25 ya − |x − xa| / 4, with yb
    // the body's last row, (xa, ya) the building's anchor in the depth shape and a
    // the unit's own adjust: −1, −3 for one leaving its factory, −14 for a miner
    // on a refinery.
    struct Standing {
        x: (f32, f32),
        y: (f32, f32),
        anchor: Vec2,
        last_row: f32,
        adjust: f32,
        depth: f32,
        refinery: bool,
        cell: (i32, i32),
        size: (i32, i32),
    }
    let standing: Vec<Standing> = sim
        .0
        .buildings()
        .iter()
        .filter_map(|building| {
            let art = view.building_art.get(building.kind as usize)?;
            let level = sim.0.map.level(building.cell.0, building.cell.1) as i32;
            let point = scene_point(view.origin, building.cell.0 * LEPTONS_PER_CELL, building.cell.1 * LEPTONS_PER_CELL, level);
            let (w, h) = (building.size.0 as f32, building.size.1 as f32);
            let art_cache = &mut view.art;
            let body = (*view.body_boxes.entry(building.kind).or_insert_with(|| {
                art_cache.building(&files, &art.art, 0, None, level, false).map(|body| (body.offset, body.width, body.height))
            }))?;
            struct Box2 {
                offset: (i32, i32),
                width: u32,
                height: u32,
            }
            let body = Box2 { offset: body.0, width: body.1, height: body.2 };
            let last_row = point.y + (body.offset.1 + body.height as i32) as f32;
            let row = building_row(building);
            Some(Standing {
                x: (point.x + (body.offset.0 as f32).min(-30.0 * h), point.x + ((body.offset.0 + body.width as i32) as f32).max(30.0 * w)),
                y: (point.y + body.offset.1 as f32, last_row.max(point.y + 15.0 * (w + h))),
                anchor: point + Vec2::new(30.0 * (w - h) - art.zshape_move.0 as f32, 15.0 * (w + h - 2.0) - art.zshape_move.1 as f32),
                last_row,
                adjust: art.normal_z_adjust as f32,
                depth: 2.0 + row * DEPTH_PER_ROW,
                refinery: sim.0.building_types.get(building.kind as usize).is_some_and(|kind| kind.refinery),
                cell: building.cell,
                size: building.size,
            })
        })
        .collect();
    let among = |x: f32, top: f32, bottom: f32, unit: &ra2_sim::Unit, mut depth: f32| -> f32 {
        let cell = unit.cell();
        for building in &standing {
            if x < building.x.0 || x > building.x.1 || bottom < building.y.0 || top > building.y.1 {
                continue;
            }
            let on_it = (building.cell.0..building.cell.0 + building.size.0).contains(&cell.0)
                && (building.cell.1..building.cell.1 + building.size.1).contains(&cell.1);
            let a = if unit.stats.infantry {
                -12.0
            } else if unit.tethered {
                -3.0
            } else if unit.stats.harvester && building.refinery && on_it {
                -14.0
            } else {
                -1.0
            };
            let line = 0.75 * (building.last_row + 10.7 - building.adjust + a) + 0.25 * building.anchor.y - (x - building.anchor.x).abs() / 4.0;
            let step = DEPTH_PER_ROW / 40.0;
            if bottom > line {
                depth = depth.max(building.depth + step * 36.0);
            } else {
                depth = depth.min(building.depth - step / 2.0);
            }
        }
        depth
    };
    // A vehicle that has appeared since the start (a building packing up).
    let tick = sim.0.current_tick();
    if tick != view.walk_tick {
        for now in view.walk_tick + 1..=tick.min(view.walk_tick + 8) {
            for unit in sim.0.units() {
                if unit.stats.infantry {
                    continue;
                }
                // "moving" for a jumpjet is
                // any state but on the ground or hovering.
                let sheet = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| k.sheet);
                let moving = match unit.jet {
                    Some(jet) => !matches!(jet.state, ra2_sim::flight::GROUNDED | ra2_sim::flight::HOVERING),
                    None => unit.is_moving(),
                };
                let rate = match (sheet, moving) {
                    (Some(sheet), true) => sheet.walk_rate,
                    (Some(sheet), false) => sheet.idle_rate,
                    (None, true) => 1,
                    (None, false) => 0,
                };
                if rate != 0 && now % rate == 0 {
                    *view.walk_counters.entry(unit.id).or_insert(0) += 1;
                }
            }
        }
        view.walk_tick = tick;
    }
    // One that had gone from the map and is back (a passenger let out) has its
    // look already and needs only a sprite again.
    let drawn: Vec<u32> = sprites.iter().map(|(_, sprite, _, _)| sprite.0).collect();
    for unit in sim.0.units() {
        if drawn.contains(&unit.id) {
            continue;
        }
        if !view.looks.contains_key(&unit.id) {
            let Some(kind) = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)) else {
                continue;
            };
            let palette = view.palette_of(unit.owner);
            view.voices.insert(unit.id, (kind.voice_select.clone(), kind.voice_move.clone()));
            view.looks.insert(unit.id, (kind.image.clone(), palette));
        }
        commands.spawn((DespawnOnExit(Screen::InGame), UnitSprite(unit.id), Sprite::default()));
    }
    for (entity, unit_sprite, mut sprite, mut transform) in &mut sprites {
        let Some(unit) = sim.0.unit(unit_sprite.0) else {
            // The unit is gone, for instance deployed into a building.
            commands.entity(entity).despawn();
            continue;
        };
        // A unit that has changed sides (a mind taken, or given back) takes its
        // new house's colours.
        if unit.owner != 0 {
            let wanted = view.palette_of(unit.owner);
            if let Some(look) = view.looks.get_mut(&unit_sprite.0).filter(|look| look.1 != wanted) {
                look.1 = wanted;
            }
        }
        let Some((model, palette)) = view.looks.get(&unit_sprite.0) else {
            continue;
        };
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
                Stance::Standing if unit.arms.firing > 0 => ("fireup", firing),
                Stance::Standing if unit.stepping => ("walk", (tick / 3) as usize),
                Stance::Standing => ("ready", 0),
            };
            // A soldier of the amphibious-destroyer
            // movement zone (Tanya, the SEAL) on water or beach swims: `Swim` for
            // the walk, `Tread` standing, `WetAttack` firing.
            let action = if unit.stats.movement_zone == 3 && sim.0.map.wet(cell.0, cell.1) {
                match action {
                    "walk" => "swim",
                    "ready" => "tread",
                    "fireup" => "wetattack",
                    other => other,
                }
            } else {
                action
            };
            let (first, count, per_facing) = sequences.and_then(|s| s.get(action).or(s.get("ready"))).copied().unwrap_or((0, 1, 1));
            let stage = if matches!(action, "walk" | "fly" | "hover" | "swim" | "tread") { stage % count } else { stage.min(count - 1) };
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
            let centre = scene_point(view.origin, unit.x, unit.y, level) + *middle - up - slope_up(&sim.0, unit);
            // A soldier on the ground is
            // tested against the ground's depth with an adjustment of -10, its
            // own (-1 less the pixels of its height) and -2.
            let shown = match terrain_depth.as_ref().filter(|_| up.y == 0.0) {
                Some(ground) => behind_ground(ground, &mut view.masked, &mut images, handle, centre, level, -13 - 15 * level),
                None => handle.clone(),
            };
            if sprite.image != shown {
                sprite.image = shown;
            }
            let row = (cell.0 + cell.1) as f32;
            let over = if up.y > 0.0 { 40.0 } else { 0.0 };
            let mut depth = 2.0 + (row + 0.9 + over) * DEPTH_PER_ROW;
            if unit.on_bridge {
                depth = 2.0 + (row + 6.0) * DEPTH_PER_ROW;
            } else if up.y == 0.0 {
                let half = sprite.custom_size.map_or(24.0, |size| size.y / 2.0);
                depth = among(centre.x, centre.y - half, centre.y + half, unit, depth);
            }
            transform.translation = Vec3::new(centre.x.round(), -centre.y.round(), depth);
            continue;
        }
        // A vehicle drawn from a sheet of pictures. The direction column is the
        // eighth-turn facing plus one; while it moves, or if it has an `IdleRate`,
        // the picture is the walk counter within the walk frames; the counter goes
        // up on frames divisible by `WalkRate` while moving and by `IdleRate`
        // while not.
        if let Some(sheet) = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| k.sheet) {
            let column = (((((unit.facing as usize) >> 4) + 1) >> 1) + 1) & 7;
            let counter = view.walk_counters.get(&unit.id).copied().unwrap_or(0) as usize;
            // For the
            // twelve frames after a shot the picture is from the firing frames:
            // a counter that runs from 11 down to 0, halved, so they play
            // backwards two frames each.
            let since = tick.saturating_sub(unit.arms.fired_at);
            let frame = if sheet.firing_frames != 0 && unit.arms.fired_at != 0 && since < 12 {
                sheet.start_firing + sheet.firing_frames * column + ((11 - since as usize) / 2).min(sheet.firing_frames - 1)
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
            let centre = scene_point(view.origin, unit.x, unit.y, level) + *middle - slope_up(&sim.0, unit);
            let shown = match terrain_depth.as_ref() {
                Some(ground) => behind_ground(ground, &mut view.masked, &mut images, handle, centre, level, -3 - 15 * level),
                None => handle.clone(),
            };
            if sprite.image != shown {
                sprite.image = shown;
            }
            let depth = 2.0 + ((cell.0 + cell.1) as f32 + 0.9) * DEPTH_PER_ROW;
            transform.translation = Vec3::new(centre.x.round(), -centre.y.round(), among(centre.x, centre.y - 24.0, centre.y + 24.0, unit, depth));
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
        let seen = view.ramps.entry(unit.id).or_insert((ramp, ramp, 0));
        if seen.0 != ramp {
            *seen = (ramp, seen.0, now);
        }
        let thirds = now.saturating_sub(seen.2).min(3) as u8;
        let tipped = match (ramp, seen.1, thirds) {
            _ if unit.rocket.is_some() => None,
            (_, old, 0..=2) if old != ramp => Some(format!("{model}@r{ramp}@b{old}_{thirds}")),
            (0, ..) => None,
            _ => Some(format!("{model}@r{ramp}")),
        };
        let model = tipped.as_ref().unwrap_or(model);
        let direction = (((unit.facing >> 2) + 1) >> 1) & 31;
        // The turret's facing in 32 steps.
        let which = unit.gun.map_or(0, |(_, turret)| turret);
        let turret = unit.stats.fighter.turret.then(|| ((((unit.arms.turret_facing() >> 2) + 1) >> 1) & 31, unit.stats.fighter.turret_offset));
        // The model's animation frame is the walk
        // counter within the frames it has (the Nighthawk's two turn its rotors).
        let frames = view.art.hva_frames(&files, model);
        let frame = if frames > 1 { view.walk_counters.get(&unit.id).copied().unwrap_or(0) as usize % frames } else { 0 };
        view.art.hva_frame = frame;
        let key = (if frame == 0 { model.clone() } else { format!("{model}#{frame}") }, direction, turret.map(|t| t.0 + 32 * which), *palette, level);
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
        let centre = scene_point(view.origin, unit.x, unit.y, level) - image.origin + image.size / 2.0 - up - slope_up(&sim.0, unit);
        // A vehicle's picture is tested
        // against the ground's depth with its own adjustment: -1 less the
        // pixels of its height.
        let shown = match terrain_depth.as_ref().filter(|_| up.y == 0.0 && unit.plane.is_none()) {
            Some(ground) => behind_ground(ground, &mut view.masked, &mut images, &image.handle, centre, level, -1 - 15 * level),
            None => image.handle.clone(),
        };
        if sprite.image != shown {
            sprite.image = shown;
        }
        let row = (cell.0 + cell.1) as f32;
        // Units draw over everything in their own row.
        let over = if up.y > 0.0 { 40.0 } else { 0.0 };
        let mut depth = 2.0 + (row + 0.9 + over) * DEPTH_PER_ROW;
        if unit.plane.is_some() || unit.on_bridge {
            depth = 2.0 + (row + 6.0) * DEPTH_PER_ROW;
        } else if up.y == 0.0 {
            depth = among(centre.x, centre.y - image.size.y / 2.0, centre.y + image.size.y / 2.0, unit, depth);
        }
        transform.translation = Vec3::new(centre.x.round(), -centre.y.round(), depth);
    }
}

/// The picture to show for a unit on the ground at a place: its own, or a copy
/// with the pixels the ground hides taken out.
// See `crate::depth`.
#[allow(clippy::too_many_arguments)]
fn behind_ground(
    depth: &crate::depth::TerrainDepth,
    masked: &mut HashMap<(bevy::asset::AssetId<Image>, i32, i32), Handle<Image>>,
    images: &mut Assets<Image>,
    handle: &Handle<Image>,
    centre: Vec2,
    level: i32,
    adjust: i32,
) -> Handle<Image> {
    let Some((width, height)) = images.get(handle).map(|image| (image.width() as i32, image.height() as i32)) else {
        return handle.clone();
    };
    let corner = (centre.round() - Vec2::new(width as f32, height as f32) / 2.0).round().as_ivec2();
    let bottom = corner.y + height - 1;
    let Some(mask) = depth.hidden(corner.x, corner.y, width, height, level, |row| crate::depth::upright(bottom, adjust, row)) else {
        return handle.clone();
    };
    let key = (handle.id(), corner.x, corner.y);
    if let Some(done) = masked.get(&key) {
        return done.clone();
    }
    let Some(mut rgba) = images.get(handle).and_then(|image| image.data.clone()) else {
        return handle.clone();
    };
    for (pixel, hide) in rgba.chunks_exact_mut(4).zip(&mask) {
        if *hide {
            pixel[3] = 0;
        }
    }
    // Pictures cut for places a unit has since left are let go.
    if masked.len() > 400 {
        masked.clear();
    }
    let cut = images.add(Image::new(
        Extent3d { width: width as u32, height: height as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    masked.insert(key, cut.clone());
    cut
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
    mut shown: Query<(Entity, &mut BuildingSprite, &mut Sprite, &mut Transform, &mut Visibility)>,
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

        // The parts wanted now: art, frame, shadow, depth.
        let mut parts: Vec<(String, usize, bool, f32)> = Vec::new();
        let with_shadow = |parts: &mut Vec<(String, usize, bool, f32)>,
                           art: &mut UnitArt,
                           files: &GameFiles,
                           art_name: &str,
                           frame: usize,
                           z: f32,
                           own_shadow: bool| {
            let count = art.frame_count(files, art_name);
            if own_shadow && count >= 2 {
                parts.push((art_name.to_owned(), frame + count / 2, true, shadow_z));
            }
            parts.push((art_name.to_owned(), frame, false, z));
        };
        match (building.buildup_frame(), &art.buildup) {
            (Some(frame), Some(buildup)) => with_shadow(&mut parts, &mut view.art, &files, buildup, frame as usize, body_z, true),
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
                if let Some(bib) = &art.bib {
                    with_shadow(&mut parts, &mut view.art, &files, bib, frame, body_z - step, true);
                }
                // While a factory lets a vehicle out its
                // body is the open-door art, with the floor under the door drawn
                // after it; there is no door animation in the stock art.
                let body = art.deploying.as_ref().filter(|_| building.unloading()).unwrap_or(&art.art);
                with_shadow(&mut parts, &mut view.art, &files, body, frame, body_z, true);
                if let Some(floor) = art.under_door.as_ref().filter(|_| building.unloading()) {
                    with_shadow(&mut parts, &mut view.art, &files, floor, frame, body_z + step / 2.0, false);
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
                    state.granted && !(charged < if state.ready { 0.0 } else { state.left as f32 / 900.0 })
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
                    with_shadow(&mut parts, &mut view.art, &files, &anim.image, frame, body_z + step * (order + 1) as f32, anim.shadow);
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
                    parts.push((format!("vxl:{model}:{x}:{y}:{}", (recoil * 2.0).round() as i32), facing as usize, false, body_z + step * 30.0));
                }
            }
        }

        // Every part's canvas is centred on
        // the top corner of the foundation's top cell.
        let per_cell = LEPTONS_PER_CELL;
        let anchor = scene_point(view.origin, building.cell.0 * per_cell, building.cell.1 * per_cell, level);
        let mut have = 0;
        for (entity, mut sprite, mut picture, mut transform, mut visibility) in &mut shown {
            if sprite.0 != building.id {
                continue;
            }
            have = have.max(sprite.1 + 1);
            let Some((name, frame, shadow, z)) = parts.get(sprite.1) else {
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
            if sprite.2.as_ref() == Some(&key) {
                continue;
            }
            let image = part_image(view, &files, &mut images, &key);
            match image {
                Some((handle, centre)) => {
                    let centre = snapped(anchor, centre);
                    picture.image = handle;
                    transform.translation = Vec3::new(centre.x, -centre.y, *z);
                    *visibility = Visibility::Inherited;
                }
                None => *visibility = Visibility::Hidden,
            }
            sprite.2 = Some(key);
        }
        for (index, (name, frame, shadow, z)) in parts.iter().enumerate().skip(have) {
            let key: PartKey = (name.clone(), *frame, palette, level, *shadow);
            let (handle, place, visibility) = match part_image(view, &files, &mut images, &key) {
                Some((handle, centre)) => {
                    let centre = snapped(anchor, centre);
                    (handle, Vec3::new(centre.x, -centre.y, *z), Visibility::Inherited)
                }
                None => (Handle::default(), Vec3::new(anchor.x, -anchor.y, *z), Visibility::Hidden),
            };
            commands.spawn((
                DespawnOnExit(Screen::InGame),
                BuildingSprite(building.id, index, Some(key)),
                Sprite::from_image(handle),
                Transform::from_translation(place),
                visibility,
            ));
        }
    }
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
        for (frame, shadow, z) in [(frame + frames / 2, true, 1.0 + row * DEPTH_PER_ROW), (frame, false, 2.0 + row * DEPTH_PER_ROW)] {
            let key: PartKey = (format!("bridge:{art}"), frame, None, level + if high { 4 } else { 0 }, shadow);
            if let Some((handle, centre)) = part_image(view, &files, &mut images, &key) {
                let centre = snapped(anchor, centre);
                commands.spawn((DespawnOnExit(Screen::InGame), BridgeSprite, Sprite::from_image(handle), Transform::from_xyz(centre.x, -centre.y, z)));
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
            let entity = commands.spawn((DespawnOnExit(Screen::InGame), Sprite::from_image(handle), Transform::from_translation(translation))).id();
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
        if !view.ore_art.contains_key(&cell) {
            if let Some(picture) = sim.0.map.ore_picture(cell.0, cell.1) {
                let family = if sim.0.map.ore_at(cell.0, cell.1).is_some_and(|(kind, _)| kind == 1) { "gem" } else { "tib" };
                view.ore_art.insert(cell, format!("{family}{:02}", picture + 1));
            }
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
    mut shown: Local<Vec<((i32, i32), bool)>>,
    mut pictures: Local<HashMap<(bool, i32), Option<(Handle<Image>, Vec2)>>>,
) {
    let now: Vec<((i32, i32), bool)> = sim.0.crates().iter().map(|c| (c.cell, c.water)).collect();
    if now == *shown {
        return;
    }
    *shown = now.clone();
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let view = &mut *view;
    for (cell, water) in now {
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
        let picture = pictures.entry((water, level)).or_insert_with(|| {
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
            commands.spawn((DespawnOnExit(Screen::InGame), CrateSprite, Sprite::from_image(handle), Transform::from_xyz(centre.x, -centre.y, 2.0 + row * DEPTH_PER_ROW)));
        }
    }
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
    mut names: Local<Option<Vec<String>>>,
    mut shown: Local<Vec<(u32, u16, (i32, i32))>>,
) {
    let names = names.get_or_insert_with(|| {
        files
            .get("rules.ini")
            .map(ra2_formats::ini::Ini::parse)
            .and_then(|rules| rules.get("General", "DefaultMirageDisguises").map(|list| list.split(',').map(|name| name.trim().to_lowercase()).filter(|name| !name.is_empty()).collect()))
            .unwrap_or_default()
    });
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
    if now == *shown {
        return;
    }
    *shown = now.clone();
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
            (data as usize, false, 2.0 + row * DEPTH_PER_ROW),
        ] {
            let key: PartKey = (art.clone(), frame, palette, level, shadow);
            if let Some((handle, centre)) = part_image(view, &files, &mut images, &key) {
                let centre = snapped(anchor, centre);
                commands.spawn((
                    DespawnOnExit(Screen::InGame),
                    WallSprite,
                    Sprite::from_image(handle),
                    Transform::from_xyz(centre.x, -centre.y, z),
                ));
            }
        }
    }
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
    if !*based {
        if let Ok(names) = std::env::var("RA2_BASE") {
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
                sim.0.destroy_house_for_tests(house as u8);
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
    mut corpses: Query<(Entity, &Corpse, &mut Sprite, &mut Transform)>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    for (entity, corpse, mut sprite, mut transform) in &mut corpses {
        let stage = tick.saturating_sub(corpse.began) as usize;
        let point = scene_point_3d(view.origin, corpse.at);
        if stage >= corpse.count {
            commands.entity(entity).despawn();
            if let Some(body) = corpse.body.as_ref().map(|body| body.to_uppercase()).filter(|body| view.anim_types.contains_key(body)) {
                commands.spawn((DespawnOnExit(Screen::InGame), Effect { anim: body, began: tick, at: point, depth: 0.7 + (corpse.depth - 2.0).max(0.0) }, Sprite::default()));
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
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let me = local();
    let mut wanted: Vec<(String, (i32, i32))> = Vec::new();
    let choosing = matches!(*mode, crate::control::Mode::Super(index) if sim.0.super_types.get(index as usize).is_some_and(|kind| kind.kind == 4));
    if let (true, Some(cell), Some(name)) = (choosing, sim.0.houses.get(me as usize).and_then(|house| house.chrono_source), crate::audio::rules_sound(&files, "ChronoPlacement")) {
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

/// A shot in flight, by its id.
#[derive(Component)]
struct ShotSprite(u32);

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
/// A cell lit by radiation: the cell, and how strongly it is shown.
#[derive(Component)]
struct RadiationPatch((i32, i32), f32);

/// Shows where the ground is radioactive.
// The engine draws nothing for
// radiation but a green light at each site, which tints the cells round it:
// round, strongest in the middle, fading to nothing at the edge and dimming
// in steps as the site wears out (`World::rad_glow`). Lighting is by the
// cell, so each cell is one shade.
// Not original behaviour: the light is laid over the ground as a green
// wash, where the engine multiplies the ground's green (up to double) and
// leaves red and blue alone; things standing there are not tinted.
fn show_radiation(
    mut commands: Commands,
    sim: Res<Sim>,
    view: Res<UnitView>,
    mut images: ResMut<Assets<Image>>,
    mut diamond: Local<Option<Handle<Image>>>,
    mut patches: Query<(Entity, &mut RadiationPatch, &mut Sprite)>,
) {
    let world = &sim.0;
    let mut cells: HashMap<(i32, i32), f32> = HashMap::new();
    for site in &world.rad_sites {
        for dy in -site.spread..=site.spread {
            for dx in -site.spread..=site.spread {
                let cell = (site.cell.0 + dx, site.cell.1 + dy);
                cells.entry(cell).or_insert_with(|| world.rad_glow(cell));
            }
        }
    }
    cells.retain(|_, glow| *glow > 0.0);
    let colour = |glow: f32| Color::srgba(0.0, 1.0, 0.0, 0.45 * glow);
    for (entity, mut patch, mut sprite) in &mut patches {
        match cells.remove(&patch.0) {
            Some(glow) if glow != patch.1 => {
                patch.1 = glow;
                sprite.color = colour(glow);
            }
            Some(_) => {}
            None => commands.entity(entity).despawn(),
        }
    }
    if cells.is_empty() {
        return;
    }
    // One cell's outline on the screen: a diamond 60 pixels by 30.
    let handle = diamond.get_or_insert_with(|| {
        let (w, h) = (60usize, 30usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = ((x as f32 + 0.5 - 30.0).abs() / 30.0, (y as f32 + 0.5 - 15.0).abs() / 15.0);
                if fx + fy <= 1.0 {
                    rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[255; 4]);
                }
            }
        }
        images.add(Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, rgba, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD))
    });
    for (cell, glow) in cells {
        let level = world.map.level(cell.0, cell.1) as i32;
        let at = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128, level);
        commands.spawn((
            DespawnOnExit(Screen::InGame),
            RadiationPatch(cell, glow),
            Sprite { image: handle.clone(), color: colour(glow), ..default() },
            Transform::from_xyz(at.x, -at.y, 0.58 + (cell.0 + cell.1) as f32 * DEPTH_PER_ROW),
        ));
    }
}

pub(crate) fn scene_point_3d(origin: Vec2, (x, y, z): (i32, i32, i32)) -> Vec2 {
    scene_point(origin, x, y, 0) - Vec2::new(0.0, z as f32 * LEVEL_HEIGHT as f32 / ra2_sim::combat::LEVEL_HEIGHT as f32)
}

/// Shows and sounds what the fighting did this frame: weapon reports, muzzle
/// flashes, lasers, explosions and deaths; and keeps the shots in flight drawn.
// The muzzle
// flash is the weapon's `Anim` (of eight, the one for the direction plus one); the
// explosion is the warhead's `AnimList` entry for the damage carried, 25 points an
// entry (the splash list at 35 on water for `Conventional` ones); a dying vehicle
// or building plays one of its `Explosion` animations and its `DieSound`; an
// animation's own sound is heard as it starts.
#[allow(clippy::too_many_arguments)]
fn show_effects(
    mut commands: Commands,
    mut sim: ResMut<Sim>,
    files: Res<GameFiles>,
    mut view: ResMut<UnitView>,
    armoury: Option<Res<crate::armoury::Armoury>>,
    units: Res<UnitCatalog>,
    catalog: Res<BuildCatalog>,
    mut images: ResMut<Assets<Image>>,
    mut cues: MessageWriter<Cue>,
    mut effects: Query<(Entity, &Effect, &mut Sprite, &mut Transform), Without<ShotSprite>>,
    mut shots: Query<(Entity, &ShotSprite, &mut Sprite, &mut Transform), Without<Effect>>,
    beams: Query<(Entity, &Beam)>,
    mut picks: Local<u32>,
    mut radar_events: ResMut<crate::radar::RadarEvents>,
    mut status: MessageWriter<crate::bar::Status>,
) {
    let Some(armoury) = armoury else {
        return;
    };
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let mut pick = |count: usize| {
        *picks = picks.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*picks >> 16) as usize % count.max(1)
    };
    let mut start = |commands: &mut Commands, cues: &mut MessageWriter<Cue>, view: &UnitView, name: &str, at: Vec2, depth: f32| {
        let name = name.to_uppercase();
        let Some(anim) = view.anim_types.get(&name) else {
            return;
        };
        cues.write_batch(anim.report.clone().map(|report| Cue::SoundOn(report, at)));
        commands.spawn((DespawnOnExit(Screen::InGame), Effect { anim: name, began: tick, at, depth }, Sprite::default()));
    };
    for event in sim.0.take_events() {
        use ra2_sim::combat::Event;
        match event {
            Event::Fired { weapon, at, to, facing, by } => {
                let Some(look) = armoury.weapon_looks.get(weapon as usize) else {
                    continue;
                };
                // A laser's line is 2
                // layers thick in house colours, 3 from a prism tower.
                // 5 and brighter when other
                // towers have lent their beams to the shot.
                if look.is_laser || look.electric_bolt {
                    let (from, end) = (scene_point_3d(view.origin, at), scene_point_3d(view.origin, to));
                    if look.electric_bolt {
                        crate::beams::bolt(&mut commands, at, to, view.origin, (tick as u32).wrapping_mul(31).wrapping_add(weapon as u32), look.alternate_colour);
                    } else {
                        let (owner, layers) = match by {
                            ra2_sim::combat::Aim::Unit(id) => (sim.0.unit(id).map(|u| u.owner), 2),
                            ra2_sim::combat::Aim::Building(id) => {
                                let tower = sim.0.buildings().iter().find(|b| b.id == id);
                                (tower.map(|b| b.owner), if tower.is_some_and(|b| b.last_supports > 0) { 5 } else { 3 })
                            }
                            ra2_sim::combat::Aim::Cell(..) => (None, 2),
                        };
                        let colour = view.art.owner_colour(owner.and_then(|owner| view.palette_of(owner)));
                        let house = look.house_colour.then_some((colour, layers));
                        crate::beams::laser(&mut commands, from, end, look.laser_inner, look.laser_outer, house, tick, look.laser_duration.max(1));
                    }
                }
                if !look.report.is_empty() {
                    cues.write(Cue::SoundOn(look.report[pick(look.report.len())].clone(), scene_point_3d(view.origin, at)));
                }
                let point = scene_point_3d(view.origin, at);
                let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                let flash = if look.anims.len() == 8 {
                    let direction = ((((facing as usize) >> 4) + 1) >> 1) & 7;
                    look.anims.get((direction + 1) % 8)
                } else {
                    look.anims.first()
                };
                if let Some(flash) = flash {
                    start(&mut commands, &mut cues, view, flash, point, depth);
                }
            }
            // The sim has chosen the explosion (by the damage carried) and
            // marked the ground under it; here it is only shown.
            Event::Detonated { at, anim, .. } => {
                if let Some(name) = view.anim_names.get(anim as usize).cloned() {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start(&mut commands, &mut cues, view, &name, scene_point_3d(view.origin, at), depth);
                }
            }
            Event::UnitDied { kind, at, owner, facing, infantry, death, explosion, .. } => {
                let Some(name) = (kind as usize).checked_sub(1).and_then(|k| units.0.get(k)).map(|entry| entry.name.to_lowercase()) else {
                    continue;
                };
                // A
                // soldier killed by a warhead of `InfDeath` 1 or 2 plays its
                // Die1 or Die2 sequence, a stage a frame, and leaves one of the
                // dead bodies; 3 to 7 show an animation of their own.
                if infantry {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    match death {
                        1 | 2 => {
                            let look = (kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k));
                            let sequence = look.and_then(|k| k.sequences.as_ref()).and_then(|s| s.get(if death == 1 { "die1" } else { "die2" })).copied();
                            if let (Some(look), Some((first, count, _))) = (look, sequence) {
                                let body = (!armoury.dead_bodies.is_empty()).then(|| armoury.dead_bodies[pick(armoury.dead_bodies.len())].clone());
                                commands.spawn((
                                    DespawnOnExit(Screen::InGame),
                                    Corpse { image: look.image.clone(), palette: view.palette_of(owner), first, count: count.max(1), began: tick, at, depth, body },
                                    Sprite::default(),
                                ));
                            }
                        }
                        3..=7 => {
                            if let Some(anim) = armoury.infantry_deaths[(death - 3) as usize].clone() {
                                start(&mut commands, &mut cues, view, &anim, scene_point_3d(view.origin, at), depth);
                            }
                        }
                        _ => {}
                    }
                    let _ = facing;
                }
                if let Some(sounds) = armoury.die_sounds.get(&name).filter(|list| !list.is_empty()) {
                    cues.write(Cue::SoundOn(sounds[pick(sounds.len())].clone(), scene_point_3d(view.origin, at)));
                }
                if let Some(name) = view.anim_names.get(explosion as usize).cloned() {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start(&mut commands, &mut cues, view, &name, scene_point_3d(view.origin, at), depth);
                }
            }
            Event::BuildingDied { kind, cell, size, explosions, .. } => {
                let Some(name) = catalog.0.get(kind as usize).map(|entry| entry.name.to_lowercase()) else {
                    continue;
                };
                if let Some(sounds) = armoury.die_sounds.get(&name).filter(|list| !list.is_empty()) {
                    let at = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + size.0 * 128, cell.1 * LEPTONS_PER_CELL + size.1 * 128, 0);
                    cues.write(Cue::SoundOn(sounds[pick(sounds.len())].clone(), at));
                }
                // One explosion on each cell of the foundation, as the sim chose.
                let mut chosen = explosions.iter();
                for dy in 0..size.1 {
                    for dx in 0..size.0 {
                        let Some(name) = chosen.next().and_then(|&anim| view.anim_names.get(anim as usize)).cloned() else {
                            continue;
                        };
                        let (x, y) = ((cell.0 + dx) * LEPTONS_PER_CELL + 128, (cell.1 + dy) * LEPTONS_PER_CELL + 128);
                        let level = sim.0.map.level(cell.0 + dx, cell.1 + dy) as i32;
                        let depth = 2.0 + ((cell.0 + dx + cell.1 + dy) as f32 + 1.5) * DEPTH_PER_ROW;
                        start(&mut commands, &mut cues, view, &name, scene_point(view.origin, x, y, level), depth);
                    }
                }
            }
            // `SellSound` as a building starts to
            // come down; the announcer when it has gone.
            Event::SaleBegan { owner } => {
                if owner == local() {
                    cues.write_batch(view.sell_sound.clone().map(Cue::Sound));
                }
            }
            Event::Sold { owner } => {
                if owner == local() {
                    cues.write(Cue::Eva("EVA_StructureSold"));
                }
            }
            // The warp animation at both ends and the
            // type's jump sound.
            Event::Teleported { kind, from, to } => {
                for at in [from, to] {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start(&mut commands, &mut cues, view, "WARPOUT", scene_point_3d(view.origin, at), depth);
                }
                // The out sound where it
                // left and the in sound where it arrived.
                let sounds = (kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).map(|k| k.chrono_sounds.clone()).unwrap_or_default();
                cues.write_batch(sounds.0.map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, from))));
                cues.write_batch(sounds.1.map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, to))));
            }
            Event::Erased { at } => {
                let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                start(&mut commands, &mut cues, view, "WARPAWAY", scene_point_3d(view.origin, at), depth);
            }
            // The announcer on a capture.
            Event::Captured { by, from, .. } => {
                if by == local() || from == local() {
                    cues.write(Cue::Eva("EVA_BuildingCaptured"));
                }
            }
            // Heard by the player whose soldiers
            // they are.
            Event::Garrisoned { by, .. } => {
                if by == local() {
                    cues.write(Cue::Eva("EVA_StructureGarrisoned"));
                    cues.write_batch(crate::audio::rules_sound(&files, "BuildingGarrisonedSound").map(Cue::Sound));
                }
            }
            Event::Abandoned { by, .. } => {
                if by == local() {
                    cues.write(Cue::Eva("EVA_StructureAbandoned"));
                    cues.write_batch(crate::audio::rules_sound(&files, "BuildingAbandonedSound").map(Cue::Sound));
                }
            }
            // The transport type's
            // `EnterTransportSound` and `LeaveTransportSound` (the IFV's is its
            // turret-change sound).
            Event::Boarded { transport } | Event::Alighted { transport } => {
                let entering = matches!(event, Event::Boarded { .. });
                let name = sim
                    .0
                    .unit(transport)
                    .and_then(|unit| (unit.stats.kind as usize).checked_sub(1))
                    .and_then(|kind| view.kinds.get(kind))
                    .and_then(|kind| if entering { kind.transport_sounds.0.clone() } else { kind.transport_sounds.1.clone() });
                if let Some(name) = name {
                    cues.write(Cue::Sound(name));
                }
            }
            Event::Forked { weapon, from, to, house } => {
                if let Some(look) = armoury.weapon_looks.get(weapon as usize).filter(|look| look.is_laser) {
                    let colour = view.art.owner_colour(view.palette_of(house));
                    let style = look.house_colour.then_some((colour, 2));
                    crate::beams::laser(&mut commands, scene_point_3d(view.origin, from), scene_point_3d(view.origin, to), look.laser_inner, look.laser_outer, style, tick, look.laser_duration.max(1));
                }
            }
            // The type's `AuxSound1` where it
            // leaves the ground, and `AuxSound2` where it nears it coming down.
            Event::TookOff { unit } | Event::Landing { unit } => {
                let up = matches!(event, Event::TookOff { .. });
                if let Some(unit) = sim.0.unit(unit) {
                    let sound = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| if up { k.aux_sounds.0.clone() } else { k.aux_sounds.1.clone() });
                    let at = (unit.x, unit.y, sim.0.ground_z(unit.x, unit.y));
                    cues.write_batch(sound.map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            Event::Landed { .. } => {}
            // `BombAttachSound` for the planter's
            // side. `YuriMindControlSound` at the
            // victim.
            Event::BombPlanted { target, house } => {
                if house == local() {
                    if let Some(at) = sim.0.aim_point(target) {
                        cues.write_batch(crate::audio::rules_sound(&files, "BombAttachSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                    }
                }
            }
            Event::MindControlled { victim, .. } => {
                if let Some(at) = sim.0.aim_point(ra2_sim::combat::Aim::Unit(victim)) {
                    cues.write_batch(crate::audio::rules_sound(&files, "YuriMindControlSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            // The announcer when the player's is
            // charged, by its type.
            Event::SuperReady { house, index } => {
                if house == local() {
                    let line = match sim.0.super_types.get(index as usize).map(|kind| kind.kind) {
                        Some(0) => Some("EVA_NuclearMissileReady"),
                        Some(1) => Some("EVA_IronCurtainReady"),
                        Some(2) => Some("EVA_LightningStormReady"),
                        Some(3) => Some("EVA_ChronosphereReady"),
                        Some(5 | 6) => Some("EVA_ReinforcementsReady"),
                        _ => None,
                    };
                    cues.write_batch(line.map(Cue::Eva));
                }
            }
            // Each super weapon's animation, voice and
            // sound as it is fired.
            // DATA: the animation and sound names are rules.ini keys.
            Event::SuperFired { kind, cell, from, .. } => {
                let point = |cell: (i32, i32), up: i32| {
                    let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                    scene_point_3d(view.origin, (x, y, sim.0.ground_z(x, y) + up))
                };
                let depth = |cell: (i32, i32)| 2.0 + ((cell.0 + cell.1) as f32 + 2.5) * DEPTH_PER_ROW;
                let key = |key: &str| crate::audio::rules_sound(&files, key);
                match kind {
                    1 => {
                        if let Some(name) = key("IronCurtainInvokeAnim") {
                            start(&mut commands, &mut cues, view, &name, point(cell, 5), depth(cell));
                        }
                        radar_events.raise(13, cell, tick);
                        cues.write(Cue::Eva("EVA_IronCurtainActivated"));
                    }
                    2 => {
                        cues.write(Cue::Eva("EVA_LightningStormCreated"));
                    }
                    4 => {
                        if let Some(name) = key("ChronoBlastDest") {
                            start(&mut commands, &mut cues, view, &name, point(cell, 5), depth(cell));
                        }
                        if let (Some(name), Some(from)) = (key("ChronoBlast"), from) {
                            start(&mut commands, &mut cues, view, &name, point(from, 5), depth(from));
                            radar_events.raise(13, from, tick);
                        }
                        radar_events.raise(13, cell, tick);
                        cues.write(Cue::Eva("EVA_ChronosphereActivated"));
                    }
                    _ => {}
                }
            }
            // The missile leaves its silo over
            // `NukeTakeOff`, the siren sounds at the target and everyone is told.
            Event::NukeLaunched { at, cell, .. } => {
                if let Some(name) = crate::audio::rules_sound(&files, "NukeTakeOff") {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 2.5) * DEPTH_PER_ROW;
                    start(&mut commands, &mut cues, view, &name, scene_point_3d(view.origin, at), depth);
                }
                let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                let target = scene_point_3d(view.origin, (x, y, sim.0.ground_z(x, y)));
                cues.write_batch(crate::audio::rules_sound(&files, "DigSound").map(|name| Cue::SoundOn(name, target)));
                cues.write(Cue::Eva("EVA_NuclearMissileLaunched"));
            }
            // The fireball where it came down.
            Event::NukeLanded { at } => {
                let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 2.5) * DEPTH_PER_ROW;
                start(&mut commands, &mut cues, view, "NUKEBALL", scene_point_3d(view.origin, at), depth);
                radar_events.raise(13, (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL)), tick);
            }
            // `StormSound` and the message as the storm breaks; the warning
            // and the announcer while it is coming; a message to a player who
            // calls a storm while one is on.
            Event::StormStarted { cell, .. } => {
                cues.write_batch(crate::audio::rules_sound(&files, "StormSound").map(Cue::Sound));
                status.write(crate::bar::Status::Key("TXT_LIGHTNING_STORM"));
                radar_events.raise(13, cell, tick);
            }
            Event::StormApproaching => {
                cues.write(Cue::Eva("EVA_LightningStormCreated"));
                status.write(crate::bar::Status::Key("TXT_LIGHTNING_STORM_APPROACHING"));
            }
            Event::StormRefused { house } => {
                if house == local() {
                    status.write(crate::bar::Status::Key("Msg:LightningStormActive"));
                }
            }
            Event::StormEnded => {}
            // The cloud the sim chose over the cell; then the bolt, and one of
            // the strike sounds.
            Event::Cloud { cell, kind } | Event::Bolt { cell, kind } => {
                let bolt = matches!(event, Event::Bolt { .. });
                let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                let ground = sim.0.ground_z(x, y);
                let names = crate::audio::rules_list(&files, if bolt { "WeatherConBolts" } else { "WeatherConClouds" });
                if let Some(name) = names.get(kind as usize).cloned() {
                    let at = scene_point_3d(view.origin, (x, y, ground + if bolt { 0 } else { 700 }));
                    start(&mut commands, &mut cues, view, &name, at, 2.0 + ((cell.0 + cell.1) as f32 + 3.0) * DEPTH_PER_ROW);
                }
                if bolt {
                    let sounds = crate::audio::rules_list(&files, "LightningSounds");
                    if !sounds.is_empty() {
                        cues.write(Cue::SoundOn(sounds[pick(sounds.len())].clone(), scene_point_3d(view.origin, (x, y, ground))));
                    }
                }
            }
            // `Wake` over a sinking vehicle; `ChronoSparkle1` over one held by a
            // time weapon or jumping.
            Event::Wake { at } | Event::Sparkle { at } => {
                let key = if matches!(event, Event::Wake { .. }) { "Wake" } else { "ChronoSparkle1" };
                if let Some(name) = crate::audio::rules_sound(&files, key) {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 2.5) * DEPTH_PER_ROW;
                    start(&mut commands, &mut cues, view, &name, scene_point_3d(view.origin, at), depth);
                }
            }
            // Another house's super weapon
            // building finished: a radar event there and the announcer, by
            // which of the first four super weapons it holds.
            Event::SuperDetected { owner, index, cell } => {
                if owner != local() {
                    radar_events.raise(12, cell, tick);
                    let line = ["EVA_NuclearSiloDetected", "EVA_IronCurtainDetected", "EVA_WeatherDeviceReady", "EVA_ChronosphereDetected"].get(index as usize);
                    cues.write_batch(line.map(|line| Cue::Eva(line)));
                }
            }
            // The player's engineer
            // mending a bridge is announced.
            Event::BridgeRepaired { house, .. } => {
                if house == local() {
                    cues.write(Cue::Eva("EVA_BridgeRepaired"));
                }
            }
            Event::BridgeBroken { .. } => {}
            // The announcer, for the depot's owner.
            Event::DepotRepairing { house } => {
                if house == local() {
                    cues.write(Cue::Eva("EVA_Repairing"));
                }
            }
            Event::UnitRepaired { house } => {
                if house == local() {
                    cues.write(Cue::Eva("EVA_UnitRepaired"));
                }
            }
            // `endgame.rs` watches for these.
            Event::SuperGranted { .. } | Event::SuperLost { .. } | Event::HouseDefeated { .. } | Event::GameOver => {}
            // `ChuteSound` at the plane.
            Event::Parachuted { from, .. } => {
                if let Some(at) = sim.0.aim_point(ra2_sim::combat::Aim::Unit(from)) {
                    cues.write_batch(crate::audio::rules_sound(&files, "ChuteSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            // What the
            // announcer says to the building's owner and to the spy's.
            Event::Infiltrated { victim, by, effect, .. } => {
                let lines: &[&'static str] = match (effect, victim == local(), by == local()) {
                    (1, true, _) => &["EVA_RadarSabotaged"],
                    (1, _, true) => &["EVA_BuildingInfRadarSabotaged"],
                    (2, true, _) => &["EVA_PowerSabotaged"],
                    (2, _, true) => &["EVA_BuildingInfiltrated", "EVA_EnemyBasePoweredDown"],
                    (3, true, _) | (3, _, true) => &["EVA_BuildingInfiltrated"],
                    (4 | 6 | 7, true, _) => &["EVA_TechnologyStolen"],
                    (4 | 6 | 7, _, true) => &["EVA_BuildingInfiltrated", "EVA_NewTechnologyAcquired"],
                    (5, true, _) => &["EVA_CashStolen"],
                    (5, _, true) => &["EVA_BuildingInfCashStolen"],
                    _ => &[],
                };
                cues.write_batch(lines.iter().copied().map(Cue::Eva));
            }
            // The player hears `UpgradeVeteranSound`
            // or `UpgradeEliteSound` where the unit is, and the announcer.
            Event::Promoted { unit, owner, elite } => {
                if owner == local() {
                    if let Some(at) = sim.0.aim_point(ra2_sim::combat::Aim::Unit(unit)) {
                        let key = if elite { "UpgradeEliteSound" } else { "UpgradeVeteranSound" };
                        cues.write_batch(crate::audio::rules_sound(&files, key).map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                    }
                    cues.write(Cue::Eva("EVA_UnitPromoted"));
                }
            }
            // The
            // powerup's animation at the cell, 200 leptons up, for everyone; the
            // sound (and for armour, firepower and speed the announcer) only
            // for the player who picked it up.
            Event::CrateCollected { house, cell, kind, .. } => {
                let ground = sim.0.ground_z(cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                let at = scene_point_3d(view.origin, (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128, ground + 200));
                if let Some(anim) = sim.0.crate_rules.powerups.get(kind as usize).and_then(|p| p.anim.clone()) {
                    let depth = 2.0 + ((cell.0 + cell.1) as f32 + 1.0) * crate::game::DEPTH_PER_ROW;
                    start(&mut commands, &mut cues, view, &anim, at, depth);
                }
                if house == local() {
                    use ra2_sim::powerups as p;
                    let (sound, eva) = match kind {
                        p::MONEY => ("CrateMoneySound", None),
                        p::UNIT => ("CrateUnitSound", None),
                        p::HEAL_BASE => ("HealCrateSound", None),
                        p::REVEAL => ("CrateRevealSound", None),
                        p::ARMOR => ("CrateArmourSound", Some("EVA_UnitArmorUpgraded")),
                        p::SPEED => ("CrateSpeedSound", Some("EVA_UnitSpeedUpgraded")),
                        p::FIREPOWER => ("CrateFireSound", Some("EVA_UnitFirePowerUpgraded")),
                        p::VETERAN => ("CratePromoteSound", None),
                        _ => ("CrateMoneySound", None),
                    };
                    cues.write_batch(crate::audio::rules_sound(&files, sound).map(|name| Cue::SoundOn(name, at)));
                    if let Some(eva) = eva {
                        cues.write(Cue::Eva(eva));
                    }
                }
            }
            // `CloakSound` where it is.
            Event::Cloaked { unit } => {
                if let Some(at) = sim.0.unit(unit).filter(|u| !sim.0.hidden_from(u, local())).and_then(|u| sim.0.aim_point(ra2_sim::combat::Aim::Unit(u.id))) {
                    cues.write_batch(crate::audio::rules_sound(&files, "CloakSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            // The victim type's `CrushSound`.
            Event::Crushed { kind, at } => {
                let sound = (kind as usize).checked_sub(1).and_then(|k| units.0.get(k)).and_then(|entry| armoury.crush_sounds.get(&entry.name.to_lowercase()));
                cues.write_batch(sound.cloned().map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
            }
            Event::Mended { .. } => {}
            // One of the player's
            // buildings hit starts a radar event at it, and only if that is
            // accepted (none like it alive within eight cells) the announcer
            // says so and the siren sounds.
            Event::Attacked { owner, at, building, miner } => {
                if building && owner == local() && radar_events.raise(3, at, tick) {
                    cues.write(Cue::Eva("EVA_OurBaseIsUnderAttack"));
                    cues.write_batch(crate::audio::rules_sound(&files, "BaseUnderAttackSound").map(Cue::Sound));
                }
                // The player's miner hurt and still alive: radar event 4 and
                // the announcer.
                if miner && owner == local() && radar_events.raise(4, at, tick) {
                    cues.write(Cue::Eva("EVA_OreMinerUnderAttack"));
                }
            }
        }
    }


    // Animations step on and end.
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

    // Shots that can be seen. One that `Rotates` shows the frame for the way it
    // is going, of 32.
    let flying: Vec<(u32, u16, (f64, f64, f64), u8, Option<u8>)> = sim
        .0
        .bullets()
        .iter()
        .map(|b| (b.id, b.weapon, b.position, b.facing(), None))
        .chain(sim.0.leaps().iter().map(|leap| (leap.id | 0x8000_0000, leap.weapon, leap.position, leap.facing, Some(leap.house))))
        .chain(sim.0.nukes().iter().filter_map(|nuke| Some((nuke.id, sim.0.nuke_weapon(nuke)?, (nuke.at.0 as f64, nuke.at.1 as f64, nuke.at.2 as f64), 0, Some(nuke.house)))))
        .collect();
    let mut seen: Vec<u32> = Vec::new();
    for (entity, shot, mut sprite, mut transform) in &mut shots {
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
        }
    }
    for &(id, weapon, ..) in &flying {
        let visible = armoury.weapon_looks.get(weapon as usize).is_some_and(|look| look.image.is_some());
        if visible && !seen.contains(&id) {
            commands.spawn((DespawnOnExit(Screen::InGame), ShotSprite(id), Sprite::default(), Transform::from_xyz(0.0, 0.0, -10.0)));
        }
    }
    for (entity, beam) in &beams {
        if tick >= beam.until {
            commands.entity(entity).despawn();
        }
    }
}
