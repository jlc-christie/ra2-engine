//! Turns a map into a scene: a texture atlas of the art it uses, and one placement
//! per terrain cell, raised tile part, ore cell, tree and shadow.
//!

use std::collections::{HashMap, HashSet};

use ra2_formats::ini::Ini;
use ra2_formats::map::Map;
use ra2_formats::pal::Palette;
use ra2_formats::shp::Shp;
use ra2_formats::tmp::{Tmp, diamond_row};
use ra2_formats::vxl::{Hva, Vpl, Vxl};

use crate::colors::HouseColors;
use crate::files::GameFiles;
use crate::voxel::{self, NormalTables};

// DATA: cell size is stored in every TMP file header (60x30).
pub const CELL_WIDTH: i32 = 60;
pub const CELL_HEIGHT: i32 = 30;
/// Vertical pixels per height level.
pub const LEVEL_HEIGHT: i32 = 15;

/// Space left above the highest row for tile art that rises above its cell.
// Not an engine value; only sizes the scene.
const TOP_MARGIN: i32 = 120;

/// Edge length of an atlas page, well inside GPU texture limits.
pub const PAGE_SIZE: u32 = 2048;

/// Which of a theatre's palettes a piece of art is drawn with.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PaletteKind {
    Iso,
    Theater,
    Unit,
}

/// A theatre's terrain art: control file, tile file extension and palettes.
struct Theater {
    extension: &'static str,
    /// The letter that replaces the second character of an art name for art that
    /// has a version per theatre (`gagate_a` becomes `gtgate_a`).
    letter: char,
    iso_palette: Palette,
    theater_palette: Palette,
    unit_palette: Palette,
    /// File name stem for each tile index, in tile set order.
    tile_names: Vec<String>,
    /// The set's `AllowTiberium`, for each tile index.
    tile_tiberium: Vec<bool>,
    /// Animations attached to tiles, by tile index: the animation, its offset
    /// in pixels from the cell's middle, and the cell of the tile it is on.
    tile_anims: HashMap<usize, (String, (i32, i32), u8)>,
}

impl Theater {
    fn load(files: &GameFiles, name: &str) -> Option<Self> {
        // DATA: file names in the theatre archives.
        let (stem, extension, letter) = match name {
            "TEMPERATE" => ("temperat", "tem", 't'),
            "SNOW" => ("snow", "sno", 'a'),
            "URBAN" => ("urban", "urb", 'u'),
            _ => return None,
        };
        let ini = Ini::parse(files.get(&format!("{stem}.ini"))?);
        let mut tile_names = Vec::new();
        let mut tile_tiberium = Vec::new();
        let mut tile_anims = HashMap::new();
        for set in 0.. {
            let section = format!("TileSet{set:04}");
            let Some(file_stem) = ini.get(&section, "FileName") else {
                break;
            };
            let count: usize = ini.get(&section, "TilesInSet")?.parse().ok()?;
            // DATA: a section named by the set's `SetName` gives a tile's
            // `TileNNAnim`, `TileNNXOffset`, `TileNNYOffset`, `TileNNAttachesTo`.
            if let Some(name) = ini.get(&section, "SetName").map(str::trim) {
                for n in 1..=count {
                    let key = |what: &str| ini.get(name, &format!("Tile{n:02}{what}")).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
                    if let Some(anim) = key("Anim").filter(|a| !a.is_empty()) {
                        let number = |what: &str| key(what).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
                        tile_anims.insert(tile_names.len() + n - 1, (anim.to_uppercase(), (number("XOffset"), number("YOffset")), number("AttachesTo") as u8));
                    }
                }
            }
            tile_names.extend((1..=count).map(|n| format!("{file_stem}{n:02}")));
            let allowed = ini.get(&section, "AllowTiberium").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "true" | "yes" | "1"));
            tile_tiberium.extend(std::iter::repeat_n(allowed, count));
        }
        // The theatre set-up loads `%s.PAL` and
        // `UNIT%s.PAL`; `ISO%s.PAL` is loaded with the tile sets.
        let palette = |name: String| Palette::parse(files.get(&name)?);
        Some(Self {
            extension,
            letter,
            iso_palette: palette(format!("iso{extension}.pal"))?,
            theater_palette: palette(format!("{stem}.pal"))?,
            unit_palette: palette(format!("unit{extension}.pal"))?,
            tile_names,
            tile_tiberium,
            tile_anims,
        })
    }

    fn palette(&self, kind: PaletteKind) -> &Palette {
        match kind {
            PaletteKind::Iso => &self.iso_palette,
            PaletteKind::Theater => &self.theater_palette,
            PaletteKind::Unit => &self.unit_palette,
        }
    }
}

/// Which alternate image a cell uses when its tile has up to four of them, indexed
/// by `(y & 3) * 4 + (x & 3)`.
const VARIANT_PATTERN: [usize; 16] = [0, 1, 2, 3, 3, 2, 1, 0, 2, 3, 0, 1, 1, 0, 3, 2];

/// The eight neighbours checked when building the table for tiles with more than
/// four alternates.
const NEIGHBOURS: [(i32, i32); 8] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// The 8x8 table of alternates for tiles with more than four, built so that no cell
/// matches a neighbour where avoidable.
// The construction (up to 64 tries per entry, values 0
// to 7, wrapping neighbour lookups).
fn large_variant_table() -> [usize; 64] {
    let mut table = [usize::MAX; 64];
    let mut state: u32 = 0x2f6e_2b1d;
    for index in 0..64i32 {
        let mut value = 0;
        for _ in 0..64 {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            value = (state >> 16) as usize & 7;
            let clash = NEIGHBOURS.iter().any(|&(dx, dy)| {
                table[(index + dy * 8 + dx).rem_euclid(64) as usize] == value
            });
            if !clash {
                break;
            }
        }
        table[index as usize] = value;
    }
    table
}

type Light = [u32; 3];

// The rows are lookup tables built when the palette is set up; that code is unread.
fn lit(color: [u8; 3], light: Light) -> [u8; 4] {
    let [r, g, b] = std::array::from_fn(|i| (color[i] as u32 * light[i] / 1000).min(255) as u8);
    [r, g, b, 255]
}

/// One piece of art in the atlas.
#[derive(Clone, Copy)]
pub struct AtlasEntry {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct AtlasPage {
    /// `PAGE_SIZE * PAGE_SIZE` RGBA pixels.
    pub rgba: Vec<u8>,
    /// The depth byte of each pixel of the tiles on the page, laid out as
    /// `rgba`; empty while the page has no tiles.
    pub z: Vec<u8>,
}

/// Packs images into pages in rows, left to right.
#[derive(Default)]
struct Atlas {
    pages: Vec<AtlasPage>,
    entries: Vec<AtlasEntry>,
    cursor: (u32, u32),
    row_height: u32,
}

impl Atlas {
    /// Adds an RGBA image and returns its entry index.
    fn add(&mut self, width: u32, height: u32, rgba: &[u8]) -> usize {
        // One transparent pixel between images keeps neighbours from bleeding.
        let (padded_w, padded_h) = (width + 1, height + 1);
        if self.pages.is_empty() || self.cursor.0 + padded_w > PAGE_SIZE {
            self.cursor = (0, self.cursor.1 + self.row_height);
            self.row_height = 0;
        }
        if self.pages.is_empty() || self.cursor.1 + padded_h > PAGE_SIZE {
            self.pages.push(AtlasPage {
                rgba: vec![0; (PAGE_SIZE * PAGE_SIZE * 4) as usize],
                z: Vec::new(),
            });
            self.cursor = (0, 0);
            self.row_height = 0;
        }
        let (x, y) = self.cursor;
        let page = self.pages.len() - 1;
        let target = &mut self.pages[page].rgba;
        for row in 0..height {
            let from = (row * width * 4) as usize;
            let to = (((y + row) * PAGE_SIZE + x) * 4) as usize;
            target[to..to + (width * 4) as usize]
                .copy_from_slice(&rgba[from..from + (width * 4) as usize]);
        }
        self.cursor.0 += padded_w;
        self.row_height = self.row_height.max(padded_h);
        self.entries.push(AtlasEntry {
            page,
            x,
            y,
            width,
            height,
        });
        self.entries.len() - 1
    }

    /// Adds an RGBA image with a depth byte for each pixel.
    fn add_with_depth(&mut self, width: u32, height: u32, rgba: &[u8], z: &[u8]) -> usize {
        let entry = self.add(width, height, rgba);
        let AtlasEntry { page, x, y, .. } = self.entries[entry];
        let page = &mut self.pages[page];
        if page.z.is_empty() {
            page.z = vec![0; (PAGE_SIZE * PAGE_SIZE) as usize];
        }
        for row in 0..height {
            let to = ((y + row) * PAGE_SIZE + x) as usize;
            page.z[to..to + width as usize].copy_from_slice(&z[(row * width) as usize..((row + 1) * width) as usize]);
        }
        entry
    }
}

/// Draw layers, back to front. Within a layer, cells nearer the viewer are in front.
// The engine's depth for a tile pixel is a base that
// depends only on the cell's ground row (screen y with the height shift removed),
// plus a per-pixel value from the tile's depth data; nearer rows win. Ordering by
// row is therefore right for flat ground.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Terrain,
    Shadow,
    Object,
}

/// What a placement shows, for code that later changes the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlacementKind {
    Tile,
    /// The part of a tile's art that rises above its cell.
    TileExtra,
    /// Ore or gems.
    Ore,
    /// Any other overlay: rocks, bridges, walls, fences.
    Overlay,
    /// A building placed by the map.
    Structure,
    /// A foot soldier, civilian or animal placed by the map.
    Infantry,
    /// A tree, lamp post, ore drill or other `[Terrain]` object.
    TerrainObject,
    Shadow,
}

/// A terrain object that plays an animation now and then.
pub struct Animation {
    pub cell: (i32, i32),
    /// Indexes into [`Scene::placements`] of its art and its shadow.
    pub art: usize,
    pub shadow: Option<usize>,
    /// For each image of the animation, the atlas entry and top-left scene pixel
    /// of the art and of its shadow.
    pub steps: Vec<(Option<(usize, i32, i32)>, Option<(usize, i32, i32)>)>,
    /// Ticks per image, and the chance each idle tick of starting.
    pub rate: u32,
    pub probability: f32,
}

/// One piece of art at a position in the scene.
pub struct Placement {
    /// Index into [`Scene::entries`].
    pub entry: usize,
    /// Top-left corner in scene pixels.
    pub x: i32,
    pub y: i32,
    pub layer: Layer,
    /// The cell whose row orders this within its layer: the object's own cell, or
    /// for a building the nearest cell of its foundation.
    pub cell: (i32, i32),
    pub kind: PlacementKind,
    /// How its pixels' depths are worked out.
    pub depth: Option<crate::depth::Depth>,
    /// For a tile, its base depth: that of the top of its cell's box at level
    /// 0, less 30; its pixels add their depth bytes.
    pub ground: Option<i32>,
    /// How it takes the light of `light_cell`, if at all.
    pub light: Option<crate::lighting::LightRule>,
    pub light_cell: (i32, i32),
}

/// One cell of the map as the simulation sees it.
pub struct SceneCell {
    pub x: i32,
    pub y: i32,
    pub level: u8,
    /// False where something standing on the cell closes it to ground units.
    pub passable: bool,
    pub land: ra2_sim::Land,
    /// The tile's slope number; zero is flat.
    pub ramp: u8,
    /// Whether any overlay lies on the cell.
    pub overlay: bool,
    /// A high bridge over the cell: bit 0 the bridge, bit 1 its deck can be
    /// driven on here.
    pub deck: u8,
    /// A bridge overlay on the cell (type and state), and the land the cell
    /// has without it.
    pub bridge: Option<(u8, u8)>,
    pub base_land: ra2_sim::Land,
    /// What stands on it, for the pathfinder's classing of cells.
    pub obstacle: ra2_sim::Obstacle,
    /// The spots a tree on the cell takes up.
    pub tree_bits: u8,
    /// Whether ore may spread onto its tile.
    pub ore_ground: bool,
}

/// A vehicle the map places.
pub struct UnitSpawn {
    /// Model file name without extension.
    pub image: String,
    pub cell: (i32, i32),
    pub facing: u8,
    /// Index of the owner's palette in `UnitArt`.
    pub palette: Option<usize>,
    /// How it moves, from rules.ini.
    pub stats: ra2_sim::UnitStats,
    /// The house that owns it: [`NEUTRAL`] or [`PLAYER`].
    pub owner: u8,
    /// `sound.ini` sections spoken when it is selected and when it is sent
    /// somewhere (`VoiceSelect`, `VoiceMove`).
    pub voice_select: Option<String>,
    pub voice_move: Option<String>,
}

/// One of a building's attached animations.
#[derive(Clone, Debug)]
pub struct AnimSlot {
    /// The firing animation: shown only while the building charges a shot.
    pub special: bool,
    /// The animation type to play, and the one to play when damaged.
    pub name: String,
    pub damaged: String,
    /// `…YSort` and `…ZAdjust`, which order it among the building's parts.
    pub y_sort: i32,
    pub z_adjust: i32,
    /// One of a refinery's fill-level lights other than the first: not shown
    /// while the refinery is empty.
    pub refinery_level: usize,
    /// `…Powered` (default yes): stops on its frame while the building is
    /// offline. Otherwise `…PoweredLight`: gone while offline, started again
    /// when it comes back on.
    pub powered: bool,
    pub powered_light: bool,
    /// `SuperAnim` to `SuperAnimFour`: charging, opening, ready, closing.
    pub super_stage: Option<u8>,
}

/// An animation type from art.ini.
#[derive(Clone, Debug)]
pub struct AnimType {
    /// Art name.
    pub image: String,
    pub start: usize,
    pub loop_start: usize,
    /// `LoopEnd` and `End`; when not given, the number of images in the art.
    pub loop_end: Option<usize>,
    pub end: Option<usize>,
    /// `LoopCount`; -1 for ever.
    pub loop_count: i32,
    /// Game ticks per image, before `Normalized` scaling.
    pub delay: u32,
    pub normalized: bool,
    pub shadow: bool,
    pub y_draw_offset: i32,
    /// `Scorch`, `Crater`, `ForceBigCraters`: the mark it leaves on the ground.
    pub scorch: bool,
    pub crater: bool,
    pub force_big_craters: bool,
    /// The size of its largest picture, which sets the size of the mark, and
    /// that picture's number.
    pub size: (i32, i32),
    pub middle: usize,
    /// `StartSound`, else `Report`: what is heard as it starts.
    pub report: Option<String>,
    /// `Translucent` and `Translucency`: how much of it is drawn (1 is solid).
    pub opacity: f32,
    /// `AltPalette`: drawn with the unit palette in the first colour scheme,
    /// not the animation palette.
    pub alt_palette: bool,
    /// For a `Bouncer`: how it flies and ends, with `ExpireAnim`,
    /// `BounceAnim` and `TrailerAnim` by name.
    pub bouncer: Option<(ra2_sim::remains::Bouncer, [Option<String>; 3])>,
}

/// A list of ability names as bits by the engine's ability numbers.
fn ability_bits(list: Option<&str>) -> u32 {
    list.map_or(0, |list| {
        list.split(';').next().unwrap_or("").split(',').filter_map(|name| ra2_sim::abilities::ability::NAMES.iter().position(|n| n.eq_ignore_ascii_case(name.trim()))).fold(0, |bits, at| bits | 1 << at)
    })
}

/// A super weapon type: what the sidebar shows and the simulation runs.
pub struct SuperInfo {
    pub name: String,
    pub ui_name: String,
    pub cameo: String,
    pub sim: ra2_sim::supers::SuperType,
}

/// A vehicle type: what the game needs to show it and the simulation to make it.
pub struct UnitInfo {
    /// The type's name in rules.ini, its sidebar picture, and the key of its name.
    pub name: String,
    pub cameo: String,
    /// `AltCameo`: the picture when the type will be built a veteran.
    pub alt_cameo: Option<String>,
    pub ui_name: String,
    /// Model file name, and what it says when selected and when sent somewhere.
    pub image: String,
    pub voice_select: Option<String>,
    pub voice_move: Option<String>,
    /// For a foot soldier: its standing and walking sequences, each as first
    /// frame, frames, and frames per facing.
    pub sequences: Option<HashMap<String, (usize, usize, usize)>>,
    /// `ConsideredAircraft`: its shadow is drawn at half size.
    pub considered_aircraft: bool,
    /// `EnterTransportSound` and `LeaveTransportSound`.
    pub transport_sounds: (Option<String>, Option<String>),
    /// `DeploySound` and `UndeploySound`.
    pub deploy_sounds: (Option<String>, Option<String>),
    /// `UnloadingClass`: the model shown while a miner empties itself.
    pub unloading: Option<String>,
    /// `ChronoOutSound` and `ChronoInSound`, the type's own or the rules'.
    pub chrono_sounds: (Option<String>, Option<String>),
    /// `AuxSound1` and `AuxSound2`: an aircraft's take-off and landing.
    pub aux_sounds: (Option<String>, Option<String>),
    /// `RollAngle` in radians (30 degrees unless given): how far an aircraft
    /// leans into a turn.
    pub roll_angle: f32,
    /// For a vehicle drawn from a sheet of pictures instead of a model.
    pub sheet: Option<Sheet>,
    /// art.ini `ShadowIndex`: the section of the model that casts the shadow;
    /// rules.ini `NoShadow`.
    pub shadow_index: usize,
    pub no_shadow: bool,
    /// rules.ini `Weight`, which damps the rocking a blast gives it.
    pub weight: f64,
    /// `IsChargeTurret` with its `TurretCount`: the turret shown counts down
    /// through the models as the weapon recharges.
    pub charge_turrets: u8,
    pub sim: ra2_sim::UnitType,
}

/// The layout of a sprite vehicle's sheet (the dolphin, squid and terror drone).
// The art.ini keys and their defaults:
// `StandingFrames` 1 when there are firing frames; `StartWalkFrame` 0;
// `StartStandFrame` WalkFrames x Facings (the walk's start with no standing
// frames); `StartFiringFrame` (WalkFrames + StandingFrames) x Facings (the
// standing start with no firing frames). `WalkRate`
// is 1 unless given; `IdleRate` 0.
#[derive(Clone, Copy, Debug)]
pub struct Sheet {
    pub walk_frames: usize,
    pub standing_frames: usize,
    pub firing_frames: usize,
    pub start_walk: usize,
    pub start_stand: usize,
    pub start_firing: usize,
    pub walk_rate: u64,
    pub idle_rate: u64,
}

/// What each cell shows as on the radar: the ground's colour, and that of whatever
/// overlay or terrain object the map starts with on it.
#[derive(bevy::prelude::Resource, Default)]
pub struct RadarGround {
    pub cells: HashMap<(i32, i32), ([u8; 3], Option<[u8; 3]>)>,
}

/// Rules for repairing and selling.
pub struct Upkeep {
    pub refund_percent: f64,
    pub repair_percent: f64,
    pub repair_step: i32,
    pub repair_interval: u32,
    pub survivor_divisors: (i32, i32),
    pub crew_types: (Option<u16>, Option<u16>, Option<u16>, Option<u16>),
}

/// A cell of ore the map starts with.
pub struct OreCell {
    pub cell: (i32, i32),
    /// The overlay art drawn for this cell.
    pub art: String,
    /// Which of `[Tiberiums]` it is.
    pub kind: u8,
    /// 0 to 11; also the frame drawn.
    pub amount: u8,
}

/// Rules for mining, from rules.ini.
pub struct MiningRules {
    pub values: Vec<i32>,
    /// Each kind's `Growth`, `GrowthPercentage`, `Spread`, `SpreadPercentage`.
    pub kinds: Vec<ra2_sim::ore::OreType>,
    pub near_scan: i32,
    pub far_scan: i32,
    pub load_rate: u32,
    pub dump_rate: f64,
    pub too_far: i32,
    pub chrono_too_far: i32,
}

/// A foot soldier the map starts with.
pub struct InfantrySpawn {
    /// Index into the unit types.
    pub kind: u16,
    pub cell: (i32, i32),
    /// The spot in the cell (2, 3 or 4; anything else means "any").
    pub spot: usize,
    pub facing: u8,
    pub palette: Option<usize>,
}

/// A building the map starts with.
pub struct MapStructure {
    /// Index into the building types.
    pub kind: u16,
    pub cell: (i32, i32),
    /// 0 to 256.
    pub health: i32,
    pub palette: Option<usize>,
}

/// A building type: what the game needs to show it, and what the simulation needs
/// to build it.
pub struct BuildingInfo {
    /// The type's name in rules.ini.
    pub name: String,
    /// art.ini `Buildup`: the art of its build-up animation, if it has one.
    pub buildup: Option<String>,
    /// art.ini `BibShape`.
    pub bib: Option<String>,
    /// `ChargedAnimTime`: minutes before its super weapon is ready at which
    /// the building opens up; 990 or more never.
    pub charged_anim_time: f32,
    /// art.ini `DeployingAnim` and `UnderDoorAnim`: a factory's body and the floor
    /// under its door while a vehicle is on its way out.
    pub deploying: Option<String>,
    pub under_door: Option<String>,
    /// art.ini `ZShapePointMove` and `NormalZAdjust`, which place the building
    /// in depth.
    pub zshape_move: (i32, i32),
    pub normal_z_adjust: i32,
    /// A voxel turret (`TurretAnim` with `TurretAnimIsVoxel`): the model's name
    /// and where it sits from the building's draw point, in pixels.
    pub turret: Option<(String, i32, i32)>,
    /// The animations it shows at rest: the active ones, the idle one, and a super
    /// weapon's.
    pub anims: Vec<AnimSlot>,
    /// Art name for the building itself, and the file name of its sidebar picture.
    pub art: String,
    pub cameo: String,
    /// `UIName`: the key of its name in the string table.
    pub ui_name: String,
    /// art.ini `Height`, in levels: how tall its selection box is.
    pub height: i32,
    /// Whether it has a `Factory` entry.
    pub factory: bool,
    /// The vehicle it packs up into (`UndeploysInto`), as it would be spawned.
    pub undeploys: Option<UnitSpawn>,
    pub sim: ra2_sim::BuildingType,
}

pub const NEUTRAL: u8 = 0;
pub const PLAYER: u8 = 1;

/// The house this machine's player sits at: whose sidebar, shroud, selection
/// and announcer this is. The first person's house unless a network game or
/// `RA2_LOCAL` says otherwise.
// Not original behaviour: the engine keeps a pointer to the player's house; a number in one place serves the same end here.
static LOCAL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(PLAYER);

pub fn local() -> u8 {
    LOCAL.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_local(house: u8) {
    LOCAL.store(house, std::sync::atomic::Ordering::Relaxed);
}

/// A building image ready to draw.
pub struct BuildingSprite {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Where the image's top-left corner goes relative to the centre of the
    /// foundation's top cell.
    pub offset: (i32, i32),
}

/// Everything needed to draw vehicles while the game runs.
pub struct UnitArt {
    models: HashMap<String, Option<(Vxl, Option<Hva>)>>,
    /// The animation frame the next model is drawn at.
    pub hva_frame: usize,
    vpl: Option<Vpl>,
    normals: NormalTables,
    base_palette: Palette,
    owner_palettes: Vec<Palette>,
    theater_palette: Palette,
    iso_palette: Palette,
    anim_palette: Option<Palette>,
    extension: &'static str,
    letter: char,
    shapes: HashMap<String, Option<Shp>>,
}

impl UnitArt {
    /// Draws a frame of a building's art, or of its build-up art, lit for a cell at
    /// a height level.
    // Same rules as the buildings the map places; see the notes there.
    pub fn effect(&mut self, files: &GameFiles, art: &str, frame: usize) -> Option<BuildingSprite> {
        let (extension, letter) = (self.extension, self.letter);
        let shp = self
            .shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()?;
        let frame = shp.frames.get(frame).filter(|frame| frame.width > 0 && frame.height > 0)?;
        let palette = self.anim_palette.as_ref().unwrap_or(&self.base_palette);
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: frame_rgba(frame, palette, [1000; 3]),
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// The colour a house's things show in on the radar.
    /// A house's palette, by its place among them.
    pub fn owner_palette(&self, palette: Option<usize>) -> Option<&Palette> {
        palette.and_then(|index| self.owner_palettes.get(index))
    }

    pub fn owner_colour(&self, palette: Option<usize>) -> [u8; 3] {
        palette.and_then(|index| self.owner_palettes.get(index)).map_or([192, 192, 192], |palette| palette.0[21])
    }

    /// Draws a mark on the ground at full light; the sprite takes the
    /// ground's light (`LightRule::Ground`). The level only keys the caches.
    pub fn smudge(&mut self, files: &GameFiles, art: &str, _level: i32) -> Option<BuildingSprite> {
        let (extension, letter) = (self.extension, self.letter);
        let shp = self
            .shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()?;
        let frame = shp.frames.first().filter(|frame| frame.width > 0 && frame.height > 0)?;
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: frame_rgba(frame, &self.iso_palette, [1000; 3]),
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// Draws a frame of a bridge's overlay art, or its shadow, at full light.
    pub fn bridge(&mut self, files: &GameFiles, art: &str, frame: usize, _level: i32, shadow: bool) -> Option<BuildingSprite> {
        let (extension, letter) = (self.extension, self.letter);
        let shp = self
            .shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()?;
        let frame = shp.frames.get(frame).filter(|frame| frame.width > 0 && frame.height > 0)?;
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: if shadow { shadow_rgba(frame) } else { frame_rgba(frame, &self.iso_palette, [1000; 3]) },
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// Draws a frame of ore.
    // At full brightness with the theatre's palette,
    // not lit by its cell.
    pub fn ore(&mut self, files: &GameFiles, art: &str, frame: usize) -> Option<BuildingSprite> {
        let (extension, letter) = (self.extension, self.letter);
        let shp = self
            .shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()?;
        let frame = shp.frames.get(frame).filter(|frame| frame.width > 0 && frame.height > 0)?;
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: frame_rgba(frame, &self.theater_palette, [1000; 3]),
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// Draws a frame of a foot soldier's art at full light; the sprite takes
    /// its cell's light (`LightRule::Object` with `ExtraInfantryLight`).
    pub fn infantry(
        &mut self,
        files: &GameFiles,
        art: &str,
        frame: usize,
        palette: Option<usize>,
        _level: i32,
    ) -> Option<BuildingSprite> {
        let (extension, letter) = (self.extension, self.letter);
        let shp = self
            .shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()?;
        let frame = shp.frames.get(frame).filter(|frame| frame.width > 0 && frame.height > 0)?;
        let palette = palette
            .and_then(|index| self.owner_palettes.get(index))
            .unwrap_or(&self.base_palette);
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: frame_rgba(frame, palette, [1000; 3]),
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// How many frames a building's or animation's art has.
    /// The size in pixels of a shape's whole picture.
    pub fn shape_size(&mut self, files: &GameFiles, art: &str) -> (i32, i32) {
        let (extension, letter) = (self.extension, self.letter);
        self.shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()
            .map_or((0, 0), |shp| (shp.width as i32, shp.height as i32))
    }

    pub fn frame_count(&mut self, files: &GameFiles, art: &str) -> usize {
        let (extension, letter) = (self.extension, self.letter);
        self.shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()
            .map_or(0, |shp| shp.frames.len())
    }

    pub fn building(
        &mut self,
        files: &GameFiles,
        art: &str,
        frame: usize,
        palette: Option<usize>,
        _level: i32,
        shadow: bool,
    ) -> Option<BuildingSprite> {
        let (extension, letter) = (self.extension, self.letter);
        let shp = self
            .shapes
            .entry(art.to_owned())
            .or_insert_with(|| find_shape(files, art, extension, letter))
            .as_ref()?;
        let frame = shp.frames.get(frame).filter(|frame| frame.width > 0 && frame.height > 0)?;
        let palette = palette
            .and_then(|index| self.owner_palettes.get(index))
            .unwrap_or(&self.base_palette);
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: if shadow { shadow_rgba(frame) } else { frame_rgba(frame, palette, [1000; 3]) },
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// Draws a vehicle facing a direction at full light; the sprite takes its
    /// cell's light (`LightRule::Object` with `ExtraUnitLight`).
    pub fn sprite(
        &mut self,
        files: &GameFiles,
        image: &str,
        facing: u8,
        palette: Option<usize>,
        _level: i32,
    ) -> Option<voxel::VoxelSprite> {
        // A name ending "@p<n>" is the model with its nose raised n sixteenths
        // of a right angle (a rocket climbing or diving); "@r<n>" is the model
        // standing on a cell with that ramp; "@b<old>_<thirds>" with it is
        // the model that many thirds of the way there from another ramp's tilt;
        // "@w<roll>_<pitch>_<half width>_<half length>" is the model rocked
        // (angles in thousandths of a radian); "@k<n>" rolled n thousandths of a
        // radian about its length (an aircraft banking); "@q<roll>_<pitch>_<facing>"
        // rocked in hundredths about the axes of a body with that facing.
        let (image, pose) = marks(image);
        let model = self.models.entry(image.to_owned()).or_insert_with(|| {
            let vxl = Vxl::parse(files.get(&format!("{image}.vxl"))?)?;
            let hva = files.get(&format!("{image}.hva")).and_then(Hva::parse);
            Some((vxl, hva))
        });
        let (vxl, hva) = model.as_ref()?;
        let light = [1000; 3];
        let palette = palette
            .and_then(|index| self.owner_palettes.get(index))
            .unwrap_or(&self.base_palette);
        voxel::render(vxl, hva.as_ref(), self.vpl.as_ref(), &self.normals, palette, facing, light, self.hva_frame, pose)
    }

    /// Half the width and half the length of a model's first section, in the
    /// model's units.
    pub fn half_extents(&mut self, files: &GameFiles, image: &str) -> (f32, f32) {
        let model = self.models.entry(image.to_owned()).or_insert_with(|| {
            let vxl = Vxl::parse(files.get(&format!("{image}.vxl"))?)?;
            let hva = files.get(&format!("{image}.hva")).and_then(Hva::parse);
            Some((vxl, hva))
        });
        model.as_ref().and_then(|(vxl, _)| vxl.limbs.first()).map_or((0.0, 0.0), |limb| ((limb.max[1] - limb.min[1]) / 2.0, (limb.max[0] - limb.min[0]) / 2.0))
    }

    /// How many animation frames a model has.
    pub fn hva_frames(&mut self, files: &GameFiles, image: &str) -> usize {
        let image = image.split_once('@').map_or(image, |(base, _)| base);
        let model = self.models.entry(image.to_owned()).or_insert_with(|| {
            let vxl = Vxl::parse(files.get(&format!("{image}.vxl"))?)?;
            let hva = files.get(&format!("{image}.hva")).and_then(Hva::parse);
            Some((vxl, hva))
        });
        model.as_ref().and_then(|(_, hva)| hva.as_ref()).map_or(1, |hva| hva.frames.len().max(1))
    }
}

/// A model's name without its marks (see `UnitArt::sprite`), and the pose
/// they give it.
fn marks(image: &str) -> (&str, voxel::Pose) {
    let (image, marks) = image.split_once('@').unwrap_or((image, ""));
    let (mut pitch, mut roll, mut ramp, mut from, mut rock, mut shake) = (0.0, 0.0, 0u8, None, None, None);
    for mark in marks.split('@') {
        match mark.split_at_checked(1) {
            Some(("p", steps)) => pitch = steps.parse::<i32>().unwrap_or(0) as f32 * std::f32::consts::FRAC_PI_2 / 16.0,
            Some(("k", thousandths)) => roll = thousandths.parse::<i32>().unwrap_or(0) as f32 / 1000.0,
            Some(("r", number)) => ramp = number.parse().unwrap_or(0),
            Some(("b", blend)) => from = blend.split_once('_').and_then(|(old, thirds)| Some((old.parse().ok()?, thirds.parse().ok()?))),
            Some(("q", angles)) => {
                let parts: Vec<i32> = angles.split('_').filter_map(|n| n.parse().ok()).collect();
                if let [roll, rock_pitch, body] = parts[..] {
                    shake = Some(voxel::rock_matrix(roll as f32 / 100.0, rock_pitch as f32 / 100.0, body as u8));
                }
            }
            Some(("w", angles)) => {
                let numbers: Vec<f32> = angles.split('_').filter_map(|n| n.parse().ok()).collect();
                if let [roll, pitch, half_width, half_length] = numbers[..] {
                    rock = Some(voxel::Rock { roll: roll / 1000.0, pitch: pitch / 1000.0, half_width, half_length });
                }
            }
            _ => {}
        }
    }
    let tilt = match (voxel::slope_matrix(ramp, from), shake) {
        (Some(slope), Some(shake)) => Some(std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| slope[i][k] * shake[k][j]).sum()))),
        (slope, shake) => slope.or(shake),
    };
    (image, voxel::Pose { pitch, roll, tilt, rock })
}

impl UnitArt {
    /// The shadow a vehicle casts from a section of its model, on ground with a
    /// ramp.
    pub fn voxel_shadow(&mut self, files: &GameFiles, image: &str, facing: u8, half: bool, section: usize, ramp: u8) -> Option<voxel::VoxelSprite> {
        let model = self.models.entry(image.to_owned()).or_insert_with(|| {
            let vxl = Vxl::parse(files.get(&format!("{image}.vxl"))?)?;
            let hva = files.get(&format!("{image}.hva")).and_then(Hva::parse);
            Some((vxl, hva))
        });
        let (vxl, hva) = model.as_ref()?;
        voxel::shadow(vxl, hva.as_ref(), facing, half, section, voxel::slope_matrix(ramp, None))
    }

    /// The shadow of a soldier in the air: frame 1 of INFSHDW.SHP.
    pub fn soldier_shadow(&mut self, files: &GameFiles) -> Option<BuildingSprite> {
        let shp = self.shapes.entry("infshdw".to_owned()).or_insert_with(|| files.get("infshdw.shp").and_then(Shp::parse)).as_ref()?;
        let frame = shp.frames.get(1).filter(|frame| frame.width > 0 && frame.height > 0)?;
        Some(BuildingSprite {
            width: frame.width as u32,
            height: frame.height as u32,
            rgba: shadow_rgba(frame),
            offset: (frame.x as i32 - shp.width as i32 / 2, frame.y as i32 - shp.height as i32 / 2),
        })
    }

    /// A building's voxel turret, with its barrel if it has one, as one picture.
    // A name with TUR in it is the turret,
    // and the same name with BARL the barrel; without, the one model is all.
    pub fn turret(&mut self, files: &GameFiles, name: &str, facing: u8, palette: Option<usize>, level: i32, recoil: f32) -> Option<BuildingSprite> {
        let back = voxel::to_view([-recoil, 0.0, 0.0], facing);
        let parts: Vec<voxel::VoxelSprite> = [name.replace("tur", "barl"), name.to_owned()]
            .into_iter()
            .enumerate()
            .filter(|(index, part)| *index == 1 || part != name)
            .filter_map(|(index, part)| {
                let mut sprite = self.sprite(files, &part, facing, palette, level)?;
                if index == 0 {
                    sprite.origin.0 -= back[0].round() as i32;
                    sprite.origin.1 += back[1].round() as i32;
                }
                Some(sprite)
            })
            .collect();
        let min = parts.iter().fold((i32::MAX, i32::MAX), |a, p| (a.0.min(-p.origin.0), a.1.min(-p.origin.1)));
        let max = parts.iter().fold((i32::MIN, i32::MIN), |a, p| (a.0.max(p.width as i32 - p.origin.0), a.1.max(p.height as i32 - p.origin.1)));
        if parts.is_empty() {
            return None;
        }
        let (width, height) = ((max.0 - min.0) as usize, (max.1 - min.1) as usize);
        let mut rgba = vec![0u8; width * height * 4];
        for part in &parts {
            for y in 0..part.height as usize {
                for x in 0..part.width as usize {
                    let from = (y * part.width as usize + x) * 4;
                    if part.rgba[from + 3] != 0 {
                        let to = ((y as i32 - part.origin.1 - min.1) as usize * width + (x as i32 - part.origin.0 - min.0) as usize) * 4;
                        rgba[to..to + 4].copy_from_slice(&part.rgba[from..from + 4]);
                    }
                }
            }
        }
        Some(BuildingSprite { width: width as u32, height: height as u32, rgba, offset: min })
    }

    /// A vehicle with its turret and barrel, if its type has them, as one picture.
    // The turret is its own model (the body's name with TUR, the barrel's with
    // BARL), turned to the turret's facing about a point `TurretOffset` along the
    // body.
    pub fn vehicle(
        &mut self,
        files: &GameFiles,
        image: &str,
        facing: u8,
        turret: Option<(u8, i32, u8)>,
        palette: Option<usize>,
        level: i32,
    ) -> Option<voxel::VoxelSprite> {
        let body = self.sprite(files, image, facing, palette, level)?;
        let Some((turret_facing, offset, which)) = turret else {
            return Some(body);
        };
        // The turret's place is `TurretOffset` along the body, tipped with it
        // on a slope.
        const MODEL_PER_LEPTON: f32 = 30.0 / 256.0 / std::f32::consts::FRAC_1_SQRT_2;
        let place = marks(image).1.place([offset as f32 * MODEL_PER_LEPTON, 0.0, 0.0], facing);
        let shift = (place[0].round() as i32, (-place[1]).round() as i32);
        let parts: Vec<(voxel::VoxelSprite, (i32, i32))> = [Some((body, (0, 0)))]
            .into_iter()
            // A type with several turrets names the
            // first plainly and the rest by number.
            .chain(["barl", "tur"].map(|part| {
                // Marks after an "@" (see `sprite`) stay at the end of the name.
                let (base, marks) = image.split_once('@').map_or((image, String::new()), |(base, marks)| (base, format!("@{marks}")));
                let name = if which == 0 { format!("{base}{part}{marks}") } else { format!("{base}{part}{which}{marks}") };
                self.sprite(files, &name, turret_facing, palette, level).map(|s| (s, shift))
            }))
            .flatten()
            .collect();
        // The corner of each part relative to the body's origin.
        let corner = |(sprite, shift): &(voxel::VoxelSprite, (i32, i32))| (shift.0 - sprite.origin.0, shift.1 - sprite.origin.1);
        let min = parts.iter().map(corner).fold((i32::MAX, i32::MAX), |a, c| (a.0.min(c.0), a.1.min(c.1)));
        let max = parts
            .iter()
            .map(|part| {
                let c = corner(part);
                (c.0 + part.0.width as i32, c.1 + part.0.height as i32)
            })
            .fold((i32::MIN, i32::MIN), |a, c| (a.0.max(c.0), a.1.max(c.1)));
        let (width, height) = ((max.0 - min.0) as usize, (max.1 - min.1) as usize);
        let mut rgba = vec![0u8; width * height * 4];
        for part in &parts {
            let c = corner(part);
            for y in 0..part.0.height as usize {
                for x in 0..part.0.width as usize {
                    let from = (y * part.0.width as usize + x) * 4;
                    if part.0.rgba[from + 3] != 0 {
                        let to = ((y as i32 + c.1 - min.1) as usize * width + (x as i32 + c.0 - min.0) as usize) * 4;
                        rgba[to..to + 4].copy_from_slice(&part.0.rgba[from..from + 4]);
                    }
                }
            }
        }
        Some(voxel::VoxelSprite { width: width as u32, height: height as u32, rgba, origin: (-min.0, -min.1) })
    }
}

/// A computer player as the scene sets it up.
pub struct Enemy {
    pub start: (i32, i32),
    pub country: u8,
    pub side: i32,
    pub palette: Option<usize>,
    /// 0 brutal, 1 medium, 2 easy.
    pub level: usize,
}

pub struct Scene {
    /// What lights the map while the game runs.
    pub light_setup: crate::lighting::LightSetup,
    /// The kinds of mark explosions leave, by art name.
    pub smudge_types: Vec<(String, ra2_sim::smudge::SmudgeType)>,
    /// Animations on tiles (waterfalls): the animation, where its middle is in
    /// scene pixels, and its cell.
    pub tile_anims: Vec<(String, (i32, i32), (i32, i32))>,
    /// The marks the map starts with: cell and type.
    pub map_smudges: Vec<((i32, i32), u16)>,
    /// The super weapon types, with each one's name, sidebar picture and the
    /// number of its `Action`, and the paradrops' rules.
    pub super_types: Vec<SuperInfo>,
    pub super_rules: ra2_sim::supers::Rules,
    /// `WeatherConBoltExplosion`, the storm's explosion.
    pub bolt_explosion: Option<String>,
    pub crate_rules: ra2_sim::powerups::Rules,
    /// The computer player's triggers, teams and scripts.
    pub team_data: ra2_sim::teams::Data,
    /// The numbers of experience.
    pub veterancy: ra2_sim::abilities::Veterancy,
    /// The rockets' flight rules.
    pub rockets: [ra2_sim::spawns::RocketRules; 2],
    pub rad_rules: ra2_sim::radiation::Rules,
    /// The rules of the special weapons.
    pub ability_rules: ra2_sim::abilities::Rules,
    /// The lists a computer house plans its base from.
    pub ai_rules: ra2_sim::ai::Rules,
    /// The computer players, as many as the map has start positions for; the
    /// first is house 2.
    pub enemies: Vec<Enemy>,
    /// Each house's palette in `UnitArt`, by owner.
    pub house_palettes: Vec<Option<usize>>,
    /// The player's start cell and each house's base unit type, by owner.
    pub base_units: Vec<Option<u16>>,
    pub building_types: Vec<BuildingInfo>,
    pub unit_types: Vec<UnitInfo>,
    pub infantry_spawns: Vec<InfantrySpawn>,
    pub ore: Vec<OreCell>,
    pub mining: MiningRules,
    pub radar_ground: RadarGround,
    pub armoury: crate::armoury::Armoury,
    pub upkeep: Upkeep,
    /// `[General] CrewEscape` and `ShipSinkingWeight`.
    pub crew_escape: f64,
    pub ship_sinking_weight: f64,
    pub chrono: (i32, i32, i32, bool),
    /// Gravity, the most damage one hit may do, how much a missile's turning
    /// varies, and the frames between looks for a target.
    pub combat_rules: (i32, i32, f64, u32),
    pub elevation: (i32, f64, f64),
    pub ballistic_scatter: i32,
    /// `[General] BuildSpeed` in thousandths of a minute per 1000 credits.
    pub build_rules: ra2_sim::BuildRules,
    /// Index of the player's palette in `UnitArt`.
    pub player_palette: Option<usize>,
    /// The cell the player starts at.
    pub player_start: Option<(i32, i32)>,
    pub land_rules: ra2_sim::LandRules,
    /// Whether construction yards may pack up again.
    pub mcv_repacks: bool,
    /// `AlliesAllowed`, which has the computer houses ally with each other.
    pub allies_allowed: bool,
    /// Ticks a build-up animation is spread over.
    pub buildup_ticks: u32,
    /// The highest `TechLevel` players may build.
    pub tech_level: i32,
    /// Animation types by name, the buildings the map starts with, and the health
    /// (as a fraction) at or below which things show damage.
    pub anim_types: HashMap<String, AnimType>,
    /// Wall pieces the map starts with (cell, overlay type, data), and the art of
    /// each wall overlay type.
    pub walls: Vec<((i32, i32), u16, u8)>,
    pub wall_art: HashMap<u16, String>,
    /// The art of each bridge overlay type.
    pub bridge_art: HashMap<u8, String>,
    /// `[MultiplayerDialogSettings] BridgeDestruction` and
    /// `[CombatDamage] BridgeStrength`.
    pub bridge_rules: (bool, i32),
    pub map_structures: Vec<MapStructure>,
    pub condition_yellow: f32,
    pub condition_red: f32,
    /// `[General] IdleActionFrequency`.
    pub idle_frequency: f64,
    /// `[General] TrackedUphill`, `TrackedDownhill`, `WheeledUphill`, `WheeledDownhill`.
    pub slope_speed: [f64; 4],
    /// The player's country (its place in `[Countries]`) and side (0 Allied, 1
    /// Soviet).
    pub player_country: u8,
    pub player_side: i32,
    /// The map's width and visible rectangle, which bound the playable area.
    pub playfield: Option<(i32, i32, [i32; 4])>,
    pub blockage_delay: i32,
    pub sight_height_step: i32,
    pub cells: Vec<SceneCell>,
    pub unit_spawns: Vec<UnitSpawn>,
    pub unit_art: UnitArt,
    pub width: u32,
    pub height: u32,
    /// Scene pixel of cell (0, 0)'s corner at height 0; add [`cell_origin`] to it.
    pub origin: (i32, i32),
    pub pages: Vec<AtlasPage>,
    pub entries: Vec<AtlasEntry>,
    pub placements: Vec<Placement>,
    pub animations: Vec<Animation>,
    /// Cells whose tile art could not be found.
    pub missing_tiles: usize,
    pub ore_cells: usize,
    pub terrain_objects: usize,
    pub structures: usize,
    pub infantry: usize,
    pub units: usize,
    /// Ore cells, overlays and terrain objects that could not be drawn.
    pub missing_objects: usize,
    /// Names of the art (with the frame, where that was the problem) behind them.
    pub missing_art: Vec<String>,
}

/// Top-left pixel of a cell's diamond bounding box, before the scene origin is applied.
pub fn cell_origin(x: i32, y: i32, level: i32) -> (i32, i32) {
    (
        (x - y) * CELL_WIDTH / 2,
        (x + y) * CELL_HEIGHT / 2 - level * LEVEL_HEIGHT,
    )
}

/// The art facing for each of 32 compass directions, clockwise from north.
pub const INFANTRY_FACINGS: [usize; 32] = [
    7, 7, 6, 6, 6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 7, 7,
];

/// Finds the sprite for an art name.
fn find_shape(files: &GameFiles, name: &str, extension: &str, letter: char) -> Option<Shp> {
    let with_letter = |letter: char| -> String {
        name.chars().enumerate().map(|(i, c)| if i == 1 { letter } else { c }).collect()
    };
    let data = files
        .get(&format!("{name}.{extension}"))
        .or_else(|| files.get(&format!("{}.shp", with_letter(letter))))
        .or_else(|| files.get(&format!("{}.shp", with_letter('g'))))
        .or_else(|| files.get(&format!("{name}.shp")))?;
    Shp::parse(data)
}

/// The land of a tile by its terrain type.
fn tile_land(terrain_type: u8) -> ra2_sim::Land {
    use ra2_sim::Land;
    match terrain_type {
        1..=4 => Land::Ice,
        5 => Land::Tunnel,
        6 => Land::Railroad,
        7 | 8 | 15 => Land::Rock,
        9 => Land::Water,
        10 => Land::Beach,
        11 | 12 => Land::Road,
        14 => Land::Rough,
        _ => Land::Clear,
    }
}

/// Rules that decide how overlays and terrain objects are drawn.
/// How to draw an overlay that is not ore.
struct OverlayArt {
    image: String,
    palette: PaletteKind,
    shadow_shift: (i32, i32),
    frame: usize,
    lift: i32,
    light_levels: i32,
    /// `DrawFlat` (default yes) and `IsARock`.
    flat: bool,
    rock: bool,
}

// DATA: positions of these types in rules.ini `[OverlayTypes]`.
const HIGH_BRIDGES: [usize; 2] = [24, 25];
const VEINS: usize = 126;
const NEVER_DRAWN: [usize; 2] = [167, 178];
const BRIDGE_B2: usize = 239;

struct ObjectRules {
    /// Names of every overlay type, indexed by overlay type.
    overlay_names: Vec<String>,
    /// Names of the overlay types that are ore or gems, indexed by overlay type.
    ore_overlays: Vec<Option<String>>,
    rules: Option<Ini>,
}

impl ObjectRules {
    fn load(files: &GameFiles, map: &Ini, mode: &str) -> Self {
        // A map's own file is read over the rules, which is how maps without
        // enough water take the shipyards away (`[GAYARD] TechLevel=-1`) and
        // others change them. Sections that are lists of numbered lines in both
        // (none in the stock maps) would need the engine's rule.
        let mut rules = files.get("rules.ini").map(Ini::parse);
        // DATA: MPModes.ini names a rules file for each game mode (MPBattle.ini,
        // MPFreeForAll.ini), laid over the rules.
        if let (Some(rules), Some(mode)) = (rules.as_mut(), files.get(&mode.to_lowercase())) {
            rules.overlay(&Ini::parse(mode));
        }
        if let Some(rules) = rules.as_mut() {
            let mut types = Ini::parse(b"");
            for name in map.sections() {
                if rules.sections().any(|section| section.eq_ignore_ascii_case(name)) && !name.ends_with("Types") {
                    let text: String = std::iter::once(format!("[{name}]\n"))
                        .chain(map.section(name).iter().map(|(k, v)| format!("{k}={v}\n")))
                        .collect();
                    types.overlay(&Ini::parse(text.as_bytes()));
                }
            }
            rules.overlay(&types);
        }
        // DATA: the overlay type index is the position in `[OverlayTypes]`, not the
        // key (ore is 102 to 121 and gems 27 to 38, as the maps expect).
        let ore_overlays = rules
            .iter()
            .flat_map(|rules| rules.section("OverlayTypes").iter().map(move |e| (rules, e)))
            .map(|(rules, (_, name))| {
                let is_ore = rules
                    .get(name, "Tiberium")
                    .is_some_and(|value| value.eq_ignore_ascii_case("yes"));
                // DATA: an ore overlay's `Image` names the art it is drawn with:
                // the third kind (TIB2_01 to TIB2_20, `Vinifera`) is drawn with the
                // ordinary ore's TIB01 to TIB20. (Its own art is a few specks.)
                let image = rules.get(name, "Image").map(|image| image.split(';').next().unwrap_or("").trim().to_owned()).filter(|image| !image.is_empty());
                is_ore.then(|| image.unwrap_or_else(|| name.clone()).to_lowercase())
            })
            .collect();
        let overlay_names = rules
            .iter()
            .flat_map(|rules| rules.section("OverlayTypes"))
            .map(|(_, name)| name.clone())
            .collect();
        Self {
            overlay_names,
            ore_overlays,
            rules,
        }
    }

    /// Whether an overlay type is a wall (`Wall=yes`): pieces that join up.
    fn is_wall(&self, overlay: usize) -> bool {
        self.overlay_names.get(overlay).is_some_and(|name| {
            self.rules.as_ref().and_then(|rules| rules.get(name, "Wall")).is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"))
        })
    }

    /// How to draw an overlay that is not ore or gems, or `None` if it is not drawn.
    // Each rule below follows the original unless it says otherwise.
    fn overlay(&self, overlay: usize, state: u8, cell: (i32, i32)) -> Option<OverlayArt> {
        if NEVER_DRAWN.contains(&overlay) || overlay == VEINS {
            return None;
        }
        let name = self.overlay_names.get(overlay)?;
        let rules = self.rules.as_ref()?;
        let flag = |key: &str| {
            rules
                .get(name, key)
                .is_some_and(|value| value.eq_ignore_ascii_case("yes"))
        };
        if flag("Crate") {
            return None;
        }
        // DATA: `Image=` names the art when it differs from the type name.
        let image = rules.get(name, "Image").unwrap_or(name).to_lowercase();
        let truth = |key: &str, default: bool| {
            rules.get(name, key).map_or(default, |value| matches!(value.trim().chars().next(), Some('y' | 'Y' | 't' | 'T' | '1')))
        };
        let mut art = OverlayArt {
            flat: truth("DrawFlat", true),
            rock: truth("IsARock", false),
            image,
            // The cell's lit palette. Low bridges need no special case. And DATA (stock maps): a low bridge
            // is three adjacent cells across, with states 0, 1 and 2, and the art's only image is frame 1,
            // so the middle cell draws the whole piece.

            palette: PaletteKind::Iso,
            shadow_shift: (0, 0),
            // The overlay state is the frame.
            frame: state as usize,
            lift: 0,
            light_levels: 0,
        };
        if HIGH_BRIDGES.contains(&overlay) {
            // Frames 0 and 9 are the undamaged deck in each direction; they get one
            // of four looks from the same pattern the tiles use.
            if state == 0 || state == 9 {
                art.frame += VARIANT_PATTERN[(((cell.1 & 3) << 2) | (cell.0 & 3)) as usize];
            }
            // Raised 16 px, and 15 more for the second direction's frames; lit as
            // if four levels above the cell.
            art.lift = if (9..=17).contains(&state) { 31 } else { 16 };
            // Those frames' shadows are moved.
            if (9..=17).contains(&state) {
                art.shadow_shift = (-15, 7);
            }
            art.light_levels = 4;
        } else if overlay == BRIDGE_B2 {
            art.lift = 15;
        } else if flag("Wall") || flag("Gate") {
            // Walls share ore's 12 px offset.
            art.lift = 12;
            // Walls are not drawn with the cell's lit palette. The palette comes
            // from an object picked by an index held in the rules data, and the
            // brightness is the cell's.
            art.palette = PaletteKind::Unit;
        }
        Some(art)
    }

    /// The art for an ore or gem overlay at a cell, or `None` if the overlay type is
    /// not ore.
    // The image is not the overlay type stored in the map. It is picked from the
    // ore kind's run of overlay types by `x * y` modulo the number of flat images,
    // or on a slope from the images for that slope. Drawn with a global palette
    // created right after `TEMPERAT.PAL` is loaded, taken to be the plain theatre
    // palette. The engine reads the counts from the ore kind at run time.
    fn ore_image(&self, overlay: usize, cell: (i32, i32), ramp: u8) -> Option<&str> {
        const FLAT_IMAGES: usize = 12;
        let prefix = |name: &str| name.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned();
        let family = prefix(self.ore_overlays.get(overlay)?.as_deref()?);
        let in_family = |index: usize| {
            self.ore_overlays
                .get(index)
                .and_then(|name| name.as_deref())
                .is_some_and(|name| prefix(name) == family)
        };
        let first = (0..=overlay).rev().take_while(|&i| in_family(i)).last()?;
        let count = (first..).take_while(|&i| in_family(i)).count();
        let flat = count.min(FLAT_IMAGES);
        let per_slope = (count - flat) / 4;
        let spot = (cell.0 * cell.1) as usize;
        let index = if (1..=4).contains(&ramp) && per_slope > 0 {
            first + flat + (ramp as usize - 1) * per_slope + spot % per_slope
        } else {
            first + spot % flat
        };
        self.ore_overlays.get(index)?.as_deref()
    }

    // Terrain objects are drawn with their cell's own lit palette, except those
    // with `SpawnsTiberium` (ore drills), which use a global palette created
    // alongside the unit palette.
    fn terrain_object_palette(&self, name: &str) -> PaletteKind {
        let spawns_ore = self
            .rules
            .as_ref()
            .and_then(|rules| rules.get(name, "SpawnsTiberium"))
            .is_some_and(|value| value.eq_ignore_ascii_case("yes"));
        if spawns_ore { PaletteKind::Unit } else { PaletteKind::Iso }
    }
}

/// An SHP frame as lit RGBA, cropped to the frame's own rectangle.
fn frame_rgba(frame: &ra2_formats::shp::Frame, palette: &Palette, light: Light) -> Vec<u8> {
    frame
        .pixels
        .iter()
        .flat_map(|&index| match index {
            0 => [0; 4],
            _ => lit(palette.0[index as usize], light),
        })
        .collect()
}

/// A frame's silhouette as translucent black.
fn shadow_rgba(frame: &ra2_formats::shp::Frame) -> Vec<u8> {
    frame
        .pixels
        .iter()
        .flat_map(|&index| if index == 0 { [0; 4] } else { [0, 0, 0, 128] })
        .collect()
}

#[derive(PartialEq, Eq, Hash)]
enum ArtKey {
    /// Tile index, alternate, cell within the template, light.
    Tile(i32, usize, u8, Light),
    TileExtra(i32, usize, u8, Light),
    Frame(String, usize, PaletteKind, Option<usize>, Light),
    Shadow(String, usize),
}

/// Who the player is: country as its place in `[Countries]`, and colour (0 to 7);
/// and the computer players against them.
#[derive(Clone, Debug)]
pub struct Player {
    pub country: usize,
    pub colour: usize,
    pub opponents: Vec<Opponent>,
    /// The game mode's rules file, laid over the rules.
    pub mode_rules: String,
    /// The number drawn for the first player's start position, taken within
    /// those the map has; none (the tests) is the first.
    pub start: Option<usize>,
}

/// A computer player as chosen on the skirmish screen.
#[derive(Clone, Copy, Debug)]
pub struct Opponent {
    /// 0 brutal, 1 medium, 2 easy.
    pub level: usize,
    /// Left out by the tests, which then get a country from the map's size and
    /// the first colour not taken.
    pub country: Option<usize>,
    pub colour: Option<usize>,
}

impl Default for Player {
    fn default() -> Self {
        Self { country: 0, colour: 0, opponents: vec![Opponent { level: 2, country: None, colour: None }], mode_rules: String::new(), start: None }
    }
}

/// The `[Colors]` entry behind each colour offered to players.
// Gold, red, blue, green, orange, sky blue,
// purple, pink map to these schemes.
/// The computer player's house.
pub const ENEMY: u8 = 2;

pub const PLAYER_COLOURS: [&str; 8] =
    ["Gold", "DarkRed", "DarkBlue", "DarkGreen", "Orange", "DarkSky", "Purple", "Magenta"];

pub fn build(files: &GameFiles, map: &Map, player: &Player) -> Option<Scene> {
    let theater = Theater::load(files, &map.theater)?;
    let object_rules = ObjectRules::load(files, &map.ini, &player.mode_rules);
    let mut light_setup = crate::lighting::LightSetup::read(&map.ini, object_rules.rules.as_ref());

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for tile in &map.tiles {
        let (x, y) = cell_origin(tile.x as i32, tile.y as i32, tile.level as i32);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + CELL_WIDTH);
        max_y = max_y.max(y + CELL_HEIGHT);
    }
    if min_x > max_x {
        return None;
    }
    min_y -= TOP_MARGIN;

    let mut atlas = Atlas::default();
    let mut art: HashMap<ArtKey, usize> = HashMap::new();
    let mut placements = Vec::new();

    let mut ramp_types: Vec<((i32, i32), u8)> = Vec::new();
    let mut tile_anims: Vec<(String, (i32, i32), (i32, i32))> = Vec::new();
    let mut ground: Vec<((i32, i32), u8, u8)> = Vec::new();
    // Ore may come to lie on the clear tile and on
    // tiles of sets with `AllowTiberium`.
    let mut ore_ground: HashSet<(i32, i32)> = HashSet::new();
    let mut radar_tiles: HashMap<(i32, i32), [u8; 3]> = HashMap::new();
    // Alternates of a tile are separate files: the base name, then the same name
    // with `a`, `b`, ... appended.
    let tile_file = |index: i32, variant: usize| -> Option<Tmp> {
        let stem = theater.tile_names.get(index as usize)?;
        let letter = match variant {
            0 => String::new(),
            n => char::from(b'a' + n as u8 - 1).to_string(),
        };
        Tmp::parse(files.get(&format!("{stem}{letter}.{}", theater.extension))?)
    };
    let large_variants = large_variant_table();
    let mut variant_counts: HashMap<i32, usize> = HashMap::new();
    let mut tiles: HashMap<(i32, usize), Option<Tmp>> = HashMap::new();
    let mut missing_tiles = 0;
    for tile in &map.tiles {
        // DATA: both -1 and mean "clear ground", which is tile 0.
        let index = if tile.tile < 0 || tile.tile == 0xFFFF { 0 } else { tile.tile };
        let base = tiles.entry((index, 0)).or_insert_with(|| tile_file(index, 0));
        let count = *variant_counts.entry(index).or_insert_with(|| {
            (1..26).find(|&n| tile_file(index, n).is_none()).unwrap_or(1)
        });

        // Which alternate a cell shows. Cells
        // with a damaged image never use alternates. Larger templates are indexed
        // by the template's position, so all their cells agree.
        // A value beyond the number of alternates wraps
        // around (modulo), and alternates are a chain of tile types.
        let mut variant = 0;
        let sub = tile.sub_tile as i32;
        if let (true, Some(tmp)) = (count >= 2, base.as_ref()) {
            let damaged = tmp
                .cells
                .get(sub as usize)
                .and_then(|cell| cell.as_ref())
                .is_some_and(|cell| cell.has_damaged_data);
            if !damaged {
                let (mut x, mut y) = (tile.x as i32, tile.y as i32);
                if sub != 0 {
                    let (bx, by) = (tmp.blocks_x.max(1) as i32, tmp.blocks_y.max(1) as i32);
                    x = (x - sub % bx) / bx;
                    y = (y - sub / bx) / by;
                }
                let pick = if count > 4 {
                    large_variants[((x & 7) + (y & 7) * 8) as usize]
                } else {
                    VARIANT_PATTERN[(((y & 3) << 2) | (x & 3)) as usize]
                };
                variant = pick % count;
            }
        }
        let tmp = tiles
            .entry((index, variant))
            .or_insert_with(|| tile_file(index, variant));
        let cell = tmp
            .as_ref()
            .and_then(|tmp| tmp.cells.get(tile.sub_tile as usize)?.as_ref());
        let Some(cell) = cell else {
            missing_tiles += 1;
            continue;
        };

        let level = tile.level as i32;
        let cell_xy = (tile.x as i32, tile.y as i32);
        ramp_types.push((cell_xy, cell.ramp_type));
        ground.push((cell_xy, cell.terrain_type, tile.level));
        if index == 0 || theater.tile_tiberium.get(index as usize).copied().unwrap_or(false) {
            ore_ground.insert(cell_xy);
        }
        radar_tiles.insert(cell_xy, cell.radar);
        let (x, y) = cell_origin(cell_xy.0, cell_xy.1, level);
        let (x, y) = (x - min_x, y - min_y);
        // Tiles are drawn with the cell's own lit palette and its
        // brightness.
        let colors = &theater.iso_palette.0;

        let light = [1000; 3];
        let entry = *art
            .entry(ArtKey::Tile(index, variant, tile.sub_tile, light))
            .or_insert_with(|| {
                let mut rgba = vec![0u8; (CELL_WIDTH * CELL_HEIGHT * 4) as usize];
                // DATA: a tile without depth data is taken as flat (29 less the row).
                let mut z = vec![0u8; (CELL_WIDTH * CELL_HEIGHT) as usize];
                for row in 0..CELL_HEIGHT as u32 {
                    let (start, length) = diamond_row(row, CELL_WIDTH as u32, CELL_HEIGHT as u32);
                    for col in start..start + length {
                        let at = (row * CELL_WIDTH as u32 + col) as usize;
                        rgba[at * 4..at * 4 + 4]
                            .copy_from_slice(&lit(colors[cell.pixels[at] as usize], light));
                        z[at] = cell.z.as_ref().map_or((29 - row as i32).max(0) as u8, |z| z[at]);
                    }
                }
                atlas.add_with_depth(CELL_WIDTH as u32, CELL_HEIGHT as u32, &rgba, &z)
            });
        let base = Some(-(y + 15 * level) - 30);
        placements.push(Placement {
            entry,
            x,
            y,
            layer: Layer::Terrain,
            cell: cell_xy,
            kind: PlacementKind::Tile,
            depth: None,
            ground: base,
            light: Some(crate::lighting::LightRule::Ground { raise: 0 }),
            light_cell: cell_xy,
        });
        // A tile's animation is started on the cell of its template it
        // attaches to, at the cell's middle on the ground moved by its offset.
        if let Some((anim, offset, attaches)) = theater.tile_anims.get(&(index as usize)) {
            if tile.sub_tile == *attaches {
                tile_anims.push((anim.clone(), (x + CELL_WIDTH / 2 + offset.0, y + CELL_HEIGHT / 2 + offset.1), cell_xy));
            }
        }

        if let Some(extra) = &cell.extra {
            // The tile is drawn in one call with one
            // brightness, so the raised part is lit like the rest of the cell.
            let entry = *art
                .entry(ArtKey::TileExtra(index, variant, tile.sub_tile, light))
                .or_insert_with(|| {
                    let rgba: Vec<u8> = extra
                        .pixels
                        .iter()
                        .flat_map(|&c| if c == 0 { [0; 4] } else { lit(colors[c as usize], light) })
                        .collect();
                    let z: Vec<u8> = match &extra.z {
                        Some(z) => z.clone(),
                        None => (0..extra.height as i32)
                            .flat_map(|row| std::iter::repeat_n((29 - extra.y - row).clamp(0, 255) as u8, extra.width as usize))
                            .collect(),
                    };
                    atlas.add_with_depth(extra.width, extra.height, &rgba, &z)
                });
            placements.push(Placement {
                entry,
                x: x + extra.x,
                y: y + extra.y,
                layer: Layer::Terrain,
                cell: cell_xy,
                kind: PlacementKind::TileExtra,
                depth: None,
                ground: base,
                light: Some(crate::lighting::LightRule::Ground { raise: 0 }),
                light_cell: cell_xy,
            });
        }
    }

    // Ore and terrain objects. Their art is centred on the cell. Terrain objects
    // are lit with their cell's brightness and drawn in one call per frame. (Ore
    // that grows and spreads is drawn by `units::show_ore`.)
    struct Object {
        cell: (i32, i32),
        art: String,
        frame: usize,
        palette: PaletteKind,
        kind: PlacementKind,
        /// Pixels the art is raised above the cell's centre point.
        lift: i32,
        /// Height levels added to the cell's when working out brightness.
        light_levels: i32,
        /// Where the shadow is drawn relative to the art.
        shadow_shift: (i32, i32),
        /// The cell whose row decides draw order; the nearest cell of a building's
        /// foundation, otherwise the object's own cell.
        sort_cell: (i32, i32),
        /// Pixels the art is moved from the cell's centre point.
        nudge: (i32, i32),
        /// The owner's remapped palette, for things a house owns.
        remap: Option<usize>,
        /// `AnimationRate` and `AnimationProbability`, for `IsAnimated` objects.
        animation: Option<(u32, f32)>,
        /// Whether it lies flat, its depth adjustment above the height's, and
        /// whether it writes depth.
        stand: (bool, i32, bool),
    }
    let ramps: HashMap<(i32, i32), u8> = ramp_types.into_iter().collect();
    let levels: HashMap<(i32, i32), i32> = map
        .tiles
        .iter()
        .map(|t| ((t.x as i32, t.y as i32), t.level as i32))
        .collect();
    let mut objects = Vec::new();
    let mut ore_cells = 0;
    let mut walls: Vec<((i32, i32), u16, u8)> = Vec::new();
    let mut ore: Vec<OreCell> = Vec::new();
    for &cell in levels.keys() {
        let Some((overlay, state)) = map.overlay_at(cell.0, cell.1) else {
            continue;
        };
        let ramp = ramps.get(&cell).copied().unwrap_or(0);
        if let Some(name) = object_rules.ore_image(overlay as usize, cell, ramp) {
            // Ore is dug up as the game goes on, so the simulation holds it and the
            // game draws it; see `units::show_ore`.
            // The kind of ore by where the overlay
            // type falls: 102 on ore, 27 on gems, 127 and 147 on the two unused.
            let kind = match overlay {
                27..=38 => 1,
                127..=146 => 2,
                147..=166 => 3,
                _ => 0,
            };
            ore.push(OreCell { cell, art: name.to_owned(), kind, amount: state.min(11) });
            ore_cells += 1;
        } else if object_rules.is_wall(overlay as usize) {
            // Walls change as the game goes on, so the simulation holds them and
            // they are drawn from it; see `units::show_walls`.
            walls.push((cell, overlay as u16, state));
        } else if matches!(overlay, 24 | 25 | 237 | 238 | 74..=101 | 205..=232) {
            // Bridges change as the game goes on, so the simulation holds them
            // and they are drawn from it; see `units::show_bridges`.
        } else if let Some(overlay) = object_rules.overlay(overlay as usize, state, cell) {
            objects.push(Object {
                cell,
                art: overlay.image,
                frame: overlay.frame,
                palette: overlay.palette,
                kind: PlacementKind::Overlay,
                lift: overlay.lift,
                light_levels: overlay.light_levels,
                shadow_shift: overlay.shadow_shift,
                sort_cell: cell,
                nudge: (0, 0),
                remap: None,
                animation: None,
                // A flat overlay is moved 2 nearer than the
                // ground, an upright one 17, a rock 2.
                stand: (overlay.flat, if overlay.flat || overlay.rock { -2 } else { -17 }, true),
            });
        }
    }
    for object in &map.terrain_objects {
        let palette = object_rules.terrain_object_palette(&object.name);
        objects.push(Object {
            cell: (object.x, object.y),
            // Ore drills are drawn 16 px higher.
            lift: if palette == PaletteKind::Unit { 16 } else { 0 },
            light_levels: 0,
            shadow_shift: (0, 0),
            sort_cell: (object.x, object.y),
            nudge: (0, 0),
            remap: None,
            // DATA: `IsAnimated`, `AnimationRate`, `AnimationProbability`.
            animation: object_rules.rules.as_ref().and_then(|rules| {
                let yes = rules.get(&object.name, "IsAnimated")?.trim().eq_ignore_ascii_case("yes");
                let rate = rules.get(&object.name, "AnimationRate")?.trim().parse().ok()?;
                let probability = rules.get(&object.name, "AnimationProbability")?.trim().parse().ok()?;
                yes.then_some((rate, probability))
            }),
            palette,
            // Upright, 12 nearer; an animated one only tests.
            stand: (false, -12, !object_rules.rules.as_ref().and_then(|rules| rules.get(&object.name, "IsAnimated")).is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"))),
            art: object.name.to_lowercase(),
            frame: 0,
            kind: PlacementKind::TerrainObject,
        });
    }
    let terrain_objects = map.terrain_objects.len();

    // Buildings placed by the map. A building's art is drawn at the point it is given without
    // adjustment, with the cell's untinted brightness and through the owner's palette. The point is
    // the top corner of the foundation's top cell, with the art's canvas centred on it.
    let art_rules = files.get("art.ini").map(Ini::parse);
    let mut house_colors = HouseColors::new(object_rules.rules.as_ref());
    let mut owner_palette = |owner: &str| {
        house_colors.palette_for(owner, map, object_rules.rules.as_ref(), &theater.unit_palette)
    };
    // The map's buildings are handed to the simulation, which draws them like any
    // other building; here only their owners' palettes and cells are worked out.
    let structures = map.structures.len();
    let mut structure_palettes = Vec::new();
    let mut structure_cells: Vec<(i32, i32)> = Vec::new();
    for structure in &map.structures {
        let rules = object_rules.rules.as_ref();
        let image = rules
            .and_then(|rules| rules.get(&structure.kind, "Image"))
            .unwrap_or(&structure.kind);
        // Foundation is `WxH` in cells.
        let (width, height) = art_rules
            .as_ref()
            .and_then(|art| art.get(image, "Foundation"))
            .and_then(|f| {
                let (w, h) = f.to_lowercase().split_once('x').map(|(w, h)| (w.to_owned(), h.to_owned()))?;
                Some((w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?))
            })
            .unwrap_or((1, 1));
        structure_palettes.push(owner_palette(&structure.owner));
        for dy in 0..height {
            for dx in 0..width {
                structure_cells.push((structure.x + dx, structure.y + dy));
            }
        }
    }

    // Infantry placed by the map. The frame is the sequence's first number, plus
    // the animation frame, plus a facing index times the sequence's third number;
    // the facing index comes from a 32-entry table indexed by the facing rounded to
    // 32 directions. Brightness is the cell's untinted value plus rules.ini
    // `ExtraInfantryLight`.
    let infantry_palettes: Vec<Option<usize>> = map.infantry.iter().map(|soldier| owner_palette(&soldier.owner)).collect();

    // Cells that something standing on them closes to ground units.
    let mut blocked: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    for object in &objects {
        let blocks = match object.kind {
            PlacementKind::TerrainObject | PlacementKind::Structure => true,
            PlacementKind::Overlay => !object.art.starts_with("bridge") && !object.art.starts_with("lobrd"),
            _ => false,
        };
        if blocks {
            for y in object.cell.1..=object.sort_cell.1 {
                for x in object.cell.0..=object.sort_cell.0 {
                    blocked.insert((x, y));
                }
            }
        }
    }
    blocked.extend(structure_cells);
    let unit_remaps: Vec<Option<usize>> = map.units.iter().map(|unit| owner_palette(&unit.owner)).collect();
    let player_palette =
        house_colors.palette_for_color(PLAYER_COLOURS[player.colour.min(7)], &theater.unit_palette);
    // Each computer player's colour: the one chosen, else the first not taken.
    // (Made here, before the palettes are handed to the art.)
    let mut taken = vec![player.colour.min(7)];
    taken.extend(player.opponents.iter().filter_map(|opponent| opponent.colour));
    let opponent_colours: Vec<usize> = player
        .opponents
        .iter()
        .map(|opponent| {
            opponent.colour.unwrap_or_else(|| {
                let free = (0..8).find(|c| !taken.contains(c)).unwrap_or(1);
                taken.push(free);
                free
            })
        })
        .collect();
    let opponent_palettes: Vec<Option<usize>> =
        opponent_colours.iter().map(|&colour| house_colors.palette_for_color(PLAYER_COLOURS[colour.min(7)], &theater.unit_palette)).collect();

    let mut sprites: HashMap<String, Option<Shp>> = HashMap::new();
    let mut animations: Vec<Animation> = Vec::new();
    let mut missing_objects = 0;
    let mut missing_art: Vec<String> = Vec::new();
    let mut note_missing = |what: String| {
        if !missing_art.contains(&what) {
            missing_art.push(what);
        }
    };
    for object in objects {
        let shp = sprites
            .entry(object.art.clone())
            .or_insert_with(|| find_shape(files, &object.art, theater.extension, theater.letter));
        let (Some(shp), Some(&level)) = (shp.as_ref(), levels.get(&object.cell)) else {
            // The map loader does not place an overlay
            // whose type has no art, so those are skipped without complaint. Base
            // RA2 has no art for the `TIB2_` and `TIB3_` ore types some maps name.
            if !matches!(object.kind, PlacementKind::Ore | PlacementKind::Overlay) {
                missing_objects += 1;
                note_missing(object.art.clone());
            }
            continue;
        };
        let Some(frame) = shp.frames.get(object.frame) else {
            missing_objects += 1;
            note_missing(format!("{} frame {}", object.art, object.frame));
            continue;
        };
        let (x, y) = cell_origin(object.cell.0, object.cell.1, level);
        let left = x - min_x + CELL_WIDTH / 2 - shp.width as i32 / 2 + object.nudge.0;
        // A building's art is centred on the top corner of its foundation's
        // top cell, not on the cell's middle.
        let down = if object.kind == PlacementKind::Structure { 0 } else { CELL_HEIGHT / 2 };
        let top = y - min_y + down - shp.height as i32 / 2 - object.lift + object.nudge.1;

        // For terrain objects and overlays
        // alike, the shadow is the frame half the file's frame count later, drawn at
        // the same point as the art.
        let shadow = shp.frames.get(object.frame + shp.frames.len() / 2);
        let mut shadow_index = None;
        if let Some(shadow) = shadow {
            if shadow.width > 0 && shadow.height > 0 {
                let entry = *art
                    .entry(ArtKey::Shadow(object.art.clone(), object.frame))
                    .or_insert_with(|| {
                        atlas.add(shadow.width as u32, shadow.height as u32, &shadow_rgba(shadow))
                    });
                shadow_index = Some(placements.len());
                placements.push(Placement {
                    entry,
                    x: left + shadow.x as i32 + object.shadow_shift.0,
                    y: top + shadow.y as i32 + object.shadow_shift.1,
                    layer: Layer::Shadow,
                    cell: object.sort_cell,
                    kind: PlacementKind::Shadow,
                    depth: None,
                    ground: None,
                    light: None,
                    light_cell: object.cell,
                });
            }
        }

        if frame.width == 0 || frame.height == 0 {
            continue;
        }
        use crate::lighting::LightRule;
        let light_rule = match (object.kind, object.palette) {
            // Ore and gems are drawn at full brightness
            // with a global palette, not lit by their cell.
            (PlacementKind::Ore, _) => None,
            (PlacementKind::Infantry, _) => Some(LightRule::Object { extra: light_setup.extra_infantry }),
            (_, PaletteKind::Unit) => Some(LightRule::Object { extra: 0 }),
            _ => Some(LightRule::Ground { raise: object.light_levels }),
        };
        let light = [1000; 3];
        let key = ArtKey::Frame(object.art.clone(), object.frame, object.palette, object.remap, light);
        let entry = *art.entry(key).or_insert_with(|| {
            let palette = match object.remap {
                Some(index) => house_colors.palette(index),
                None => theater.palette(object.palette),
            };
            let rgba = frame_rgba(frame, palette, light);
            atlas.add(frame.width as u32, frame.height as u32, &rgba)
        });
        placements.push(Placement {
            entry,
            x: left + frame.x as i32,
            y: top + frame.y as i32,
            layer: Layer::Object,
            cell: object.sort_cell,
            kind: object.kind,
            depth: Some(match object.stand {
                (true, adjust, write) => crate::depth::Depth::Flat { adjust: adjust - 15 * level, write },
                (false, adjust, write) => crate::depth::Depth::Upright { adjust: adjust - 15 * level, write },
            }),
            ground: None,
            light: light_rule,
            light_cell: object.cell,
        });

        // An animated object's image is its stage, and
        // its shadow the frame half the file later.
        if let Some((rate, probability)) = object.animation {
            let half = shp.frames.len() / 2;
            let mut steps = Vec::new();
            for index in 0..half {
                let image = &shp.frames[index];
                let drawn = (image.width > 0 && image.height > 0).then(|| {
                    let key = ArtKey::Frame(object.art.clone(), index, object.palette, object.remap, light);
                    let entry = *art.entry(key).or_insert_with(|| {
                        let palette = match object.remap {
                            Some(index) => house_colors.palette(index),
                            None => theater.palette(object.palette),
                        };
                        atlas.add(image.width as u32, image.height as u32, &frame_rgba(image, palette, light))
                    });
                    (entry, left + image.x as i32, top + image.y as i32)
                });
                let shadow = shp.frames.get(index + half).filter(|s| s.width > 0 && s.height > 0).map(|shadow| {
                    let entry = *art.entry(ArtKey::Shadow(object.art.clone(), index)).or_insert_with(|| {
                        atlas.add(shadow.width as u32, shadow.height as u32, &shadow_rgba(shadow))
                    });
                    (entry, left + shadow.x as i32 + object.shadow_shift.0, top + shadow.y as i32 + object.shadow_shift.1)
                });
                steps.push((drawn, shadow));
            }
            animations.push(Animation {
                cell: object.cell,
                art: placements.len() - 1,
                shadow: shadow_index,
                steps,
                rate,
                probability,
            });
        }
    }

    let all_palettes = house_colors.palettes();

    // Building types, in rules.ini order. DATA: `[BuildingTypes]`, each type's
    // `Cost`, `Power`, `TechLevel`, `Owner`, `Prerequisite` and `Adjacent`, the
    // `[General] Prerequisite...` groups, and art.ini `Foundation` and `Cameo`.
    let building_names: Vec<String> = object_rules
        .rules
        .as_ref()
        .map(|rules| rules.section("BuildingTypes").iter().map(|(_, name)| name.clone()).collect())
        .unwrap_or_default();
    let armoury = std::cell::RefCell::new(object_rules.rules.as_ref().map(crate::armoury::Armoury::load).unwrap_or_default());
    let building_index = |name: &str| -> Option<u16> {
        building_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16)
    };
    // DATA: `[Countries]`; a country's place in it is its bit in the owner masks.
    let countries: Vec<String> = object_rules
        .rules
        .as_ref()
        .map(|rules| rules.section("Countries").iter().map(|(_, name)| name.trim().to_owned()).collect())
        .unwrap_or_default();
    let country_mask = |list: &str| -> u32 {
        list.split(',')
            .filter_map(|name| countries.iter().position(|c| c.eq_ignore_ascii_case(name.trim())))
            .fold(0, |mask, index| mask | 1 << index.min(31))
    };
    let strings = files.get("ra2.csf").and_then(ra2_formats::csf::Csf::parse);
    // DATA: rules.ini `[SuperWeaponTypes]`. The keys and their defaults; `RechargeTime` is minutes, 900 frames each.
    let super_types: Vec<SuperInfo> = object_rules
        .rules
        .as_ref()
        .map(|rules| {
            rules
                .section("SuperWeaponTypes")
                .iter()
                .map(|(_, name)| {
                    let name = name.trim();
                    let get = |key: &str| rules.get(name, key).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
                    let yes = |key: &str, default: bool| get(key).map_or(default, |v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "1"));
                    let kind = get("Type").map_or(-1, |v| {
                        ["MultiMissile", "IronCurtain", "LightningStorm", "ChronoSphere", "ChronoWarp", "ParaDrop", "AmerParaDrop"]
                            .iter()
                            .position(|n| n.eq_ignore_ascii_case(&v))
                            .map_or(-1, |i| i as i32)
                    });
                    SuperInfo {
                        name: name.to_owned(),
                        ui_name: get("UIName").unwrap_or_default(),
                        cameo: get("SidebarImage").map_or_else(|| "xxicon".to_owned(), |v| v.to_lowercase()),
                        sim: ra2_sim::supers::SuperType {
                            recharge: get("RechargeTime").and_then(|v| v.parse::<f64>().ok()).map(|minutes| (minutes * 900.0) as u32).filter(|&frames| frames > 0).unwrap_or(4500),
                            kind,
                            powered: yes("IsPowered", true),
                            show_timer: yes("ShowTimer", false),
                            disableable: yes("DisableableFromShell", false),
                            weapon: get("WeaponType").and_then(|weapon| armoury.borrow_mut().weapon_named(rules, &weapon)),
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let super_names: Vec<String> = super_types.iter().map(|info| info.name.clone()).collect();
    let wanted_anims: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let mut building_types: Vec<BuildingInfo> = building_names
        .iter()
        .map(|name| {
            let rules = object_rules.rules.as_ref();
            let get = |key: &str| rules.and_then(|rules| rules.get(name, key));
            let number = |key: &str| get(key).and_then(|v| v.trim().parse::<i32>().ok());
            let image = get("Image").unwrap_or(name).to_lowercase();
            let art = art_rules.as_ref();
            let size = art
                .and_then(|art| art.get(&image, "Foundation"))
                .and_then(|f| {
                    let f = f.to_lowercase();
                    let (w, h) = f.split_once('x')?;
                    Some((w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?))
                })
                .unwrap_or((1, 1));
            let prerequisites = get("Prerequisite")
                .map(|list| {
                    list.split(',')
                        .map(|entry| {
                            let entry = entry.trim();
                            let group = rules.and_then(|rules| {
                                rules.get("General", &format!("Prerequisite{entry}"))
                            });
                            match group {
                                Some(members) => members.split(',').filter_map(|m| building_index(m)).collect(),
                                None => building_index(entry).into_iter().collect(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let yes = |key: &str| get(key).is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"));
            let list = |key: &str| -> Vec<u16> {
                get(key).map(|list| list.split(',').filter_map(|m| building_index(m)).collect()).unwrap_or_default()
            };
            let ui_name = get("UIName").map_or_else(String::new, |name| name.trim().to_owned());
            // DATA: art.ini `Buildup`. Half its frames
            // are the animation, the other half shadows.
            let buildup = art
                .and_then(|art| art.get(&image, "Buildup"))
                .map(|name| name.trim().to_lowercase())
                .filter(|name| !name.is_empty());
            let buildup_frames = buildup
                .as_deref()
                .and_then(|name| find_shape(files, name, theater.extension, theater.letter))
                .map_or(0, |shp| shp.frames.len() as u32 / 2);
            // DATA: art.ini animation slots. The keys, and which slots run at rest; a refinery shows one of its
            // four active slots by how full it is.
            let image_name = image.clone();
            let art_key = |key: &str| {
                art.and_then(|art| art.get(&image_name, key)).map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty())
            };
            let art_number = |key: &str| art_key(key).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            let (queueing_cell, impassable_rows) = (art_key("QueueingCell"), art_key("NumberImpassableRows"));
            let refinery = yes("Refinery");
            let anims: Vec<AnimSlot> = ["ActiveAnim", "ActiveAnimTwo", "ActiveAnimThree", "ActiveAnimFour", "IdleAnim", "SuperAnim", "SuperAnimTwo", "SuperAnimThree", "SuperAnimFour", "SpecialAnim"]
                .iter()
                .enumerate()
                .filter_map(|(index, prefix)| {
                    let name = art_key(prefix)?;
                    Some(AnimSlot {
                        damaged: art_key(&format!("{prefix}Damaged")).unwrap_or_else(|| name.clone()),
                        name,
                        y_sort: art_number(&format!("{prefix}YSort")),
                        z_adjust: art_number(&format!("{prefix}ZAdjust")),
                        refinery_level: if refinery && index < 4 { index } else { 0 },
                        // A refinery's and a silo's `SpecialAnim` are not firing
                        // animations.
                        special: *prefix == "SpecialAnim",
                        powered: art_key(&format!("{prefix}Powered")).is_none_or(|v| matches!(v.as_str(), "yes" | "true" | "1")),
                        powered_light: art_key(&format!("{prefix}PoweredLight")).is_some_and(|v| matches!(v.as_str(), "yes" | "true" | "1")),
                        super_stage: ["SuperAnim", "SuperAnimTwo", "SuperAnimThree", "SuperAnimFour"].iter().position(|p| p == prefix).map(|i| i as u8),
                    })
                })
                .collect();
            for slot in &anims {
                wanted_anims.borrow_mut().extend([slot.name.clone(), slot.damaged.clone()]);
            }
            BuildingInfo {
                bib: art_key("BibShape"),
                charged_anim_time: get("ChargedAnimTime").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(999.0),
                zshape_move: art_key("ZShapePointMove")
                    .and_then(|v| {
                        let mut n = v.split(',').filter_map(|n| n.split(';').next()?.trim().parse::<i32>().ok());
                        Some((n.next()?, n.next()?))
                    })
                    .unwrap_or((0, 0)),
                normal_z_adjust: art_number("NormalZAdjust"),
                turret: (yes("Turret") && yes("TurretAnimIsVoxel"))
                    .then(|| get("TurretAnim").map(|name| (name.trim().to_lowercase(), number("TurretAnimX").unwrap_or(0), number("TurretAnimY").unwrap_or(0))))
                    .flatten(),
                deploying: art_key("DeployingAnim"),
                under_door: art_key("UnderDoorAnim"),
                anims,
                buildup: buildup.filter(|_| buildup_frames > 0),
                name: name.clone(),
                cameo: art
                    .and_then(|art| art.get(&image, "Cameo"))
                    .map_or_else(|| "xxicon".to_owned(), |c| c.trim().to_lowercase()),
                ui_name: ui_name.clone(),
                height: art
                    .and_then(|art| art.get(&image, "Height"))
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(2),
                factory: get("Factory").is_some_and(|v| !v.trim().is_empty()),
                undeploys: None,
                art: image,
                sim: ra2_sim::BuildingType {
                    leavings: Default::default(),
                    explosions: Vec::new(),
                    cost: number("Cost").unwrap_or(0),
                    power: number("Power").unwrap_or(0),
                    size,
                    prerequisites,
                    // The defaults.
                    adjacent: number("Adjacent").unwrap_or(3),
                    base_normal: get("BaseNormal").is_none_or(|v| {
                        matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1")
                    }),
                    construction_yard: get("ConstructionYard")
                        .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1")),
                    fighter: rules.map_or_else(Default::default, |rules| {
                        armoury.borrow_mut().fighter(rules, art, name, &image_name, None)
                    }),
                    // A building's turret
                    // turns at the type's `ROT`.
                    turret_rot: number("ROT").unwrap_or(0),
                    powered: yes("Powered") && !get("TogglePower").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "no" | "false" | "0")),
                    cloning: yes("Cloning"),
                    soylent: number("Soylent").unwrap_or(0),
                    crewed: yes("Crewed"),
                    radar: yes("Radar"),
                    reveal_to_all: yes("RevealToAll"),
                    refinery: yes("Refinery"),
                    can_be_occupied: yes("CanBeOccupied"),
                    max_occupants: number("MaxNumberOccupants").unwrap_or(0).max(0) as usize,
                    muzzles: (0..10)
                        .map_while(|i| {
                            let value = art_key(&format!("MuzzleFlash{i}"))?;
                            let mut n = value.split(',').filter_map(|n| n.trim().parse::<i32>().ok());
                            Some((n.next()?, n.next()?))
                        })
                        .collect(),
                    needs_engineer: yes("NeedsEngineer"),
                    spy_sat: yes("SpySat"),
                    ore_purifier: yes("OrePurifier"),
                    overpowerable: yes("Overpowerable"),
                    psychic_radius: number("PsychicDetectionRadius").unwrap_or(0),
                    cash: (number("ProduceCashStartup").unwrap_or(0), number("ProduceCashAmount").unwrap_or(0), number("ProduceCashDelay").unwrap_or(0)),
                    unit_repair: yes("UnitRepair"),
                    bridge_repair_hut: yes("BridgeRepairHut"),
                    gap_radius: if yes("GapGenerator") { get("GapRadiusInCells").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0) } else { 0 },
                    radial_indicator: yes("HasRadialIndicator"),
                    crate_beneath: yes("CrateBeneath"),
                    crate_beneath_money: yes("CrateBeneathIsMoney"),
                    concentric_indicator: yes("ConcentricRadialIndicator"),
                    hospital: yes("Hospital"),
                    armory: yes("Armory"),
                    ammo: get("Ammo").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(-1),
                    ai_build_this: yes("AIBuildThis"),
                    plug: name.eq_ignore_ascii_case("GAPLUG"),
                    extra_space: yes("ProtectWithWall") || yes("WantsExtraSpace"),
                    protect_with_wall: yes("ProtectWithWall"),
                    delayed_fire: if art_key("IsAnimDelayedFire").is_some_and(|v| matches!(v.split(';').next().unwrap_or("").trim(), "yes" | "true" | "1")) {
                        art_key("DelayedFireDelay").and_then(|v| v.split(';').next()?.trim().parse::<u32>().ok()).unwrap_or(0)
                    } else {
                        0
                    },
                    number_of_docks: if yes("Helipad") { number("NumberOfDocks").unwrap_or(0).max(0) as usize } else { 0 },
                    docking_offsets: (0..number("NumberOfDocks").unwrap_or(0).max(0))
                        .map(|i| {
                            // From art.ini, under the
                            // building's image (so the American airfield has the
                            // Allied one's pads).
                            let mut n = art_key(&format!("DockingOffset{i}")).into_iter().flat_map(|v| {
                                v.split(';').next().unwrap_or("").split(',').filter_map(|n| n.trim().parse::<i32>().ok()).collect::<Vec<_>>()
                            });
                            (n.next().unwrap_or(0), n.next().unwrap_or(0))
                        })
                        .collect(),
                    // DATA: on unless the type says otherwise.
                    can_c4: !get("CanC4").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "no" | "false" | "0")),
                    barrel_recoil: {
                        let number = |key: &str| get(key).and_then(|v| v.split(';').next()?.trim().parse::<i32>().ok()).unwrap_or(0);
                        (number("BarrelTravel"), number("BarrelCompressFrames"), number("BarrelHoldFrames"), number("BarrelRecoverFrames"))
                    },
                    explodes: get("Explodes").is_some_and(|v| matches!(v.split(';').next().unwrap_or("").trim().to_lowercase().as_str(), "yes" | "true" | "1")),
                    death_weapon: rules.and_then(|rules| armoury.borrow_mut().death_weapon(rules, name)),
                    death_percent: get("DeathWeaponDamageModifier").and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).map_or(100, |share| (share * 100.0) as i32),
                    free_unit: get("FreeUnit").and_then(|unit| {
                        let rules = rules?;
                        let vehicles = rules.section("VehicleTypes");
                        vehicles.iter().position(|(_, n)| n.trim().eq_ignore_ascii_case(unit.trim())).map(|i| i as u16)
                    }),
                    queueing_cell: queueing_cell
                        .as_deref()
                        .and_then(|v| {
                            let mut n = v.split(',').filter_map(|n| n.trim().parse::<i32>().ok());
                            Some((n.next()?, n.next()?))
                        })
                        .unwrap_or((0, 0)),
                    impassable_rows: get("NumberImpassableRows")
                        .map(str::to_owned)
                        .or(impassable_rows)
                        .and_then(|v| v.trim().parse::<i32>().ok())
                        .filter(|&rows| rows >= 0),
                    weapons_factory: yes("WeaponsFactory"),
                    gdi_barracks: yes("GDIBarracks"),
                    nod_barracks: yes("NODBarracks"),
                    // From the rules, not the art.
                    exit_coord: get("ExitCoord")
                        .and_then(|v| {
                            let mut n = v.split(',').filter_map(|n| n.trim().parse::<i32>().ok());
                            Some((n.next()?, n.next()?))
                        })
                        .unwrap_or((0, 0)),
                    door_ticks: (get("DeployTime").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0) * 900.0) as u32,
                    undeploys_into: None,
                    buildup_frames,
                    strength: number("Strength").unwrap_or(1),
                    // DATA: the keys. How each
                    // is used; see `World::may_build`.
                    tech_level: number("TechLevel").unwrap_or(-1),
                    owners: get("Owner").map_or(0, &country_mask),
                    required_houses: get("RequiredHouses").map(&country_mask),
                    forbidden_houses: get("ForbiddenHouses").map(&country_mask),
                    prerequisite_override: list("PrerequisiteOverride"),
                    build_limit: number("BuildLimit"),
                    factory: get("Factory").and_then(|kind| match kind.trim().to_lowercase().as_str() {
                        "buildingtype" => Some(ra2_sim::Factory::Building),
                        "infantrytype" => Some(ra2_sim::Factory::Infantry),
                        "unittype" => Some(ra2_sim::Factory::Unit),
                        "aircrafttype" => Some(ra2_sim::Factory::Aircraft),
                        _ => None,
                    }),
                    combat: get("BuildCat").is_some_and(|cat| cat.trim().eq_ignore_ascii_case("Combat")),
                    planning_side: number("AIBasePlanningSide").unwrap_or(-1),
                    water_bound: yes("WaterBound"),
                    naval: yes("Naval"),
                    // DATA: `Wall=yes`, with the overlay it becomes named by `ToOverlay=`.
                    wall_overlay: Some(get("ToOverlay").map_or(name.as_str(), str::trim))
                        .filter(|_| yes("Wall"))
                        .and_then(|overlay| {
                            object_rules.overlay_names.iter().position(|n| n.eq_ignore_ascii_case(overlay))
                        })
                        .filter(|&overlay| object_rules.is_wall(overlay))
                        .map(|overlay| overlay as u16),
                    guard_range: number("GuardRange").unwrap_or(0),
                    spyable: yes("Spyable"),
                    storage: number("Storage").unwrap_or(0),
                    // `DetectDisguiseRange`, for a
                    // building with `DetectDisguise`.
                    detect_disguise: if yes("DetectDisguise") { number("DetectDisguiseRange").unwrap_or(0) } else { 0 },
                    super_weapon: get("SuperWeapon").and_then(|name| super_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim()))).map(|i| i as u16),
                    super_weapon2: get("SuperWeapon2").and_then(|name| super_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim()))).map(|i| i as u16),
                    sort_name: strings
                        .as_ref()
                        .and_then(|strings| strings.get(&ui_name))
                        .map_or_else(|| name.clone(), str::to_owned),
                },
            }
        })
        .collect();
    // The animation types the buildings name.
    // DATA: art.ini. The keys, their defaults, and that
    // the delay between images is 900 / `Rate`, rounded down (1 with no `Rate`).
    wanted_anims.borrow_mut().extend(armoury.borrow().animations());
    // And those the super weapons show.
    // DATA: rules.ini `[AudioVisual]` keys.
    if let Some(rules) = object_rules.rules.as_ref() {
        for key in [
            "NukeTakeOff", "IronCurtainInvokeAnim", "ChronoPlacement", "ChronoBlast", "ChronoBlastDest", "WarpOut", "ChronoSparkle1", "Wake",
            "WeatherConClouds", "WeatherConBolts", "WeatherConBoltExplosion", "WeaponNullifyAnim",
        ] {
            let names = ["AudioVisual", "General"].iter().find_map(|section| rules.get(section, key)).unwrap_or("");
            wanted_anims.borrow_mut().extend(names.split(',').map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()));
        }
        wanted_anims.borrow_mut().extend(["NUKEBALL".to_owned(), "PSIWARN".to_owned()]);
    }
    wanted_anims.borrow_mut().extend(tile_anims.iter().map(|(anim, _, _)| anim.clone()));
    // And those the crates show (`[Powerups]`, the second value).
    if let Some(rules) = object_rules.rules.as_ref() {
        for name in ra2_sim::powerups::NAMES {
            if let Some(anim) = rules.get("Powerups", name).and_then(|v| v.split(',').nth(1).map(|a| a.split(';').next().unwrap_or("").trim().to_owned())) {
                if !anim.is_empty() && !anim.eq_ignore_ascii_case("none") {
                    wanted_anims.borrow_mut().push(anim);
                }
            }
        }
    }
    // And those that debris shows as it flies and ends.
    let mut wanted: std::collections::BTreeSet<String> = wanted_anims.into_inner().into_iter().map(|name| name.to_uppercase()).collect();
    if let Some(art) = art_rules.as_ref() {
        let mut queue: Vec<String> = wanted.iter().cloned().collect();
        while let Some(name) = queue.pop() {
            if !art.get_exact(&name, "Bouncer").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1")) {
                continue;
            }
            for key in ["ExpireAnim", "BounceAnim", "TrailerAnim"] {
                if let Some(next) = art.get_exact(&name, key).map(|v| v.split(';').next().unwrap_or("").trim().to_uppercase()).filter(|v| !v.is_empty()) {
                    if wanted.insert(next.clone()) {
                        queue.push(next);
                    }
                }
            }
        }
    }
    let anim_types: HashMap<String, AnimType> = wanted
        .into_iter()
        .filter_map(|name| {
            let art = art_rules.as_ref()?;
            let get = |key: &str| art.get(&name, key).map(|v| v.trim().to_lowercase());
            let number = |key: &str| get(key).and_then(|v| v.parse::<i32>().ok());
            let yes = |key: &str| get(key).is_some_and(|v| matches!(v.as_str(), "yes" | "true" | "1"));
            // A name in rules.ini's
            // `[Animations]` is a type with every default even when art.ini
            // says nothing of it (the Grand Cannon's GCMUZZLE).
            let listed = object_rules.rules.as_ref().is_some_and(|rules| rules.section("Animations").iter().any(|(_, entry)| entry.trim().eq_ignore_ascii_case(&name)));
            if art.section(&name).is_empty() && !listed {
                return None;
            }
            let anim = AnimType {
                image: get("Image").filter(|v| !v.is_empty()).unwrap_or_else(|| name.clone()),
                start: number("Start").unwrap_or(0).max(0) as usize,
                loop_start: number("LoopStart").unwrap_or(0).max(0) as usize,
                loop_end: number("LoopEnd").filter(|v| *v > 0).map(|v| v as usize),
                end: number("End").filter(|v| *v > 0).map(|v| v as usize),
                loop_count: number("LoopCount").unwrap_or(1),
                delay: match number("Rate") {
                    Some(rate) if rate > 0 => (900 / rate) as u32,
                    Some(_) => 0,
                    None => 1,
                },
                normalized: yes("Normalized"),
                shadow: yes("Shadow"),
                y_draw_offset: number("YDrawOffset").unwrap_or(0),
                scorch: yes("Scorch"),
                crater: yes("Crater"),
                force_big_craters: yes("ForceBigCraters"),
                size: (0, 0),
                middle: 0,
                report: art.get(&name, "StartSound").or(art.get(&name, "Report")).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()),
                // `Translucent` alone is half see-through.
                opacity: match number("Translucency") {
                    Some(25) => 0.75,
                    Some(50) => 0.5,
                    Some(75) => 0.25,
                    _ if yes("Translucent") => 0.5,
                    _ => 1.0,
                },
                alt_palette: yes("AltPalette"),
                bouncer: None,
            };
            // DATA: a `Bouncer` animation's flight keys, matched exactly.
            let exact = |key: &str| art.get_exact(&name, key).map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).filter(|v| !v.is_empty());
            let bouncer = exact("Bouncer").is_some_and(|v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "1")).then(|| {
                let real = |key: &str, default: f64| exact(key).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
                let defaults = ra2_sim::remains::Bouncer::anim();
                let warhead = exact("Warhead").and_then(|w| object_rules.rules.as_ref().map(|rules| armoury.borrow_mut().warhead_named(rules, &w)));
                let motion = ra2_sim::remains::Bouncer {
                    elasticity: real("Elasticity", defaults.elasticity),
                    max_xy: real("MaxXYVel", defaults.max_xy),
                    min_z: real("MinZVel", defaults.min_z),
                    damage: real("Damage", 0.0) as i32,
                    radius: exact("DamageRadius").and_then(|v| v.parse().ok()).unwrap_or(0),
                    warhead,
                    trailer_every: exact("TrailerSeperation").and_then(|v| v.parse().ok()).unwrap_or(1),
                    ..defaults
                };
                (motion, [exact("ExpireAnim"), exact("BounceAnim"), exact("TrailerAnim")])
            });
            let anim = AnimType { bouncer, ..anim };
            // Only the ones that mark the ground need their largest picture:
            // the first of the greatest width times height (30 by 30 without
            // art).
            let (size, middle) = if anim.scorch || anim.crater {
                find_shape(files, &anim.image, theater.extension, theater.letter)
                    .and_then(|shp| {
                        let frames = if anim.shadow { &shp.frames[..shp.frames.len() / 2] } else { &shp.frames[..] };
                        let (index, frame) = frames.iter().enumerate().rev().max_by_key(|(_, f)| f.width as u32 * f.height as u32)?;
                        Some(((frame.width as i32, frame.height as i32), index))
                    })
                    .unwrap_or(((30, 30), 0))
            } else {
                ((0, 0), 0)
            };
            Some((name, AnimType { size, middle, ..anim }))
        })
        .collect();
    let map_structures: Vec<MapStructure> = map
        .structures
        .iter()
        .zip(&structure_palettes)
        .filter_map(|(structure, &palette)| {
            Some(MapStructure {
                kind: building_index(&structure.kind)?,
                cell: (structure.x, structure.y),
                health: structure.health,
                palette,
            })
        })
        .collect();
    // DATA: `[AudioVisual] ConditionYellow`, the health at or below which things
    // show damage.
    let condition_yellow = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("AudioVisual", "ConditionYellow"))
        .and_then(|v| v.trim().trim_end_matches('%').parse::<f32>().ok())
        .map_or(0.5, |v| if v > 1.0 { v / 100.0 } else { v });
    let condition_red = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("AudioVisual", "ConditionRed"))
        .and_then(|v| v.trim().trim_end_matches('%').parse::<f32>().ok())
        .map_or(0.25, |v| if v > 1.0 { v / 100.0 } else { v });

    let idle_frequency = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("General", "IdleActionFrequency"))
        .and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok())
        .unwrap_or(0.15);

    // DATA: the `[General]` keys that set production time.
    let build_rules = {
        let general = |key: &str, default: f64| {
            object_rules
                .rules
                .as_ref()
                .and_then(|rules| rules.get("General", key))
                .and_then(|value| value.trim().parse::<f64>().ok())
                .unwrap_or(default)
        };
        let defaults = ra2_sim::BuildRules::default();
        ra2_sim::BuildRules {
            build_speed: general("BuildSpeed", defaults.build_speed),
            low_power_penalty: general("LowPowerPenaltyModifier", defaults.low_power_penalty),
            min_low_power_speed: general("MinLowPowerProductionSpeed", defaults.min_low_power_speed),
            max_low_power_speed: general("MaxLowPowerProductionSpeed", defaults.max_low_power_speed),
            multiple_factory: general("MultipleFactory", defaults.multiple_factory),
            wall_coefficient: general("WallBuildSpeedCoefficient", defaults.wall_coefficient),
        }
    };
    // DATA: `[General] BuildupTime`, minutes. Times 900.
    let buildup_ticks = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("General", "BuildupTime"))
        .and_then(|value| value.trim().parse::<f64>().ok())
        .map_or(54, |minutes| (minutes * 900.0) as u32);
    struct UnitKind<'a> {
        kind: &'a str,
    }
    // DATA: `[VehicleTypes]` then `[InfantryTypes]`; a unit's type is its place in
    // the two lists run together.
    let type_names = |section: &str| -> Vec<String> {
        object_rules
            .rules
            .as_ref()
            .map(|rules| rules.section(section).iter().map(|(_, name)| name.trim().to_owned()).collect())
            .unwrap_or_default()
    };
    let mut vehicle_names = type_names("VehicleTypes");
    let vehicle_count = vehicle_names.len();
    vehicle_names.extend(type_names("InfantryTypes"));
    // Aircraft come after the foot soldiers.
    let infantry_end = vehicle_names.len();
    vehicle_names.extend(type_names("AircraftTypes"));
    // Vehicles placed by the map. They move, so they are not part of the static
    // scene; the game draws them through `UnitArt`.
    // DATA: rules.ini `Speed`, `ROT`, `Accelerates` and the acceleration keys.
    let spawn = |kind: &str, cell: (i32, i32), facing: u8, palette: Option<usize>, owner: u8| {
            let unit = UnitKind { kind };
            let rules = object_rules.rules.as_ref();
            // DATA: `DeploysInto` names a building type.
            let deploys = rules
                .and_then(|rules| rules.get(kind, "DeploysInto"))
                .and_then(|building| building_index(building));
            let voice = |key: &str| {
                rules
                    .and_then(|rules| rules.get(kind, key))
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
            };
            UnitSpawn {
                owner,
                voice_select: voice("VoiceSelect"),
                voice_move: voice("VoiceMove"),
                image: rules
                    .and_then(|rules| rules.get(unit.kind, "Image"))
                    .unwrap_or(unit.kind)
                    .to_lowercase(),
                cell,
                facing,
                palette,
                stats: {
                    let defaults = ra2_sim::UnitStats::default();
                    let number = |key: &str| {
                        rules
                            .and_then(|rules| rules.get(unit.kind, key))
                            .and_then(|value| value.trim().parse::<f64>().ok())
                    };
                    let millionths = |key: &str, default: i32| {
                        number(key).map_or(default, |value| (value * 1_000_000.0) as i32)
                    };
                    ra2_sim::UnitStats {
                        spawns: None,
                        crusher: false,
                        crushable: false,
                        explodes: false,
                        death_weapon: None,
                        death_percent: 100,
                        cloakable: false,
                        cloaking_speed: 7,
                        sensors: false,
                        sensors_sight: 0,
                        trainable: false,
                        unselectable: false,
                        no_spawn_alt: false,
                        veteran_abilities: 0,
                        elite_abilities: 0,
                        elite_primary: None,
                        elite_secondary: None,
                        self_healing: false,
                        leadership: 5,
                        missile_spawn: false,
                        rocket: false,
                        allowed_to_start: defaults.allowed_to_start,
                        base_unit: defaults.base_unit,
                        speed: number("Speed").map_or(defaults.speed, |v| v as i32),
                        rot: number("ROT").map_or(defaults.rot, |v| v as i32),
                        accelerates: rules
                            .and_then(|rules| rules.get(&unit.kind, "Accelerates"))
                            .map_or(defaults.accelerates, |v| {
                                matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1")
                            }),
                        acceleration: millionths("AccelerationFactor", defaults.acceleration),
                        deceleration: millionths("DeaccelerationFactor", defaults.deceleration),
                        slowdown_distance: number("SlowdownDistance")
                            .map_or(defaults.slowdown_distance, |v| v as i32),
                        speed_type: {
                            use ra2_sim::SpeedType;
                            let name = rules
                                .and_then(|rules| rules.get(unit.kind, "SpeedType"))
                                .map(|v| v.trim().to_lowercase());
                            #[allow(clippy::match_same_arms)]
                            match name.as_deref() {
                                Some("foot") => SpeedType::Foot,
                                Some("wheel") => SpeedType::Wheel,
                                Some("hover") => SpeedType::Hover,
                                Some("float") => SpeedType::Float,
                                Some("floatbeach") => SpeedType::FloatBeach,
                                Some("amphibious") => SpeedType::Amphibious,
                                Some("track") => SpeedType::Track,
                                // Unset, a
                                // vehicle's is Track for a crusher and Wheel
                                // otherwise. A soldier's is Foot (the infantry type hands 0
                                // to the constructor that stores it).
                                _ if vehicle_names.iter().position(|name| name.eq_ignore_ascii_case(kind)).is_some_and(|index| index >= vehicle_count && index < infantry_end) => SpeedType::Foot,
                                _ => defaults.speed_type,
                            }
                        },
                        deploys_into: deploys,
                        strength: number("Strength").map_or(defaults.strength, |v| v as i32),
                        kind: vehicle_names
                            .iter()
                            .position(|name| name.eq_ignore_ascii_case(kind))
                            .map_or(0, |index| index as u16 + 1),
                        infantry: vehicle_names
                            .iter()
                            .position(|name| name.eq_ignore_ascii_case(kind))
                            .is_some_and(|index| index >= vehicle_count && index < infantry_end),
                        fighter: rules.map_or_else(Default::default, |rules| {
                            let image = rules.get(unit.kind, "Image").unwrap_or(unit.kind).to_lowercase();
                            let sequence = art_rules.as_ref().and_then(|art| art.get(&image, "Sequence")).map(str::to_owned);
                            armoury.borrow_mut().fighter(rules, art_rules.as_ref(), unit.kind, &image, sequence.as_deref())
                        }),
                        // The Fly
                        // locomotor by its class id, and the aircraft's keys.
                        // DATA: `[General] FlightLevel` where the type has none.
                        plane: rules.filter(|r| r.get(unit.kind, "Locomotor").is_some_and(|id| id.to_uppercase().contains("4A582746"))).map(|r| {
                            let get = |key: &str| r.get(unit.kind, key).map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase());
                            let yes = |key: &str| get(key).is_some_and(|v| matches!(v.as_str(), "yes" | "true" | "1"));
                            let whole = |key: &str| get(key).and_then(|v| v.parse::<i32>().ok());
                            let level = r.get("General", "FlightLevel").and_then(|v| v.split(';').next()?.trim().parse::<i32>().ok()).unwrap_or(1500);
                            ra2_sim::aircraft::PlaneStats {
                                flight_level: whole("FlightLevel").filter(|&v| v >= 0).unwrap_or(level),
                                fighter: yes("Fighter"),
                                airport_bound: yes("AirportBound"),
                                landable: yes("Landable"),
                                ammo: whole("Ammo").unwrap_or(-1),
                                slowdown: whole("SlowdownDistance").unwrap_or(500),
                                pitch_speed: get("PitchSpeed").and_then(|v| v.parse::<f64>().ok()).map_or(250, |v| (v * 1000.0).round() as i32),
                            }
                        }),
                        // The jumpjet
                        // locomotor by its class id, and its keys.
                        jet: rules.filter(|r| r.get(unit.kind, "Locomotor").is_some_and(|id| id.to_uppercase().contains("92612C46"))).map(|r| {
                            let defaults = ra2_sim::flight::JetStats::default();
                            // Keys are matched exactly, and rules.ini spells two
                            // of these `JumpJetAccel` and `JumpJetTurnRate`, so the
                            // game never sees them and every stock jumpjet flies
                            // with the built-in 2.0 and 4.
                            let get = |key: &str| r.get_exact(unit.kind, key).map(|v| v.trim().to_lowercase());
                            let whole = |key: &str, default: i32| get(key).and_then(|v| v.parse::<f64>().ok()).map_or(default, |v| v as i32);
                            let milli = |key: &str, default: i32| get(key).and_then(|v| v.parse::<f64>().ok()).map_or(default, |v| (v * 1000.0) as i32);
                            let yes = |key: &str| get(key).is_some_and(|v| matches!(v.as_str(), "yes" | "true" | "1"));
                            ra2_sim::flight::JetStats {
                                turn_rate: whole("JumpjetTurnRate", defaults.turn_rate),
                                speed: whole("JumpjetSpeed", defaults.speed),
                                climb_milli: milli("JumpjetClimb", defaults.climb_milli),
                                crash_milli: milli("JumpjetCrash", defaults.crash_milli),
                                height: whole("JumpjetHeight", defaults.height),
                                accel_milli: milli("JumpjetAccel", defaults.accel_milli),
                                wobbles_milli: milli("JumpjetWobbles", defaults.wobbles_milli),
                                no_wobbles: yes("JumpjetNoWobbles"),
                                deviation: whole("JumpjetDeviation", defaults.deviation),
                                balloon_hover: yes("BalloonHover"),
                                hover_attack: yes("HoverAttack"),
                                crashable: yes("Crashable"),
                            }
                        }),
                        harvester: rules.and_then(|r| r.get(unit.kind, "Harvester")).is_some_and(|v| v.trim().eq_ignore_ascii_case("yes")),
                        storage: number("Storage").map_or(0, |v| v as u32),
                        teleporter: rules.and_then(|r| r.get(unit.kind, "Teleporter")).is_some_and(|v| v.trim().eq_ignore_ascii_case("yes")),
                        // DATA: `MovementZone`. Twelve of them.
                        movement_zone: {
                            const ZONES: [&str; 12] = [
                                "normal", "crusher", "destroyer", "amphibiousdestroyer", "amphibiouscrusher",
                                "amphibious", "subterrannean", "infantry", "infantrydestroyer", "fly", "water",
                                "waterbeach",
                            ];
                            let name = rules
                                .and_then(|rules| rules.get(unit.kind, "MovementZone"))
                                .map(|v| v.trim().to_lowercase());
                            name.and_then(|name| ZONES.iter().position(|z| *z == name)).unwrap_or(0) as u8
                        },
                    }
                },
            }
    };
    // Every vehicle type: what the simulation needs to produce it and the game to
    // show it.
    // DATA: the same keys as for buildings; see `World::may_build`.
    let unit_types: Vec<UnitInfo> = vehicle_names
        .iter()
        .enumerate()
        .map(|(type_index, name)| {
            let rules = object_rules.rules.as_ref();
            let get = |key: &str| rules.and_then(|rules| rules.get(name, key));
            let number = |key: &str| get(key).and_then(|v| v.trim().parse::<i32>().ok());
            let yes = |key: &str| get(key).is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"));
            let made = spawn(name, (0, 0), 128, player_palette, PLAYER);
            let ui_name = get("UIName").map_or_else(String::new, |name| name.trim().to_owned());
            let prerequisites = get("Prerequisite")
                .map(|list| {
                    list.split(',')
                        .map(|entry| {
                            let entry = entry.trim();
                            match rules.and_then(|rules| rules.get("General", &format!("Prerequisite{entry}"))) {
                                Some(members) => members.split(',').filter_map(|m| building_index(m)).collect(),
                                None => building_index(entry).into_iter().collect(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            UnitInfo {
                name: name.clone(),
                cameo: art_rules
                    .as_ref()
                    .and_then(|art| art.get(&made.image, "Cameo"))
                    .map_or_else(|| "xxicon".to_owned(), |c| c.trim().to_lowercase()),
                alt_cameo: art_rules.as_ref().and_then(|art| art.get(&made.image, "AltCameo")).map(|c| c.trim().to_lowercase()).filter(|c| !c.is_empty()),
                sim: ra2_sim::UnitType {
                    explosions: Vec::new(),
                    death_anims: Vec::new(),
                    leavings: Default::default(),
                    crewed: yes("Crewed"),
                    weight: get("Weight").and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(0.0),
                    underwater: yes("Underwater"),
                    not_human: yes("NotHuman"),
                    suppression_threshold: number("SuppressionThreshold").unwrap_or(0),
                    passengers: number("Passengers").unwrap_or(0),
                    size_limit: number("SizeLimit").unwrap_or(0),
                    carries_crate: yes("CarriesCrate"),
                    size: number("Size").unwrap_or(1),
                    gunner: yes("Gunner"),
                    gunner_weapons: object_rules
                        .rules
                        .as_ref()
                        .filter(|_| yes("Gunner"))
                        .map(|rules| armoury.borrow_mut().gunner_weapons(rules, art_rules.as_ref(), name, &made.image))
                        .unwrap_or_default(),
                    gunner_elite: object_rules
                        .rules
                        .as_ref()
                        .filter(|_| yes("Gunner"))
                        .map(|rules| armoury.borrow_mut().gunner_elite(rules, art_rules.as_ref(), name, &made.image))
                        .unwrap_or_default(),
                    ifv_mode: number("IFVMode").unwrap_or(0).max(0) as usize,
                    // The frame by colour name.
                    pip: match get("Pip").map(|v| v.trim().to_lowercase()).as_deref() {
                        Some("yellow") => 2,
                        Some("white") => 3,
                        Some("red") => 4,
                        Some("blue") => 5,
                        _ => 1,
                    },
                    // An unknown word is 0.
                    pip_scale: match get("PipScale").map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase()).as_deref() {
                        Some("ammo") => 1,
                        Some("tiberium") => 2,
                        Some("passengers") => 3,
                        Some("power") => 4,
                        _ => 0,
                    },
                    pip_wrap: number("PipWrap").unwrap_or(0),
                    stats: ra2_sim::UnitStats {
                        // `Spawns`
                        // with its numbers, `MissileSpawn`, and the Rocket
                        // locomotor by its class id.
                        spawns: get("Spawns").and_then(|kind| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(kind.trim()))).map(|kind| ra2_sim::spawns::SpawnStats {
                            kind: kind as u16,
                            number: number("SpawnsNumber").unwrap_or(0).max(0) as u32,
                            regen: number("SpawnRegenRate").unwrap_or(0).max(0) as u32,
                            reload: number("SpawnReloadRate").unwrap_or(0).max(0) as u32,
                        }),
                        // Experience
                        // and what it brings; `Trainable` is on unless the type says no.
                        no_spawn_alt: yes("NoSpawnAlt"),
                        unselectable: get("Selectable").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "no" | "false" | "0")),
                        trainable: !get("Trainable").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "no" | "false" | "0")),
                        veteran_abilities: ability_bits(get("VeteranAbilities")),
                        elite_abilities: ability_bits(get("EliteAbilities")),
                        elite_primary: rules.and_then(|rules| armoury.borrow_mut().elite(rules, art_rules.as_ref(), name, &made.image).0),
                        elite_secondary: rules.and_then(|rules| armoury.borrow_mut().elite(rules, art_rules.as_ref(), name, &made.image).1),
                        self_healing: yes("SelfHealing"),
                        // The cloak's keys. Crushing and death weapons. DATA:
                        // `Crushable` is on for foot soldiers unless given.
                        crusher: yes("Crusher"),
                        crushable: get("Crushable").map_or(type_index >= vehicle_count && type_index < infantry_end, |v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1")),
                        explodes: yes("Explodes"),
                        death_weapon: rules.and_then(|rules| armoury.borrow_mut().death_weapon(rules, name)),
                        death_percent: get("DeathWeaponDamageModifier").and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).map_or(100, |share| (share * 100.0) as i32),
                        cloakable: yes("Cloakable"),
                        cloaking_speed: number("CloakingSpeed").unwrap_or(7),
                        sensors: yes("Sensors"),
                        sensors_sight: number("SensorsSight").unwrap_or(0),
                        leadership: number("LeadershipRating").unwrap_or(5),
                        missile_spawn: yes("MissileSpawn"),
                        rocket: get("Locomotor").is_some_and(|id| id.to_uppercase().contains("B7B49766")),
                        // DATA: on unless the type says otherwise.
                        allowed_to_start: !get("AllowedToStartInMultiplayer").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "no" | "false" | "0")),
                        base_unit: rules
                            .and_then(|rules| rules.get("General", "BaseUnit"))
                            .is_some_and(|list| list.split(',').any(|entry| entry.trim().eq_ignore_ascii_case(name))),
                        ..made.stats
                    },
                    dock: get("Dock").map(|list| list.split(',').filter_map(|m| building_index(m)).collect()).unwrap_or_default(),
                    cost: number("Cost").unwrap_or(0),
                    tech_level: number("TechLevel").unwrap_or(-1),
                    owners: get("Owner").map_or(0, &country_mask),
                    required_houses: get("RequiredHouses").map(&country_mask),
                    forbidden_houses: get("ForbiddenHouses").map(&country_mask),
                    prerequisites,
                    prerequisite_override: get("PrerequisiteOverride")
                        .map(|list| list.split(',').filter_map(|m| building_index(m)).collect())
                        .unwrap_or_default(),
                    build_limit: number("BuildLimit"),
                    requires_stolen_tech: if yes("RequiresStolenSovietTech") { 1 } else if yes("RequiresStolenAlliedTech") { 2 } else { 0 },
                    infantry: type_index >= vehicle_count && type_index < infantry_end,
                    naval: yes("Naval"),
                    planning_side: number("AIBasePlanningSide").unwrap_or(-1),
                    sort_name: strings
                        .as_ref()
                        .and_then(|strings| strings.get(&ui_name))
                        .map_or_else(|| name.clone(), str::to_owned),
                },
                ui_name,
                considered_aircraft: yes("ConsideredAircraft"),
                // DATA: `ShadowIndex` 0 and `Weight` 1 unless given.
                shadow_index: art_rules.as_ref().and_then(|art| art.get(&made.image, "ShadowIndex")).and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0),
                no_shadow: yes("NoShadow"),
                weight: get("Weight").and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).filter(|w| *w > 0.0).unwrap_or(1.0),
                charge_turrets: if yes("IsChargeTurret") { number("TurretCount").unwrap_or(0).clamp(0, 15) as u8 } else { 0 },
                transport_sounds: (
                    get("EnterTransportSound").map(|v| v.trim().to_owned()),
                    get("LeaveTransportSound").map(|v| v.trim().to_owned()),
                ),
                deploy_sounds: (
                    get("DeploySound").map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()),
                    get("UndeploySound").map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()),
                ),
                // The type's sound, else
                // the rules' own.
                chrono_sounds: {
                    let pick = |key: &str| get(key).or_else(|| rules.and_then(|rules| rules.get("General", key))).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
                    (pick("ChronoOutSound"), pick("ChronoInSound"))
                },
                aux_sounds: {
                    let pick = |key: &str| get(key).map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).filter(|v| !v.is_empty());
                    (pick("AuxSound1"), pick("AuxSound2"))
                },
                roll_angle: get("RollAngle").and_then(|v| v.split(';').next()?.trim().parse::<f32>().ok()).map_or(std::f32::consts::FRAC_PI_6, f32::to_radians),
                // A type named by
                // `UnloadingClass`, drawn in the miner's place as it dumps.
                unloading: get("UnloadingClass").map(str::trim).filter(|v| !v.is_empty()).map(|class| rules.and_then(|rules| rules.get(class, "Image")).unwrap_or(class).trim().to_lowercase()),
                sheet: art_rules.as_ref().filter(|_| type_index < vehicle_count).and_then(|art| {
                    let key = |key: &str| art.get(&made.image, key).and_then(|v| v.trim().parse::<usize>().ok());
                    if !art.get(&made.image, "Voxel").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "no" | "false" | "0")) {
                        return None;
                    }
                    let (walk_frames, firing_frames) = (key("WalkFrames").unwrap_or(1).max(1), key("FiringFrames").unwrap_or(0));
                    let standing_frames = key("StandingFrames").unwrap_or(usize::from(firing_frames > 0));
                    let facings = 8;
                    let start_walk = key("StartWalkFrame").unwrap_or(0);
                    let start_stand = key("StartStandFrame").unwrap_or(if standing_frames == 0 { start_walk } else { walk_frames * facings });
                    let start_firing = key("StartFiringFrame")
                        .unwrap_or(if firing_frames == 0 { start_stand } else { (walk_frames + standing_frames) * facings });
                    Some(Sheet {
                        walk_frames,
                        standing_frames,
                        firing_frames,
                        start_walk,
                        start_stand,
                        start_firing,
                        walk_rate: number("WalkRate").unwrap_or(1).max(1) as u64,
                        idle_rate: number("IdleRate").unwrap_or(0).max(0) as u64,
                    })
                }),
                // DATA: art.ini `Sequence` names a section whose `Ready` and `Walk`
                // entries are "first frame, frames, frames per facing".
                sequences: (type_index >= vehicle_count && type_index < infantry_end).then(|| {
                    let section = art_rules.as_ref().and_then(|art| art.get(&made.image, "Sequence")).unwrap_or("").to_owned();
                    art_rules
                        .as_ref()
                        .map(|art| art.section(&section))
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|(action, entry)| {
                            let numbers: Vec<usize> = entry.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                            match numbers[..] {
                                [first, count, step, ..] => Some((action.to_lowercase(), (first, count.max(1), step))),
                                _ => None,
                            }
                        })
                        .collect()
                }),
                image: made.image,
                voice_select: made.voice_select,
                voice_move: made.voice_move,
            }
        })
        .collect();

    let infantry_spawns: Vec<InfantrySpawn> = map
        .infantry
        .iter()
        .zip(&infantry_palettes)
        .filter_map(|(soldier, &palette)| {
            Some(InfantrySpawn {
                kind: vehicle_names.iter().position(|name| name.eq_ignore_ascii_case(&soldier.kind))? as u16,
                cell: (soldier.x, soldier.y),
                spot: soldier.sub_cell as usize,
                facing: soldier.facing,
                palette,
            })
        })
        .collect();

    // DATA: `UndeploysInto` names the vehicle a building packs up into.
    let undeploys: Vec<Option<UnitSpawn>> = building_names
        .iter()
        .map(|name| {
            let kind = object_rules.rules.as_ref()?.get(name, "UndeploysInto")?.trim().to_owned();
            (!kind.is_empty()).then(|| spawn(&kind, (0, 0), 128, player_palette, PLAYER))
        })
        .collect();
    // DATA: `[MultiplayerDialogSettings] MCVRedeploys`, the default of the "MCV
    // repacks" option.DATA: `[MultiplayerDialogSettings] AlliesAllowed`: no in the rules, yes
    // in both stock skirmish modes' files.
    let allies_allowed = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("MultiplayerDialogSettings", "AlliesAllowed"))
        .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"));
    // Not original behaviour: in Free
    // For All the computer houses are not allied. Base Red Alert 2's
    // MPFreeForAll.ini says `AlliesAllowed=yes`, the same as Battle's, so in the
    // original both modes set every computer house against the player; Yuri's
    // Revenge's file says no, which is what is done here.
    let allies_allowed = allies_allowed && !player.mode_rules.eq_ignore_ascii_case("MPFreeForAll.ini");
    let mcv_repacks = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("MultiplayerDialogSettings", "MCVRedeploys"))
        .is_none_or(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"));
    let mut unit_spawns: Vec<UnitSpawn> = map
        .units
        .iter()
        .zip(&unit_remaps)
        .map(|(unit, &palette)| spawn(&unit.kind, (unit.x, unit.y), unit.facing, palette, NEUTRAL))
        .collect();

    // The player starts with their side's construction vehicle at a start position.
    // DATA: `[General] BaseUnit` lists the construction vehicles; `[Waypoints]` 0 to 7
    // are the start positions, stored as `y * 1000 + x`.
    // The base unit is the first in the list whose
    // `Owner` includes the player's country.
    // The first house's start is one
    // of the map's start positions at random; the number is drawn when the
    // match is set up, so that a saved or network game gets the same.
    let all_starts: Vec<(i32, i32)> = (0..8)
        .filter_map(|n| map.ini.get("Waypoints", &n.to_string())?.trim().parse::<i32>().ok())
        .map(|value| (value % 1000, value / 1000))
        .collect();
    let player_start = match player.start {
        Some(drawn) if !all_starts.is_empty() => Some(all_starts[drawn % all_starts.len()]),
        _ => all_starts.first().copied(),
    };
    let country_name = countries.get(player.country).cloned().unwrap_or_default();
    let base_unit = object_rules.rules.as_ref().and_then(|rules| {
        let list = rules.get("General", "BaseUnit")?;
        let owned = |unit: &&str| {
            rules.get(unit.trim(), "Owner").is_some_and(|owners| country_mask(owners) & (1 << player.country.min(31)) != 0)
        };
        list.split(',').find(owned).or_else(|| list.split(',').next()).map(|name| name.trim().to_owned())
    });
    // DATA: the country's `Side`: GDI is the Allies (0), Nod the Soviets (1).
    let player_side = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get(&country_name, "Side"))
        .map_or(0, |side| i32::from(side.trim().eq_ignore_ascii_case("Nod")));
    if let (Some(cell), Some(kind)) = (player_start, base_unit.as_deref()) {
        unit_spawns.push(spawn(kind, cell, 64, player_palette, PLAYER));
    }
    // The computer houses after the human, in the order of their rows; each
    // starts at the valid start waypoint with the largest sum of distances from
    // those taken; its base unit is the first `BaseUnit` its country owns,
    // facing north.
    let mut enemies = Vec::new();
    if let Some(mine) = player_start {
        let starts: Vec<(i32, i32)> = (0..8)
            .filter_map(|n| map.ini.get("Waypoints", &n.to_string())?.trim().parse::<i32>().ok())
            .map(|value| (value % 1000, value / 1000))
            .collect();
        let mut used = vec![mine];
        for (index, opponent) in player.opponents.iter().enumerate() {
            let far = |cell: &(i32, i32)| used.iter().map(|taken| (((cell.0 - taken.0) as f64).powi(2) + ((cell.1 - taken.1) as f64).powi(2)).sqrt() as i64).sum::<i64>();
            let Some(start) = starts.iter().copied().filter(|cell| !used.contains(cell)).max_by_key(far) else {
                break;
            };
            used.push(start);
            let country = opponent.country.unwrap_or(((map.width * 31 + map.height * 17 + 5) as usize) % countries.len().clamp(1, 9));
            let name = countries.get(country).cloned().unwrap_or_default();
            let Some(rules) = object_rules.rules.as_ref() else {
                break;
            };
            let side = rules.get(&name, "Side").map_or(0, |side| i32::from(side.trim().eq_ignore_ascii_case("Nod")));
            let palette = opponent_palettes[index];
            let Some(list) = rules.get("General", "BaseUnit") else {
                break;
            };
            let kind = list
                .split(',')
                .find(|unit| rules.get(unit.trim(), "Owner").is_some_and(|owners| country_mask(owners) & (1 << country.min(31)) != 0))
                .or_else(|| list.split(',').next())
                .unwrap_or("")
                .trim();
            unit_spawns.push(spawn(kind, start, 0, palette, ENEMY + enemies.len() as u8));
            enemies.push(Enemy { start, country: country as u8, side, palette, level: opponent.level });
        }
    }
    let house_palettes: Vec<Option<usize>> = [None, player_palette].into_iter().chain(enemies.iter().map(|enemy| enemy.palette)).collect();

    // DATA: the `[AI]` and `[General]` lists a computer house plans from.
    let ai_rules = {
        let rules = object_rules.rules.as_ref();
        let buildings = |section: &str, key: &str| -> Vec<u16> {
            rules
                .and_then(|rules| rules.get(section, key))
                .map(|list| list.split(',').filter_map(|name| building_index(name.split(';').next().unwrap_or(""))).collect())
                .unwrap_or_default()
        };
        let counts = |key: &str| -> [i32; 3] {
            let list: Vec<i32> = rules
                .and_then(|rules| rules.get("General", key).or_else(|| rules.get("AI", key)))
                .map(|list| list.split(',').filter_map(|n| n.split(';').next()?.trim().parse().ok()).collect())
                .unwrap_or_default();
            [list.first().copied().unwrap_or(0), list.get(1).copied().unwrap_or(0), list.get(2).copied().unwrap_or(0)]
        };
        let number = |section: &str, key: &str, default: i32| {
            rules.and_then(|rules| rules.get(section, key)).and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(default)
        };
        ra2_sim::ai::Rules {
            build_const: buildings("AI", "BuildConst"),
            build_power: buildings("AI", "BuildPower"),
            build_barracks: buildings("AI", "BuildBarracks"),
            build_weapons: buildings("AI", "BuildWeapons"),
            build_refinery: buildings("AI", "BuildRefinery"),
            allied_power: buildings("General", "GDIPowerPlant").first().copied(),
            soviet_power: buildings("General", "NodRegularPower").first().copied(),
            harvesters: rules
                .and_then(|rules| rules.get("General", "HarvesterUnit"))
                .map(|list| list.split(',').filter_map(|name| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16)).collect())
                .unwrap_or_default(),
            harvesters_per_refinery: number("General", "HarvestersPerRefinery", 2),
            defence_counts: (counts("AlliedBaseDefenseCounts"), counts("SovietBaseDefenseCounts")),
            base_spacing: number("AI", "AIBaseSpacing", 1),
            rof: ["Easy", "Normal", "Difficult"].map(|section| rules.and_then(|rules| rules.get(section, "ROF")).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(1.0)),
            repair_delay: ["Easy", "Normal", "Difficult"].map(|section| rules.and_then(|rules| rules.get(section, "RepairDelay")).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(0.02)),
            credit_reserve: number("General", "CreditReserve", 100),
            credit_cutoff: counts("AIAlternateProductionCreditCutoff")[0],
            auto_deploy_delay: counts("AIAutoDeployFrameDelay"),
            target_values: ["ConYard", "WarFactory", "Power", "Engineer", "Thief", "Harvester", "MCV", "APC", "BaseDefense", "Plug", "Helipad", "Temple"].map(|what| counts(&format!("AIIonCannon{what}Value"))),
            minor_super_ready: rules.and_then(|rules| rules.get("General", "AIMinorSuperReadyPercent")).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(0.7),
            credit_bonus: counts("MultiplayerAICM"),
            virtual_purifiers: counts("AIVirtualPurifiers"),
            concrete_walls: buildings("AI", "ConcreteWalls"),
            wall_percent: counts("AIPickWallDefensePercent"),
            purifier_bonus: rules.and_then(|rules| rules.get("General", "PurifierBonus")).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(0.25),
        }
    };

    // Each building type's light and art.ini `ExtraLight`.
    light_setup.buildings = building_names
        .iter()
        .map(|name| {
            let rules = object_rules.rules.as_ref();
            let light = crate::lighting::TypeLight::read(|key| rules?.get(name, key).map(str::to_owned));
            let image = rules.and_then(|rules| rules.get(name, "Image")).map_or(name.as_str(), |image| image.trim());
            let extra = art_rules.as_ref().and_then(|art| art.get(image, "ExtraLight")).and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0);
            (light, extra)
        })
        .collect();

    let unit_art = UnitArt {
        hva_frame: 0,
        models: HashMap::new(),
        vpl: files.get("voxels.vpl").and_then(Vpl::parse),
        normals: files
            .loose("FinalAlert2/voxel_normal_tables.bin")
            .map(|data| NormalTables::parse(&data))
            .unwrap_or_else(NormalTables::empty),
        base_palette: Palette(theater.unit_palette.0),
        owner_palettes: all_palettes,
        theater_palette: Palette(theater.theater_palette.0),
        iso_palette: Palette(theater.iso_palette.0),
        anim_palette: files.get("anim.pal").and_then(Palette::parse),
        extension: theater.extension,
        letter: theater.letter,
        shapes: HashMap::new(),
    };

    // DATA: rules.ini's land sections give each speed type a percentage of full
    // speed on that ground, and `[General] BlockagePathDelay` a wait in ticks.
    let land_rules = {
        use ra2_sim::{LAND_COUNT, LandRules, SPEED_TYPE_COUNT};
        const LANDS: [&str; LAND_COUNT] = [
            "Clear", "Road", "Water", "Rock", "Rough", "Beach", "Ice", "Railroad", "Tunnel",
            "Tiberium", "Weeds", "Wall",
        ];
        const SPEED_TYPES: [&str; SPEED_TYPE_COUNT] =
            ["Foot", "Track", "Wheel", "Hover", "Float", "FloatBeach", "Amphibious"];
        let mut table = LandRules::open();
        if let Some(rules) = object_rules.rules.as_ref() {
            for (land, section) in LANDS.iter().enumerate() {
                for (speed_type, key) in SPEED_TYPES.iter().enumerate() {
                    let percent = rules
                        .get(section, key)
                        .and_then(|value| value.trim().trim_end_matches('%').trim().parse::<u8>().ok());
                    if let Some(percent) = percent {
                        table.0[land][speed_type] = percent;
                    }
                }
                // DATA: `Buildable`.
                table.1[land] = rules
                    .get(section, "Buildable")
                    .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"));
            }
        }
        table
    };
    let blockage_delay = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("General", "BlockagePathDelay"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(ra2_sim::DEFAULT_BLOCKAGE_DELAY);
    // DATA: rules.ini `[General] LeptonsPerSightIncrease`.
    let sight_height_step = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("General", "LeptonsPerSightIncrease"))
        .and_then(|value| value.split(';').next()?.trim().parse().ok())
        .unwrap_or(2000);
    let ore_cells_set: std::collections::HashSet<(i32, i32)> = levels
        .keys()
        .copied()
        .filter(|&(x, y)| {
            map.overlay_at(x, y)
                .is_some_and(|(overlay, _)| matches!(object_rules.ore_overlays.get(overlay as usize), Some(Some(_))))
        })
        .collect();

    // What stands on each cell, as the pathfinder classes it. The overlay and
    // terrain object tests. DATA: the rules keys.
    let mut obstacles: HashMap<(i32, i32), ra2_sim::Obstacle> = HashMap::new();
    let mut tree_bits: HashMap<(i32, i32), u8> = HashMap::new();
    {
        use ra2_sim::Obstacle;
        let rules = object_rules.rules.as_ref();
        let yes = |section: &str, key: &str| {
            rules
                .and_then(|rules| rules.get(section, key))
                .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"))
        };
        for structure in &map.structures {
            let image = rules.and_then(|rules| rules.get(&structure.kind, "Image")).unwrap_or(&structure.kind);
            let (w, h) = art_rules
                .as_ref()
                .and_then(|art| art.get(image, "Foundation"))
                .and_then(|f| {
                    let f = f.to_lowercase();
                    let (w, h) = f.split_once('x')?;
                    Some((w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?))
                })
                .unwrap_or((1, 1));
            for dy in 0..h {
                for dx in 0..w {
                    obstacles.insert((structure.x + dx, structure.y + dy), Obstacle::Building);
                }
            }
        }
        let bits_key = if map.theater.eq_ignore_ascii_case("SNOW") { "SnowOccupationBits" } else { "TemperateOccupationBits" };
        for object in &map.terrain_objects {
            let bits = rules
                .and_then(|rules| rules.get(&object.name, bits_key))
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(7);
            obstacles.insert((object.x, object.y), if bits == 7 { Obstacle::TreeFull } else { Obstacle::Tree });
            tree_bits.insert((object.x, object.y), bits as u8);
        }
        for &(cell, _, _) in &ground {
            let Some(name) = map.overlay_at(cell.0, cell.1).and_then(|(o, _)| object_rules.overlay_names.get(o as usize)) else {
                continue;
            };
            let closed_land = rules.and_then(|rules| rules.get(name, "Land")).is_some_and(|land| {
                rules
                    .and_then(|rules| rules.get(land.trim(), "Wheel"))
                    .and_then(|v| v.trim().trim_end_matches('%').trim().parse::<i32>().ok())
                    == Some(0)
            });
            let kind = if yes(name, "Crushable") {
                Obstacle::Crushable
            } else if yes(name, "Wall") {
                Obstacle::Wall
            } else if closed_land || yes(name, "IsARock") {
                Obstacle::Rock
            } else if yes(name, "IsRubble") {
                Obstacle::Rubble
            } else {
                continue;
            };
            obstacles.insert(cell, kind);
        }
    }
    // DATA: the map's `[Map] Size` and `LocalSize`, which bound the playable area.
    let playfield = map.ini.get("Map", "LocalSize").and_then(|local| {
        let local: Vec<i32> = local.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        Some((map.width, map.height, <[i32; 4]>::try_from(local).ok()?))
    });

    // What the simulation needs to know about each cell. The land for each tile
    // terrain type: 0 and 13 clear, 1 to 4 ice, 5 tunnel, 6 railroad, 7, 8 and
    // 15 rock, 9 water, 10 beach, 11 and 12 road, 14 rough. Ore on flat or
    // gently sloped clear ground makes the cell `Tiberium` land.
    let mut deck: HashMap<(i32, i32), u8> = HashMap::new();
    for &(cell, _, _) in &ground {
        let across = match map.overlay_at(cell.0, cell.1) {
            Some((24 | 237, _)) => (0, -1),
            Some((25 | 238, _)) => (-1, 0),
            _ => continue,
        };
        for step in -1..=2 {
            let flags = if step == 2 { 1 } else { 3 };
            let entry = deck.entry((cell.0 + across.0 * step, cell.1 + across.1 * step)).or_insert(0);
            *entry |= flags;
        }
    }
    // `CliffBackImpassability=2`: clear ground, water, beach and ice with a cell
    // four or more levels higher among six of its neighbours (one row and one
    // column up, the two side diagonals, and one and two cells down the
    // screen) is rock: the foot of a cliff.
    let cliff_back = object_rules.rules.as_ref().and_then(|rules| rules.get("General", "CliffBackImpassability")).and_then(|v| v.split(';').next()?.trim().parse::<i32>().ok()) == Some(2);
    let levels: HashMap<(i32, i32), i32> = ground.iter().map(|&(cell, _, level)| (cell, level as i32)).collect();
    let below_cliff = |cell: (i32, i32), level: u8| {
        cliff_back
            && [(0, -1), (-1, 0), (2, 2), (1, 1), (-1, 1), (1, -1)]
                .iter()
                .any(|&(dx, dy)| levels.get(&(cell.0 + dx, cell.1 + dy)).copied().unwrap_or(0) >= level as i32 + 4)
    };
    let cells = ground
        .iter()
        .map(|&(cell, terrain_type, level)| SceneCell {
            x: cell.0,
            y: cell.1,
            level,
            passable: !blocked.contains(&cell),
            ramp: ramps.get(&cell).copied().unwrap_or(0),
            overlay: map.overlay_at(cell.0, cell.1).is_some(),
            deck: deck.get(&cell).copied().unwrap_or(0),
            obstacle: obstacles.get(&cell).copied().unwrap_or(ra2_sim::Obstacle::None),
            tree_bits: tree_bits.get(&cell).copied().unwrap_or(0),
            ore_ground: ore_ground.contains(&cell),
            bridge: map.overlay_at(cell.0, cell.1).filter(|(overlay, _)| matches!(overlay, 24 | 25 | 237 | 238 | 74..=101 | 205..=232)),
            base_land: tile_land(terrain_type),
            land: {
                use ra2_sim::Land;
                // A low bridge is three cells of
                // overlay at the water's level, and plain road to whatever
                // crosses it; its last two types (a destroyed section) are not.
                let low_bridge = map.overlay_at(cell.0, cell.1).is_some_and(|(overlay, _)| matches!(overlay, 74..=99 | 205..=230));
                if low_bridge {
                    Land::Road
                } else if ore_cells_set.contains(&cell) {
                    Land::Tiberium
                } else {
                    let land = match terrain_type {
                        1..=4 => Land::Ice,
                        5 => Land::Tunnel,
                        6 => Land::Railroad,
                        7 | 8 | 15 => Land::Rock,
                        9 => Land::Water,
                        10 => Land::Beach,
                        11 | 12 => Land::Road,
                        14 => Land::Rough,
                        _ => Land::Clear,
                    };
                    if matches!(land, Land::Clear | Land::Water | Land::Beach | Land::Ice) && below_cliff(cell, level) {
                        Land::Rock
                    } else {
                        land
                    }
                }
            },
        })
        .collect();

    for (info, undeploys) in building_types.iter_mut().zip(undeploys) {
        info.undeploys = undeploys;
    }
    // DATA: `[MultiplayerDialogSettings] TechLevel`.
    let tech_level = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("MultiplayerDialogSettings", "TechLevel"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(10);
    let bridge_art: HashMap<u8, String> = object_rules
        .overlay_names
        .iter()
        .enumerate()
        .filter(|(index, _)| matches!(index, 24 | 25 | 237 | 238 | 74..=101 | 205..=232))
        .map(|(index, name)| {
            let image = object_rules.rules.as_ref().and_then(|rules| rules.get(name, "Image")).unwrap_or(name);
            (index as u8, image.to_lowercase())
        })
        .collect();
    let bridge_rules = {
        let get = |section: &str, key: &str| object_rules.rules.as_ref().and_then(|rules| rules.get(section, key)).map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase());
        (
            get("MultiplayerDialogSettings", "BridgeDestruction").is_none_or(|v| matches!(v.as_str(), "yes" | "true" | "1")),
            get("CombatDamage", "BridgeStrength").and_then(|v| v.parse().ok()).unwrap_or(1500),
        )
    };
    // The art of each wall overlay type.
    let wall_art: HashMap<u16, String> = object_rules
        .overlay_names
        .iter()
        .enumerate()
        .filter(|(index, _)| object_rules.is_wall(*index))
        .map(|(index, name)| {
            let image = object_rules.rules.as_ref().and_then(|rules| rules.get(name, "Image")).unwrap_or(name);
            (index as u16, image.to_lowercase())
        })
        .collect();
    walls.sort_unstable();
    // What each cell shows as on the radar.
    // A cell with a terrain object is one
    // fixed colour; one with an overlay takes the radar colour in the header of the
    // overlay art's frame (for ore the frame is the amount); otherwise the tile's
    // own radar colour, times the theatre's brightness, halved.
    // DATA: the theatre brightness: snow 0.8, the others 1.
    let radar_ground = {
        let brightness = if map.theater.eq_ignore_ascii_case("SNOW") { 0.8f32 } else { 1.0 };
        let trees: std::collections::HashSet<(i32, i32)> = map.terrain_objects.iter().map(|object| (object.x, object.y)).collect();
        let mut shapes: HashMap<String, Option<Shp>> = HashMap::new();
        let mut colour_of = |art: &str, frame: usize| -> [u8; 3] {
            shapes
                .entry(art.to_owned())
                .or_insert_with(|| find_shape(files, art, theater.extension, theater.letter))
                .as_ref()
                .and_then(|shp| shp.frames.get(frame))
                .map_or([0; 3], |frame| frame.radar)
        };
        let mut cells = HashMap::new();
        for (&cell, tile) in &radar_tiles {
            let tile = tile.map(|c| ((c as f32 * brightness) as u32).min(255) as u8 >> 1);
            let over = if trees.contains(&cell) {
                Some([200, 200, 160])
            } else if let Some((overlay, state)) = map.overlay_at(cell.0, cell.1) {
                let ramp = ramps.get(&cell).copied().unwrap_or(0);
                if let Some(name) = object_rules.ore_image(overlay as usize, cell, ramp) {
                    Some(colour_of(name, state as usize))
                } else {
                    object_rules.overlay(overlay as usize, state, cell).map(|art| colour_of(&art.image, art.frame))
                }
            } else {
                None
            };
            cells.insert(cell, (tile, over));
        }
        RadarGround { cells }
    };

    // DATA: `[Tiberiums]` and each one's `Value`; the mining keys of `[General]`
    // and `[AI]`. The defaults.
    let mining = {
        let rules = object_rules.rules.as_ref();
        let number = |section: &str, key: &str| rules.and_then(|r| r.get(section, key)).and_then(|v| v.trim().parse::<f64>().ok());
        MiningRules {
            values: rules
                .map(|r| r.section("Tiberiums").iter().map(|(_, name)| number(name.trim(), "Value").unwrap_or(0.0) as i32).collect())
                .unwrap_or_default(),
            kinds: rules
                .map(|r| {
                    r.section("Tiberiums")
                        .iter()
                        .map(|(_, name)| ra2_sim::ore::OreType {
                            growth: number(name.trim(), "Growth").unwrap_or(0.0) as u32,
                            spread: number(name.trim(), "Spread").unwrap_or(0.0) as u32,
                            growth_percentage: number(name.trim(), "GrowthPercentage").unwrap_or(0.0),
                            spread_percentage: number(name.trim(), "SpreadPercentage").unwrap_or(0.0),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            near_scan: number("AI", "TiberiumNearScan").unwrap_or(6.0) as i32,
            far_scan: number("AI", "TiberiumFarScan").unwrap_or(32.0) as i32,
            load_rate: number("General", "HarvesterLoadRate").unwrap_or(2.0) as u32,
            dump_rate: number("General", "HarvesterDumpRate").unwrap_or(0.016),
            too_far: number("General", "HarvesterTooFarDistance").unwrap_or(5.0) as i32,
            chrono_too_far: number("General", "ChronoHarvTooFarDistance").unwrap_or(50.0) as i32,
        }
    };

    // DATA: `[General] Gravity`, `MissileROTVar` and `NormalTargetingDelay`, and
    // `[CombatDamage] MaxDamage`. The defaults.
    // DATA: the `[General]` keys for a teleporting unit's delay.
    let chrono = {
        let rules = object_rules.rules.as_ref();
        let get = |key: &str| rules.and_then(|r| r.get("General", key)).map(|v| v.trim().to_lowercase());
        let number = |key: &str, default: i32| get(key).and_then(|v| v.parse::<f64>().ok()).map_or(default, |v| v as i32);
        (
            number("ChronoDistanceFactor", 48),
            number("ChronoMinimumDelay", 16),
            number("ChronoRangeMinimum", 0),
            get("ChronoTrigger").is_none_or(|v| matches!(v.as_str(), "yes" | "true" | "1")),
        )
    };
    // DATA: rules.ini `[ElevationModel]`.
    let elevation = {
        let rules = object_rules.rules.as_ref();
        let number = |key: &str| rules.and_then(|r| r.get("ElevationModel", key)).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok());
        (number("ElevationIncrement").unwrap_or(4.0) as i32, number("ElevationIncrementBonus").unwrap_or(2.0), number("ElevationBonusCap").unwrap_or(2.0))
    };
    // DATA: rules.ini `[CombatDamage] BallisticScatter`, in cells.
    let ballistic_scatter = object_rules
        .rules
        .as_ref()
        .and_then(|rules| rules.get("CombatDamage", "BallisticScatter").or_else(|| rules.get("General", "BallisticScatter")))
        .and_then(|value| value.split(';').next()?.trim().parse::<f64>().ok())
        .map_or(256, |cells| (cells * 256.0) as i32);
    let combat_rules = {
        let rules = object_rules.rules.as_ref();
        let number = |section: &str, key: &str| rules.and_then(|r| r.get(section, key)).and_then(|v| v.trim().parse::<f64>().ok());
        (
            number("General", "Gravity").unwrap_or(3.0) as i32,
            number("CombatDamage", "MaxDamage").unwrap_or(10_000.0) as i32,
            number("General", "MissileROTVar").unwrap_or(0.25),
            number("General", "NormalTargetingDelay").map_or(27, |frames| frames as u32),
        )
    };
    // DATA: rules.ini `[SmudgeTypes]` with each type's `Crater`, `Burn`, `Width`
    // and `Height`. Those four keys and no others.
    let smudge_types: Vec<(String, ra2_sim::smudge::SmudgeType)> = object_rules
        .rules
        .as_ref()
        .map(|rules| {
            rules
                .section("SmudgeTypes")
                .iter()
                .map(|(_, name)| {
                    let name = name.trim();
                    let yes = |key: &str| rules.get(name, key).is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"));
                    let number = |key: &str| rules.get(name, key).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(1);
                    (name.to_lowercase(), ra2_sim::smudge::SmudgeType { crater: yes("Crater"), burn: yes("Burn"), width: number("Width"), height: number("Height") })
                })
                .collect()
        })
        .unwrap_or_default();
    // The map's `[Smudge]`: `type,x,y,…` per line.
    let map_smudges: Vec<((i32, i32), u16)> = map
        .ini
        .section("Smudge")
        .iter()
        .filter_map(|(_, line)| {
            let mut fields = line.split(',').map(str::trim);
            let name = fields.next()?.to_lowercase();
            let (x, y) = (fields.next()?.parse().ok()?, fields.next()?.parse().ok()?);
            let kind = smudge_types.iter().position(|(n, _)| *n == name)?;
            Some(((x, y), kind as u16))
        })
        .collect();
    // DATA: rules.ini `[General]` paradrop lists, `ParadropRadius`,
    // `ParachuteMaxFallRate`. The plane is the
    // aircraft type PDPLANE.
    // DATA: rules.ini `[Powerups]` (share, animation, allowed on water,
    // argument; a `%` value is a hundredth), `[General]` `CrateMinimum`,
    // `CrateMaximum`, `CrateRadius` (cells), `CrateRegen` (minutes),
    // `UnitCrateType`, `BaseUnit`, and the unit types with `CrateGoodie=yes`.
    let crate_rules = {
        let rules = object_rules.rules.as_ref();
        let general = |key: &str| rules.and_then(|rules| rules.get("General", key)).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
        let unit = |name: &str| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16);
        let powerups = ra2_sim::powerups::NAMES
            .iter()
            .map(|name| {
                let text = rules.and_then(|rules| rules.get("Powerups", name)).map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).unwrap_or_else(|| "0,NONE".to_owned());
                let parts: Vec<&str> = text.split(',').map(str::trim).collect();
                let argument = parts.get(3).map_or(0.0, |v| match v.strip_suffix('%') {
                    Some(n) => n.trim().parse::<f64>().unwrap_or(0.0) * 0.01,
                    None => v.parse::<f64>().unwrap_or(0.0),
                });
                ra2_sim::powerups::Powerup {
                    share: parts.first().and_then(|v| v.parse().ok()).unwrap_or(0),
                    anim: parts.get(1).filter(|v| !v.eq_ignore_ascii_case("none") && !v.is_empty()).map(|v| v.to_uppercase()),
                    water: parts.get(2).is_some_and(|v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "1")),
                    argument,
                }
            })
            .collect();
        let number = |key: &str, default: f64| general(key).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
        let goodies = vehicle_names
            .iter()
            .enumerate()
            .filter(|(_, name)| rules.and_then(|rules| rules.get(name, "CrateGoodie")).is_some_and(|v| matches!(v.split(';').next().unwrap_or("").trim().to_lowercase().as_str(), "yes" | "true" | "1")))
            .map(|(i, _)| i as u16)
            .collect();
        ra2_sim::powerups::Rules {
            powerups,
            minimum: number("CrateMinimum", 1.0) as i32,
            maximum: number("CrateMaximum", 255.0) as i32,
            radius: (number("CrateRadius", 3.0) * 256.0) as i32,
            regen: number("CrateRegen", 3.0),
            unit_type: general("UnitCrateType").and_then(|name| unit(&name)),
            base_units: general("BaseUnit").map(|list| list.split(',').filter_map(&unit).collect()).unwrap_or_default(),
            truck_crate: map.ini.get("Basic", "TruckCrate").is_some_and(|v| matches!(v.trim().chars().next(), Some('y' | 'Y' | 't' | 'T' | '1'))),
            goodies,
        }
    };
    let super_rules = {
        let rules = object_rules.rules.as_ref();
        let visual_list = |key: &str| -> Vec<String> {
            rules.and_then(|rules| rules.get("AudioVisual", key)).map(|v| v.split(',').map(|n| n.trim().to_uppercase()).filter(|n| !n.is_empty()).collect()).unwrap_or_default()
        };
        // How many frames an animation runs once through, and after how many
        // it is past half its pictures.
        let anim_life = |name: &str| -> (u32, u32) {
            let Some(anim) = anim_types.get(&name.to_uppercase()) else {
                return (0, 0);
            };
            let images = find_shape(files, &anim.image, theater.extension, theater.letter).map_or(0, |shp| shp.frames.len());
            let images = if anim.shadow { images / 2 } else { images };
            let end = anim.end.unwrap_or(images).saturating_sub(anim.start) as u32;
            (end * anim.delay, (end / 2 + 1) * anim.delay)
        };
        let get = |key: &str| rules.and_then(|rules| rules.get("General", key)).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
        let unit = |name: &str| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16);
        let list = |types: &str, counts: &str| -> Vec<(u16, u32)> {
            let types: Vec<Option<u16>> = get(types).map(|v| v.split(',').map(&unit).collect()).unwrap_or_default();
            let counts: Vec<u32> = get(counts).map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default();
            // Lists of different lengths drop nothing.
            if types.len() != counts.len() {
                return Vec::new();
            }
            types.into_iter().zip(counts).filter_map(|(kind, count)| Some((kind?, count))).collect()
        };
        ra2_sim::supers::Rules {
            plane: unit("PDPLANE"),
            american: list("AmerParaDropInf", "AmerParaDropNum"),
            allied: list("AllyParaDropInf", "AllyParaDropNum"),
            soviet: list("SovParaDropInf", "SovParaDropNum"),
            radius: get("ParadropRadius").and_then(|v| v.parse().ok()).unwrap_or(1024),
            fall_rate: get("ParachuteMaxFallRate").and_then(|v| v.parse::<i32>().ok()).map_or(3, i32::abs),
            curtain_duration: rules.and_then(|rules| rules.get("CombatDamage", "IronCurtainDuration")).and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(750),
            storm: {
                let number = |key: &str, default: i32| get(key).and_then(|v| v.parse::<i32>().ok()).unwrap_or(default);
                ra2_sim::supers::StormRules {
                    deferment: number("LightningDeferment", 250) as u32,
                    duration: number("LightningStormDuration", 180) as u32,
                    damage: number("LightningDamage", 250),
                    warhead: rules.map_or(0, |rules| armoury.borrow_mut().warhead_named(rules, &get("LightningWarhead").unwrap_or_else(|| "IonWH".to_owned()))),
                    hit_delay: number("LightningHitDelay", 10) as u32,
                    scatter_delay: number("LightningScatterDelay", 5) as u32,
                    spread: number("LightningCellSpread", 10),
                    separation: number("LightningSeparation", 3),
                    clouds: visual_list("WeatherConClouds").iter().map(|name| anim_life(name)).map(|(life, half)| (half, life)).collect(),
                    bolts: visual_list("WeatherConBolts").len() as u32,
                    bolt_explosion: None,
                }
            },
            nuke_weapon: rules.and_then(|rules| armoury.borrow_mut().weapon_named(rules, "NukePayload")),
            nuke_ball: (anim_life("NUKEBALL").0, None),
            chrono_delay: get("ChronoDelay").and_then(|v| v.parse().ok()).unwrap_or(60),
        }
    };
    // DATA: ai.ini, the computer player's triggers, teams and scripts.
    let team_data = match (files.get("ai.ini").map(ra2_formats::ini::Ini::parse), object_rules.rules.as_ref()) {
        (Some(ai), Some(rules)) => crate::aidata::load(
            &ai,
            rules,
            &|name: &str| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16),
            &|name: &str| building_index(name),
            &|name: &str| countries.iter().position(|c| c.eq_ignore_ascii_case(name.trim())).map(|i| i as u8),
        ),
        _ => ra2_sim::teams::Data::default(),
    };
    let rad_rules = object_rules.rules.as_ref().map(|rules| armoury.borrow_mut().radiation(rules)).unwrap_or_default();
    let rockets = object_rules
        .rules
        .as_ref()
        .map(|rules| armoury.borrow_mut().rocket_rules(rules, &|name: &str| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16)))
        .unwrap_or_default();
    let ability_rules = object_rules
        .rules
        .as_ref()
        .map(|rules| {
            armoury.borrow_mut().ability_rules(
                rules,
                &|name: &str| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(name.trim())).map(|i| i as u16),
                &|name: &str| building_index(name),
            )
        })
        .unwrap_or_default();
    let armoury = armoury.borrow().clone();
    // DATA: `[General]` keys for repairs, refunds and crews.
    // How each value is read.
    let upkeep = {
        let rules = object_rules.rules.as_ref();
        let get = |key: &str| rules.and_then(|r| r.get("General", key)).map(|v| v.trim().to_owned());
        let share = |key: &str, default: f64| {
            get(key).and_then(|v| if v.contains('%') { v.trim_end_matches('%').parse::<f64>().ok().map(|p| p / 100.0) } else { v.parse::<f64>().ok() }).unwrap_or(default)
        };
        let unit = |key: &str| get(key).and_then(|name| vehicle_names.iter().position(|n| n.eq_ignore_ascii_case(&name))).map(|i| i as u16);
        Upkeep {
            refund_percent: share("RefundPercent", 0.5),
            repair_percent: share("RepairPercent", 0.15),
            repair_step: get("RepairStep").and_then(|v| v.parse().ok()).unwrap_or(8),
            repair_interval: (share("RepairRate", 0.016) * 900.0) as u32,
            survivor_divisors: (
                get("AlliedSurvivorDivisor").and_then(|v| v.parse().ok()).unwrap_or(500),
                get("SovietSurvivorDivisor").and_then(|v| v.parse().ok()).unwrap_or(250),
            ),
            crew_types: (unit("AlliedCrew"), unit("SovietCrew"), unit("Engineer"), unit("Technician")),
        }
    };

    let general = |key: &str| object_rules.rules.as_ref().and_then(|rules| rules.get("General", key)).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
    let crew_escape = general("CrewEscape").and_then(|v| if v.contains('%') { v.trim_end_matches('%').parse::<f64>().ok().map(|p| p / 100.0) } else { v.parse::<f64>().ok() }).unwrap_or(0.5);
    let ship_sinking_weight = general("ShipSinkingWeight").and_then(|v| v.parse::<f64>().ok()).unwrap_or(3.0);
    Some(Scene {
        crew_escape,
        ship_sinking_weight,
        light_setup,
        chrono,
        upkeep,
        armoury,
        combat_rules,
        elevation,
        ballistic_scatter,
        radar_ground,
        ore,
        mining,
        infantry_spawns,
        unit_types,
        walls,
        wall_art,
        bridge_art,
        bridge_rules,
        anim_types,
        map_structures,
        condition_yellow,
        condition_red,
        idle_frequency,
        slope_speed: ["TrackedUphill", "TrackedDownhill", "WheeledUphill", "WheeledDownhill"].map(|key| object_rules.rules.as_ref().and_then(|rules| rules.get("General", key)).and_then(|v| v.trim().parse().ok()).unwrap_or(1.0)),
        enemies,
        house_palettes,
        ability_rules,
        veterancy: object_rules.rules.as_ref().map(crate::armoury::Armoury::veterancy).unwrap_or_default(),
        team_data,
        rockets,
        rad_rules,
        super_types,
        super_rules,
        bolt_explosion: object_rules.rules.as_ref().and_then(|rules| rules.get("AudioVisual", "WeatherConBoltExplosion")).map(|v| v.trim().to_uppercase()).filter(|v| !v.is_empty()),
        crate_rules,
        smudge_types,
        map_smudges,
        tile_anims,
        ai_rules,
        base_units: Vec::new(),
        player_country: player.country as u8,
        player_side,
        tech_level,
        buildup_ticks,
        playfield,
        mcv_repacks,
        allies_allowed,
        building_types,
        build_rules,
        player_palette,
        player_start,
        land_rules,
        blockage_delay,
        sight_height_step,
        cells,
        unit_spawns,
        unit_art,
        origin: (-min_x, -min_y),
        width: (max_x - min_x) as u32,
        height: (max_y - min_y) as u32,
        pages: atlas.pages,
        entries: atlas.entries,
        placements,
        animations,
        missing_tiles,
        ore_cells,
        terrain_objects,
        structures,
        infantry: map.infantry.len(),
        units: map.units.len(),
        missing_objects,
        missing_art,
    })
}
