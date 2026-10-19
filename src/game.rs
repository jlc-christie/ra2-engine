//! The in-game view. For now this shows the chosen map, one sprite per terrain cell
//! and object, and lets the camera scroll over it.
//!

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use ra2_formats::map::Map;

use crate::files::GameFiles;
use crate::shell::Screen;
use crate::terrain::{self, Layer, PlacementKind};

/// Depth added per isometric row, small enough that every row of the largest map
/// fits inside one draw layer.
pub(crate) const DEPTH_PER_ROW: f32 = 0.001;

/// One image of a map animation: where it is in the atlas and in the world.
pub struct AnimationStep {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
    index: usize,
    centre: Vec2,
}

/// For each animated terrain object, the art and shadow of each of its images.
#[derive(Resource)]
pub struct MapAnimations(Vec<Vec<(Option<AnimationStep>, Option<AnimationStep>)>>);

/// Marks the sprite that shows an animated terrain object, or its shadow.
#[derive(Component)]
pub struct Animated {
    animation: usize,
    shadow: bool,
}

/// Shows each animated terrain object at the image the simulation has it on.
fn animate_map(
    sim: Option<Res<crate::units::Sim>>,
    animations: Option<Res<MapAnimations>>,
    mut sprites: Query<(&Animated, &mut Sprite, &mut Transform, &mut Visibility)>,
) {
    let (Some(sim), Some(animations)) = (sim, animations) else {
        return;
    };
    for (animated, mut sprite, mut transform, mut visibility) in &mut sprites {
        let stage = sim.0.animated_terrain().get(animated.animation).map_or(0, |object| object.stage as usize);
        let step = animations.0.get(animated.animation).and_then(|steps| steps.get(stage));
        let step = step.and_then(|(art, shadow)| if animated.shadow { shadow.as_ref() } else { art.as_ref() });
        let Some(step) = step else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Inherited;
        if sprite.texture_atlas.as_ref().is_some_and(|atlas| atlas.index == step.index) {
            continue;
        }
        sprite.image = step.image.clone();
        sprite.texture_atlas = Some(TextureAtlas { layout: step.layout.clone(), index: step.index });
        transform.translation.x = step.centre.x;
        transform.translation.y = step.centre.y;
    }
}

/// A sprite belonging to the map, tagged so that game logic can find and change it.
// (Ore, bridges and walls are drawn from the simulation instead; see `units`.)
#[derive(Component)]
#[allow(dead_code)]
pub struct MapSprite {
    pub cell: IVec2,
    pub kind: PlacementKind,
}
/// Camera speed in pixels per second, at the middle scroll rate.
const SCROLL_SPEED: f32 = 900.0;
/// Distance from a window edge at which the mouse scrolls the view.
const EDGE_SCROLL_MARGIN: f32 = 4.0;

/// What the next game should load.
#[derive(Resource)]
pub struct GameSetup {
    /// Map file name without extension, such as `mp02t2`.
    pub map: String,
    /// The map's name as the lists show it, if known.
    pub map_name: String,
    pub options: Options,
    /// The map's `MaxPlayers`.
    pub max_players: usize,
    /// The game mode's rules file, from MPModes.ini (such as `MPBattle.ini`).
    pub mode_rules: String,
}

/// A match as it was actually set up: the same as [`GameSetup`] with every
/// choice left to chance settled, so that setting it up again gives the same
/// game. Kept in a save (and, later, sent to the other players of a network
/// game).
#[derive(Resource, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MatchSetup {
    pub map: String,
    pub map_name: String,
    pub options: Options,
    pub max_players: usize,
    pub mode_rules: String,
}

/// The match's options that are rules to the simulation (not kept in its
/// snapshot), for a world set up afresh or rebuilt to replay.
pub fn match_rules(world: &mut ra2_sim::World, options: &Options) {
    world.mcv_repacks = options.mcv_repacks;
    // Houses can be defeated in a skirmish, by the
    // short game's rule or the long one's.
    world.end_checks = true;
    world.short_game = options.short_game;
    world.supers_allowed = options.super_weapons;
}

/// The world a settled match is played in, without the app: the map, the
/// houses as chosen, and the match's rules. Its state is the start's; a
/// recording or save is laid over it (`saves::replay`).
#[cfg(test)]
pub fn match_world(files: &GameFiles, setup: &MatchSetup) -> Option<ra2_sim::World> {
    let options = setup.options;
    let opponents = options
        .opponents
        .iter()
        .take(setup.max_players.saturating_sub(1))
        .filter(|row| row.level >= 0)
        .map(|row| terrain::Opponent { level: row.level as usize, country: (row.country >= 0).then_some(row.country as usize), colour: (row.colour >= 0).then_some(row.colour as usize) })
        .collect();
    let player = terrain::Player {
        country: options.country.max(0) as usize,
        colour: options.colour.max(0) as usize,
        opponents,
        mode_rules: setup.mode_rules.clone(),
        start: Some(options.start.max(0) as usize),
    };
    let map = files.get(&format!("{}.map", setup.map)).or_else(|| files.get(&format!("{}.mpr", setup.map))).and_then(Map::parse)?;
    let mut scene = terrain::build(files, &map, &player)?;
    let mut world = crate::units::build_world(files, &mut scene).world;
    match_rules(&mut world, &options);
    Some(world)
}

impl MatchSetup {
    pub fn as_game_setup(&self) -> GameSetup {
        GameSetup { map: self.map.clone(), map_name: self.map_name.clone(), options: self.options, max_players: self.max_players, mode_rules: self.mode_rules.clone() }
    }
}

/// The choices made on the skirmish screen.
// What there is to choose, the ranges, and the first-run defaults (from rules.ini
// `[MultiplayerDialogSettings]`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Options {
    /// The player's country as its place in `[Countries]` (0 to 8), or -2 for random.
    pub country: i32,
    /// The player's colour (0 gold, red, blue, green, orange, sky, purple, 7 pink),
    /// or -2 for random.
    pub colour: i32,
    /// Game speed as the engine stores it: 0 fastest to 6 slowest.
    pub speed: i32,
    pub credits: i32,
    pub unit_count: i32,
    pub short_game: bool,
    pub mcv_repacks: bool,
    pub crates: bool,
    pub super_weapons: bool,
    /// Where the first player starts: -2 until the match is settled, then a
    /// number drawn for it (taken within the map's start positions).
    pub start: i32,
    /// The seven computer rows of the skirmish screen.
    pub opponents: [Opponent; 7],
}

/// A computer row: its level (-1 none, 0 brutal, 1 medium, 2 easy), and country
/// and colour as for the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Opponent {
    pub level: i32,
    pub country: i32,
    pub colour: i32,
}

impl Opponent {
    pub const NONE: Self = Self { level: -1, country: -2, colour: -2 };
}

impl Default for Options {
    fn default() -> Self {
        Self {
            country: 0,
            colour: 0,
            speed: 0,
            credits: 10_000,
            unit_count: 10,
            short_game: true,
            mcv_repacks: true,
            crates: true,
            super_weapons: true,
            start: -2,
            // The first-run default is
            // one easy enemy, random.
            opponents: {
                let mut rows = [Opponent::NONE; 7];
                rows[0].level = 2;
                rows
            },
        }
    }
}

impl Options {
    /// Simulation ticks a second for the chosen game speed.
    // Each logic frame waits out the speed
    // value in ticks of 16 ms, so 1 is 62.5 a second and 6 about 10.4; 0 does not
    // wait at all.
    // Not original behaviour: the fastest setting is held to 60 a second here,
    // where the original runs as fast as the machine allows.
    pub fn ticks_per_second(&self) -> f64 {
        match self.speed {
            speed if speed <= 0 => 60.0,
            speed => 1000.0 / (16.0 * speed as f64),
        }
    }
}

/// The composed terrain's size in pixels, and whether the camera has been placed
/// over it yet.
#[derive(Resource)]
struct TerrainView {
    size: Vec2,
    /// The map's visible area (`LocalSize`), in scene pixels: the middle of the
    /// view is kept inside it.
    area: Rect,
    /// Where the camera starts, in scene pixels.
    start: Vec2,
    camera_placed: bool,
}

/// A request to centre the view on a point of the scene, in scene pixels.
#[derive(Resource, Default)]
pub struct CentreOn(pub Option<Vec2>);

/// The middle of the view, in scene pixels.
#[derive(Resource, Default)]
pub struct ViewCentre(pub Vec2);

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CentreOn>()
            .init_resource::<ViewCentre>()
            .init_resource::<ScrollBlocked>()
            .init_resource::<RightDrag>()
            .add_systems(OnEnter(Screen::InGame), load_map)
            .add_systems(OnExit(Screen::InGame), reset_camera)
            .add_systems(
                Update,
                (scroll_camera.run_if(crate::pause::playing), animate_map).run_if(in_state(Screen::InGame)),
            );
    }
}

/// The player's country when it was to be random and the loading screen has
/// already drawn it.
#[derive(Resource)]
pub(crate) struct DrawnCountry(pub usize);

pub(crate) fn load_map(
    mut commands: Commands,
    files: Res<GameFiles>,
    setup: Res<GameSetup>,
    pending: Option<Res<crate::saves::PendingLoad>>,
    seat: Option<Res<crate::net::NetSeat>>,
    lobby: Option<Res<crate::net::Lobby>>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut next_screen: ResMut<NextState<Screen>>,
    drawn: Option<Res<DrawnCountry>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut ground: ResMut<Assets<crate::depth::GroundMaterial>>,
) {
    // A random country or colour is settled now, or for the country, on the
    // loading screen before.
    // Any of the nine countries, any of the
    // eight colours. Not original behaviour: the clock as the source of chance.
    let options = setup.options;
    // Drawn in this order: the human's
    // country, the human's colour, then each row's country and colour; a colour
    // is drawn again until nobody has it.
    let mut chance = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |since| since.subsec_nanos() as u64 | 1);
    let mut draw = |below: usize| {
        chance = chance.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((chance >> 33) as usize) % below.max(1)
    };
    // DATA: missions.pkt `MaxPlayers`: no more computer players than the map
    // has room for beside the human.
    let room = setup.max_players.saturating_sub(1);
    let rows: Vec<crate::game::Opponent> = options.opponents.iter().copied().take(room).filter(|row| row.level >= 0).collect();
    let mut colours: Vec<usize> = std::iter::once(options.colour).chain(rows.iter().map(|row| row.colour)).filter(|&c| c >= 0).map(|c| c as usize).collect();
    let mut colour_for = |chosen: i32, draw: &mut dyn FnMut(usize) -> usize| {
        if chosen >= 0 {
            return chosen as usize;
        }
        loop {
            let colour = draw(8);
            if !colours.contains(&colour) || colours.len() >= 8 {
                colours.push(colour);
                return colour;
            }
        }
    };
    // The first house's start is
    // drawn from the map's start positions (the others then take the
    // furthest left). RA2_START=n fixes it, for checks.
    let start = std::env::var("RA2_START").ok().and_then(|n| n.trim().parse().ok()).unwrap_or(if options.start < 0 { draw(1 << 16) as i32 } else { options.start });
    let country = match drawn {
        Some(drawn) => {
            commands.remove_resource::<DrawnCountry>();
            drawn.0
        }
        None if options.country < 0 => draw(9),
        None => options.country as usize,
    };
    let colour = colour_for(options.colour, &mut draw);
    let opponents = rows
        .iter()
        .map(|row| terrain::Opponent {
            level: row.level as usize,
            country: Some(if row.country < 0 { draw(9) } else { row.country as usize }),
            colour: Some(colour_for(row.colour, &mut draw)),
        })
        .collect();
    let opponents: Vec<terrain::Opponent> = opponents;
    // The match as settled, for saving.
    let mut settled = options;
    settled.start = start;
    settled.country = country as i32;
    settled.colour = colour as i32;
    settled.opponents = std::array::from_fn(|n| {
        opponents.get(n).map_or(Opponent::NONE, |row| Opponent { level: row.level as i32, country: row.country.map_or(-2, |c| c as i32), colour: row.colour.map_or(-2, |c| c as i32) })
    });
    commands.insert_resource(MatchSetup { map: setup.map.clone(), map_name: setup.map_name.clone(), options: settled, max_players: setup.max_players, mode_rules: setup.mode_rules.clone() });
    let player = terrain::Player { country, colour, opponents, mode_rules: setup.mode_rules.clone(), start: Some(start as usize) };
    commands.insert_resource(Time::<Fixed>::from_hz(options.ticks_per_second()));
    let scene = files
        .get(&format!("{}.map", setup.map))
        .or_else(|| files.get(&format!("{}.mpr", setup.map)))
        .and_then(Map::parse)
        .and_then(|map| terrain::build(&files, &map, &player));
    let Some(mut scene) = scene else {
        error!("could not load map {}", setup.map);
        next_screen.set(Screen::Skirmish);
        return;
    };
    info!("the player starts at {:?}", scene.player_start);
    if scene.missing_tiles > 0 {
        warn!("{}: {} cells have no tile art", setup.map, scene.missing_tiles);
    }
    if scene.missing_objects > 0 {
        warn!(
            "{}: {} objects could not be drawn: {}",
            setup.map,
            scene.missing_objects,
            scene.missing_art.join(", ")
        );
    }
    info!(
        "{}: {} ore cells, {} terrain objects, {} structures, {} infantry, {} units, {} sprites on {} atlas pages",
        setup.map,
        scene.ore_cells,
        scene.terrain_objects,
        scene.structures,
        scene.infantry,
        scene.units,
        scene.placements.len(),
        scene.pages.len()
    );

    let mut built = crate::units::build_world(&files, &mut scene);
    // RA2_NOSHROUD=1 turns the shroud off for checks; never in a network game,
    // where it is shared state.
    if seat.is_none() && std::env::var_os("RA2_NOSHROUD").is_some() {
        built.world.map.shroud_on = false;
    }
    let built_levels: std::collections::HashMap<(i32, i32), i32> =
        scene.placements.iter().map(|p| p.light_cell).map(|cell| (cell, built.world.map.level(cell.0, cell.1) as i32)).collect();
    commands.insert_resource(std::mem::take(&mut scene.radar_ground));
    commands.insert_resource(scene.armoury.clone());
    commands.insert_resource(crate::lighting::WorldLight::new(std::mem::take(&mut scene.light_setup)));
    commands.insert_resource(crate::lighting::StormLight::default());
    // A computer house
    // starts with `MultiplayerAICM` per cent more (the hardest with five times
    // the credits).
    // RA2_LOCAL=n sits this machine's player at house n (2 is the first
    // computer house), for looking at a game from another seat.
    let networked = seat.is_some();
    terrain::set_local(seat.map(|seat| seat.0).or(std::env::var("RA2_LOCAL").ok().and_then(|n| n.trim().parse().ok())).unwrap_or(terrain::PLAYER));
    built.world.map.viewer = terrain::local();
    // The sidebar art is the player's side's.
    files.set_side(built.world.houses.get(terrain::local() as usize).map_or(0, |house| house.side.max(0) as usize));
    let bonus = built.world.ai_rules.credit_bonus;
    for house in built.world.houses.iter_mut().skip(terrain::PLAYER as usize) {
        house.credits = options.credits;
        if let Some(brain) = &house.ai {
            house.credits += options.credits * bonus[brain.difficulty.min(2)] / 100;
        }
    }
    built.world.create_starting_units(options.unit_count);
    match_rules(&mut built.world, &options);
    // With "Crates Appear" they go down at the start.
    // RA2_CRATES=1 or 0 overrides the option, for checks.
    let crates = std::env::var("RA2_CRATES").map_or(options.crates, |v| v.trim() != "0");
    if crates {
        // The session's count of people playing over the network (0 in
        // skirmish); see `start_crates`.
        let people = if networked { built.world.houses.iter().filter(|house| house.human).count() as i32 } else { 0 };
        built.world.start_crates(people);
        info!("crates at {:?}", built.world.crates().iter().map(|c| c.cell).collect::<Vec<_>>());
    }
    // RA2_CRATE=x,y puts a crate (of random content) at that cell, for checks.
    if let Some((x, y)) = std::env::var("RA2_CRATE").ok().and_then(|v| v.split_once(',').and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))) {
        built.world.put_crate_for_tests((x, y), 0xff);
    }
    let strings = files.get("ra2.csf").and_then(ra2_formats::csf::Csf::parse);
    let label = |key: &str| strings.as_ref().and_then(|strings| strings.get(key)).unwrap_or(key).to_owned();
    let mut names = vec![String::new(), label("GUI:Player")];
    names.extend(rows.iter().enumerate().map(|(n, row)| if row.level >= crate::net::HUMAN_ROW { format!("{} {}", label("GUI:Player"), n + 2) } else { label(["GUI:AIHard", "GUI:AINormal", "GUI:AIEasy"][row.level.clamp(0, 2) as usize]) }));
    // The player goes by the name they gave; in a network game, so do the others.
    if let Some(lobby) = &lobby {
        if !lobby.name.is_empty() && lobby.house_names.is_empty() {
            names[1] = lobby.name.clone();
        }
        for (house, name) in lobby.house_names.iter().enumerate().filter(|(_, name)| !name.is_empty()) {
            if let Some(slot) = names.get_mut(house) {
                *slot = name.clone();
            }
        }
    }
    commands.insert_resource(crate::endgame::HouseNames(names));
    let mut saved_view = None;
    // The game is recorded from its start: the world as it begins, and (in
    // `Sim`) every order. A loaded game carries its recording on, or starts
    // one where it was saved.
    let mut recording = crate::saves::Recording { start: built.world.snapshot(), start_tick: built.world.current_tick(), orders: Vec::new() };
    if let Some(pending) = pending {
        commands.remove_resource::<crate::saves::PendingLoad>();
        match built.world.restore(&pending.0.state) {
            Ok(()) => {
                terrain::set_local(pending.0.local);
                built.world.map.viewer = pending.0.local;
                saved_view = Some(pending.0.camera);
                recording = pending.0.recording.clone().unwrap_or(crate::saves::Recording { start: pending.0.state.clone(), start_tick: pending.0.tick, orders: Vec::new() });
            }
            Err(error) => error!("the saved game could not be restored: {error}"),
        }
    }
    commands.insert_resource(recording);
    crate::units::spawn_tile_anims(&mut commands, &scene.tile_anims);
    crate::units::setup(&mut commands, &files, built, scene.player_palette, scene.unit_art, scene.origin, options.speed);

    // One texture and one layout per atlas page; `slots[entry]` is the entry's index
    // within its page's layout.
    let page_size = UVec2::splat(terrain::PAGE_SIZE);
    let mut page_layouts: Vec<TextureAtlasLayout> =
        scene.pages.iter().map(|_| TextureAtlasLayout::new_empty(page_size)).collect();
    let slots: Vec<usize> = scene
        .entries
        .iter()
        .map(|entry| {
            let min = UVec2::new(entry.x, entry.y);
            let max = min + UVec2::new(entry.width, entry.height);
            page_layouts[entry.page].add_texture(URect::from_corners(min, max))
        })
        .collect();
    let mut heights: Vec<Option<Handle<Image>>> = Vec::new();
    let pages: Vec<(Handle<Image>, Handle<TextureAtlasLayout>)> = scene
        .pages
        .into_iter()
        .zip(page_layouts)
        .map(|(page, layout)| {
            heights.push((!page.z.is_empty()).then(|| {
                images.add(Image::new(
                    Extent3d { width: terrain::PAGE_SIZE, height: terrain::PAGE_SIZE, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    page.z,
                    TextureFormat::R8Uint,
                    RenderAssetUsages::RENDER_WORLD,
                ))
            }));
            let image = Image::new(
                Extent3d {
                    width: terrain::PAGE_SIZE,
                    height: terrain::PAGE_SIZE,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                page.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            (images.add(image), layouts.add(layout))
        })
        .collect();

    // World space has the scene's top-left corner at the origin and y pointing up,
    // so a scene pixel (x, y) is at world (x, -y).
    let step = |step: &Option<(usize, i32, i32)>| {
        step.map(|(entry, x, y)| {
            let size = scene.entries[entry];
            let (image, layout) = &pages[size.page];
            AnimationStep {
                image: image.clone(),
                layout: layout.clone(),
                index: slots[entry],
                centre: Vec2::new(x as f32 + size.width as f32 / 2.0, -(y as f32 + size.height as f32 / 2.0)),
            }
        })
    };
    commands.insert_resource(MapAnimations(
        scene
            .animations
            .iter()
            .map(|animation| animation.steps.iter().map(|(art, shadow)| (step(art), step(shadow))).collect())
            .collect(),
    ));
    // The ground is drawn first, as one mesh for each atlas page.
    let tiles: Vec<crate::depth::GroundTile> = scene
        .placements
        .iter()
        .filter_map(|placement| {
            let entry = scene.entries[placement.entry];
            Some(crate::depth::GroundTile {
                page: entry.page,
                source: URect::new(entry.x, entry.y, entry.x + entry.width, entry.y + entry.height),
                at: IVec2::new(placement.x, placement.y),
                base: placement.ground?,
            })
        })
        .collect();
    for (page, mesh) in crate::depth::ground_meshes(&tiles, pages.len(), terrain::PAGE_SIZE).into_iter().enumerate() {
        let Some(heights) = heights[page].clone() else {
            continue;
        };
        commands.spawn((
            DespawnOnExit(Screen::InGame),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(ground.add(crate::depth::GroundMaterial { colours: pages[page].0.clone(), heights })),
            Transform::default(),
        ));
    }
    for (number, placement) in scene.placements.iter().enumerate() {
        if placement.ground.is_some() {
            continue;
        }
        let animated = scene.animations.iter().enumerate().find_map(|(index, animation)| {
            if animation.art == number {
                Some(Animated { animation: index, shadow: false })
            } else if animation.shadow == Some(number) {
                Some(Animated { animation: index, shadow: true })
            } else {
                None
            }
        });
        let entry = scene.entries[placement.entry];
        let (image, layout) = &pages[entry.page];
        // Layers are a whole unit apart; within one, cells nearer the viewer (a
        // larger x + y) are in front.
        let row = (placement.cell.0 + placement.cell.1) as f32;
        let raised = if placement.kind == PlacementKind::TileExtra { 0.5 } else { 0.0 };
        // Overlays are drawn with the ground, before any object.
        let (layer, row) = match placement.layer {
            Layer::Terrain => (0.0, row),
            Layer::Shadow => (1.0, row),
            Layer::Object if placement.kind == PlacementKind::Overlay => (crate::depth::FIRST, row / 100.0),
            Layer::Object => (2.0, row),
        };
        let mut spawned = commands.spawn((
            DespawnOnExit(Screen::InGame),
            MapSprite {
                cell: IVec2::new(placement.cell.0, placement.cell.1),
                kind: placement.kind,
            },
            Sprite::from_atlas_image(
                image.clone(),
                TextureAtlas {
                    layout: layout.clone(),
                    index: slots[placement.entry],
                },
            ),
            Transform::from_xyz(
                placement.x as f32 + entry.width as f32 / 2.0,
                -(placement.y as f32 + entry.height as f32 / 2.0),
                layer + (row + raised) * DEPTH_PER_ROW,
            ),
        ));
        if let Some(animated) = animated {
            spawned.insert(animated);
        }
        if let Some(depth) = placement.depth.clone() {
            spawned.insert(depth);
        }
        if let Some(rule) = placement.light {
            let level = built_levels.get(&placement.light_cell).copied().unwrap_or(0);
            spawned.insert(crate::lighting::Lit { cell: placement.light_cell, level, rule });
        }
    }

    // The view starts on the player's start position. RA2_CELL=x,y starts it on that
    // cell instead, for checking a particular spot.
    let size = Vec2::new(scene.width as f32, scene.height as f32);
    let cell: Vec<i32> = std::env::var("RA2_CELL")
        .unwrap_or_default()
        .split(',')
        .filter_map(|n| n.parse().ok())
        .collect();
    let start = match cell[..] {
        [x, y] => {
            let (px, py) = terrain::cell_origin(x, y, 0);
            Vec2::new((scene.origin.0 + px) as f32, (scene.origin.1 + py) as f32)
        }
        // This machine's player may sit at another house than the first.
        _ => match (terrain::local() as usize).checked_sub(2).and_then(|n| scene.enemies.get(n)).map(|house| house.start).or(scene.player_start) {
            Some((x, y)) => {
                let (px, py) = terrain::cell_origin(x, y, 0);
                Vec2::new((scene.origin.0 + px) as f32, (scene.origin.1 + py) as f32)
            }
            None => size / 2.0,
        },
    };
    // The camera sits at (x, -y) of the view's start.
    let start = saved_view.map_or(start, |(x, y)| Vec2::new(x, -y));
    // The visible area, from the map's width W and
    // `LocalSize` (lx, ly, lw, lh): across from 60 lx - 30 W for 60 lw pixels,
    // down from 15 (W + 2 ly - 5) for 30 lh + 135 (cell (x, y)'s corner is at
    // 30 (x - y), 15 (x + y)). With no `LocalSize`, the whole scene.
    let area = match scene.playfield {
        Some((w, _, [lx, ly, lw, lh])) => {
            let x0 = (scene.origin.0 + 60 * lx - 30 * w) as f32;
            let y0 = (scene.origin.1 + 15 * (w + 2 * ly - 5)) as f32;
            Rect::new(x0, y0, x0 + 60.0 * lw as f32, y0 + 30.0 * lh as f32 + 135.0)
        }
        None => Rect::new(0.0, 0.0, size.x, size.y),
    };
    commands.insert_resource(TerrainView {
        start,
        size,
        area,
        camera_placed: false,
    });
    const BORDER: f32 = 20_000.0;
    let black = |x: f32, y: f32, w: f32, h: f32| {
        (
            DespawnOnExit(Screen::InGame),
            Sprite { color: Color::BLACK, custom_size: Some(Vec2::new(w, h)), ..default() },
            Transform::from_xyz(x + w / 2.0, -(y + h / 2.0), 870.0),
        )
    };
    let (min, max) = (area.min, area.max);
    commands.spawn(black(min.x - BORDER, min.y - BORDER, BORDER, area.height() + 2.0 * BORDER));
    commands.spawn(black(max.x, min.y - BORDER, BORDER, area.height() + 2.0 * BORDER));
    commands.spawn(black(min.x, min.y - BORDER, area.width(), BORDER));
    commands.spawn(black(min.x, max.y, area.width(), BORDER));
}

fn reset_camera(mut camera: Single<&mut Transform, With<Camera2d>>) {
    camera.translation.x = 0.0;
    camera.translation.y = 0.0;
}

/// Whether the last try at scrolling the view got nowhere: it is at the map's
/// edge that way.
#[derive(Resource, Default)]
pub struct ScrollBlocked(pub bool);

/// A right press on the map view, which becomes a drag that scrolls once the
/// pointer has gone more than 8 pixels from where it was pressed. `blocked`
/// has a bit for each way the view cannot go: north, east, south, west.
#[derive(Resource, Default)]
pub struct RightDrag {
    pub press: Option<Vec2>,
    pub started: bool,
    pub blocked: u8,
}

/// Scrolls with the arrow keys or the mouse at a window edge, keeping the view
/// inside the terrain. Letter keys are left for the game's own shortcuts.
fn scroll_camera(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    view: Option<ResMut<TerrainView>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<&mut Transform, With<Camera2d>>,
    mut centre_on: ResMut<CentreOn>,
    mut centre: ResMut<ViewCentre>,
    mut blocked: ResMut<ScrollBlocked>,
    prefs: Res<crate::prefs::Prefs>,
    (buttons, mut drag, pointer): (Res<ButtonInput<MouseButton>>, ResMut<RightDrag>, Res<crate::cursor::Pointer>),
) {
    let Some(mut view) = view else {
        return;
    };
    let size = view.size;
    if !view.camera_placed {
        view.camera_placed = true;
        camera.translation.x = view.start.x;
        camera.translation.y = -view.start.y;
    }
    let pressed = |code: KeyCode| keys.pressed(code);
    let cursor = window.cursor_position();
    let window_size = Vec2::new(window.width(), window.height());
    let mut direction = Vec2::ZERO;
    if pressed(KeyCode::ArrowLeft) || cursor.is_some_and(|c| c.x < EDGE_SCROLL_MARGIN) {
        direction.x -= 1.0;
    }
    if pressed(KeyCode::ArrowRight)
        || cursor.is_some_and(|c| c.x > window_size.x - EDGE_SCROLL_MARGIN)
    {
        direction.x += 1.0;
    }
    if pressed(KeyCode::ArrowUp) || cursor.is_some_and(|c| c.y < EDGE_SCROLL_MARGIN) {
        direction.y += 1.0;
    }
    if pressed(KeyCode::ArrowDown)
        || cursor.is_some_and(|c| c.y > window_size.y - EDGE_SCROLL_MARGIN)
    {
        direction.y -= 1.0;
    }

    // With Shift held the arrow keys scroll two and a half times as fast, and with
    // Ctrl they jump to the map's edge.
    let arrows = [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown];
    let held = |a: KeyCode, b: KeyCode| keys.pressed(a) || keys.pressed(b);
    let mut step = direction * SCROLL_SPEED * 4.0 / (prefs.scroll_rate + 1) as f32 * time.delta_secs();
    let tactical = Vec2::new(window_size.x - crate::hud::SIDEBAR_WIDTH, window_size.y - crate::hud::BOTTOM_BAR_HEIGHT);
    if buttons.just_pressed(MouseButton::Right) {
        *drag = RightDrag { press: pointer.0.filter(|c| c.x < tactical.x && c.y < tactical.y), ..default() };
    } else if !buttons.pressed(MouseButton::Right) && !buttons.just_released(MouseButton::Right) {
        *drag = RightDrag::default();
    }
    // Each update scrolls by the pointer's distance from the press point over
    // `ScrollRate` + 1, faster still when pressed at the screen's edge.
    if let (Some(press), Some(at), true) = (drag.press, pointer.0, buttons.pressed(MouseButton::Right)) {
        let mut d = at - press;
        if !drag.started && (d.x.abs() > 8.0 || d.y.abs() > 8.0) {
            drag.started = true;
        }
        if drag.started {
            if d.x == 0.0 {
                d.x = if press.x == 0.0 { -1.0 } else if press.x >= window_size.x - 1.0 { 1.0 } else { 0.0 };
            }
            if d.y == 0.0 {
                d.y = if press.y == 0.0 { -1.0 } else if press.y >= window_size.y { 1.0 } else { 0.0 };
            }
            if press.x < 10.0 && d.x < 0.0 {
                d.x = 4.0 * d.x.min(-5.0);
            } else if press.x > window_size.x - 10.0 && d.x > 0.0 {
                d.x = 4.0 * d.x.max(5.0);
            }
            if press.y < 10.0 && d.y < 0.0 {
                d.y = 4.0 * d.y.min(-5.0);
            } else if press.y > window_size.y - 10.0 && d.y > 0.0 {
                d.y = 4.0 * d.y.max(5.0);
            }
            let f = 1.0 / (prefs.scroll_rate + 1) as f32;
            // Paced to sixty updates a second.
            let updates = time.delta_secs() * 60.0;
            step = Vec2::new((d.x * f).trunc(), -(d.y * f).trunc()) * updates;
        }
    }
    if keys.any_pressed(arrows) {
        if held(KeyCode::ControlLeft, KeyCode::ControlRight) {
            step = direction * (size.x + size.y);
        } else if held(KeyCode::ShiftLeft, KeyCode::ShiftRight) {
            step *= 2.5;
        }
    }
    // A request to look somewhere: the point goes to the middle of the map view,
    // which is the window less the sidebar and the bar along the bottom.
    if let Some(point) = centre_on.0.take() {
        camera.translation.x = point.x + crate::hud::SIDEBAR_WIDTH / 2.0;
        camera.translation.y = -(point.y + crate::hud::BOTTOM_BAR_HEIGHT / 2.0);
    }
    // The middle of the view (the window less the sidebar and the bar) is
    // kept half a view inside the visible area; where the view is the larger,
    // the low edge is tested first, so the view sits at the area's left or
    // top.
    let clamp = |value: f32, low: f32, high: f32| if value < low { low } else if value > high { high } else { value };
    let area = view.area;
    let before = camera.translation;
    if drag.started {
        // The middle of the view, as kept, and whether a pixel's scroll a way
        // (in screen terms, y down) would move it.
        let middle = Vec2::new(camera.translation.x - crate::hud::SIDEBAR_WIDTH / 2.0, -camera.translation.y - crate::hud::BOTTOM_BAR_HEIGHT / 2.0);
        let kept = |point: Vec2| Vec2::new(clamp(point.x, area.min.x + tactical.x / 2.0, area.max.x - tactical.x / 2.0), clamp(point.y, area.min.y + tactical.y / 2.0, area.max.y - tactical.y / 2.0));
        let moves = |way: Vec2| kept(middle + way) != kept(middle);
        drag.blocked = [-Vec2::Y, Vec2::X, Vec2::Y, -Vec2::X].iter().enumerate().fold(0, |mask, (bit, &way)| if moves(way) { mask } else { mask | 1 << bit });
    }
    let middle_x = camera.translation.x + step.x - crate::hud::SIDEBAR_WIDTH / 2.0;
    let middle_y = -(camera.translation.y + step.y) - crate::hud::BOTTOM_BAR_HEIGHT / 2.0;
    let middle_x = clamp(middle_x, area.min.x + tactical.x / 2.0, area.max.x - tactical.x / 2.0);
    let middle_y = clamp(middle_y, area.min.y + tactical.y / 2.0, area.max.y - tactical.y / 2.0);
    camera.translation.x = (middle_x + crate::hud::SIDEBAR_WIDTH / 2.0).round();
    camera.translation.y = -(middle_y + crate::hud::BOTTOM_BAR_HEIGHT / 2.0).round();
    if direction != Vec2::ZERO && time.delta_secs() > 0.0 {
        blocked.0 = camera.translation == before;
    } else if direction == Vec2::ZERO {
        blocked.0 = false;
    }
    centre.0 = Vec2::new(
        camera.translation.x - crate::hud::SIDEBAR_WIDTH / 2.0,
        -camera.translation.y - crate::hud::BOTTOM_BAR_HEIGHT / 2.0,
    );
}

