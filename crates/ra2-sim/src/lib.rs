//! The deterministic game simulation. It has no dependencies, uses only integer
//! arithmetic, and changes only through `World::order` and `World::tick`, so the
//! same orders on the same ticks always give the same `World::hash`.

pub mod abilities;
pub mod ai;
pub mod aircraft;
pub mod bridges;
pub mod combat;
pub mod flight;
pub mod path;
pub mod radiation;
pub mod remains;
pub mod ore;
pub mod smudge;
pub mod snapshot;
pub mod spawns;
mod soldiers;
pub mod endgame;
pub mod supers;
pub mod powerups;
pub mod teams;
pub mod zones;

use path::{DIRECTIONS, Mode, PATH_LENGTH, code};

/// World distance units per cell edge.
pub const LEPTONS_PER_CELL: i32 = 256;

/// Ticks per second the simulation is designed to run at.
pub const TICKS_PER_SECOND: u32 = 15;

/// Speed a unit must save up to advance one point along its track.
// The drive locomotor adds the unit's speed to a
// running total each tick and advances one track point for every 7 while the total
// is above 7, carrying the remainder over.
const SPEED_PER_TRACK_POINT: i32 = 7;

/// Leptons between track points when driving straight along a cell edge direction.
const STRAIGHT_POINT: i32 = 11;
/// Leptons along each axis between track points when driving diagonally.
const DIAGONAL_POINT: i32 = 8;

/// The speed the engine stores for a rules.ini `Speed` value.
// `Speed` is capped at 100, multiplied by 256, divided
// by 100, and capped at 255.
pub fn stored_speed(rules_speed: i32) -> i32 {
    (rules_speed.clamp(0, 100) * 256 / 100).min(255)
}

/// The kinds of ground rules.ini gives movement speeds for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Land {
    Clear,
    Road,
    Water,
    Rock,
    Rough,
    Beach,
    Ice,
    Railroad,
    Tunnel,
    Tiberium,
    Weeds,
    Wall,
}

pub const LAND_COUNT: usize = 12;

/// A house's bit in a per-cell mask.
fn house_bit(house: u8) -> u32 {
    1u32.checked_shl(house as u32).unwrap_or(0)
}

/// How a unit type gets about, which decides its speed over each kind of ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum SpeedType {
    Foot,
    Track,
    Wheel,
    Hover,
    Float,
    FloatBeach,
    Amphibious,
}

pub const SPEED_TYPE_COUNT: usize = 7;

/// Percentage of full speed for each speed type on each kind of ground, from the
/// land sections of rules.ini. Zero means the ground cannot be entered. The second
/// table is each land's `Buildable`.
#[derive(Clone)]
pub struct LandRules(pub [[u8; SPEED_TYPE_COUNT]; LAND_COUNT], pub [bool; LAND_COUNT]);

impl LandRules {
    /// Every kind of ground at full speed for everything, and buildable.
    pub fn open() -> Self {
        Self([[100; SPEED_TYPE_COUNT]; LAND_COUNT], [true; LAND_COUNT])
    }
}

/// Ticks after first being blocked by moving units before a unit paths round all of
/// them.
// DATA: rules.ini `[General] BlockagePathDelay`.
pub const DEFAULT_BLOCKAGE_DELAY: i32 = 60;
/// Ticks between attempts to find a path.
// DATA: rules.ini `[General] PathDelay` is .01 minutes; the engine multiplies by 900.
pub const DEFAULT_PATH_DELAY: i32 = 9;
/// Leptons from its destination at which a blocked unit counts as having arrived.
// DATA: rules.ini `[General] CloseEnough` is 2.25 cells.
pub const DEFAULT_CLOSE_ENOUGH: i32 = 576;

/// The terrain the simulation needs: which cells ground units can enter, and how
/// high each is.
mod tracks;

pub use tracks::TrackTables;

pub struct Map {
    width: i32,
    height: i32,
    passable: Vec<bool>,
    level: Vec<u8>,
    land: Vec<u8>,
    /// The tile's slope number; zero is flat.
    ramp: Vec<u8>,
    /// A high bridge over the cell: bit 0 the bridge (the engine's cell flag
    /// 0x100), bit 1 a deck that can be driven on (0x200); and for a deck cell
    /// the index, plus one, of a ground cell at an end of its bridge.
    bridge: Vec<u8>,
    bridge_end: Vec<u32>,
    /// The bridge overlay on each cell (a low bridge's section, a high
    /// bridge's owner cell) with its state, as the map first had it, the
    /// cell's land without it, and a count of changes for whoever draws them.
    pub(crate) bridge_kind: Vec<u8>,
    pub(crate) bridge_data: Vec<u8>,
    pub(crate) bridge_home: Vec<u8>,
    pub(crate) base_land: Vec<u8>,
    pub bridge_version: u64,
    /// Whether an overlay (ore, wall, bridge, rubble) lies on the cell.
    overlay: Vec<bool>,
    /// Ore: its kind plus one (zero for none) and how much, 0 to 11, on each cell;
    /// the land under it; and a count that goes up whenever any of it changes.
    ore_kind: Vec<u8>,
    /// Which of its kind's twelve pictures ore that grew here shows, plus one;
    /// zero for ore the map began with.
    ore_picture: Vec<u8>,
    /// Whether the cell's tile is of a set that lets ore lie on it.
    ore_ground: Vec<bool>,
    ore_amount: Vec<u8>,
    ore_under: Vec<u8>,
    pub ore_version: u64,
    /// The mark on each cell (its type plus one) and which part of it this is.
    smudge: Vec<u16>,
    smudge_part: Vec<u8>,
    pub smudge_types: Vec<smudge::SmudgeType>,
    pub smudge_version: u64,
    /// What stands on the cell, as far as it decides how the cell is classed.
    obstacle: Vec<u8>,
    pub(crate) tree_bits: Vec<u8>,
    /// Wall pieces: the overlay type on each cell (or [`NO_WALL`]), its data
    /// (damage level times 16 plus connection bits), and its owner.
    wall_kind: Vec<u16>,
    wall_data: Vec<u8>,
    wall_owner: Vec<u8>,
    /// The cells that have wall pieces, in the order they were made.
    wall_cells: Vec<usize>,
    /// Goes up whenever a wall piece appears or changes.
    pub walls_version: u64,
    /// The map's width in the map file's terms and its visible rectangle, which
    /// bound the playable area.
    playfield: Option<(i32, [i32; 4])>,
    /// The map file's `Size` height, beside the width the playfield keeps.
    size_height: i32,
    /// Cells a route may use even off its corridor, for what stands beside them.
    exempt: Vec<bool>,
    zones: Option<zones::Zones>,
    /// The shroud, for the one player who has one: whether each cell has been
    /// seen, whether there is a shroud at all, and a count of changes.
    /// Each cell's bit for every house that has seen it.
    revealed: Vec<u32>,
    /// The house whose shroud the plain questions (`revealed`, `shrouded`) are
    /// about: the one this machine's player sits at. Not part of the game's
    /// state: each machine of a network game has its own.
    pub viewer: u8,
    /// Whether a spy satellite is showing the player everything.
    pub satellite: u32,
    /// How many enemy gap generators were at work when last looked.
    pub gaps: [u16; 32],
    pub shroud_on: bool,
    pub shroud_version: u64,
    pub lands: LandRules,
    /// `TrackedUphill`, `TrackedDownhill`, `WheeledUphill`, `WheeledDownhill`,
    /// and `ConditionYellow`: what a vehicle's speed is multiplied by going up
    /// or down, and the health at or under which it goes at three quarters.
    pub slope_speed: [f64; 4],
    pub condition_yellow: f64,
}

/// No wall on a cell.
pub const NO_WALL: u16 = u16::MAX;

/// What stands on a cell, as far as it decides the cell's passability class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Obstacle {
    None,
    /// An overlay vehicles can drive over and flatten.
    Crushable,
    /// A wall overlay.
    Wall,
    /// An overlay nothing can cross.
    Rock,
    /// Rubble: an overlay that leaves the cell clear whatever its land.
    Rubble,
    /// A terrain object that fills its cell, and one that does not.
    TreeFull,
    Tree,
    /// A building placed by the map.
    Building,
}

impl Map {
    /// An empty map covering cells `0..width` by `0..height`, all impassable.
    pub fn new(width: i32, height: i32) -> Self {
        let cells = (width.max(0) * height.max(0)) as usize;
        Self {
            width,
            height,
            passable: vec![false; cells],
            level: vec![0; cells],
            land: vec![Land::Clear as u8; cells],
            ramp: vec![0; cells],
            bridge: vec![0; cells],
            bridge_end: vec![0; cells],
            bridge_kind: vec![bridges::NONE; cells],
            bridge_data: vec![0; cells],
            bridge_home: vec![bridges::NONE; cells],
            base_land: vec![0; cells],
            bridge_version: 0,
            overlay: vec![false; cells],
            ore_kind: vec![0; cells],
            ore_picture: vec![0; cells],
            ore_ground: vec![true; cells],
            ore_amount: vec![0; cells],
            ore_under: vec![0; cells],
            ore_version: 0,
            smudge: vec![0; cells],
            smudge_part: vec![0; cells],
            smudge_types: Vec::new(),
            smudge_version: 0,
            obstacle: vec![Obstacle::None as u8; cells],
            tree_bits: vec![0; cells],
            wall_kind: vec![NO_WALL; cells],
            wall_data: vec![0; cells],
            wall_owner: vec![u8::MAX; cells],
            wall_cells: Vec::new(),
            walls_version: 0,
            playfield: None,
            size_height: 0,
            exempt: vec![false; cells],
            zones: None,
            revealed: vec![0; cells],
            viewer: 1,
            satellite: 0,
            gaps: [0; 32],
            shroud_on: false,
            shroud_version: 0,
            lands: LandRules::open(),
            // DATA: rules.ini `[General]` and `[AudioVisual]`.
            slope_speed: [1.0, 1.2, 1.0, 1.2],
            condition_yellow: 0.5,
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.width && y < self.height)
            .then(|| (y * self.width + x) as usize)
    }

    pub fn set_cell(&mut self, x: i32, y: i32, passable: bool, level: u8) {
        if let Some(index) = self.index(x, y) {
            self.passable[index] = passable;
            self.level[index] = level;
        }
    }

    pub fn set_land(&mut self, x: i32, y: i32, land: Land) {
        if let Some(index) = self.index(x, y) {
            self.land[index] = land as u8;
        }
    }

    /// Records a cell's slope number and whether it has an overlay on it.
    /// Puts a high bridge over a cell: bit 0 the bridge, bit 1 a deck to drive on.
    pub fn set_bridge(&mut self, x: i32, y: i32, flags: u8) {
        if let Some(index) = self.index(x, y) {
            self.bridge[index] = flags & 3;
        }
    }

    /// Puts a bridge overlay on a cell: its type and state, and the land the
    /// cell has without it.
    pub fn set_bridge_overlay(&mut self, x: i32, y: i32, kind: u8, data: u8, base_land: Land) {
        if let Some(index) = self.index(x, y) {
            self.bridge_kind[index] = kind;
            self.bridge_home[index] = kind;
            self.bridge_data[index] = data;
            self.base_land[index] = base_land as u8;
            self.bridge_version += 1;
        }
    }

    /// Works out where the sections of high bridges that the map starts with
    /// already fallen would stand, so that they can be mended. Returns how
    /// many it found.
    // Not original behaviour: the engine knows a bridge's whole run from its
    // bridge-head tiles and the links between them.
    // Here each standing section's line is followed along the bridge, over
    // ground lower than the deck, to the bank; cells on it with no section are
    // remembered as ones that had one.
    pub fn complete_bridges(&mut self) -> usize {
        let mut found = 0;
        for index in 0..self.bridge_home.len() {
            let kind = self.bridge_home[index];
            if !matches!(kind, 24 | 25 | 237 | 238) || self.bridge_kind[index] != kind {
                continue;
            }
            let owner = (index as i32 % self.width, index as i32 / self.width);
            let deck = self.level(owner.0, owner.1) as i32 + 4;
            let along = if matches!(kind, 25 | 238) { (0, 1) } else { (1, 0) };
            for way in [1, -1] {
                let mut gap = Vec::new();
                let mut reached = false;
                for step in 1..80 {
                    let cell = (owner.0 + along.0 * step * way, owner.1 + along.1 * step * way);
                    let Some(at) = self.index(cell.0, cell.1) else {
                        break;
                    };
                    if self.level(cell.0, cell.1) as i32 >= deck {
                        reached = true;
                        break;
                    }
                    if self.bridge_home[at] == bridges::NONE {
                        gap.push(at);
                    }
                }
                if reached {
                    for at in gap {
                        self.bridge_home[at] = kind;
                        found += 1;
                    }
                }
            }
        }
        found
    }

    /// The cells where a bridge section belongs and none stands.
    pub fn fallen_bridge_cells(&self) -> Vec<(i32, i32)> {
        (0..self.bridge_home.len())
            .filter(|&index| self.bridge_home[index] != bridges::NONE && self.bridge_kind[index] == bridges::NONE)
            .map(|index| (index as i32 % self.width, index as i32 / self.width))
            .collect()
    }

    /// The bridge overlays now on the map: cell, type, state.
    pub fn bridge_overlays(&self) -> impl Iterator<Item = ((i32, i32), u8, u8)> + '_ {
        let width = self.width;
        self.bridge_kind.iter().enumerate().filter(|(_, kind)| **kind != bridges::NONE).map(move |(index, &kind)| (((index as i32) % width, (index as i32) / width), kind, self.bridge_data[index]))
    }

    /// Whether a high bridge passes over a cell.
    pub fn bridged(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.bridge[index] & 1 != 0)
    }

    /// Whether the cell has a bridge deck that can be driven on.
    pub fn deck(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.bridge[index] & 2 != 0)
    }

    /// The speed a kind of mover has on a cell, on its deck or on the ground.
    // On a deck the land under it is
    // not asked about.
    pub fn speed_on(&self, x: i32, y: i32, speed_type: SpeedType, deck: bool) -> i32 {
        if deck && self.deck(x, y) {
            self.lands.0[Land::Road as usize][speed_type as usize] as i32
        } else {
            self.speed_percent(x, y, speed_type)
        }
    }

    /// The cell a route's corridor starts or ends at for a cell: on a deck,
    /// the nearer end of its bridge.
    pub fn corridor_cell(&self, cell: (i32, i32), deck: bool) -> (i32, i32) {
        self.index(cell.0, cell.1)
            .filter(|_| deck)
            .map(|index| self.bridge_end[index])
            .filter(|&end| end != 0)
            .map_or(cell, |end| (((end - 1) as i32) % self.width, ((end - 1) as i32) / self.width))
    }

    /// The zone of a cell for a movement zone, on its deck or on the ground.
    // A deck cell is in the zone
    // of its bridge's end.
    pub fn zone_on(&self, cell: (i32, i32), movement_zone: usize, deck: bool) -> Option<u16> {
        let zones = self.zones.as_ref()?;
        let end = self.index(cell.0, cell.1).filter(|_| deck).map(|index| self.bridge_end[index]).filter(|&end| end != 0);
        Some(match end {
            Some(end) => zones.zone_of((((end - 1) as i32) % self.width, ((end - 1) as i32) / self.width), movement_zone),
            None => zones.zone_of(cell, movement_zone),
        })
    }

    /// Finds each bridge's ends and joins the zones at them.
    // The map keeps a link for each
    // bridge between the cells at its two ends, and what can be reached counts
    // the link. Not original behaviour: the link is found from the deck's cells
    // (the engine walks from the bridge-head tiles), and joins the zones for
    // good (the engine tests the link as it goes).
    fn link_bridges(&mut self) {
        let (width, height) = (self.width, self.height);
        self.bridge_end.iter_mut().for_each(|end| *end = 0);
        let mut seen = vec![false; self.bridge.len()];
        for start in 0..self.bridge.len() {
            if self.bridge[start] & 2 == 0 || seen[start] {
                continue;
            }
            let mut cells = vec![start];
            seen[start] = true;
            let mut at = 0;
            let mut ends: Vec<usize> = Vec::new();
            while at < cells.len() {
                let (x, y) = ((cells[at] as i32) % width, (cells[at] as i32) / width);
                let level = self.level[cells[at]];
                at += 1;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= width || ny >= height {
                        continue;
                    }
                    let next = (ny * width + nx) as usize;
                    if self.bridge[next] & 2 != 0 {
                        if !seen[next] {
                            seen[next] = true;
                            cells.push(next);
                        }
                    } else if self.bridge[next] == 0 && self.level[next] == level + 4 && self.passable[next] && !ends.contains(&next) {
                        ends.push(next);
                    }
                }
            }
            let Some(&first) = ends.first() else {
                continue;
            };
            // Each deck cell answers for the nearer of its bridge's ends.
            let distance = |a: usize, b: usize| {
                let (dx, dy) = ((a as i32 % width - b as i32 % width) as f64, (a as i32 / width - b as i32 / width) as f64);
                (dx * dx + dy * dy).sqrt() as i32
            };
            for cell in cells {
                let nearest = ends.iter().copied().min_by_key(|&end| distance(cell, end)).unwrap_or(first);
                self.bridge_end[cell] = nearest as u32 + 1;
            }
            if let Some(zones) = self.zones.as_mut() {
                let cell_of = |index: usize| ((index as i32) % width, (index as i32) / width);
                for &end in &ends[1..] {
                    zones.link(cell_of(first), cell_of(end));
                }
            }
        }
    }

    pub fn set_ground(&mut self, x: i32, y: i32, ramp: u8, overlay: bool) {
        if let Some(index) = self.index(x, y) {
            self.ramp[index] = ramp;
            self.overlay[index] = overlay;
        }
    }

    /// Whether ore may come to lie on a cell's tile (`AllowTiberium`).
    pub fn ore_ground(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.ore_ground[index])
    }

    pub fn set_ore_ground(&mut self, x: i32, y: i32, allowed: bool) {
        if let Some(index) = self.index(x, y) {
            self.ore_ground[index] = allowed;
        }
    }

    /// The picture of ore that grew on a cell during play: 0 to 11.
    pub fn ore_picture(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).and_then(|index| self.ore_picture[index].checked_sub(1))
    }

    pub(crate) fn set_ore_picture(&mut self, x: i32, y: i32, picture: u8) {
        if let Some(index) = self.index(x, y) {
            self.ore_picture[index] = picture + 1;
        }
    }

    /// Whether the land of a cell is of a kind that can be built on.
    pub fn land_buildable(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.lands.1[(self.land[index] as usize).min(LAND_COUNT - 1)])
    }

    /// Puts ore of a kind on a cell, 0 to 11 deep. The cell's land becomes ore.
    pub fn put_ore(&mut self, x: i32, y: i32, kind: u8, amount: u8) {
        if let Some(index) = self.index(x, y) {
            if self.ore_kind[index] == 0 {
                self.ore_under[index] = self.land[index];
            }
            self.ore_kind[index] = kind + 1;
            self.ore_amount[index] = amount.min(11);
            self.land[index] = Land::Tiberium as u8;
            self.overlay[index] = true;
            self.ore_version += 1;
        }
    }

    /// Whether any overlay lies on a cell.
    /// Whether a cell has been seen (always, with no shroud).
    /// Covers the whole map again.
    pub(crate) fn reshroud(&mut self) {
        self.revealed.iter_mut().for_each(|seen| *seen = 0);
        self.shroud_version += 1;
    }

    /// Puts a cell back under a house's shroud.
    pub(crate) fn hide(&mut self, house: u8, cell: (i32, i32)) {
        if let Some(index) = self.index(cell.0, cell.1) {
            if self.revealed[index] & house_bit(house) != 0 {
                self.revealed[index] &= !house_bit(house);
                self.shroud_version += 1;
            }
        }
    }

    /// Shows or hides the whole map for a house.
    pub(crate) fn reveal_all(&mut self, house: u8, seen: bool) {
        let bit = house_bit(house);
        self.revealed.iter_mut().for_each(|cell| if seen { *cell |= bit } else { *cell &= !bit });
        self.shroud_version += 1;
    }

    /// Whether the viewer has a spy satellite up.
    pub fn has_satellite(&self) -> bool {
        self.satellite & house_bit(self.viewer) != 0
    }

    /// Whether the viewer has seen a cell (always, with no shroud).
    pub fn revealed(&self, x: i32, y: i32) -> bool {
        self.revealed_to(self.viewer, x, y)
    }

    /// Whether a house has seen a cell.
    pub fn revealed_to(&self, house: u8, x: i32, y: i32) -> bool {
        !self.shroud_on || self.index(x, y).is_some_and(|index| self.revealed[index] & house_bit(house) != 0)
    }

    /// Whether a point on the ground of a cell is under the viewer's shroud.
    pub fn shrouded(&self, x: i32, y: i32) -> bool {
        self.shrouded_to(self.viewer, x, y)
    }

    /// Whether a point on the ground of a cell is under a house's shroud.
    // The shroud lies flat at height 0, so the cell
    // asked about is the one the point is drawn over: back by half the height
    // level; at an odd level, back one more, and seen if that cell or the next
    // one down the screen is.
    pub fn shrouded_to(&self, house: u8, x: i32, y: i32) -> bool {
        if !self.shroud_on {
            return false;
        }
        let level = self.level(x, y) as i32;
        if level % 2 == 0 {
            !self.revealed_to(house, x - level / 2, y - level / 2)
        } else {
            let back = level / 2 + 1;
            !self.revealed_to(house, x - back, y - back) && !self.revealed_to(house, x - back + 1, y - back + 1)
        }
    }

    /// Lifts a house's shroud round a cell.
    // The cells of the spread table's
    // rings out to the radius, at most 10, round the cell the viewer is drawn
    // over (its own, back by half its height level).
    pub fn reveal(&mut self, house: u8, cell: (i32, i32), level: i32, radius: i32) {
        if !self.shroud_on || radius <= 0 {
            return;
        }
        let bit = house_bit(house);
        let centre = (cell.0 - level / 2, cell.1 - level / 2);
        let mut changed = false;
        for (dx, dy) in combat::spread_offsets(radius.min(10)) {
            // High ground hides what is behind it: a cell is not seen when
            // the cell one step back towards the viewer, taken from the
            // viewer's own cell moved two cells down, is more than three
            // levels above the viewer.
            let (sx, sy) = sight_step(dx, dy);
            let (tx, ty) = (cell.0 + dx + 2 + sx, cell.1 + dy + 2 + sy);
            if self.index(tx, ty).is_some() && self.level(tx, ty) as i32 > level + 3 {
                continue;
            }
            if let Some(index) = self.index(centre.0 + dx, centre.1 + dy) {
                if self.revealed[index] & bit == 0 {
                    self.revealed[index] |= bit;
                    changed = true;
                }
            }
        }
        if changed {
            self.shroud_version += 1;
        }
    }

    /// Whether a cell is water or beach.
    pub fn wet(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.land[index] == Land::Water as u8 || self.land[index] == Land::Beach as u8)
    }

    /// The movement zone a cell is in for a kind of mover (0 without zones).
    pub fn zone_at(&self, cell: (i32, i32), movement_zone: usize) -> u16 {
        self.zones.as_ref().map_or(0, |zones| zones.zone_of(cell, movement_zone))
    }

    pub fn has_overlay(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.overlay[index])
    }

    /// The ore on a cell: its kind and how much (0 to 11).
    pub fn ore_at(&self, x: i32, y: i32) -> Option<(u8, u8)> {
        let index = self.index(x, y)?;
        (self.ore_kind[index] != 0).then(|| (self.ore_kind[index] - 1, self.ore_amount[index]))
    }

    /// Every cell with ore on it, with its kind and amount.
    pub fn ore_cells(&self) -> impl Iterator<Item = ((i32, i32), u8, u8)> + '_ {
        self.ore_kind.iter().enumerate().filter(|(_, kind)| **kind != 0).map(|(index, &kind)| {
            ((index as i32 % self.width, index as i32 / self.width), kind - 1, self.ore_amount[index])
        })
    }

    /// Takes bales of ore off a cell and returns how many came.
    // A cell holds its amount plus one; taking the last
    // removes the overlay and the land goes back to what lies under it.
    fn reduce_ore(&mut self, x: i32, y: i32, amount: u32) -> u32 {
        let Some(index) = self.index(x, y) else {
            return 0;
        };
        if amount == 0 || self.ore_kind[index] == 0 {
            return 0;
        }
        self.ore_version += 1;
        let held = self.ore_amount[index] as u32 + 1;
        if amount < held {
            self.ore_amount[index] -= amount as u8;
            return amount;
        }
        self.ore_kind[index] = 0;
        self.ore_picture[index] = 0;
        self.ore_amount[index] = 0;
        self.overlay[index] = false;
        self.land[index] = self.ore_under[index];
        held
    }

    /// Every wall piece: its cell, overlay type and data (the frame to draw).
    /// The house a cell's wall belongs to, if it has one that a house built.
    pub fn wall_owner(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).map(|index| self.wall_owner[index]).filter(|&owner| owner != u8::MAX && self.is_wall((x, y)))
    }

    pub fn walls(&self) -> impl Iterator<Item = ((i32, i32), u16, u8)> + '_ {
        self.wall_cells.iter().map(|&index| {
            let cell = (index as i32 % self.width, index as i32 / self.width);
            (cell, self.wall_kind[index], self.wall_data[index])
        })
    }

    /// Writes a wall piece into a cell.
    fn put_wall(&mut self, cell: (i32, i32), overlay: u16, data: u8, owner: u8) {
        let Some(index) = self.index(cell.0, cell.1) else {
            return;
        };
        if self.wall_kind[index] == NO_WALL {
            self.wall_cells.push(index);
        }
        self.wall_kind[index] = overlay;
        self.wall_data[index] = data;
        self.wall_owner[index] = owner;
        self.passable[index] = false;
        self.overlay[index] = true;
        self.obstacle[index] = Obstacle::Wall as u8;
        self.walls_version += 1;
    }

    /// Whether a cell holds a wall piece.
    pub fn is_wall(&self, cell: (i32, i32)) -> bool {
        self.index(cell.0, cell.1).is_some_and(|index| self.wall_kind[index] != NO_WALL)
    }

    /// Takes a wall piece off its cell.
    fn clear_wall(&mut self, cell: (i32, i32)) {
        let Some(index) = self.index(cell.0, cell.1) else {
            return;
        };
        if self.wall_kind[index] == NO_WALL {
            return;
        }
        self.wall_kind[index] = NO_WALL;
        self.wall_data[index] = 0;
        self.wall_owner[index] = u8::MAX;
        self.wall_cells.retain(|&at| at != index);
        self.passable[index] = true;
        self.overlay[index] = false;
        self.obstacle[index] = Obstacle::None as u8;
        self.walls_version += 1;
    }

    /// A hit that takes on a wall piece. Returns the cells cleared.
    // Each taking hit adds a damage
    // level; on reaching the last, each same-type neighbour not yet damaged
    // is weakened too (the caller gives it its 200-point hit), and the piece
    // stands only while it is joined to another; a further hit, or losing
    // its joins, removes it.
    // DATA: three damage levels (`DamageLevels` of the stock walls).
    fn hit_wall(&mut self, cell: (i32, i32)) -> (Vec<(i32, i32)>, Vec<(i32, i32)>) {
        const LEVELS: u8 = 3;
        const SIDES: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        let (mut cleared, mut shaken) = (Vec::new(), Vec::new());
        let Some(index) = self.index(cell.0, cell.1) else {
            return (cleared, shaken);
        };
        let kind = self.wall_kind[index];
        if kind == NO_WALL {
            return (cleared, shaken);
        }
        let level = self.wall_data[index] >> 4;
        if level + 1 < LEVELS {
            self.wall_data[index] = ((level + 1) << 4) | (self.wall_data[index] & 0x0f);
            self.walls_version += 1;
            if level + 2 == LEVELS {
                for side in SIDES {
                    let next = (cell.0 + side.0, cell.1 + side.1);
                    if self.index(next.0, next.1).is_some_and(|at| self.wall_kind[at] == kind && self.wall_data[at] >> 4 == 0) {
                        shaken.push(next);
                    }
                }
                if self.wall_data[index] & 0x0f == 0 {
                    self.clear_wall(cell);
                    cleared.push(cell);
                }
            }
        } else {
            self.clear_wall(cell);
            cleared.push(cell);
        }
        if !cleared.is_empty() {
            self.link_walls(cell);
            // Neighbours at their last level that have lost their joins go too.
            for side in SIDES {
                let next = (cell.0 + side.0, cell.1 + side.1);
                if self.index(next.0, next.1).is_some_and(|at| self.wall_kind[at] != NO_WALL && self.wall_data[at] >> 4 == LEVELS - 1 && self.wall_data[at] & 0x0f == 0) {
                    self.clear_wall(next);
                    cleared.push(next);
                }
            }
        }
        (cleared, shaken)
    }

    /// Works out afresh how a wall piece and its four neighbours join up.
    // Bit 1 for a piece at (0, -1), 2 at
    // (+1, 0), 4 at (0, +1), 8 at (-1, 0); a neighbour joins if it is the same
    // kind of wall, whoever owns it.
    fn link_walls(&mut self, cell: (i32, i32)) {
        const SIDES: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        for target in SIDES.iter().map(|side| (cell.0 + side.0, cell.1 + side.1)).chain([cell]) {
            let Some(index) = self.index(target.0, target.1) else {
                continue;
            };
            let kind = self.wall_kind[index];
            if kind == NO_WALL {
                continue;
            }
            let mut bits = 0;
            for (bit, side) in SIDES.iter().enumerate() {
                let joined = self
                    .index(target.0 + side.0, target.1 + side.1)
                    .is_some_and(|neighbour| self.wall_kind[neighbour] == kind);
                if joined {
                    bits |= 1 << bit;
                }
            }
            self.wall_data[index] = (self.wall_data[index] & 0xf0) | bits;
        }
        self.walls_version += 1;
    }

    pub(crate) fn obstacle_kind(&self, cell: (i32, i32)) -> u8 {
        self.index(cell.0, cell.1).map_or(Obstacle::None as u8, |index| self.obstacle[index])
    }

    pub fn set_obstacle(&mut self, x: i32, y: i32, obstacle: Obstacle) {
        if let Some(index) = self.index(x, y) {
            self.obstacle[index] = obstacle as u8;
        }
    }

    /// The spots of a cell that a tree takes up (its occupation bits).
    // A terrain
    // object whose bits are not all three shuts its cell to vehicles but not
    // to foot soldiers, who have the spots it leaves.
    pub fn set_tree_bits(&mut self, x: i32, y: i32, bits: u8) {
        if let Some(index) = self.index(x, y) {
            self.tree_bits[index] = bits & SPOT_BITS;
        }
    }

    /// The map file's `Size` width and `LocalSize`, if set.
    pub fn playfield(&self) -> Option<(i32, [i32; 4])> {
        self.playfield
    }

    pub fn size(&self) -> (i32, i32) {
        (self.width, self.passable.len() as i32 / self.width.max(1))
    }

    /// Sets the playable area from the map file's `Size` width and `LocalSize`.
    pub fn set_playfield(&mut self, width: i32, height: i32, local: [i32; 4]) {
        self.playfield = Some((width, local));
        self.size_height = height;
    }

    /// The side of the square round the whole diamond map, `Size` width plus
    /// height less one (); none without a map file.
    pub(crate) fn diamond_side(&self) -> Option<i32> {
        self.playfield.map(|(width, _)| width + self.size_height - 1)
    }

    /// The engine's count of cells in its per-cell arrays.
    // `(Size height + 4) x Size width x 2`.
    pub(crate) fn cell_array_size(&self) -> usize {
        match self.playfield {
            Some((width, _)) => ((self.size_height + 4) * width * 2).max(0) as usize,
            None => (self.width * self.height) as usize,
        }
    }

    /// Whether a cell is inside the playable area.
    // The test, with the cell's height taken into account.
    pub fn in_playfield(&self, x: i32, y: i32) -> bool {
        let Some(index) = self.index(x, y) else {
            return false;
        };
        let Some((w, [lx, ly, lw, lh])) = self.playfield else {
            return true;
        };
        let mut h = self.level[index] as i32;
        if self.ramp[index] != 0 && x + y < w + 4 + 2 * ly + h {
            h += 1;
        }
        w + 2 * ly + h < x + y
            && x + y <= w + 2 + 2 * (ly + lh) + h
            && x - y < 2 * (lx + lw) - w
            && y - x < w - 2 * lx
    }

    /// A cell's passability class.
    // The order of the tests. The land tests use the wheeled speed whatever is
    // asking.
    fn class(&self, x: i32, y: i32) -> u8 {
        use zones::class;
        let Some(index) = self.index(x, y) else {
            return class::OUTSIDE;
        };
        if !self.in_playfield(x, y) {
            return class::OUTSIDE;
        }
        let obstacle = self.obstacle[index];
        if obstacle == Obstacle::Crushable as u8 {
            return class::CRUSHABLE;
        } else if obstacle == Obstacle::Wall as u8 {
            return class::WALL;
        } else if obstacle == Obstacle::Rock as u8 {
            return class::IMPASSABLE;
        } else if obstacle == Obstacle::Rubble as u8 {
            return class::CLEAR;
        }
        let land = self.land[index];
        if land == Land::Water as u8 {
            return class::WATER;
        } else if land == Land::Beach as u8 {
            return class::BEACH;
        } else if self.lands.0[land as usize][SpeedType::Wheel as usize] == 0 {
            return class::IMPASSABLE;
        }
        if obstacle == Obstacle::TreeFull as u8 {
            class::WALL
        } else if obstacle == Obstacle::Tree as u8 {
            class::TREE
        } else if obstacle == Obstacle::None as u8 && !self.passable[index] {
            class::IMPASSABLE
        } else {
            class::CLEAR
        }
    }

    /// Works out the regions, zones and subzones, and which cells have something
    /// standing beside them. Call once every cell has been described.
    // A terrain object marks the eight cells
    // round it, and a building its own cells and the ring round them, as usable
    // off the corridor.
    pub fn build_zones(&mut self) {
        let (width, height) = (self.width, self.height);
        let mut classes = Vec::with_capacity(self.passable.len());
        let mut playfield = Vec::with_capacity(self.passable.len());
        for y in 0..height {
            for x in 0..width {
                classes.push(self.class(x, y));
                playfield.push(self.in_playfield(x, y));
                let index = (y * width + x) as usize;
                let obstacle = self.obstacle[index];
                let tree = obstacle == Obstacle::Tree as u8 || obstacle == Obstacle::TreeFull as u8;
                let building = obstacle == Obstacle::Building as u8;
                if tree || building {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if let Some(near) = self.index(x + dx, y + dy) {
                                if building || (dx, dy) != (0, 0) {
                                    self.exempt[near] = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        let levels = self.level.clone();
        // The fills recurse deeply on a large map.
        let built = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || zones::Zones::build(width, height, classes, levels, playfield))
            .ok()
            .and_then(|thread| thread.join().ok());
        self.zones = built;
        self.link_bridges();
    }

    /// Builds the zones again if the ground no longer matches them anywhere.
    /// Games saved before placed buildings marked their cells can hold
    /// patches of ground the zones count as solid where a building stood; a
    /// unit on one could not be sent anywhere. Returns whether they were built.
    pub(crate) fn mend_zones(&mut self) -> bool {
        let Some(zones) = &self.zones else {
            return false;
        };
        let stale = (0..self.height).any(|y| (0..self.width).any(|x| zones.class_at(x, y) != self.class(x, y)));
        if stale {
            self.build_zones();
        }
        stale
    }

    pub fn zones(&self) -> Option<&zones::Zones> {
        self.zones.as_ref()
    }

    /// The cell's slope number; zero is flat.
    pub fn ramp(&self, x: i32, y: i32) -> u8 {
        self.index(x, y).map_or(0, |index| self.ramp[index])
    }

    /// How far the ground at a point stands above its cell's level, in leptons
    /// of height: nothing on flat ground, up to a level on a slope.
    // Each slope is a plane over the cell, held between the cell's level and
    // one level up (two for the steep diagonals 13 to 16); 17 and 18 stand half
    // a level up.
    pub fn slope_z(&self, x: i32, y: i32) -> i32 {
        let ramp = self.ramp(x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        let level = combat::LEVEL_HEIGHT;
        let fx = x.rem_euclid(LEPTONS_PER_CELL) * level / LEPTONS_PER_CELL;
        let fy = y.rem_euclid(LEPTONS_PER_CELL) * level / LEPTONS_PER_CELL;
        let (plane, top) = match ramp {
            1 => (fx, level),
            2 => (fy, level),
            3 => (level - fx, level),
            4 => (level - fy, level),
            5 => (fx + fy - level, level),
            6 => (fy - fx, level),
            7 => (level - fx - fy, level),
            8 => (fx - fy, level),
            9 => (fx + fy, level),
            10 => (fy - fx + level, level),
            11 => (2 * level - fx - fy, level),
            12 => (fx - fy + level, level),
            13 => (fx + fy, 2 * level),
            14 => (fy - fx + level, 2 * level),
            15 => (2 * level - fx - fy, 2 * level),
            16 => (fx - fy + level, 2 * level),
            17..=20 => (level / 2, level),
            _ => (0, 0),
        };
        plane.clamp(0, top)
    }

    /// What is known of a cell's ground, for diagnostics: passable, overlay, slope
    /// number and land.
    pub fn describe(&self, x: i32, y: i32) -> (bool, bool, u8, u8) {
        self.index(x, y).map_or((false, false, 0, 0), |index| {
            (self.passable[index], self.overlay[index], self.ramp[index], self.land[index])
        })
    }

    /// Whether the ground of a cell could carry a building, leaving aside what
    /// stands on it.
    // No overlay of any kind, flat, and a land type with `Buildable=yes`.
    fn buildable(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| {
            self.passable[index]
                && !self.overlay[index]
                && self.ramp[index] == 0
                && self.lands.1[self.land[index] as usize]
        })
    }

    /// Percentage of its speed a unit of a speed type keeps in a cell; zero if it
    /// cannot enter.
    // DATA: the percentages.
    pub fn speed_percent(&self, x: i32, y: i32, speed_type: SpeedType) -> i32 {
        match self.index(x, y) {
            Some(index) if self.passable[index] || (speed_type == SpeedType::Foot && self.obstacle[index] == Obstacle::Tree as u8) => {
                self.lands.0[self.land[index] as usize][speed_type as usize] as i32
            }
            _ => 0,
        }
    }

    pub fn level(&self, x: i32, y: i32) -> u8 {
        self.index(x, y).map_or(0, |index| self.level[index])
    }

    pub fn passable(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.passable[index])
    }

    /// What stands on a cell and the spots a tree there takes, for diagnostics.
    #[doc(hidden)]
    pub fn obstacle_at(&self, x: i32, y: i32) -> (u8, u8) {
        self.index(x, y).map_or((0, 0), |index| (self.obstacle[index], self.tree_bits[index]))
    }

}

/// One, for the fractions the engine keeps as floating point. The simulation keeps
/// them as whole millionths so that it stays exact and repeatable.
pub const UNIT_FRACTION: i32 = 1_000_000;

/// The speed fraction a unit slows to as it nears its destination.
const ARRIVAL_FRACTION: i32 = 300_000;

/// How a unit type moves, from rules.ini.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnitStats {
    /// `Speed`.
    pub speed: i32,
    /// `ROT`, the turn rate.
    pub rot: i32,
    /// `Accelerates`.
    pub accelerates: bool,
    /// `AccelerationFactor`, in millionths.
    pub acceleration: i32,
    /// `DeaccelerationFactor`, in millionths.
    pub deceleration: i32,
    /// `SlowdownDistance`, in leptons.
    pub slowdown_distance: i32,
    /// `SpeedType`.
    pub speed_type: SpeedType,
    /// The building type the unit deploys into (`DeploysInto`), if it has one.
    pub deploys_into: Option<u16>,
    /// `Strength`: full health.
    pub strength: i32,
    /// `MovementZone`, as the engine numbers them (0 is Normal).
    pub movement_zone: u8,
    /// A foot soldier: walks from spot to spot, three to a cell.
    pub infantry: bool,
    /// `AllowedToStartInMultiplayer`, and whether it is one of `[General]
    /// BaseUnit`.
    pub allowed_to_start: bool,
    pub base_unit: bool,
    /// Its armour, weapons and how it fights.
    pub fighter: combat::Fighter,
    /// For a type with the jumpjet locomotor: how it flies.
    pub jet: Option<flight::JetStats>,
    /// The Fly locomotor's figures, for an aircraft.
    pub plane: Option<aircraft::PlaneStats>,
    /// `Harvester`, with `Storage` in bales, and `Teleporter`.
    pub harvester: bool,
    pub storage: u32,
    pub teleporter: bool,
    /// A tag for whoever draws the unit to tell its type by; the simulation does
    /// not look at it.
    pub kind: u16,
    /// `Spawns` and its numbers; `MissileSpawn`; the Rocket locomotor.
    pub spawns: Option<spawns::SpawnStats>,
    pub missile_spawn: bool,
    pub rocket: bool,
    /// `LeadershipRating`.
    pub leadership: i32,
    /// `Trainable`; `VeteranAbilities` and `EliteAbilities` as bits by ability
    /// number; `ElitePrimary` and `EliteSecondary`; `SelfHealing`.
    pub trainable: bool,
    /// `NoSpawnAlt`: drawn as another model while it has no spawn aboard.
    pub no_spawn_alt: bool,
    /// `Selectable=no`: the player cannot pick it.
    pub unselectable: bool,
    pub veteran_abilities: u32,
    pub elite_abilities: u32,
    pub elite_primary: Option<combat::Mount>,
    pub elite_secondary: Option<combat::Mount>,
    pub self_healing: bool,
    /// `Cloakable` with `CloakingSpeed` (frames a stage); `Sensors` and
    /// `SensorsSight` in cells.
    pub cloakable: bool,
    pub cloaking_speed: i32,
    pub sensors: bool,
    pub sensors_sight: i32,
    /// `Crusher`, `Crushable`; `Explodes`, `DeathWeapon` and
    /// `DeathWeaponDamageModifier`.
    pub crusher: bool,
    pub crushable: bool,
    pub explodes: bool,
    pub death_weapon: Option<u16>,
    pub death_percent: i32,
}

impl Default for UnitStats {
    // The type constructor's defaults for the acceleration keys.
    fn default() -> Self {
        Self {
            speed: 4,
            rot: 5,
            accelerates: true,
            acceleration: 30_000,
            deceleration: 2_000,
            slowdown_distance: 500,
            speed_type: SpeedType::Track,
            deploys_into: None,
            strength: 1,
            movement_zone: 0,
            infantry: false,
            fighter: combat::Fighter::default(),
            jet: None,
            plane: None,
            allowed_to_start: true,
            base_unit: false,
            harvester: false,
            storage: 0,
            teleporter: false,
            kind: 0,
            spawns: None,
            missile_spawn: false,
            rocket: false,
            leadership: 5,
            trainable: false,
            unselectable: false,
            no_spawn_alt: false,
            veteran_abilities: 0,
            elite_abilities: 0,
            elite_primary: None,
            elite_secondary: None,
            self_healing: false,
            cloakable: false,
            cloaking_speed: 7,
            sensors: false,
            sensors_sight: 0,
            crusher: false,
            crushable: false,
            explodes: false,
            death_weapon: None,
            death_percent: 100,
        }
    }
}

/// The facing a vehicle turns to before it becomes a building, and the one it has
/// when a building turns back into it.
// 0x80 for ordinary buildings.
const DEPLOY_FACING: u8 = 0x80;

/// A turn on the spot.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Turn {
    /// Facing being turned to, in 65536ths of a circle.
    desired: u16,
    /// Signed difference from the facing the turn started at.
    difference: i16,
    /// Ticks the turn takes, and how many are left.
    steps: i16,
    remaining: i16,
}

impl Turn {
    fn facing(facing: u8) -> Self {
        Self {
            desired: (facing as u16) << 8,
            ..Self::default()
        }
    }

    /// Starts turning from one facing to another.
    // The rate is `ROT` (at most 127) times
    // 256 per tick; a turn takes the difference divided by the rate, truncated, and
    // the facing moves linearly over those ticks.
    fn start(from: u16, to: u16, rot: i32) -> Self {
        let difference = to.wrapping_sub(from) as i16;
        let rate = rot.clamp(1, 127) * 256;
        let steps = (difference as i32).abs() / rate;
        Self {
            desired: to,
            difference,
            steps: steps as i16,
            remaining: steps as i16,
        }
    }

    fn current(&self) -> u16 {
        if self.steps < 1 || self.remaining < 1 {
            return self.desired;
        }
        let per_step = self.difference / self.steps;
        self.desired.wrapping_sub((per_step * self.remaining) as u16)
    }
}

/// A track being followed.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
struct Drive {
    /// Which selector entry chose the track.
    selector: usize,
    /// The next point to move to.
    index: usize,
    /// The cell whose centre the track's points are measured from, and ends at.
    head: (i32, i32),
    /// The cell a two-cell curve passes through on the way.
    via: Option<(i32, i32)>,
    /// The cell the track started in.
    from: (i32, i32),
    /// The speed fraction, in millionths, the unit's speed climbs or falls
    /// to on this track: the ground it runs onto, the slope and its health.
    target: i32,
}

/// A miner's missions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Job {
    #[default]
    Idle,
    Harvest,
    Enter,
    Unload,
}

/// What a miner is doing and carrying.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Work {
    pub job: Job,
    pub status: u8,
    /// The tick the mission next runs.
    next: u64,
    /// Bales aboard, by kind of ore.
    pub cargo: [u32; 4],
    /// The stage counter that times digging and dumping: its value, the ticks per
    /// step (zero when stopped) and the tick it last stepped.
    stage: u32,
    stage_rate: u32,
    stage_at: u64,
    /// The ore cell to come back to after unloading.
    pub archive: Option<(i32, i32)>,
    dumping: bool,
    /// The refinery it is in touch with.
    pub dock: Option<u32>,
    /// Digging, for whoever draws it.
    pub digging: bool,
    /// Set by a move order: on coming to rest on ore it starts to dig.
    settle: bool,
}

impl Work {
    pub fn load(&self) -> u32 {
        self.cargo.iter().sum()
    }

    fn step_stage(&mut self, now: u64) {
        while self.stage_rate > 0 && now - self.stage_at >= self.stage_rate as u64 {
            self.stage += 1;
            self.stage_at += self.stage_rate as u64;
        }
    }

    fn start_stage(&mut self, rate: u32, now: u64) {
        self.stage = 0;
        self.stage_rate = rate;
        self.stage_at = now;
    }
}

/// A ground unit.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Unit {
    pub id: u32,
    /// The house that owns it.
    pub owner: u8,
    /// Position in leptons.
    pub x: i32,
    pub y: i32,
    /// 0 to 255, clockwise from north (towards smaller y).
    pub facing: u8,
    /// The engine's stored speed; see [`stored_speed`].
    pub speed: i32,
    /// Health left, out of `stats.strength`.
    pub health: i32,
    /// Still joined to the factory it is leaving, and deaf to orders.
    pub tethered: bool,
    /// Just out of its factory this tick.
    left_factory: bool,
    /// Its factory's rally point, to go to once it has come to rest outside.
    rally: Option<(i32, i32)>,
    /// What a miner is doing.
    pub work: Work,
    /// Its target and weapon state.
    pub arms: combat::Arms,
    /// The tick until which it is out of action after a jump.
    pub warp_until: u64,
    /// For a parasite inside something: until when a hit on its host by
    /// somebody else would be the end of it if it had to leave. And for
    /// anything, until when it stays where it is and holds its fire.
    pub suppressed_until: u64,
    pub paralysed_until: u64,
    /// Held by a `Temporal` weapon: what is left of it to wear away.
    pub frozen: Option<i32>,
    /// The building it has been sent into.
    capture: Option<u32>,
    /// The cell it last looked round from, for the shroud.
    looked: Option<(i32, i32)>,
    /// The unit whose mind it holds, and for a held unit who holds it and the
    /// house it comes from.
    pub controls: Option<u32>,
    pub controlled_by: Option<(u32, u8)>,
    /// A drone or dog inside it; and for one that is inside something, the
    /// weapon it bites with and when it next does.
    pub parasite: Vec<Unit>,
    pub bite: Option<(u16, u64)>,
    /// Coming down under a parachute: how high it still is.
    /// Its own aircraft or missiles; for one of those, whose it is; and a
    /// rocket's flight.
    /// The team it is in; whether it has reached the team's gathering point;
    /// whether a team may take it; and until when the defenders it roused
    /// leave it be.
    /// Experience: 1 a veteran, 2 an elite; the rank last noticed; and
    /// whether the speed its rank brings has been given.
    /// Killed by its own `Suicide` weapon.
    pub died_by_suicide: bool,
    pub cloak: abilities::Cloak,
    pub veterancy: f64,
    pub shown_rank: u8,
    pub sped_up: bool,
    pub team: Option<u32>,
    pub initiated: bool,
    pub recruitable: bool,
    pub rescue_until: u64,
    pub spawner: Option<spawns::Spawner>,
    pub spawn_owner: Option<u32>,
    pub rocket: Option<spawns::Rocket>,
    /// When a soldier that deployed gets up again by itself.
    pub undeploy_at: Option<u64>,
    pub falling: Option<i32>,
    /// Until when the iron curtain keeps it from harm.
    pub curtain_until: u64,
    /// Being moved by the chronosphere.
    pub chrono: Option<supers::Warp>,
    /// What crates have done for it: damage taken is divided by the first,
    /// damage done multiplied by the second, speed by the third.
    pub crate_armour: f32,
    pub crate_firepower: f32,
    pub crate_speed: f32,
    /// On the deck of a high bridge (the engine's `OnBridge` byte).
    pub on_bridge: bool,
    /// Falling from a fallen bridge, to die where it lands.
    pub doomed: bool,
    /// The tick it last had somewhere to go (for the computer's soldiers, who
    /// dig in after standing a while).
    pub busy_at: u64,
    pub dropped_at: u64,
    /// When it came down from a parachute drop (0: never).
    pub landed_at: u64,
    /// What an enemy takes it for: a unit type and a house, or with the house
    /// `TREE_DISGUISE`, one of `DefaultMirageDisguises`.
    pub disguise: Option<(u16, u8)>,
    pub disguised_at: u64,
    /// A Mirage's disguise: not put on again before this tick, after an enemy
    /// soldier came beside it; and not shown, though still worn, before this
    /// one, after it fired.
    pub undisguised_until: u64,
    pub blinks_until: u64,
    /// The foundation (top cell and size) of the building it has been sent
    /// into, whose cells it may walk onto.
    enter_area: Option<((i32, i32), (i32, i32))>,
    /// Whether a foot soldier is dug in.
    pub stance: combat::Stance,
    /// A flyer's state.
    pub jet: Option<flight::Jet>,
    /// An aircraft's flight and mission.
    pub plane: Option<aircraft::Plane>,
    /// What it carries, last aboard last.
    pub cargo: Vec<Unit>,
    /// The transport it is on its way to board.
    boarding: Option<u32>,
    /// Letting its passengers out: the tick the next one goes.
    unloading: Option<u64>,
    /// A gunner vehicle's weapon and turret for whoever is aboard.
    pub gun: Option<(combat::Mount, u8)>,
    /// Which of its type's gunner weapons that is.
    pub gun_mode: u8,
    /// It took to charging the coil beside it by itself, and leaves off when
    /// an enemy comes in reach.
    pub charging: bool,
    /// The cell it is guarding the ground round, while it guards an area.
    pub post: Option<(i32, i32)>,
    pub stats: UnitStats,
    /// How much of its speed the unit is using, in millionths.
    fraction: i32,
    turn: Turn,
    drive: Option<Drive>,
    /// Speed saved up towards the next track point.
    progress: i32,
    /// The cell the unit is making for.
    dest: Option<(i32, i32)>,
    /// The directions of the next steps of its route; -1 ends them.
    path: [i8; PATH_LENGTH],
    /// The tick from which it may look for a path again.
    path_timer: u64,
    /// Failed attempts left before it gives up on its destination.
    retries: i32,
    /// While blocked by moving units, the tick from which it paths round all of
    /// them.
    blocked_until: Option<u64>,
    /// Whether it has been told to deploy and is stopping or turning to do so.
    deploying: bool,
    /// For a foot soldier: the spot being walked to, in leptons, and whether it is
    /// stepping (which is when its walking animation plays).
    head_to: Option<(i32, i32)>,
    pub stepping: bool,
    /// A foot soldier's fear: raised when it is hurt, worn off a point a tick.
    pub fear: u16,
    /// Lying down (crawling when it moves), and the tick it last went down or
    /// got up.
    pub prone: bool,
    pub prone_at: u64,
    /// A soldier's fidget at rest (1 for `Idle1`, 2 for `Idle2`) and the tick it
    /// began; and the tick from which it may fidget again.
    pub fidget: Option<(u8, u64)>,
    pub fidget_at: u64,
}

/// Where in a cell each foot-soldier spot is, from the cell's corner.
// Five spots; the game uses 2, 3 and 4.
const SPOT_OFFSETS: [(i32, i32); 5] = [(128, 128), (64, 64), (192, 64), (64, 192), (192, 192)];
/// The occupation bits of the three spots in use.
const SPOT_BITS: u8 = 0x1c;

/// The spot a position within a cell belongs to: 0 near the middle or in the top
/// quarter, else 2, 3 or 4.
fn spot_index(x: i32, y: i32) -> usize {
    let (lx, ly) = (x & 0xff, y & 0xff);
    let (dx, dy) = (lx - 0x80, ly - 0x80);
    if (path::engine_sqrt((dx * dx + dy * dy) as f32) as i32) < 60 {
        return 0;
    }
    match usize::from(lx > 0x80) | usize::from(ly > 0x80) << 1 {
        0 => 0,
        quarter => quarter + 1,
    }
}

impl Unit {
    fn new(id: u32, owner: u8, cell: (i32, i32), facing: u8, stats: UnitStats) -> Self {
        let (x, y) = cell_centre(cell);
        Self {
            id,
            owner,
            x,
            y,
            facing,
            speed: stored_speed(stats.speed),
            health: stats.strength,
            tethered: false,
            left_factory: false,
            rally: None,
            looked: None,
            controls: None,
            controlled_by: None,
            parasite: Vec::new(),
            bite: None,
            died_by_suicide: false,
            cloak: abilities::Cloak::default(),
            veterancy: 0.0,
            shown_rank: 2,
            sped_up: false,
            team: None,
            initiated: false,
            recruitable: true,
            rescue_until: 0,
            spawner: None,
            spawn_owner: None,
            rocket: None,
            undeploy_at: None,
            falling: None,
            curtain_until: 0,
            chrono: None,
            crate_armour: 1.0,
            crate_firepower: 1.0,
            crate_speed: 1.0,
            on_bridge: false,
            doomed: false,
            busy_at: 0,
            dropped_at: 0,
            landed_at: 0,
            disguise: None,
            disguised_at: 0,
            undisguised_until: 0,
            blinks_until: 0,
            enter_area: None,
            work: Work::default(),
            arms: combat::Arms::facing(facing),
            warp_until: 0,
            suppressed_until: 0,
            paralysed_until: 0,
            frozen: None,
            capture: None,
            stance: combat::Stance::Standing,
            jet: stats.jet.map(|_| flight::Jet::default()),
            plane: stats.plane.map(|plane| aircraft::Plane::parked(&plane)),
            cargo: Vec::new(),
            boarding: None,
            unloading: None,
            gun: None,
            gun_mode: 0,
            charging: false,
            post: None,
            stats,
            fraction: 0,
            turn: Turn::facing(facing),
            drive: None,
            progress: 0,
            dest: None,
            path: [-1; PATH_LENGTH],
            path_timer: 0,
            retries: 10,
            blocked_until: None,
            deploying: false,
            head_to: None,
            stepping: false,
            fear: 0,
            prone: false,
            prone_at: 0,
            fidget: None,
            fidget_at: 0,
        }
    }

    pub fn cell(&self) -> (i32, i32) {
        (self.x.div_euclid(LEPTONS_PER_CELL), self.y.div_euclid(LEPTONS_PER_CELL))
    }

    /// The cell the unit has been ordered to, while it is on its way.
    pub fn destination(&self) -> Option<(i32, i32)> {
        self.dest
    }

    // What the engine counts as moving when one unit
    // meets another: it has a destination, is turning, or is driving.
    pub fn is_moving(&self) -> bool {
        if let Some(plane) = self.plane {
            return plane.is_moving();
        }
        self.drive.is_some() || self.dest.is_some() || self.turn.remaining > 0 || self.head_to.is_some()
    }

    /// The direction (0 to 7) nearest its facing.
    fn heading(&self) -> usize {
        ((self.facing as usize + 16) >> 5) & 7
    }

    /// Forgets the path, all of it: a soldier finishing the step it is on
    /// moves its path along by one, which would bring the old steps back if
    /// only the first were cleared (soldiers sent elsewhere mid-step walked
    /// on the old way).
    fn drop_path(&mut self) {
        self.path = [-1; PATH_LENGTH];
    }

    fn shift_path(&mut self, steps: usize) {
        self.path.copy_within(steps.., 0);
        for slot in &mut self.path[PATH_LENGTH - steps..] {
            *slot = -1;
        }
    }

    fn set_facing(&mut self, facing: u8) {
        self.facing = facing;
        self.turn = Turn::facing(facing);
    }
}

/// A terrain object that plays its animation now and then (the ore drill).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnimatedTerrain {
    pub cell: (i32, i32),
    /// Images in the animation (half of the art's frames; the rest are shadows).
    pub frames: u32,
    /// `AnimationRate`: ticks per image.
    pub rate: u32,
    /// `AnimationProbability`: the chance each idle tick of starting.
    pub probability: f32,
    /// The image showing.
    pub stage: u32,
    /// Ticks per image while playing; zero while idle.
    playing: u32,
    timer_start: u64,
    timer_length: u64,
}

/// A kind of building, from rules.ini and art.ini.
#[derive(Clone, Debug, Default)]
pub struct BuildingType {
    /// `Cost`.
    pub cost: i32,
    /// `Power`: positive is output, negative is drain.
    pub power: i32,
    /// art.ini `Foundation`, in cells.
    pub size: (i32, i32),
    /// `Prerequisite`: for each entry, the building types any one of which
    /// satisfies it.
    pub prerequisites: Vec<Vec<u16>>,
    /// `Adjacent`: how many cells from the player's other buildings it may be placed.
    pub adjacent: i32,
    /// `TechLevel`; -1, or none given, means it can never be built.
    pub tech_level: i32,
    /// `Owner`, as a bit per country: who can make it, matched against the owner
    /// bits of the factory building, not the player's country.
    pub owners: u32,
    /// `RequiredHouses` and `ForbiddenHouses`, as a bit per country; `None` where
    /// the key is not given.
    pub required_houses: Option<u32>,
    pub forbidden_houses: Option<u32>,
    /// `PrerequisiteOverride`: owning any of these waives the tech level, country
    /// and prerequisite tests.
    pub prerequisite_override: Vec<u16>,
    /// `BuildLimit`, if given.
    pub build_limit: Option<i32>,
    /// `Explosion`: what it may go up in, as numbers in the world's list of
    /// animation types.
    pub explosions: Vec<u16>,
    /// What it throws as it is destroyed.
    pub leavings: remains::Leavings,
    /// `Factory`: what kind of thing it produces, if any.
    pub factory: Option<Factory>,
    /// `BuildCat=Combat`: it goes in the defences tab and has its own production.
    pub combat: bool,
    /// `AIBasePlanningSide`: the side whose sidebar lists it first.
    pub planning_side: i32,
    /// `WaterBound`: it stands on water.
    pub water_bound: bool,
    /// `Naval`: as a factory, it makes ships and nothing else.
    pub naval: bool,
    /// Its name as shown, which settles the order of otherwise equal cameos.
    pub sort_name: String,
    /// For a wall (`Wall=yes` with `ToOverlay`): the overlay it becomes.
    pub wall_overlay: Option<u16>,
    /// `GuardRange` in cells: how far a wall reaches to join a line.
    pub guard_range: i32,
    /// `SuperWeapon` and `SuperWeapon2`: the super weapons owning it gives.
    /// `Spyable`, `Storage`, and `DetectDisguiseRange` in cells for a building
    /// with `DetectDisguise`.
    pub spyable: bool,
    pub storage: i32,
    pub detect_disguise: i32,
    pub super_weapon: Option<u16>,
    pub super_weapon2: Option<u16>,
    /// `BaseNormal`: whether other buildings may be placed next to this one.
    pub base_normal: bool,
    /// `ConstructionYard`.
    pub construction_yard: bool,
    /// Its armour, weapons and how it fights, and how fast its turret turns.
    pub fighter: combat::Fighter,
    pub turret_rot: i32,
    /// `Cloning`: it copies every soldier the house's barracks turn out.
    pub cloning: bool,
    /// `Powered` and not `TogglePower=no`: it stops when power is short.
    pub powered: bool,
    /// `Soylent`: what it sells for, if not zero; and `Crewed`: people come out.
    pub soylent: i32,
    pub crewed: bool,
    /// `Radar`: gives its owner the radar while it has power.
    pub radar: bool,
    /// `RevealToAll`: the others see it when it is finished.
    pub reveal_to_all: bool,
    /// `Refinery`: miners unload here.
    pub refinery: bool,
    /// `CanBeOccupied`, `MaxNumberOccupants`, and art `MuzzleFlash0..`: soldiers
    /// can go in and fire from these points, given in screen pixels.
    pub can_be_occupied: bool,
    pub max_occupants: usize,
    pub muzzles: Vec<(i32, i32)>,
    /// `NeedsEngineer`: off until captured.
    pub needs_engineer: bool,
    /// `CanC4`: a charge can be planted in it.
    pub can_c4: bool,
    /// `Helipad` with `NumberOfDocks` and `DockingOffset0..`: aircraft park on
    /// it, each at its own offset from the building's middle.
    /// `AIBuildThis`: a computer house plans one; whether it is the power-up
    /// "plug" the plan keeps for last; and whether it wants a cell more room
    /// (`ProtectWithWall` or `WantsExtraSpace`).
    pub ai_build_this: bool,
    pub plug: bool,
    pub extra_space: bool,
    /// `ProtectWithWall`: a computer house may put a ring of walls round it.
    pub protect_with_wall: bool,
    /// `SpySat`: while its owner has one the whole map is seen.
    pub spy_sat: bool,
    /// `OrePurifier`.
    pub ore_purifier: bool,
    /// `ProduceCashStartup`, `ProduceCashAmount`, `ProduceCashDelay`.
    pub cash: (i32, i32, i32),
    /// `Overpowerable`: tesla troopers can charge it.
    pub overpowerable: bool,
    /// `PsychicDetectionRadius`, in cells.
    pub psychic_radius: i32,
    /// `UnitRepair`: a repair depot.
    pub unit_repair: bool,
    /// `BridgeRepairHut`.
    pub bridge_repair_hut: bool,
    /// `GapRadiusInCells` of a `GapGenerator`; 0 for any other type.
    pub gap_radius: i32,
    /// `HasRadialIndicator`: its ring is shown while it is selected; and
    /// `ConcentricRadialIndicator`: as rings spreading out rather than a sweep.
    pub radial_indicator: bool,
    pub concentric_indicator: bool,
    /// `CrateBeneath` and `CrateBeneathIsMoney`: destroyed, it leaves a crate.
    pub crate_beneath: bool,
    pub crate_beneath_money: bool,
    /// `Hospital`, `Armory`, and `Ammo` (how many soldiers it will take; -1
    /// for any number).
    pub hospital: bool,
    pub armory: bool,
    pub ammo: i32,
    /// `IsAnimDelayedFire` with `DelayedFireDelay`: the frames it charges
    /// before each shot (0 for none).
    pub delayed_fire: u32,
    pub number_of_docks: usize,
    pub docking_offsets: Vec<(i32, i32)>,
    /// `BarrelTravel`, `BarrelCompressFrames`, `BarrelHoldFrames` and
    /// `BarrelRecoverFrames`, for whoever draws its turret.
    pub barrel_recoil: (i32, i32, i32, i32),
    /// `Explodes`, `DeathWeapon` and `DeathWeaponDamageModifier` in hundredths.
    pub explodes: bool,
    pub death_weapon: Option<u16>,
    pub death_percent: i32,
    /// `FreeUnit`: the unit type that comes with it.
    pub free_unit: Option<u16>,
    /// art.ini `QueueingCell`: where a miner waits its turn, from the top cell.
    pub queueing_cell: (i32, i32),
    /// `NumberImpassableRows`: columns from this one on can be driven over; `None`
    /// closes the whole foundation.
    pub impassable_rows: Option<i32>,
    /// `WeaponsFactory`: vehicles drive out through its door.
    pub weapons_factory: bool,
    /// `GDIBarracks` and `NODBarracks`: which cell soldiers come out at first.
    pub gdi_barracks: bool,
    pub nod_barracks: bool,
    /// rules.ini `ExitCoord`, in leptons from the centre of the top cell.
    pub exit_coord: (i32, i32),
    /// `DeployTime` × 900, in ticks: how long its door takes to open or close.
    pub door_ticks: u32,
    /// `UndeploysInto`: the vehicle the building can pack up into.
    pub undeploys_into: Option<UnitStats>,
    /// Images in its build-up animation (half the build-up art's frames); zero if
    /// it has none.
    pub buildup_frames: u32,
    /// `Strength`: full health.
    pub strength: i32,
}

/// A kind of vehicle (or, in time, foot soldier) that can be produced.
#[derive(Clone, Debug, Default)]
pub struct UnitType {
    pub stats: UnitStats,
    /// `CarriesCrate`: destroyed, it leaves a crate (see `crate_from_unit`).
    pub carries_crate: bool,
    /// `Passengers` (room, in `Size` points) and `SizeLimit` (the biggest it takes).
    pub passengers: i32,
    pub size_limit: i32,
    /// `Size`: the room it takes aboard something.
    pub size: i32,
    /// `Gunner`, with `Weapon1` onward and the turret each goes with: its weapon
    /// is its passenger's.
    pub gunner: bool,
    pub gunner_weapons: Vec<(combat::Mount, u8)>,
    /// `EliteWeapon1` onward, in step with `gunner_weapons`.
    pub gunner_elite: Vec<combat::Mount>,
    /// `IFVMode`: the weapon a gunner vehicle has with this aboard.
    pub ifv_mode: usize,
    /// `Pip`: the pip frame it shows as aboard something.
    pub pip: u8,
    /// `PipScale`: 0 none, 1 `Ammo`, 2 `Tiberium`, 3 `Passengers`, 4 `Power`;
    /// and `PipWrap`.
    pub pip_scale: u8,
    pub pip_wrap: i32,
    /// `Dock`: the building types it unloads at.
    pub dock: Vec<u16>,
    /// `Cost`.
    pub cost: i32,
    /// `TechLevel`; -1, or none given, means it can never be built.
    pub tech_level: i32,
    /// `Owner`, `RequiredHouses`, `ForbiddenHouses`: as for buildings.
    pub owners: u32,
    pub required_houses: Option<u32>,
    pub forbidden_houses: Option<u32>,
    /// `Prerequisite`, as for buildings: building types, any one of each entry.
    pub prerequisites: Vec<Vec<u16>>,
    pub prerequisite_override: Vec<u16>,
    /// `BuildLimit`, if given.
    pub build_limit: Option<i32>,
    /// `Explosion`: what it may go up in, as numbers in the world's list of
    /// animation types.
    pub explosions: Vec<u16>,
    /// `Crewed`, `Weight`, `Underwater`; for a soldier `NotHuman` and
    /// `DeathAnims` (animations by number); and what it throws as it dies.
    pub crewed: bool,
    pub weight: f64,
    pub underwater: bool,
    pub not_human: bool,
    pub death_anims: Vec<u16>,
    pub leavings: remains::Leavings,
    /// `SuppressionThreshold`: a parasite's.
    pub suppression_threshold: i32,
    /// `RequiresStolenAlliedTech` or `RequiresStolenSovietTech`: only for a house
    /// whose spy has been into the right laboratory.
    /// `RequiresStolenSovietTech` (1) or `RequiresStolenAlliedTech` (2).
    pub requires_stolen_tech: u8,
    /// An `[InfantryTypes]` entry rather than a `[VehicleTypes]` one.
    pub infantry: bool,
    /// `Naval`: built by a shipyard.
    pub naval: bool,
    /// `AIBasePlanningSide`, and the name as shown: for ordering cameos.
    pub planning_side: i32,
    pub sort_name: String,
}

/// Something a house can produce: a building type or a unit type, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Building(u16),
    Unit(u16),
}

/// The rules for producing an item, whichever kind it is.
struct Terms<'a> {
    cost: i32,
    tech_level: i32,
    owners: u32,
    required: Option<u32>,
    forbidden: Option<u32>,
    prerequisites: &'a [Vec<u16>],
    overrides: &'a [u16],
    build_limit: Option<i32>,
    /// The kind of factory that makes it, and whether that must be a naval one.
    factory: Factory,
    naval: bool,
    wall: bool,
    stolen_tech: u8,
}

/// What a factory building produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factory {
    Building,
    Infantry,
    Unit,
    Aircraft,
}

/// A building being produced by a house.
// Production runs through 54 stages; the
// cost is paid a stage at a time; on hold it keeps its stage and what was paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Production {
    pub building: u16,
    /// Stages done, of [`PRODUCTION_STAGES`].
    pub stage: i32,
    /// Ticks per stage while running; zero while stopped.
    rate: i32,
    /// The tick the next stage is due.
    next: u64,
    /// Credits still to pay.
    balance: i32,
    /// Put on hold by the player (also set when finished).
    pub suspended: bool,
    /// Whether the last stage could not be paid for.
    pub stalled: bool,
}

/// Stages a production runs through.
pub const PRODUCTION_STAGES: i32 = 54;

impl Production {
    pub fn is_ready(&self) -> bool {
        self.stage >= PRODUCTION_STAGES
    }

    /// On hold: stopped by the player before it was finished.
    pub fn on_hold(&self) -> bool {
        self.suspended && !self.is_ready()
    }
}

/// The rules.ini `[General]` values that set how long production takes.
#[derive(Clone, Copy, Debug)]
pub struct BuildRules {
    /// `BuildSpeed`: minutes to produce 1000 credits' worth.
    pub build_speed: f64,
    pub low_power_penalty: f64,
    pub min_low_power_speed: f64,
    pub max_low_power_speed: f64,
    pub multiple_factory: f64,
    pub wall_coefficient: f64,
}

impl Default for BuildRules {
    // DATA: the stock rules.ini values.
    fn default() -> Self {
        Self {
            build_speed: 0.7,
            low_power_penalty: 1.0,
            min_low_power_speed: 0.5,
            max_low_power_speed: 0.8,
            multiple_factory: 0.8,
            wall_coefficient: 3.0,
        }
    }
}

/// A player's side.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct House {
    pub credits: i32,
    /// The building in production, and the defence (`BuildCat=Combat`) in
    /// production; each has its own.
    // Separate factories for the two.
    pub production: Option<Production>,
    pub defence: Option<Production>,
    /// Foot soldiers and vehicles in production (the `building` of each is a unit
    /// type), and the unit types waiting their turn behind each.
    // A queue per factory, across types.
    pub infantry: Option<Production>,
    pub vehicles: Option<Production>,
    pub infantry_queue: Vec<u16>,
    pub vehicle_queue: Vec<u16>,
    /// Ships and aircraft in production, each with its own factory and queue.
    // A house has one factory object for each
    // of aircraft, infantry, vehicles, ships, buildings and combat buildings.
    pub ships: Option<Production>,
    pub aircraft: Option<Production>,
    pub ship_queue: Vec<u16>,
    pub aircraft_queue: Vec<u16>,
    /// The house's country, as its place in rules.ini `[Countries]`.
    pub country: u8,
    /// Its side: 0 for the Allied countries, 1 for the Soviet ones.
    pub side: i32,
    /// The highest `TechLevel` it may build.
    pub tech_level: i32,
    /// A computer player's own state; `None` for a person's house and the
    /// neutral one.
    pub ai: Option<ai::Brain>,
    /// A person plays it.
    pub human: bool,
    /// The houses it is allied with, a bit each.
    pub allies: u32,
    /// One of its miners has found no ore to go to. Never cleared.
    pub no_ore: bool,
    /// Its hold on each super weapon type.
    pub supers: Vec<supers::Super>,
    /// What its spies have brought it: Soviet and Allied technology, and
    /// soldiers and vehicles that start as veterans.
    pub stolen: [bool; 2],
    pub veterans: [bool; 2],
    /// Until when a spy has put its power out.
    pub blackout_until: u64,
    /// Where its base is taken to be, in leptons.
    pub base_centre: (i32, i32),
    /// The cell its chronosphere's first click chose.
    pub chrono_source: Option<(i32, i32)>,
    /// Until when a lightning storm jams its radar.
    pub jammed_until: u64,
    /// Beaten and out of the game; and its figures for the score screen.
    pub defeated: bool,
    pub score: endgame::Score,
}

impl House {
    /// What is in production for the buildings tab or for the defences tab.
    pub fn slot(&self, combat: bool) -> Option<Production> {
        if combat { self.defence } else { self.production }
    }

    fn slot_mut(&mut self, combat: bool) -> &mut Option<Production> {
        if combat { &mut self.defence } else { &mut self.production }
    }

    /// The four things in production, by sidebar tab: buildings, defences, foot
    /// soldiers, vehicles.
    pub fn tab_slot(&self, tab: usize) -> Option<Production> {
        [self.production, self.defence, self.infantry, self.vehicles, self.ships, self.aircraft].get(tab).copied().flatten()
    }

    fn tab_slot_mut(&mut self, tab: usize) -> &mut Option<Production> {
        match tab {
            0 => &mut self.production,
            1 => &mut self.defence,
            2 => &mut self.infantry,
            4 => &mut self.ships,
            5 => &mut self.aircraft,
            _ => &mut self.vehicles,
        }
    }

    pub fn queue(&self, tab: usize) -> &Vec<u16> {
        match tab {
            2 => &self.infantry_queue,
            4 => &self.ship_queue,
            5 => &self.aircraft_queue,
            _ => &self.vehicle_queue,
        }
    }

    fn queue_mut(&mut self, tab: usize) -> &mut Vec<u16> {
        match tab {
            2 => &mut self.infantry_queue,
            4 => &mut self.ship_queue,
            5 => &mut self.aircraft_queue,
            _ => &mut self.vehicle_queue,
        }
    }
}

/// A building standing on the map.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Building {
    pub id: u32,
    pub owner: u8,
    /// Index into the world's building types.
    pub kind: u16,
    /// Health left, out of the type's `strength`.
    pub health: i32,
    /// The top cell of its foundation, and the foundation's size in cells.
    pub cell: (i32, i32),
    pub size: (i32, i32),
    /// Its build-up animation, while it is going up or coming down.
    pub buildup: Option<Buildup>,
    /// The tick its build-up finished and its animations started.
    pub opened: u64,
    /// A war factory letting a vehicle out.
    unload: Option<Unload>,
    /// The miner a refinery is dealing with.
    pub docked: Option<u32>,
    /// Where the units it makes go once they are out.
    pub rally: Option<(i32, i32)>,
    /// Its target and weapon state.
    pub arms: combat::Arms,
    /// The factory of its kind that the house's production comes out of.
    pub primary: bool,
    /// Being repaired at its owner's cost.
    pub repairing: bool,
    /// Until when the iron curtain keeps it from harm.
    pub curtain_until: u64,
    /// A repair depot's next look at what stands on it, and whether it is
    /// in the middle of mending something.
    pub depot_next: u64,
    pub depot_busy: bool,
    /// When a cash building next pays (0 before its clock is started).
    pub cash_at: u64,
    /// Soldiers inside a hospital or armory, each with the tick he comes out;
    /// and the uses it has left (-1 for any number).
    pub patients: Vec<(u64, Unit)>,
    pub ammo: i32,
    /// Held by a `Temporal` weapon: what is left of it to wear away.
    pub frozen: Option<i32>,
    /// The soldiers inside firing from it, first in first, and which of its
    /// `MuzzleFlash` points the next shot comes from.
    pub occupants: Vec<Unit>,
    pub muzzle: usize,
    /// Whether it has looked round itself yet, for the shroud.
    looked: bool,
    /// A charge planted in it: the tick it goes off, and who planted it.
    pub c4: Option<(u64, u32)>,
    /// Taken by an engineer.
    pub captured: bool,
    /// The aircraft holding each of its pads.
    pub docks: Vec<Option<u32>>,
    /// A shot being charged: the tick it began, the tick it is let go, and
    /// the weapon.
    pub charge: Option<(u64, u64, usize)>,
    /// When a miner's load began to be paid for: a refinery's `SpecialAnim`
    /// runs from then until the miner is let go.
    pub refining: Option<u64>,
    /// A prism tower: the beams lent to its next shot, and for one lending
    /// its beam the tower it lends to.
    pub supports: u32,
    pub supporting: Option<(i32, i32, i32)>,
    /// How many beams were lent to the shot it last fired.
    pub last_supports: u32,
    /// The foot soldiers charging it, and whether that has it fire its second
    /// weapon.
    pub chargers: Vec<u32>,
    pub charged: bool,
}

/// A war factory's Unload mission.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
struct Unload {
    status: u8,
    /// The tick the mission next runs.
    next: u64,
    /// The vehicle on its way out.
    unit: u32,
    /// The tick the door finishes opening or closing.
    door: u64,
}

/// How an attempt to put a finished unit on the map went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exit {
    Out,
    Failed,
    Busy,
}

/// A building's build-up animation in progress.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Buildup {
    stage: u32,
    count: u32,
    /// Ticks per image, and the tick the next one is due.
    rate: u32,
    next: u64,
    /// Set while the building is packing up into its vehicle: where that is to go.
    packing: Option<(i32, i32)>,
    /// Set while the building is being sold.
    selling: bool,
}

impl Building {
    /// The image of the build-up art to show, while it is going up or coming down.
    // Coming down is the same animation backwards.
    /// Whether it is a war factory with a vehicle on its way out, when it shows
    /// its open-door art.
    pub fn unloading(&self) -> bool {
        self.unload.is_some()
    }

    pub fn buildup_frame(&self) -> Option<u32> {
        self.buildup.map(|b| if b.packing.is_some() || b.selling { b.count - 1 - b.stage.min(b.count - 1) } else { b.stage })
    }
}

/// Something a player tells the simulation to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Order {
    /// Send a unit to a cell.
    Move { unit: u32, x: i32, y: i32 },
    /// Send a unit to board a transport.
    Board { unit: u32, transport: u32 },
    /// Fire a house's super weapon at a cell.
    Super { house: u8, index: u16, cell: (i32, i32) },
    /// Send a unit into a building (an engineer to take or mend it).
    Enter { unit: u32, building: u32 },
    /// Start or stop repairing a building.
    Repair { building: u32 },
    /// Sell a building.
    Sell { building: u32 },
    /// Tell a unit to attack something.
    Attack { unit: u32, target: combat::Aim },
    /// The stop order, for a building.
    Halt { building: u32 },
    /// Send an aircraft to land on a pad building.
    Dock { unit: u32, building: u32 },
    /// Have a building fire on something.
    Aim { building: u32, target: combat::Aim },
    /// Have everyone in a building come out of it.
    Evacuate { building: u32 },
    /// Set a factory's rally point.
    Rally { building: u32, x: i32, y: i32 },
    /// Make a factory the primary one of its kind, or stop it being so.
    Primary { building: u32 },
    /// Send a miner to dig at a cell.
    Harvest { unit: u32, x: i32, y: i32 },
    /// Turn a unit into the building it deploys into, where it stands.
    Deploy { unit: u32 },
    /// Pack a building up into its vehicle and send that to a cell.
    Undeploy { building: u32, x: i32, y: i32 },
    /// Halt a unit where it is (once it finishes the track it is on).
    Stop { unit: u32 },
    /// Have a resting unit shuffle to a neighbouring cell.
    Scatter { unit: u32 },
    /// Start producing a building.
    Build { house: u8, building: u16 },
    /// Start producing a unit, or add one to its factory's queue.
    BuildUnit { house: u8, unit: u16 },
    /// Put a unit in production on hold.
    SuspendUnit { house: u8, unit: u16 },
    /// Take a unit out of the queue, or abandon the one in production.
    CancelUnit { house: u8, unit: u16 },
    /// Put a building in production on hold.
    Suspend { house: u8, building: u16 },
    /// Abandon a building in production and refund what was paid.
    CancelBuild { house: u8, building: u16 },
    /// Put a finished building on the map with its foundation's top cell at a cell.
    Place { house: u8, building: u16, x: i32, y: i32 },
    /// Have all of a house's soldiers cheer.
    Cheer { house: u8 },
}

impl Order {
    /// The unit an order is for, if it is for one.
    pub fn unit(&self) -> Option<u32> {
        match *self {
            Order::Move { unit, .. }
            | Order::Board { unit, .. }
            | Order::Enter { unit, .. }
            | Order::Attack { unit, .. }
            | Order::Dock { unit, .. }
            | Order::Harvest { unit, .. }
            | Order::Deploy { unit }
            | Order::Stop { unit }
            | Order::Scatter { unit } => Some(unit),
            _ => None,
        }
    }

    /// The building an order is given to, if it is given to one.
    pub fn building(&self) -> Option<u32> {
        match *self {
            Order::Repair { building }
            | Order::Sell { building }
            | Order::Halt { building }
            | Order::Aim { building, .. }
            | Order::Evacuate { building }
            | Order::Rally { building, .. }
            | Order::Primary { building }
            | Order::Undeploy { building, .. } => Some(building),
            _ => None,
        }
    }

    /// The house an order names as the one giving it, if it names one.
    pub fn house(&self) -> Option<u8> {
        match *self {
            Order::Super { house, .. }
            | Order::Build { house, .. }
            | Order::BuildUnit { house, .. }
            | Order::SuspendUnit { house, .. }
            | Order::CancelUnit { house, .. }
            | Order::Suspend { house, .. }
            | Order::CancelBuild { house, .. }
            | Order::Place { house, .. }
            | Order::Cheer { house } => Some(house),
            _ => None,
        }
    }
}

pub struct World {
    /// Whether houses can be defeated and the game end (off in tests that
    /// arrange things as no game would); the short game option; and, once the
    /// game is over, the houses left.
    pub end_checks: bool,
    pub short_game: bool,
    pub over: Option<Vec<u8>>,
    /// The skirmish screen's super weapons option.
    pub supers_allowed: bool,
    /// Whether bridges can be destroyed, and `[CombatDamage] BridgeStrength`.
    pub bridges_destroyable: bool,
    pub bridge_strength: i32,
    /// Orders given to soldiers still under their parachutes, kept for landing.
    landing_orders: Vec<Order>,
    pub map: Map,
    tracks: Option<TrackTables>,
    /// Which unit, by id plus one, holds each cell; zero for none. A unit holds the
    /// cell it is in and the cells its current track runs into.
    // The engine keeps one "vehicle here"
    // bit per cell, set on the cell a unit rests in and on the cells its track runs
    // into. Not original behaviour: remembering which unit set it.
    claims: Vec<u32>,
    /// The foot-soldier spots taken in each cell (bits 4, 8 and 16), and the house
    /// that has them.
    spots: Vec<u8>,
    spot_house: Vec<u8>,
    pub blockage_delay: i32,
    pub path_delay: i32,
    pub close_enough: i32,
    /// `LeptonsPerSightIncrease`: the height for each tenth more sight.
    pub sight_height_step: i32,
    units: Vec<Unit>,
    buildings: Vec<Building>,
    pub building_types: Vec<BuildingType>,
    /// Unit types; a unit's `stats.kind` is its type's index plus one.
    pub unit_types: Vec<UnitType>,
    /// Most items that can wait in a factory's queue behind the one in hand.
    // DATA: rules.ini `[General] MaximumQueuedObjects`.
    pub queue_limit: usize,
    /// Houses by owner number.
    pub houses: Vec<House>,
    pub build_rules: BuildRules,
    /// Whether construction yards may pack up again (the "MCV repacks" option).
    pub mcv_repacks: bool,
    /// Ticks a build-up animation is spread over.
    // DATA: rules.ini `[General] BuildupTime` is .06 minutes; times 900.
    pub buildup_ticks: u32,
    /// `[General] ChronoDistanceFactor`, `ChronoMinimumDelay`,
    /// `ChronoRangeMinimum` and `ChronoTrigger`.
    pub chrono: (i32, i32, i32, bool),
    /// `[General] RefundPercent`, `RepairStep`, `RepairPercent`, and `RepairRate` ×
    /// 900; the survivor divisors (Allied, Soviet); and the crew types (Allied,
    /// Soviet, engineer) as unit types.
    pub refund_percent: f64,
    pub repair_step: i32,
    pub repair_percent: f64,
    pub repair_interval: u32,
    pub survivor_divisors: (i32, i32),
    pub crew_types: (Option<u16>, Option<u16>, Option<u16>, Option<u16>),
    /// Debris, crews and sinking ships, and the pieces and wrecks now shown.
    pub remains_rules: remains::Rules,
    wreckage: remains::Wreckage,
    /// Weapon, warhead and projectile types.
    pub weapons: Vec<combat::Weapon>,
    pub warheads: Vec<combat::Warhead>,
    pub projectiles: Vec<combat::Projectile>,
    /// `[General] Gravity`, `[CombatDamage] MaxDamage`, `[General] MissileROTVar`
    /// and `NormalTargetingDelay`.
    pub gravity: i32,
    /// `BallisticScatter` in leptons.
    pub ballistic_scatter: i32,
    /// `[ElevationModel]`: `ElevationIncrement`, `ElevationIncrementBonus`,
    /// `ElevationBonusCap`.
    pub elevation: (i32, f64, f64),
    pub max_damage: i32,
    pub missile_rot_var: f64,
    pub targeting_delay: u32,
    /// `ConditionRed`: the share of its strength at or below which a thing is
    /// badly damaged.
    pub condition_red: f64,
    /// `IdleActionFrequency`: sets how often a soldier at rest fidgets.
    pub idle_frequency: f64,
    /// `C4Delay` in frames.
    pub c4_delay: u32,
    /// The lists a computer house plans from.
    pub ai_rules: ai::Rules,
    /// Bombs stuck on things, and the rules of the special weapons.
    /// The super weapon types and the paradrops' rules.
    /// The V3 rocket's flight rules, and the dreadnought missile's.
    /// The computer's teams, and what ai.ini says of them.
    pub teams: Vec<teams::Team>,
    pub team_data: teams::Data,
    /// How each AI trigger has done in this game: its weight and record.
    pub(crate) trigger_scores: Vec<teams::Score>,
    pub rockets: [spawns::RocketRules; 2],
    pub super_types: Vec<supers::SuperType>,
    pub super_rules: supers::Rules,
    /// Crates: their rules, those on the map, and whether the match has them.
    pub crate_rules: powerups::Rules,
    pub(crate) crates: Vec<powerups::Crate>,
    pub(crate) crates_on: bool,
    pub veterancy: abilities::Veterancy,
    pub bombs: Vec<abilities::Bomb>,
    pending_bites: Vec<(u32, combat::Aim, u16)>,
    leaps: Vec<abilities::Leap>,
    waves: Vec<combat::Wave>,
    /// Marks due when an animation reaches its largest picture: the tick, the
    /// point and the animation.
    pending_marks: Vec<(u64, (i32, i32), u16)>,
    pub rad_sites: Vec<radiation::Site>,
    pub rad_rules: radiation::Rules,
    pub storm: Option<supers::Storm>,
    pub nukes: Vec<supers::Nuke>,
    pub ability_rules: abilities::Rules,
    /// `ReloadRate` in frames: the time a pad takes over each point of an
    /// aircraft's ammunition.
    pub reload_delay: u32,
    events: Vec<combat::Event>,
    bullets: Vec<combat::Bullet>,
    /// Things killed this tick, with the kind of death.
    dying: Vec<(combat::Aim, i32)>,
    /// `[Tiberiums]` `Value`: credits a bale, by kind of ore.
    pub ore_values: Vec<i32>,
    /// `[AI] TiberiumNearScan` and `TiberiumFarScan`, in cells.
    pub ore_near_scan: i32,
    pub ore_far_scan: i32,
    /// `[General] HarvesterLoadRate`, `HarvesterDumpRate`, `HarvesterTooFarDistance`
    /// and `ChronoHarvTooFarDistance`.
    pub harvester_load_rate: u32,
    pub harvester_dump_rate: f64,
    pub harvester_too_far: i32,
    pub chrono_too_far: i32,
    /// Buildings that have packed up, with the vehicle each became.
    unpacked: Vec<(u32, u32)>,
    animated: Vec<AnimatedTerrain>,
    /// What each animation type leaves on the ground, and the splash list
    /// (`SplashList`), as numbers in that list.
    pub anim_marks: Vec<smudge::AnimMark>,
    pub splash_anims: Vec<u16>,
    /// The kinds of ore and each one's queues of cells to grow and spread.
    pub ore_types: Vec<ore::OreType>,
    ore_queues: Vec<ore::Queues>,
    ore_ready: bool,
    random: u64,
    /// The stream soldiers' fidgets are drawn from.
    // Not original behaviour: the engine draws them from its one stream; kept
    // apart here so that soldiers standing about do not change every other
    // draw.
    idle_random: u64,
    tick: u64,
    next_id: u32,
}

fn cell_centre(cell: (i32, i32)) -> (i32, i32) {
    (
        cell.0 * LEPTONS_PER_CELL + LEPTONS_PER_CELL / 2,
        cell.1 * LEPTONS_PER_CELL + LEPTONS_PER_CELL / 2,
    )
}

fn step(cell: (i32, i32), direction: usize) -> (i32, i32) {
    (cell.0 + DIRECTIONS[direction].0, cell.1 + DIRECTIONS[direction].1)
}

impl World {
    /// A world on a map. With the engine's drive tracks, units follow them through
    /// turns; without, they turn on the spot.
    pub fn new(mut map: Map, tracks: Option<TrackTables>) -> Self {
        if map.zones.is_none() {
            map.build_zones();
        }
        Self {
            // The second half is the layer of bridge decks.
            claims: vec![0; map.passable.len() * 2],
            spots: vec![0; map.passable.len()],
            spot_house: vec![u8::MAX; map.passable.len()],
            blockage_delay: DEFAULT_BLOCKAGE_DELAY,
            path_delay: DEFAULT_PATH_DELAY,
            close_enough: DEFAULT_CLOSE_ENOUGH,
            sight_height_step: 2000,
            map,
            tracks,
            units: Vec::new(),
            buildings: Vec::new(),
            building_types: Vec::new(),
            unit_types: Vec::new(),
            queue_limit: 29,
            houses: Vec::new(),
            build_rules: BuildRules::default(),
            mcv_repacks: true,
            buildup_ticks: 54,
            chrono: (48, 16, 0, true),
            refund_percent: 0.5,
            repair_step: 8,
            repair_percent: 0.15,
            repair_interval: 14,
            survivor_divisors: (500, 250),
            crew_types: (None, None, None, None),
            remains_rules: remains::Rules::default(),
            wreckage: remains::Wreckage::default(),
            weapons: Vec::new(),
            warheads: Vec::new(),
            projectiles: Vec::new(),
            gravity: 6,
            ballistic_scatter: 256,
            elevation: (4, 2.0, 2.0),
            max_damage: 10_000,
            missile_rot_var: 0.25,
            targeting_delay: 27,
            condition_red: 0.25,
            idle_frequency: 0.15,
            c4_delay: 27,
            reload_delay: 270,
            ai_rules: ai::Rules::default(),
            teams: Vec::new(),
            team_data: teams::Data::default(),
            trigger_scores: Vec::new(),
            rockets: [spawns::RocketRules::default(); 2],
            super_types: Vec::new(),
            landing_orders: Vec::new(),
            end_checks: false,
            short_game: true,
            over: None,
            supers_allowed: true,
            bridges_destroyable: true,
            bridge_strength: 1500,
            super_rules: supers::Rules { radius: 1024, fall_rate: 3, ..supers::Rules::default() },
            crate_rules: powerups::Rules::default(),
            crates: Vec::new(),
            crates_on: false,
            veterancy: abilities::Veterancy::default(),
            bombs: Vec::new(),
            pending_bites: Vec::new(),
            leaps: Vec::new(),
            waves: Vec::new(),
            pending_marks: Vec::new(),
            rad_sites: Vec::new(),
            rad_rules: radiation::Rules::default(),
            storm: None,
            nukes: Vec::new(),
            ability_rules: abilities::Rules::default(),
            events: Vec::new(),
            bullets: Vec::new(),
            dying: Vec::new(),
            ore_values: vec![25, 50],
            ore_near_scan: 6,
            ore_far_scan: 48,
            harvester_load_rate: 2,
            harvester_dump_rate: 0.016,
            harvester_too_far: 5,
            chrono_too_far: 50,
            unpacked: Vec::new(),
            animated: Vec::new(),
            anim_marks: Vec::new(),
            splash_anims: Vec::new(),
            // DATA: rules.ini `[Riparius]` and `[Cruentus]`.
            ore_types: vec![
                ore::OreType { growth: 2200, spread: 2200, growth_percentage: 0.06, spread_percentage: 0.06 },
                ore::OreType { growth: 10000, spread: 10000, growth_percentage: 0.0, spread_percentage: 0.0 },
            ],
            ore_queues: vec![Default::default(); 2],
            ore_ready: false,
            random: 0x2545_f491_4f6c_dd1d,
            idle_random: 0x9e37_79b9_7f4a_7c15,
            tick: 0,
            next_id: 0,
        }
    }

    /// Adds a unit at the centre of a cell and returns its id.
    pub fn add_unit(&mut self, cell: (i32, i32), facing: u8, stats: UnitStats, owner: u8) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.units.push(Unit::new(id, owner, cell, facing, stats));
        // A gunner vehicle starts with its first
        // weapon and turret.
        let gun = (stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).filter(|k| k.gunner).and_then(|k| k.gunner_weapons.first().copied());
        // Once a spy
        // has been in an enemy barracks or vehicle factory, the house's new
        // `Trainable` soldiers or (non-naval) vehicles start as veterans.
        let naval = (stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).is_some_and(|k| k.naval);
        let veteran = stats.trainable
            && self.houses.get(owner as usize).is_some_and(|house| if stats.infantry { house.veterans[0] } else { house.veterans[1] && stats.plane.is_none() && !naval });
        if let Some(unit) = self.units.last_mut() {
            unit.gun = gun;
            if veteran {
                unit.veterancy = 1.0;
                unit.shown_rank = 1;
            }
        }
        let ground = self.map.level(cell.0, cell.1) as i32 * combat::LEVEL_HEIGHT;
        if let Some(jet) = self.units.last_mut().and_then(|unit| unit.jet.as_mut()) {
            *jet = flight::Jet::on_ground(ground);
        }
        if let Some(index) = self.map.index(cell.0, cell.1) {
            self.claims[index] = id + 1;
        }
        id
    }

    /// Adds a foot soldier on one of a cell's spots (2, 3 or 4; anything else
    /// takes the first free one) and returns its id, or `None` if the cell is full.
    pub fn add_infantry(&mut self, cell: (i32, i32), spot: usize, facing: u8, stats: UnitStats, owner: u8) -> Option<u32> {
        let index = self.map.index(cell.0, cell.1)?;
        let free = |spot: usize| self.spots[index] & (1 << spot) == 0;
        let spot = Some(spot).filter(|&s| (2..=4).contains(&s) && free(s)).or_else(|| (2..=4).find(|&s| free(s)))?;
        let id = self.add_unit(cell, facing, UnitStats { infantry: true, ..stats }, owner);
        self.claims[index] = 0;
        let unit = self.units.last_mut()?;
        unit.x = cell.0 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].0;
        unit.y = cell.1 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].1;
        self.spots[index] |= 1 << spot;
        self.spot_house[index] = owner;
        Some(id)
    }

    /// Adds an animated terrain object; they are numbered in the order added.
    pub fn add_animated_terrain(&mut self, cell: (i32, i32), frames: u32, rate: u32, probability: f32) {
        self.animated.push(AnimatedTerrain {
            cell,
            frames,
            rate,
            probability,
            stage: 0,
            playing: 0,
            timer_start: 0,
            timer_length: 0,
        });
    }

    pub fn animated_terrain(&self) -> &[AnimatedTerrain] {
        &self.animated
    }

    /// The next number from the simulation's random stream.
    fn next_random(&mut self) -> u32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 16) as u32 & 0x7fff_ffff
    }

    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    pub fn unit(&self, id: u32) -> Option<&Unit> {
        self.units.iter().find(|unit| unit.id == id)
    }

    pub fn buildings(&self) -> &[Building] {
        &self.buildings
    }

    /// The top cell of the building a deploying unit at `cell` would become.
    // The foundation is placed so that the unit's cell is the one down and right of
    // its top cell.
    pub fn deploy_origin(cell: (i32, i32)) -> (i32, i32) {
        (cell.0 - 1, cell.1 - 1)
    }

    fn size_of(&self, building: u16) -> (i32, i32) {
        self.building_types.get(building as usize).map_or((1, 1), |kind| kind.size)
    }

    /// Whether a foundation of a size could go with its top cell at `origin`,
    /// ignoring anything held by `except` (a unit id plus one, or zero).
    // Every foundation cell is tested on its
    // own; a cell fails if a vehicle or building is on it, it is outside the
    // playable area, or its ground will not do: for a `WaterBound` building, land
    // a floating thing can cross and no overlay; for the rest, see `Map::buildable`.
    fn cell_clear(&self, cell: (i32, i32), except: u32, kind: u16, house: u8) -> bool {
        let Some(index) = self.map.index(cell.0, cell.1) else {
            return false;
        };
        let free = self.claims[index] == 0 || self.claims[index] == except;
        if !free || !self.map.in_playfield(cell.0, cell.1) || self.building_on(cell).is_some() {
            return false;
        }
        let kind = self.building_types.get(kind as usize);
        // A wall may go over the placer's own wall of
        // the same kind once that is damaged.
        if let Some(overlay) = kind.and_then(|kind| kind.wall_overlay) {
            if self.map.wall_kind[index] == overlay && self.map.wall_owner[index] == house && self.map.wall_data[index] > 15 {
                return true;
            }
        }
        if kind.is_some_and(|kind| kind.water_bound) {
            self.map.passable[index]
                && !self.map.overlay[index]
                && self.map.lands.0[self.map.land[index] as usize][SpeedType::Float as usize] > 0
        } else {
            self.map.buildable(cell.0, cell.1)
        }
    }

    fn foundation_clear(&self, origin: (i32, i32), kind: u16, except: u32) -> bool {
        let size = self.size_of(kind);
        let house = self.units.iter().find(|u| u.id + 1 == except).map_or(u8::MAX, |u| u.owner);
        (0..size.1).all(|dy| (0..size.0).all(|dx| self.cell_clear((origin.0 + dx, origin.1 + dy), except, kind, house)))
    }

    /// Whether the building a unit deploys into could stand where the unit is.
    // The unit's own hold on its cells is
    // lifted for the test, and neither motion nor facing enters into it; an order to
    // deploy waits for the unit to stop.
    // Its other tests (a tunnel entrance, being
    // deactivated, the rules for transports) never apply to an MCV in base RA2.
    pub fn can_deploy(&self, unit: u32) -> bool {
        let Some(unit) = self.unit(unit) else {
            return false;
        };
        let Some(building) = unit.stats.deploys_into else {
            return false;
        };
        self.foundation_clear(Self::deploy_origin(unit.cell()), building, unit.id + 1)
    }

    /// Whether an order to go to a cell shows the move cursor rather than the
    /// no-move one.
    // The cell must be in the playable area and free
    // to enter, or hold only something cloaked.
    pub fn can_move_to(&self, unit: u32, cell: (i32, i32)) -> bool {
        let Some(unit) = self.unit(unit) else {
            return false;
        };
        let others: Vec<Other> = self.others();
        entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, unit, cell, None, unit.cell(), -1) <= code::CLOAK
    }

    /// Whether a building can pack up into a vehicle.
    // It needs `UndeploysInto`, and a construction yard
    // also needs the repack option and a person as its owner (there is no
    // campaign here).
    pub fn can_undeploy(&self, building: u32) -> bool {
        self.buildings.iter().find(|b| b.id == building && b.buildup.is_none()).is_some_and(|b| {
            self.building_types
                .get(b.kind as usize)
                .is_some_and(|kind| kind.undeploys_into.is_some() && (!kind.construction_yard || (self.mcv_repacks && self.human(b.owner))))
        })
    }

    /// The cells that stop a unit deploying where it stands, with what holds each
    /// (a unit id plus one, or zero for the ground itself). For diagnostics.
    pub fn deploy_obstacles(&self, unit: u32) -> Vec<((i32, i32), u32)> {
        let Some(unit) = self.unit(unit) else {
            return Vec::new();
        };
        let Some(building) = unit.stats.deploys_into else {
            return Vec::new();
        };
        let origin = Self::deploy_origin(unit.cell());
        let size = self.size_of(building);
        let mut found = Vec::new();
        for dy in 0..size.1 {
            for dx in 0..size.0 {
                let cell = (origin.0 + dx, origin.1 + dy);
                let claim = self.map.index(cell.0, cell.1).map_or(0, |index| self.claims[index]);
                let ground = self.map.buildable(cell.0, cell.1);
                if !ground || (claim != 0 && claim != unit.id + 1) {
                    found.push((cell, if ground { claim } else { 0 }));
                }
            }
        }
        found
    }

    fn add_building(&mut self, owner: u8, kind: u16, origin: (i32, i32)) {
        let size = self.size_of(kind);
        // The foundation closes its cells to ground units.
        // With `NumberImpassableRows`, columns from
        // that one on stay open (a refinery's dock).
        let open_from = self.building_types.get(kind as usize).and_then(|kind| kind.impassable_rows).unwrap_or(i32::MAX);
        for dy in 0..size.1 {
            for dx in 0..size.0 {
                let cell = (origin.0 + dx, origin.1 + dy);
                if dx < open_from {
                    let level = self.map.level(cell.0, cell.1);
                    self.map.set_cell(cell.0, cell.1, false, level);
                }
                // Its cells are marked as a building's, as the map's own
                // buildings' are: the movement zones are the ground's and
                // don't count buildings, so one built and then lost leaves the
                // zones as they were. (Unmarked, a zone rebuild while it stood
                // made its cells solid ground for good, and a unit that later
                // drove onto them could never be sent anywhere.)
                if self.map.obstacle_kind(cell) == Obstacle::None as u8 {
                    self.map.set_obstacle(cell.0, cell.1, Obstacle::Building);
                }
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.buildings.push(Building {
            id,
            owner,
            kind,
            health: self.building_types.get(kind as usize).map_or(1, |kind| kind.strength),
            cell: origin,
            size,
            buildup: None,
            opened: self.tick,
            unload: None,
            docked: None,
            rally: None,
            primary: false,
            arms: combat::Arms::default(),
            repairing: false,
            curtain_until: 0,
            patients: Vec::new(),
            depot_next: 0,
            depot_busy: false,
            cash_at: 0,
            ammo: self.building_types.get(kind as usize).map_or(-1, |kind| kind.ammo),
            frozen: None,
            occupants: Vec::new(),
            muzzle: 0,
            looked: false,
            c4: None,
            captured: false,
            docks: vec![None; self.building_types.get(kind as usize).map_or(0, |kind| kind.number_of_docks)],
            charge: None,
            refining: None,
            supports: 0,
            supporting: None,
            last_supports: 0,
            chargers: Vec::new(),
            charged: false,
        });
        // The first factory of its kind (ships apart)
        // that a house has becomes its primary one.
        if let Some(kind_type) = self.building_types.get(kind as usize).filter(|t| t.factory.is_some()).cloned() {
            let taken = self.buildings.iter().any(|b| {
                b.owner == owner
                    && b.primary
                    && self.building_types.get(b.kind as usize).is_some_and(|t| t.factory == kind_type.factory && t.naval == kind_type.naval)
            });
            if let Some(building) = self.buildings.last_mut() {
                building.primary = !taken;
            }
        }
        self.buildings.last_mut().unwrap().buildup = self.start_buildup(kind, None);
    }

    /// Puts a building straight onto the map for a house, if it may stand there.
    /// For tests and debugging: no production, no payment.
    pub fn place_now(&mut self, house: u8, kind: u16, origin: (i32, i32)) -> bool {
        let ok = self.can_place(house, kind, origin);
        if ok {
            match self.building_types[kind as usize].wall_overlay {
                Some(overlay) => {
                    let line = self.wall_line(house, kind, origin);
                    for cell in std::iter::once(origin).chain(line) {
                        self.map.put_wall(cell, overlay, 0, house);
                        self.map.link_walls(cell);
                    }
                    self.map.build_zones();
                }
                None => self.add_building(house, kind, origin),
            }
        }
        ok
    }

    /// Adds a building the map starts with: standing, with a share of its health
    /// out of 256. Returns its id.
    pub fn add_map_building(&mut self, owner: u8, kind: u16, origin: (i32, i32), health: i32) -> u32 {
        self.add_building(owner, kind, origin);
        let full = self.building_types.get(kind as usize).map_or(1, |kind| kind.strength);
        let building = self.buildings.last_mut().unwrap();
        building.buildup = None;
        building.health = (full as i64 * health.clamp(1, 256) as i64 / 256).max(1) as i32;
        building.id
    }

    /// A build-up animation for a building type, from its first image.
    // Each image lasts the build-up time divided by the number of images, rounded
    // down, and the animation is over on reaching the last one.
    fn start_buildup(&self, kind: u16, packing: Option<(i32, i32)>) -> Option<Buildup> {
        let count = self.building_types.get(kind as usize).map_or(0, |kind| kind.buildup_frames);
        let rate = self.buildup_ticks.checked_div(count).unwrap_or(0);
        (count > 1 && rate > 0).then(|| Buildup { stage: 0, count, rate, next: self.tick + rate as u64, packing, selling: false })
    }

    /// The vehicle a packed-up building became, once it has.
    pub fn unpacked_into(&self, building: u32) -> Option<u32> {
        self.unpacked.iter().find(|(from, _)| *from == building).map(|(_, unit)| *unit)
    }

    /// The production rules of an item.
    fn terms(&self, item: Item) -> Option<Terms<'_>> {
        match item {
            Item::Building(index) => self.building_types.get(index as usize).map(|kind| Terms {
                cost: kind.cost,
                tech_level: kind.tech_level,
                owners: kind.owners,
                required: kind.required_houses,
                forbidden: kind.forbidden_houses,
                prerequisites: &kind.prerequisites,
                overrides: &kind.prerequisite_override,
                build_limit: kind.build_limit,
                factory: Factory::Building,
                naval: false,
                wall: kind.wall_overlay.is_some(),
                stolen_tech: 0,
            }),
            Item::Unit(index) => self.unit_types.get(index as usize).map(|kind| Terms {
                cost: kind.cost,
                tech_level: kind.tech_level,
                owners: kind.owners,
                required: kind.required_houses,
                forbidden: kind.forbidden_houses,
                prerequisites: &kind.prerequisites,
                overrides: &kind.prerequisite_override,
                build_limit: kind.build_limit,
                factory: if kind.infantry {
                    Factory::Infantry
                } else if kind.stats.plane.is_some() {
                    Factory::Aircraft
                } else {
                    Factory::Unit
                },
                naval: kind.naval,
                wall: false,
                stolen_tech: kind.requires_stolen_tech,
            }),
        }
    }

    /// Whether a house may build a type at all.
    // In this order: an owned `PrerequisiteOverride`
    // building waives the rest; no `TechLevel` or -1 never; `RequiredHouses` and
    // `ForbiddenHouses` against the house's own country; `TechLevel` against the
    // house's; then every `Prerequisite` entry needs a building on the map, counted
    // from the moment it is placed.
    fn may_build(&self, house: u8, item: Item) -> bool {
        let (Some(terms), Some(owner)) = (self.terms(item), self.houses.get(house as usize)) else {
            return false;
        };
        // With super weapons
        // off, a building whose `SuperWeapon` can be disabled cannot be built.
        if let (false, Item::Building(kind)) = (self.supers_allowed, item) {
            let barred = self.building_types.get(kind as usize).and_then(|t| t.super_weapon).and_then(|index| self.super_types.get(index as usize)).is_some_and(|t| t.disableable);
            if barred {
                return false;
            }
        }
        let owns = |kind: u16| self.buildings.iter().any(|b| b.owner == house && b.kind == kind);
        if !terms.overrides.iter().any(|&k| owns(k)) {
            let bit = 1u32 << owner.country.min(31);
            // A type that needs stolen technology
            // waits for a spy to have been in the right laboratory.
            if terms.tech_level < 0
                || (terms.stolen_tech != 0 && !owner.stolen[(terms.stolen_tech as usize - 1).min(1)])
                || terms.required.is_some_and(|mask| mask & bit == 0)
                || terms.forbidden.is_some_and(|mask| mask & bit != 0)
                || terms.tech_level > owner.tech_level
            {
                return false;
            }
        }
        terms.prerequisites.iter().all(|any| any.iter().any(|&k| owns(k)))
    }

    /// The house's buildings that can make an item.
    // A factory of the right kind that is not being
    // sold, whose own `Owner` shares a country with the type's (the player's
    // country does not come into it), and naval for ships and only for ships.
    fn factories(&self, house: u8, item: Item) -> impl Iterator<Item = &Building> {
        let terms = self.terms(item);
        self.buildings.iter().filter(move |b| {
            let Some(terms) = &terms else {
                return false;
            };
            b.owner == house
                && b.buildup.is_none_or(|buildup| buildup.packing.is_none() && !buildup.selling)
                && self.building_types.get(b.kind as usize).is_some_and(|factory| {
                    factory.factory == Some(terms.factory)
                        && factory.owners & terms.owners != 0
                        && (terms.factory != Factory::Unit || factory.naval == terms.naval)
                })
        })
    }

    fn has_factory(&self, house: u8, item: Item) -> bool {
        self.factories(house, item).next().is_some()
    }

    /// Whether a house already has as many of a type as its `BuildLimit` allows.
    // The cameo stays, greyed.
    pub fn at_limit(&self, house: u8, item: Item) -> bool {
        let limit = self.terms(item).and_then(|terms| terms.build_limit);
        let owned = match item {
            Item::Building(kind) => self.buildings.iter().filter(|b| b.owner == house && b.kind == kind).count(),
            Item::Unit(kind) => {
                let tab = self.unit_slot(kind);
                let queued = self.houses.get(house as usize).map_or(0, |h| {
                    h.tab_slot(tab).filter(|p| p.building == kind).into_iter().count() + h.queue(tab).iter().filter(|&&k| k == kind).count()
                });
                self.units.iter().filter(|u| u.owner == house && u.stats.kind == kind + 1).count() + queued
            }
        } as i32;
        // The aircraft that live on pads, together,
        // owned and being built, may not outnumber the house's pads.
        if let Item::Unit(kind) = item {
            if self.unit_types.get(kind as usize).is_some_and(|kind| kind.stats.plane.is_some_and(|plane| plane.airport_bound)) {
                let bound = |stats: &UnitStats| stats.plane.is_some_and(|plane| plane.airport_bound);
                let owned = self.units.iter().filter(|u| u.owner == house && bound(&u.stats)).count();
                let queued = self.houses.get(house as usize).map_or(0, |h| {
                    h.aircraft.iter().map(|p| p.building).chain(h.aircraft_queue.iter().copied()).filter(|&unit| self.unit_types.get(unit as usize).is_some_and(|k| bound(&k.stats))).count()
                });
                return owned + queued >= self.dock_count(house);
            }
        }
        limit.is_some_and(|limit| limit > 0 && owned >= limit)
    }

    /// Whether a request for a unit would be turned away because the queue
    /// behind the one being made is full.
    // With the factory busy, a request past
    // `MaximumQueuedObjects` is refused, and a person hears `ScoldSound`.
    pub fn queue_full(&self, house: u8, unit: u16) -> bool {
        let tab = self.unit_slot(unit);
        self.houses.get(house as usize).is_some_and(|h| h.tab_slot(tab).is_some() && h.queue(tab).len() >= self.queue_limit)
    }

    pub fn at_build_limit(&self, house: u8, building: u16) -> bool {
        self.at_limit(house, Item::Building(building))
    }

    /// Ticks an item takes to produce for a house, as things stand.
    // Cost times `BuildSpeed` times 0.9,
    // slowed by low power, quickened by each further factory of the kind, and
    // multiplied for walls; every step rounded down.
    pub fn time_to_produce(&self, house: u8, item: Item) -> i32 {
        let Some(terms) = self.terms(item) else {
            return 1;
        };
        let rules = &self.build_rules;
        let mut time = (terms.cost as f64 * rules.build_speed * 0.9) as i32;
        let (output, drain) = self.power(house);
        let fraction = if output < drain && drain != 0 { output as f64 / drain as f64 } else { 1.0 };
        let mut speed = 1.0 - (1.0 - fraction) * rules.low_power_penalty;
        if speed <= rules.min_low_power_speed {
            speed = rules.min_low_power_speed;
        }
        if fraction < 1.0 && speed >= rules.max_low_power_speed {
            speed = rules.max_low_power_speed;
        }
        if speed == 0.0 {
            speed = 0.01;
        }
        time = (time as f64 / speed) as i32;
        let factories = self
            .buildings
            .iter()
            .filter(|b| {
                b.owner == house
                    && self.building_types.get(b.kind as usize).is_some_and(|k| {
                        k.factory == Some(terms.factory) && (terms.factory != Factory::Unit || k.naval == terms.naval)
                    })
            })
            .count();
        if rules.multiple_factory > 0.0 {
            for _ in 1..factories {
                time = (time as f64 * rules.multiple_factory) as i32;
            }
        }
        if terms.wall {
            time = (time as f64 * rules.wall_coefficient) as i32;
        }
        time
    }

    pub fn time_to_build(&self, house: u8, building: u16) -> i32 {
        self.time_to_produce(house, Item::Building(building))
    }

    /// Ticks per production stage.
    fn stage_delay(&self, house: u8, item: Item) -> i32 {
        (self.time_to_produce(house, item) / PRODUCTION_STAGES).clamp(1, 255)
    }

    /// The unit types a house's sidebar offers, in the order it shows them.
    /// Foot soldiers go in the third tab and the rest in the fourth.
    // As for buildings; ships sort after
    // everything else.
    pub fn buildable_units(&self, house: u8) -> Vec<u16> {
        let side = self.houses.get(house as usize).map_or(0, |house| house.side);
        let mut list: Vec<u16> = (0..self.unit_types.len() as u16)
            .filter(|&index| {
                self.may_build(house, Item::Unit(index))
                    && self.has_factory(house, Item::Unit(index))
            })
            .collect();
        list.sort_by(|&a, &b| {
            let (a, b) = (&self.unit_types[a as usize], &self.unit_types[b as usize]);
            (a.planning_side != side, a.naval, a.tech_level, a.cost, &a.sort_name)
                .cmp(&(b.planning_side != side, b.naval, b.tech_level, b.cost, &b.sort_name))
        });
        list
    }

    /// How many of a unit type a house has in hand: in production plus queued.
    pub fn units_in_hand(&self, house: u8, unit: u16) -> usize {
        let Some(owner) = self.houses.get(house as usize) else {
            return 0;
        };
        let (slot, queue) = match self.unit_slot(unit) {
            2 => (&owner.infantry, &owner.infantry_queue),
            4 => (&owner.ships, &owner.ship_queue),
            5 => (&owner.aircraft, &owner.aircraft_queue),
            _ => (&owner.vehicles, &owner.vehicle_queue),
        };
        usize::from(slot.is_some_and(|p| p.building == unit)) + queue.iter().filter(|&&queued| queued == unit).count()
    }

    /// The sidebar tab, and so the production slot, of a unit type: 2 for foot
    /// soldiers, 3 for everything else.
    pub fn unit_tab(&self, unit: u16) -> usize {
        if self.unit_types.get(unit as usize).is_some_and(|kind| kind.infantry) { 2 } else { 3 }
    }

    /// The production slot of a unit type: 2 foot soldiers, 3 vehicles, 4 ships,
    /// 5 aircraft (0 and 1 are buildings and combat buildings).
    pub fn unit_slot(&self, unit: u16) -> usize {
        match self.unit_types.get(unit as usize) {
            Some(kind) if kind.infantry => 2,
            Some(kind) if kind.stats.plane.is_some() => 5,
            Some(kind) if kind.naval => 4,
            _ => 3,
        }
    }

    fn start_unit(&mut self, house: u8, unit: u16) {
        let tab = self.unit_slot(unit);
        let rate = self.stage_delay(house, Item::Unit(unit));
        let cost = self.unit_types.get(unit as usize).map_or(0, |kind| kind.cost);
        let next = self.tick + rate as u64;
        if let Some(owner) = self.houses.get_mut(house as usize) {
            *owner.tab_slot_mut(tab) =
                Some(Production { building: unit, stage: 0, rate, next, balance: cost, suspended: false, stalled: false });
        }
    }

    // The front of the queue is started.
    fn start_next_queued(&mut self, house: u8, tab: usize) {
        let Some(owner) = self.houses.get_mut(house as usize) else {
            return;
        };
        if owner.queue_mut(tab).is_empty() {
            return;
        }
        let unit = owner.queue_mut(tab).remove(0);
        self.start_unit(house, unit);
    }

    /// Puts a finished unit on the map at its factory.
    // A war factory's vehicle appears inside it at `ExitCoord`, facing east, and
    // the factory then lets it out; a factory still doing that hands the job to
    // an idle one of the same type, or is busy. A foot soldier appears on the
    // foundation cell next to its exit cell, facing away from the building's
    // middle. Other vehicles appear likewise and drive to the exit cell. Ships
    // appear on a free water cell near the yard's middle.
    // From the online
    // factories not being sold, the first primary, else the last.
    fn exit_unit(&mut self, house: u8, unit: u16) -> Exit {
        let Some(kind) = self.unit_types.get(unit as usize).cloned() else {
            return Exit::Failed;
        };
        let stats = UnitStats { kind: unit + 1, ..kind.stats };
        let ids: Vec<u32> = self.factories(house, Item::Unit(unit)).filter(|b| self.online(b)).map(|b| b.id).collect();
        let at = |world: &Self, id: u32| world.buildings.iter().position(|b| b.id == id);
        // The primary factory, else the last of them.
        let chosen = ids.iter().copied().find(|&id| self.buildings.iter().any(|b| b.id == id && b.primary)).or(ids.last().copied());
        let Some(mut index) = chosen.and_then(|id| at(self, id)) else {
            return Exit::Failed;
        };
        let factory = self.building_types[self.buildings[index].kind as usize].clone();
        let (origin, size) = (self.buildings[index].cell, self.buildings[index].size);

        if stats.plane.is_some() {
            return if self.exit_aircraft(house, stats) { Exit::Out } else { Exit::Failed };
        }
        if factory.weapons_factory && !factory.naval {
            if self.buildings[index].unload.is_some() {
                let kind = self.buildings[index].kind;
                let idle = self.buildings.iter().position(|b| ids.contains(&b.id) && b.kind == kind && b.unload.is_none());
                match idle {
                    Some(other) => index = other,
                    None => return Exit::Busy,
                }
            }
            let origin = self.buildings[index].cell;
            let (cx, cy) = cell_centre(origin);
            let (x, y) = (cx + factory.exit_coord.0, cy + factory.exit_coord.1);
            let id = self.add_unit((x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL)), 0x40, stats, house);
            let rally = self.buildings[index].rally;
            if let Some(unit) = self.units.last_mut() {
                unit.x = x;
                unit.y = y;
                unit.tethered = true;
                unit.rally = rally;
            }
            self.buildings[index].unload = Some(Unload { status: 0, next: self.tick + 1, unit: id, door: 0 });
            return Exit::Out;
        }

        let probe = Unit::new(u32::MAX - 1, house, origin, 0, stats);
        if factory.naval {
            let centre = (origin.0 + size.0 / 2, origin.1 + size.1 / 2);
            // Any zone, any level, picked by frame.
            return match nearby(&self.map, &self.claims, &self.spots, &probe, centre, Near::new(true, false, None), self.tick) {
                Some(cell) => {
                    self.add_unit(cell, 0x40, stats, house);
                    Exit::Out
                }
                None => Exit::Failed,
            };
        }

        // Once a barracks has let a soldier out, each
        // of the house's cloning vats makes a free one of the same type and lets
        // it out by its own exit.
        let made = self.exit_on_foot(index, stats, house);
        if made == Exit::Out && stats.infantry && !factory.cloning {
            let vats: Vec<usize> = (0..self.buildings.len())
                .filter(|&i| self.buildings[i].owner == house && self.building_types.get(self.buildings[i].kind as usize).is_some_and(|k| k.cloning))
                .collect();
            for vat in vats {
                self.exit_on_foot(vat, stats, house);
            }
        }
        made
    }

    /// Puts a unit out of a building that has no door for it: on the foundation
    /// cell beside its exit cell.
    fn exit_on_foot(&mut self, index: usize, stats: UnitStats, house: u8) -> Exit {
        let factory = self.building_types[self.buildings[index].kind as usize].clone();
        let (origin, size) = (self.buildings[index].cell, self.buildings[index].size);
        let probe = Unit::new(u32::MAX - 1, house, origin, 0, stats);
        let others: Vec<Other> = self.others();
        let free = |cell: (i32, i32)| {
            self.map.index(cell.0, cell.1).is_some()
                && entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, &probe, cell, None, cell, -1) == code::OK
        };
        let special = if factory.gdi_barracks {
            Some((origin.0 + 1, origin.1 + 2))
        } else if factory.nod_barracks {
            Some((origin.0 + 2, origin.1 + 2))
        } else {
            None
        };
        let out = special.filter(|&cell| free(cell)).or_else(|| {
            exit_list(size).into_iter().map(|(dx, dy)| (origin.0 + dx, origin.1 + dy)).find(|&cell| free(cell))
        });
        let Some(out) = out else {
            return Exit::Failed;
        };
        // The cell it appears on: the exit cell pulled back into the foundation.
        let inside = (
            out.0.clamp(origin.0, origin.0 + size.0 - 1),
            out.1.clamp(origin.1, origin.1 + size.1 - 1),
        );
        let (mut x, mut y) = cell_centre(inside);
        if Some(out) == special {
            x += factory.exit_coord.0;
            y += factory.exit_coord.1;
        }
        let middle = (origin.0 * LEPTONS_PER_CELL + size.0 * 128, origin.1 * LEPTONS_PER_CELL + size.1 * 128);
        let (ox, oy) = cell_centre(out);
        let aim = ((((middle.1 - oy) as f64).atan2((ox - middle.0) as f64) - std::f64::consts::FRAC_PI_2) * -10430.06) as i32;
        let facing = ((((aim as u16) >> 7) + 1) >> 1) as u8;
        if stats.infantry {
            // On the free spot nearest that point. No order is given: a soldier
            // standing on a building walks off it. With a rally point it walks to
            // the exit cell and from there to the point.
            let rally = self.buildings[index].rally;
            match self.add_infantry(inside, spot_index(x, y), facing, stats, house) {
                Some(id) => {
                    // One that flies does not walk to the exit cell: with a rally
                    // point it takes off from where it stands and flies straight
                    // there; with none it stays down until it is moved off the
                    // building.
                    let flyer = self.units.iter().position(|u| u.id == id && u.jet.is_some());
                    if let (Some(rally), Some(flyer)) = (rally, flyer) {
                        self.send(flyer, rally);
                    } else if let (Some(rally), Some(unit)) = (rally, self.units.iter_mut().find(|u| u.id == id)) {
                        unit.dest = Some(out);
                        unit.rally = Some(rally);
                    }
                    Exit::Out
                }
                None => Exit::Failed,
            }
        } else {
            let id = self.add_unit(inside, facing, stats, house);
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                unit.dest = Some(out);
            }
            Exit::Out
        }
    }

    /// What a building is sold for.
    // `Soylent` if it has one; otherwise its
    // cost, less that of a unit that came free with it, times `RefundPercent`.
    pub fn refund(&self, kind: u16) -> i32 {
        let Some(kind) = self.building_types.get(kind as usize) else {
            return 0;
        };
        if kind.soylent != 0 {
            return kind.soylent;
        }
        let free = kind.free_unit.and_then(|unit| self.unit_types.get(unit as usize)).map_or(0, |unit| unit.cost);
        ((kind.cost - free).max(0) as f64 * self.refund_percent) as i32
    }

    /// The people who walk out of a building being sold.
    // As many as the building's crew count; each its crew type, and at most
    // one engineer; each put on a cell of the foundation picked at random and
    // told to scatter, at full health.
    fn sale_crew(&mut self, index: usize) {
        let building = self.buildings[index].clone();
        let Some(kind) = self.building_types.get(building.kind as usize).cloned() else {
            return;
        };
        let count = self.crew_count(&building, &kind);
        let mut engineer = false;
        for _ in 0..count {
            let mut crew = self.building_crew_kind(&building, &kind);
            while engineer && crew.is_some() && crew == self.crew_types.2 {
                crew = self.building_crew_kind(&building, &kind);
            }
            if crew.is_some() && crew == self.crew_types.2 {
                engineer = true;
            }
            let Some(stats) = crew.and_then(|crew| self.unit_types.get(crew as usize).map(|t| UnitStats { kind: crew + 1, ..t.stats })) else {
                continue;
            };
            let pick = self.next_random() as i32 % (building.size.0 * building.size.1).max(1);
            let cell = (building.cell.0 + pick % building.size.0, building.cell.1 + pick / building.size.0);
            self.add_infantry(cell, 0, 0x80, stats, building.owner);
        }
    }

    /// A sold building's last moment: the money, and it is gone.
    fn finish_sale(&mut self, id: u32) {
        let Some(index) = self.buildings.iter().position(|b| b.id == id) else {
            return;
        };
        let building = self.demolish(index);
        let refund = self.refund(building.kind);
        if let Some(house) = self.houses.get_mut(building.owner as usize) {
            house.credits += refund;
        }
        self.events.push(combat::Event::Sold { owner: building.owner });
    }

    /// Takes a building off the map and opens its cells again.
    pub(crate) fn demolish(&mut self, index: usize) -> Building {
        let building = self.buildings.remove(index);
        for dy in 0..building.size.1 {
            for dx in 0..building.size.0 {
                let cell = (building.cell.0 + dx, building.cell.1 + dy);
                let level = self.map.level(cell.0, cell.1);
                self.map.set_cell(cell.0, cell.1, true, level);
                if self.map.obstacle_kind(cell) == Obstacle::Building as u8 {
                    self.map.set_obstacle(cell.0, cell.1, Obstacle::None);
                }
            }
        }
        for unit in &mut self.units {
            if unit.work.dock == Some(building.id) {
                unit.work.dock = None;
            }
        }
        // Aircraft parked on a pad building go with
        // it; those in the air only lose their place.
        let mut parked = Vec::new();
        for unit in &mut self.units {
            if let Some(plane) = unit.plane.as_mut().filter(|plane| plane.contact == Some(building.id)) {
                plane.contact = None;
                if plane.z == 0 {
                    parked.push(unit.id);
                }
            }
        }
        for id in parked {
            self.dying.push((combat::Aim::Unit(id), 0));
        }
        building
    }

    /// Whether a building is damaged and standing, so that repairs would help.
    pub fn can_repair(&self, building: u32) -> bool {
        self.buildings.iter().find(|b| b.id == building && b.buildup.is_none()).is_some_and(|b| {
            self.building_types.get(b.kind as usize).is_some_and(|kind| b.health < kind.strength)
        })
    }

    /// One step of repairs for every building being repaired.
    // On every frame that divides by
    // `RepairRate` × 900 (14), each takes `RepairStep` points for a price of its
    // cost over the steps a full repair takes, times `RepairPercent`, at least one
    // credit; it stops without a word when full or when the money runs out.
    fn repair_buildings(&mut self) {
        // A computer house with
        // `CreditReserve` to its name starts mending one damaged building, then
        // waits `RepairDelay` times 225 to 1800 frames, drawn at random, before
        // it starts another.
        for house in 1..self.houses.len() {
            let Some((difficulty, due)) = self.houses[house].ai.as_ref().map(|brain| (brain.difficulty.min(2), brain.repair_at)) else {
                continue;
            };
            if self.tick < due || self.houses[house].credits < self.ai_rules.credit_reserve {
                continue;
            }
            let damaged = self.buildings.iter().position(|b| {
                b.owner as usize == house && !b.repairing && b.buildup.is_none() && b.health > 0 && self.building_types.get(b.kind as usize).is_some_and(|kind| b.health < kind.strength)
            });
            if let Some(index) = damaged {
                self.buildings[index].repairing = true;
                let delay = self.ai_rules.repair_delay[difficulty];
                let (low, high) = ((delay * 225.0) as u32, (delay * 1800.0) as u32);
                let wait = low + self.random_below(high.saturating_sub(low) + 1);
                if let Some(brain) = self.houses[house].ai.as_mut() {
                    brain.repair_at = self.tick + wait as u64;
                }
            }
        }
        if self.repair_interval == 0 || self.tick % self.repair_interval as u64 != 0 {
            return;
        }
        for index in 0..self.buildings.len() {
            if !self.buildings[index].repairing {
                continue;
            }
            let (kind, owner) = (self.buildings[index].kind, self.buildings[index].owner as usize);
            let Some(kind) = self.building_types.get(kind as usize) else {
                continue;
            };
            let free = kind.free_unit.and_then(|unit| self.unit_types.get(unit as usize)).map_or(0, |unit| unit.cost);
            let steps = (kind.strength / self.repair_step.max(1)).max(1);
            let cost = ((((kind.cost - free).max(0) / steps) as f64 * self.repair_percent) as i32).max(1);
            let strength = kind.strength;
            let Some(house) = self.houses.get_mut(owner) else {
                continue;
            };
            let building = &mut self.buildings[index];
            if house.credits < cost {
                building.repairing = false;
                continue;
            }
            house.credits -= cost;
            building.health += self.repair_step;
            if building.health >= strength {
                building.health = strength;
                building.repairing = false;
            }
        }
    }

    fn kind_of(&self, unit: &Unit) -> Option<&UnitType> {
        self.unit_types.get((unit.stats.kind as usize).checked_sub(1)?)
    }

    /// Whether a unit may board a transport.
    // The transport is a friend's,
    // the unit is no bigger than its `SizeLimit`, and there is room for its `Size`
    // within `Passengers`. A soldier that hovers never boards.
    pub fn can_board(&self, unit: u32, transport: u32) -> bool {
        let (Some(unit), Some(transport)) = (self.unit(unit), self.unit(transport)) else {
            return false;
        };
        let (Some(kind), Some(carrier)) = (self.kind_of(unit), self.kind_of(transport)) else {
            return false;
        };
        let aboard: i32 = transport.cargo.iter().filter_map(|u| self.kind_of(u)).map(|k| k.size).sum();
        unit.id != transport.id
            && unit.owner == transport.owner
            && carrier.passengers > 0
            && kind.size <= carrier.size_limit
            && aboard + kind.size <= carrier.passengers
            && !unit.stats.jet.is_some_and(|jet| jet.balloon_hover)
    }

    /// Puts a unit that had been off the map back among the others.
    // The list is kept in id order: units act in that order, and who holds a
    // cell is looked up by it.
    pub(crate) fn put_back(&mut self, unit: Unit) {
        let at = self.units.partition_point(|other| other.id < unit.id);
        self.units.insert(at, unit);
    }

    /// Takes a unit off the map, freeing the ground it held.
    fn lift(&mut self, index: usize) -> Unit {
        self.leave_dock(index);
        let unit = self.units.remove(index);
        for claim in &mut self.claims {
            if *claim == unit.id + 1 {
                *claim = 0;
            }
        }
        if unit.stats.infantry {
            let (x, y) = unit.head_to.unwrap_or((unit.x, unit.y));
            if let Some(at) = self.map.index(x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL)) {
                self.spots[at] &= !(1 << spot_index(x, y));
            }
        }
        unit
    }

    /// Passengers getting aboard and getting out.
    // A passenger is taken aboard when it reaches a transport that is standing
    // still; the first aboard a gunner vehicle sets its weapon and turret, and the
    // last out sets them back. Out, one goes every 14 to 16 frames, the last aboard
    // first, to the first free cell round the transport looking clockwise from
    // straight behind it, and walks one cell further if that is free.
    fn board_and_unload(&mut self) {
        let mut index = 0;
        while index < self.units.len() {
            let unit = &self.units[index];
            let Some(wanted) = unit.boarding else {
                index += 1;
                continue;
            };
            if unit.is_moving() || self.tick < unit.warp_until {
                index += 1;
                continue;
            }
            let (id, cell, on_foot) = (unit.id, unit.cell(), unit.stats.infantry);
            // A soldier walks on to the transport's own cell and goes aboard
            // when it reaches its spot there.
            let carrier = self.unit(wanted).map(|t| (t.cell(), t.is_moving(), self.altitude(t)));
            let Some((at, moving, height)) = carrier.filter(|_| self.can_board(id, wanted)) else {
                self.units[index].boarding = None;
                index += 1;
                continue;
            };
            let reach = if on_foot { 0 } else { 1 };
            let ready = !moving && height == 0 && (at.0 - cell.0).abs() <= reach && (at.1 - cell.1).abs() <= reach;
            if !ready {
                // Not there yet (the transport has moved, or the way was shut):
                // go to where it is now, once it is still and down.
                if !moving && height == 0 && !(self.units[index].stats.teleporter) {
                    self.send(index, at);
                }
                index += 1;
                continue;
            }
            let mut passenger = self.lift(index);
            passenger.boarding = None;
            passenger.dest = None;
            passenger.head_to = None;
            passenger.drop_path();
            passenger.arms = combat::Arms::default();
            let mode = self.kind_of(&passenger).map_or(0, |kind| kind.ifv_mode);
            if let Some(at) = self.units.iter().position(|u| u.id == wanted) {
                let gun = self.kind_of(&self.units[at]).filter(|kind| kind.gunner).and_then(|kind| kind.gunner_weapons.get(mode).copied());
                let transport = &mut self.units[at];
                if transport.cargo.is_empty() && gun.is_some() {
                    transport.gun = gun;
                    transport.gun_mode = mode as u8;
                }
                transport.cargo.push(passenger);
                // With the transport's own sound.
                self.events.push(combat::Event::Boarded { transport: wanted });
            }
        }

        for index in 0..self.units.len() {
            let Some(due) = self.units[index].unloading else {
                continue;
            };
            if self.units[index].cargo.is_empty() {
                self.units[index].unloading = None;
                continue;
            }
            if self.tick < due || self.units[index].is_moving() || self.altitude(&self.units[index]) != 0 {
                continue;
            }
            let delay = self.mission_delay();
            self.units[index].unloading = Some(self.tick + delay);
            let Some(mut passenger) = self.units[index].cargo.pop() else {
                continue;
            };
            let (here, behind, owner) = (self.units[index].cell(), (self.units[index].heading() + 4) & 7, self.units[index].owner);
            let others: Vec<Other> = self.others();
            let free = |world: &Self, passenger: &Unit, cell: (i32, i32)| {
                entry_code(&world.map, &world.claims, (&world.spots, &world.spot_house), &others, &world.buildings, passenger, cell, None, cell, -1) == code::OK
            };
            let way = (0..8).map(|turn| (behind + turn) & 7).find(|&dir| free(self, &passenger, step(here, dir)));
            let Some(dir) = way else {
                // Nowhere to go: back aboard until the next try.
                self.units[index].cargo.push(passenger);
                continue;
            };
            let cell = step(here, dir);
            let Some(at) = self.map.index(cell.0, cell.1) else {
                self.units[index].cargo.push(passenger);
                continue;
            };
            (passenger.x, passenger.y) = cell_centre(cell);
            if passenger.stats.infantry {
                let spot = (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0).unwrap_or(2);
                passenger.x = cell.0 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].0;
                passenger.y = cell.1 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].1;
                self.spots[at] |= 1 << spot;
                self.spot_house[at] = owner;
            } else {
                self.claims[at] = passenger.id + 1;
            }
            passenger.facing = (dir as u8) << 5;
            passenger.turn = Turn::facing(passenger.facing);
            let further = step(cell, dir);
            if free(self, &passenger, further) {
                passenger.dest = Some(further);
                passenger.path_timer = 0;
                passenger.retries = 10;
            }
            if let Some(jet) = passenger.jet.as_mut() {
                *jet = flight::Jet::on_ground(self.map.level(cell.0, cell.1) as i32 * combat::LEVEL_HEIGHT);
            }
            // Putting the passenger back among the units can move the
            // transport's own place in the list.
            let transport = self.units[index].id;
            self.put_back(passenger);
            let Some(index) = self.units.iter().position(|u| u.id == transport) else {
                continue;
            };
            // With the transport's own sound.
            self.events.push(combat::Event::Alighted { transport });
            if self.units[index].cargo.is_empty() {
                let rest = self.kind_of(&self.units[index]).filter(|kind| kind.gunner).and_then(|kind| kind.gunner_weapons.first().copied());
                self.units[index].gun = rest;
                self.units[index].gun_mode = 0;
                self.units[index].unloading = None;
            }
        }
    }

    /// Whether a building has a rally point to set.
    // Factories of vehicles, soldiers and aircraft,
    // cloning vats and repair depots.
    pub fn has_rally_point(&self, building: u32) -> bool {
        self.buildings.iter().find(|b| b.id == building).and_then(|b| self.building_types.get(b.kind as usize)).is_some_and(|kind| {
            matches!(kind.factory, Some(Factory::Unit | Factory::Infantry | Factory::Aircraft)) || kind.cloning || kind.unit_repair
        })
    }

    fn building_on(&self, cell: (i32, i32)) -> Option<usize> {
        self.buildings.iter().position(|b| {
            (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1)
        })
    }

    /// Gives a unit somewhere to go.
    fn send(&mut self, index: usize, cell: (i32, i32)) {
        // A flyer is given a point to come down at.
        if let Some(stats) = self.units[index].stats.jet {
            let point = flight::landing_point(&self.map, &self.claims, &self.spots, &self.units[index], cell, self.tick);
            let unit = &mut self.units[index];
            if let (Some(point), Some(jet)) = (point, unit.jet.as_mut()) {
                jet.fly_to(point, stats.height);
                unit.dest = Some((point.0.div_euclid(LEPTONS_PER_CELL), point.1.div_euclid(LEPTONS_PER_CELL)));
            }
            return;
        }
        // Whatever sends a `Teleporter` somewhere
        // (an order, the approach to a target, going into something), it
        // jumps there. (The chrono miner drives out and jumps home by its own
        // rules.)
        if self.units[index].stats.teleporter && !self.units[index].stats.harvester {
            self.teleport(index, cell);
            return;
        }
        let unit = &mut self.units[index];
        unit.dest = Some(cell);
        unit.drop_path();
        unit.path_timer = 0;
        unit.retries = 10;
    }

    /// A mission's ordinary wait before it runs again.
    // The mission's `Rate` × 900 (14 for these) plus
    // 0, 1 or 2 at random.
    fn mission_delay(&mut self) -> u64 {
        14 + (self.next_random() % 3) as u64
    }

    /// A building that has just finished going up.
    // A type with `FreeUnit` gets one, on the cell below its middle, facing
    // west and set to mining; if that fails, on a free cell near the building;
    // if that fails too its price is handed back.
    fn grand_opening(&mut self, id: u32) {
        let Some(building) = self.buildings.iter().find(|b| b.id == id) else {
            return;
        };
        // A `RevealToAll` building shows itself to
        // the houses of people other than its owner, out to its `Sight` and
        // three more; and they are told which super weapon it holds.
        if let Some(kind) = self.building_types.get(building.kind as usize).filter(|kind| kind.reveal_to_all) {
            let (owner, cell, size, sight, index) = (building.owner, building.cell, building.size, kind.fighter.sight, kind.super_weapon);
            let centre = (cell.0 + size.0 / 2, cell.1 + size.1 / 2);
            let level = self.map.level(centre.0, centre.1) as i32;
            for house in 0..self.houses.len() as u8 {
                if house != owner && self.human(house) {
                    self.map.reveal(house, centre, level, sight + 3);
                }
            }
            if let Some(index) = index {
                self.events.push(combat::Event::SuperDetected { owner, index, cell: centre });
            }
        }
        let Some(building) = self.buildings.iter().find(|b| b.id == id) else {
            return;
        };
        let Some(free) = self.building_types.get(building.kind as usize).and_then(|kind| kind.free_unit) else {
            return;
        };
        let Some(kind) = self.unit_types.get(free as usize) else {
            return;
        };
        let stats = UnitStats { kind: free + 1, ..kind.stats };
        let cell = (building.cell.0 + (building.size.0 - 1) / 2 + (building.size.0 - 1) % 2, building.cell.1 + (building.size.1 - 1) / 2 + 1);
        let owner = building.owner;
        self.add_unit(cell, 0xc0, stats, owner);
        let now = self.tick;
        if let Some(unit) = self.units.last_mut() {
            unit.work.job = Job::Harvest;
            unit.work.next = now + 1;
        }
    }

    /// Lets go of the refinery a miner is in touch with.
    fn leave_dock(&mut self, index: usize) {
        let id = self.units[index].id;
        if let Some(dock) = self.units[index].work.dock.take() {
            if let Some(building) = self.buildings.iter_mut().find(|b| b.id == dock && b.docked == Some(id)) {
                building.docked = None;
                building.refining = None;
            }
        }
    }

    /// Runs the missions of the miners that are due.
    fn run_miners(&mut self) {
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            if !unit.stats.harvester || unit.tethered {
                continue;
            }
            if unit.work.job == Job::Idle {
                if unit.work.settle && !unit.is_moving() {
                    let cell = unit.cell();
                    let on_ore = self.map.ore_at(cell.0, cell.1).is_some();
                    let work = &mut self.units[index].work;
                    work.settle = false;
                    if on_ore {
                        work.job = Job::Harvest;
                        work.status = 0;
                        work.next = self.tick;
                    }
                }
                // A computer house's miner on
                // guard goes back to mining while the house has a building
                // it can dock at, unless one of its miners has found no ore.
                let unit = &self.units[index];
                let owner = unit.owner;
                if !self.human(owner) && owner != 0 && unit.dest.is_none() && self.tick >= unit.work.next && !self.houses[owner as usize].no_ore && unit.arms.target.is_none() {
                    let docks = self.unit_types.get(unit.stats.kind as usize - 1).map(|kind| kind.dock.clone()).unwrap_or_default();
                    if self.buildings.iter().any(|b| b.owner == owner && docks.contains(&b.kind)) {
                        let work = &mut self.units[index].work;
                        work.job = Job::Harvest;
                        work.status = 0;
                        work.next = self.tick + 15;
                    }
                }
                continue;
            }
            if self.tick < unit.work.next {
                continue;
            }
            let now = self.tick;
            self.units[index].work.step_stage(now);
            let delay = match self.units[index].work.job {
                Job::Harvest => self.mission_harvest(index),
                Job::Enter => self.mission_enter(index),
                Job::Unload => self.mission_unload(index),
                Job::Idle => 1,
            };
            self.units[index].work.next = self.tick + delay;
        }
    }

    /// Whether a miner could dig at a cell.
    // In the playable area, joined to where the miner
    // is for its movement zone, ore land, and free to drive onto.
    fn can_harvest(&self, index: usize, others: &[Other], cell: (i32, i32)) -> bool {
        let unit = &self.units[index];
        if self.map.ore_at(cell.0, cell.1).is_none() || !self.map.in_playfield(cell.0, cell.1) {
            return false;
        }
        if let Some(zones) = self.map.zones() {
            // A miner standing on its refinery (where a new one appears) is
            // in no zone itself: the zones it can drive out into count.
            let zone = unit.stats.movement_zone as usize;
            if !unit_zones(&self.map, zones, unit).contains(&dest_zone(&self.map, cell, zone)) {
                return false;
            }
        }
        entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), others, &self.buildings, unit, cell, None, cell, -1) == code::OK
    }

    /// The ore cell a miner would go for within a number of cells.
    // Its own cell if that has ore; otherwise square
    // rings outward, stopping at the first ring with a cell it could dig at and
    // taking the richest there, the earliest on a tie.
    fn find_ore(&self, index: usize, radius: i32) -> Option<(i32, i32)> {
        let here = self.units[index].cell();
        if self.map.ore_at(here.0, here.1).is_some() {
            return Some(here);
        }
        let others: Vec<Other> = self.others();
        for ring in 1..radius {
            let mut best: Option<(i32, (i32, i32))> = None;
            for i in -ring..=ring {
                for cell in [(here.0 + i, here.1 - ring), (here.0 + i, here.1 + ring), (here.0 - ring, here.1 + i), (here.0 + ring, here.1 + i)] {
                    if !self.can_harvest(index, &others, cell) {
                        continue;
                    }
                    let value = self.map.ore_at(cell.0, cell.1).map_or(0, |(kind, amount)| {
                        self.ore_values.get(kind as usize).copied().unwrap_or(0) * (amount as i32 + 1)
                    });
                    if best.is_none_or(|(most, _)| value > most) {
                        best = Some((value, cell));
                    }
                }
            }
            if let Some((_, cell)) = best {
                return Some(cell);
            }
        }
        None
    }

    /// Heads for ore within a number of cells. True only if the miner is on it.
    fn go_to_ore(&mut self, index: usize, radius: i32) -> bool {
        if self.units[index].dest.is_some() {
            return false;
        }
        match self.find_ore(index, radius) {
            None => false,
            Some(cell) if cell == self.units[index].cell() => true,
            Some(cell) => {
                self.send(index, cell);
                false
            }
        }
    }

    /// The refinery a miner would use, with how far off its middle is in leptons.
    // The nearest of the house's buildings
    // of its `Dock` types, the earlier on a tie, passing over ones dealing with
    // another miner unless asked not to.
    fn find_dock(&self, index: usize, include_busy: bool) -> Option<(usize, i64)> {
        let unit = &self.units[index];
        let docks = &self.unit_types.get((unit.stats.kind as usize).checked_sub(1)?)?.dock;
        let mut best: Option<(usize, i64)> = None;
        for &dock in docks {
            for (at, building) in self.buildings.iter().enumerate() {
                if building.owner != unit.owner || building.kind != dock || building.buildup.is_some() {
                    continue;
                }
                if !include_busy && building.docked.is_some_and(|other| other != unit.id) {
                    continue;
                }
                let middle = (
                    building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128,
                    building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128,
                );
                let (dx, dy) = ((middle.0 - unit.x) as f64, (middle.1 - unit.y) as f64);
                let distance = (dx * dx + dy * dy).sqrt() as i64;
                if best.is_none_or(|(_, nearest)| distance < nearest) {
                    best = Some((at, distance));
                }
            }
        }
        best
    }

    /// The Harvest mission. Returns the ticks until it runs again.
    fn mission_harvest(&mut self, index: usize) -> u64 {
        let stats = self.units[index].stats;
        let owner = self.units[index].owner;
        let docks = self.unit_types.get(stats.kind as usize - 1).map(|kind| kind.dock.clone()).unwrap_or_default();
        if !self.buildings.iter().any(|b| b.owner == owner && docks.contains(&b.kind)) {
            self.units[index].work.job = Job::Idle;
            return 1;
        }
        let full = self.units[index].work.load() >= stats.storage;
        let now = self.tick;
        match self.units[index].work.status {
            0 => {
                if full {
                    self.units[index].work.status = 2;
                    return 1;
                }
                if let Some(back) = self.units[index].work.archive.take() {
                    self.send(index, back);
                }
                self.units[index].work.digging = false;
                if self.go_to_ore(index, self.ore_far_scan) {
                    let work = &mut self.units[index].work;
                    work.digging = true;
                    work.start_stage(2, now);
                    work.status = 1;
                    return 1;
                }
                if self.units[index].dest.is_none() {
                    // And its house is marked as
                    // having found no ore, for good.
                    self.units[index].work.status = 4;
                    self.houses[owner as usize].no_ore = true;
                    return 105;
                }
                self.mission_delay()
            }
            1 => {
                if self.units[index].work.stage_rate == 0 {
                    let rate = self.harvester_load_rate;
                    self.units[index].work.start_stage(rate, now);
                }
                if self.units[index].work.stage < 9 {
                    return 1;
                }
                if self.harvest_step(index) {
                    return 1;
                }
                self.units[index].work.digging = false;
                if self.units[index].work.load() >= stats.storage {
                    self.units[index].work.status = 2;
                    self.units[index].work.archive = self.find_ore(index, self.ore_near_scan);
                    return 1;
                }
                if !self.go_to_ore(index, self.ore_near_scan) && self.units[index].dest.is_none() {
                    self.units[index].work.archive = None;
                    self.units[index].work.status = 2;
                    return 1;
                }
                self.units[index].work.digging = true;
                1
            }
            2 => {
                if self.units[index].dest.is_some() {
                    return self.mission_delay();
                }
                let limit = if stats.teleporter { self.chrono_too_far } else { self.harvester_too_far } as i64 * LEPTONS_PER_CELL as i64;
                if let Some((dock, distance)) = self.find_dock(index, false) {
                    if distance <= limit {
                        self.buildings[dock].docked = Some(self.units[index].id);
                        self.units[index].work.dock = Some(self.buildings[dock].id);
                        self.units[index].work.status = 3;
                        return self.mission_delay();
                    }
                }
                if let Some((dock, distance)) = self.find_dock(index, true) {
                    if distance > 0x300 || stats.teleporter {
                        let building = &self.buildings[dock];
                        let queue = self.building_types[building.kind as usize].queueing_cell;
                        let wait = (building.cell.0 + queue.0, building.cell.1 + queue.1);
                        // Any zone, any level, picked
                        // by frame; with none, the miner's destination is cleared.
                        let near = nearby(&self.map, &self.claims, &self.spots, &self.units[index], wait, Near::new(true, false, None), self.tick);
                        match near {
                            Some(cell) if cell != self.units[index].cell() => self.send(index, cell),
                            Some(_) => {}
                            None => self.units[index].dest = None,
                        }
                    }
                }
                self.mission_delay()
            }
            3 => {
                self.units[index].work.job = Job::Enter;
                1
            }
            _ => {
                // A miner with nothing
                // to mine that stands on a refinery drives off it, then goes
                // on guard.
                // To a free cell near the
                // refinery's centre, in the miner's own zone, at any level,
                // picked by frame. (The engine first queues Repair or Hunt for
                // a miner that found no ore, but the Guard queued straight after
                // replaces it.)
                let cell = self.units[index].cell();
                if let Some(at) = self.building_on(cell).filter(|&at| self.building_types.get(self.buildings[at].kind as usize).is_some_and(|kind| kind.refinery)) {
                    let building = &self.buildings[at];
                    let wait = (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2);
                    let near = nearby(&self.map, &self.claims, &self.spots, &self.units[index], wait, Near::new(false, false, None), self.tick);
                    if let Some(to) = near.filter(|&to| to != cell) {
                        self.send(index, to);
                    }
                }
                self.units[index].work.job = Job::Idle;
                self.mission_delay()
            }
        }
    }

    /// One bale from the cell under a miner. False when there is no more to do here.
    fn harvest_step(&mut self, index: usize) -> bool {
        let unit = &self.units[index];
        if unit.dest.is_some() {
            return true;
        }
        let cell = unit.cell();
        let room = unit.stats.storage.saturating_sub(unit.work.load());
        let now = self.tick;
        if let (true, Some((kind, _))) = (room > 0, self.map.ore_at(cell.0, cell.1)) {
            let removed = self.map.reduce_ore(cell.0, cell.1, 1);
            if removed > 0 && self.map.ore_at(cell.0, cell.1).is_none() {
                self.ore_emptied(cell, kind as usize);
            }
            if removed > 0 {
                let rate = self.harvester_load_rate;
                let work = &mut self.units[index].work;
                work.cargo[kind as usize & 3] += removed;
                work.start_stage(rate, now);
                return true;
            }
            return false;
        }
        self.units[index].work.start_stage(0, now);
        false
    }

    /// The Enter mission: getting onto the refinery's dock.
    // The dock is the cell three across and one down from the refinery's top
    // cell; the miner goes there (a `Teleporter` by teleporting), turns to
    // face east, and starts to unload.
    fn mission_enter(&mut self, index: usize) -> u64 {
        let id = self.units[index].id;
        let dock = self.units[index].work.dock.and_then(|dock| self.buildings.iter().position(|b| b.id == dock && b.docked == Some(id)));
        let Some(dock) = dock else {
            let work = &mut self.units[index].work;
            work.dock = None;
            work.job = Job::Harvest;
            work.status = 0;
            return 1;
        };
        if self.units[index].is_moving() {
            return self.mission_delay();
        }
        let spot = (self.buildings[dock].cell.0 + 3, self.buildings[dock].cell.1 + 1);
        let here = self.units[index].cell();
        if here != spot {
            if self.units[index].stats.teleporter {
                let free = self.map.index(spot.0, spot.1).is_some_and(|at| self.claims[at] == 0);
                if free {
                    self.parasite_exit(index);
                    if let Some(at) = self.map.index(here.0, here.1) {
                        if self.claims[at] == id + 1 {
                            self.claims[at] = 0;
                        }
                    }
                    if let Some(at) = self.map.index(spot.0, spot.1) {
                        self.claims[at] = id + 1;
                    }
                    let unit = &mut self.units[index];
                    let from = (unit.x, unit.y, 0);
                    (unit.x, unit.y) = cell_centre(spot);
                    let (kind, to) = (unit.stats.kind, (unit.x, unit.y, 0));
                    self.events.push(combat::Event::Teleported { kind, from, to });
                }
            } else {
                self.send(index, spot);
            }
            return self.mission_delay();
        }
        if self.face_east(index) {
            let work = &mut self.units[index].work;
            work.job = Job::Unload;
            work.status = 0;
            return 1;
        }
        self.mission_delay()
    }

    /// Turns a unit to face east. True once it does.
    fn face_east(&mut self, index: usize) -> bool {
        let unit = &mut self.units[index];
        if unit.facing == 0x40 {
            return true;
        }
        if unit.turn.remaining == 0 {
            unit.turn = Turn::start(unit.turn.current(), 0x4000, unit.stats.rot);
            if unit.turn.remaining == 0 {
                unit.facing = 0x40;
                unit.turn = Turn::facing(0x40);
                return true;
            }
        }
        false
    }

    /// A miner's Unload mission.
    // Facing east on the dock, each kind of
    // ore aboard is handed over whole, one kind every `HarvesterDumpRate` × 900
    // ticks, and pays its `Value` a bale; then the miner goes back to mining.
    fn mission_unload(&mut self, index: usize) -> u64 {
        let id = self.units[index].id;
        let here = self.units[index].cell();
        let now = self.tick;
        let back_to_work = |world: &mut Self| {
            world.leave_dock(index);
            let work = &mut world.units[index].work;
            work.dumping = false;
            work.job = Job::Harvest;
            work.status = 0;
        };
        if !self.buildings.iter().any(|b| b.docked == Some(id)) {
            back_to_work(self);
            return 1;
        }
        if !self.face_east(index) {
            return 5;
        }
        if !self.units[index].work.dumping {
            let work = &mut self.units[index].work;
            work.dumping = true;
            work.start_stage(1, now);
            work.status = 3;
        } else if self.units[index].work.status == 3 {
            let Some(refinery) = self.building_on((here.0 - 1, here.1)) else {
                back_to_work(self);
                return self.mission_delay();
            };
            if self.units[index].work.stage as f64 >= self.harvester_dump_rate * 900.0 {
                let owner = self.buildings[refinery].owner as usize;
                // From the first
                // step that pays, the refinery's `SpecialAnim` runs.
                self.buildings[refinery].refining.get_or_insert(now);
                match self.units[index].work.cargo.iter().position(|&bales| bales > 0) {
                    None => self.units[index].work.status = 4,
                    Some(kind) => {
                        let bales = std::mem::take(&mut self.units[index].work.cargo[kind]);
                        let value = self.ore_values.get(kind).copied().unwrap_or(0);
                        let real = if self.low_power(owner as u8) {
                            0
                        } else {
                            self.buildings.iter().filter(|b| b.owner as usize == owner && b.buildup.is_none() && self.building_types.get(b.kind as usize).is_some_and(|kind| kind.ore_purifier)).count() as i32
                        };
                        let purifiers = real + self.houses.get(owner).and_then(|house| house.ai.as_ref()).map_or(0, |brain| self.ai_rules.virtual_purifiers[brain.difficulty.min(2)]);
                        let amount = value * bales as i32;
                        let bonus = (purifiers as f64 * self.ai_rules.purifier_bonus * amount as f64) as i32;
                        if let Some(house) = self.houses.get_mut(owner) {
                            house.credits += amount + bonus;
                        }
                        self.units[index].work.start_stage(1, now);
                    }
                }
            }
            return 1;
        } else {
            back_to_work(self);
        }
        self.mission_delay()
    }

    /// Steps the war factories that are letting a vehicle out.
    // The Unload mission, run every 14 to 16 ticks.
    // The door opens (taking `DeployTime` × 900 ticks); anything on the last cell
    // inside the door, or around it, is asked to move until none is left; with the
    // door open the vehicle is driven along the way-out track at half speed; when
    // it lets go the door closes, and once shut the factory is idle again.
    fn unload_factories(&mut self) {
        for index in 0..self.buildings.len() {
            let Some(mut unload) = self.buildings[index].unload else {
                continue;
            };
            if self.tick < unload.next {
                continue;
            }
            let building = &self.buildings[index];
            let door_ticks = self.building_types.get(building.kind as usize).map_or(0, |kind| kind.door_ticks) as u64;
            let inside = door_cell(building);
            match unload.status {
                0 => {
                    unload.door = self.tick + door_ticks;
                    unload.status = 1;
                }
                1 => {
                    // The way is blocked only by
                    // something on the cell inside the door itself. Then that
                    // cell's occupants are asked to move, and those of the
                    // eight cells round it too; things standing round the
                    // door with the cell itself clear do not hold the vehicle.
                    // The cell itself is
                    // cleared with no point given, so what is there goes to the
                    // nearest free place; those round it are sent away from
                    // the middle of that cell.
                    let taken = |world: &World, cell: (i32, i32)| world.units.iter().any(|u| u.id != unload.unit && u.cell() == cell);
                    if taken(self, inside) {
                        let around: Vec<(i32, i32)> = (-1..=1)
                            .flat_map(|dy| (-1..=1).map(move |dx| (inside.0 + dx, inside.1 + dy)))
                            .filter(|&cell| cell != inside && taken(self, cell))
                            .collect();
                        self.scatter_from(inside, Scatter::Nearest);
                        for cell in around {
                            self.scatter_from(cell, Scatter::Away(cell_centre(inside)));
                        }
                    } else {
                        unload.status = 2;
                    }
                }
                2 if self.tick >= unload.door => {
                    // Anything not on the Drive
                    // locomotor is not put on the track but sent to the cell at
                    // the building's origin + (5, 1).
                    let flyer = self.units.iter().position(|u| u.id == unload.unit && u.jet.is_some());
                    if let Some(flyer) = flyer {
                        let origin = self.buildings[index].cell;
                        self.send(flyer, (origin.0 + 5, origin.1 + 1));
                        unload.status = 3;
                        unload.next = self.tick + 14 + (self.next_random() % 3) as u64;
                        self.buildings[index].unload = Some(unload);
                        continue;
                    }
                    match self.units.iter_mut().find(|u| u.id == unload.unit) {
                        Some(unit) => {
                            unit.drive = Some(Drive {
                                selector: tracks::FACTORY_EXIT,
                                index: 0,
                                head: inside,
                                via: None,
                                from: unit.cell(),
                                target: UNIT_FRACTION,
                            });
                            unit.progress = 0;
                            unload.status = 3;
                        }
                        None => {
                            unload.door = self.tick + door_ticks;
                            unload.status = 4;
                        }
                    }
                }
                3 => {
                    let (origin, size) = (self.buildings[index].cell, self.buildings[index].size);
                    if let Some(unit) = self.units.iter_mut().find(|u| u.id == unload.unit && u.tethered && u.jet.is_some()) {
                        let cell = unit.cell();
                        if !((origin.0..origin.0 + size.0).contains(&cell.0) && (origin.1..origin.1 + size.1).contains(&cell.1)) {
                            unit.tethered = false;
                        }
                    }
                    if !self.units.iter().any(|u| u.id == unload.unit && u.tethered) {
                        unload.door = self.tick + door_ticks;
                        unload.status = 4;
                    }
                }
                4 if self.tick >= unload.door => {
                    self.buildings[index].unload = None;
                    continue;
                }
                _ => {}
            }
            unload.next = self.tick + 14 + (self.next_random() % 3) as u64;
            self.buildings[index].unload = Some(unload);
        }
    }

    /// Whether a building type goes in the defences tab.
    pub fn is_defence(&self, building: u16) -> bool {
        self.building_types.get(building as usize).is_some_and(|kind| kind.combat)
    }

    /// The building types a house's sidebar offers, in the order it shows them;
    /// `is_defence` says which tab each is in.
    // A type is listed if the
    // house may build it and has a factory for it. Order: types of the house's own
    // side first, then lower `TechLevel`, lower cost, and name.
    pub fn buildable(&self, house: u8) -> Vec<u16> {
        let side = self.houses.get(house as usize).map_or(0, |house| house.side);
        let mut list: Vec<u16> = (0..self.building_types.len() as u16)
            .filter(|&index| self.may_build(house, Item::Building(index)) && self.has_factory(house, Item::Building(index)))
            .collect();
        list.sort_by(|&a, &b| {
            let (a, b) = (&self.building_types[a as usize], &self.building_types[b as usize]);
            (a.planning_side != side, a.tech_level, a.cost, &a.sort_name)
                .cmp(&(b.planning_side != side, b.tech_level, b.cost, &b.sort_name))
        });
        list
    }

    /// Output and drain of a house's buildings.
    pub fn power(&self, house: u8) -> (i32, i32) {
        let mut totals = (0, 0);
        // What a plant gives falls
        // with its health; what a building draws does not.
        // A spy in a power plant
        // leaves the house with no output for `SpyPowerBlackout` frames.
        let dark = self.houses.get(house as usize).is_some_and(|h| self.tick < h.blackout_until);
        for building in self.buildings.iter().filter(|b| b.owner == house) {
            let (power, strength) = self.building_types.get(building.kind as usize).map_or((0, 1), |kind| (kind.power, kind.strength.max(1)));
            if power >= 0 && dark {
            } else if power >= 0 {
                totals.0 += (building.health as f64 / strength as f64 * power as f64) as i32;
            } else {
                totals.1 -= power;
            }
        }
        totals
    }

    /// Whether a house is short of power.
    // Output below a drain that is not zero.
    pub fn low_power(&self, house: u8) -> bool {
        let (output, drain) = self.power(house);
        output < drain && drain != 0
    }

    /// Whether a building is working.
    // One with `Powered`, a drain, and `TogglePower`
    // not turned off stops while its house is short of power.
    pub fn online(&self, building: &Building) -> bool {
        let Some(kind) = self.building_types.get(building.kind as usize) else {
            return true;
        };
        // Two or more
        // soldiers charging it count as power.
        building.health > 0 && (building.chargers.len() >= 2 || !(kind.powered && kind.power < 0 && self.low_power(building.owner)))
    }

    /// Whether a house has its radar.
    // Not while short of power or jammed by a
    // storm; otherwise with any `Radar` building that is standing and not
    // being sold.
    pub fn radar_available(&self, house: u8) -> bool {
        !self.low_power(house)
            && !self.radar_jammed(house)
            && self.buildings.iter().any(|b| {
                b.owner == house && b.buildup.is_none() && self.building_types.get(b.kind as usize).is_some_and(|kind| kind.radar)
            })
    }

    /// Each cell of a building's foundation with its top cell at `origin`, and
    /// whether the house's building may stand on it.
    // The building must be within `Adjacent`
    // cells, on both axes, of a cell held by one of the house's `BaseNormal`
    // buildings; if it is not, every cell counts as blocked. Otherwise each cell is
    // judged on its own.
    pub fn placement(&self, house: u8, building: u16, origin: (i32, i32)) -> Vec<((i32, i32), bool)> {
        let size = self.size_of(building);
        let reach = self.building_types.get(building as usize).map_or(0, |kind| kind.adjacent);
        let gap = |a: i32, a_len: i32, b: i32, b_len: i32| (b - (a + a_len)).max(a - (b + b_len)).max(0);
        let near = self.buildings.iter().filter(|b| b.owner == house).any(|b| {
            self.building_types.get(b.kind as usize).is_some_and(|kind| kind.base_normal)
                && gap(origin.0, size.0, b.cell.0, b.size.0) <= reach
                && gap(origin.1, size.1, b.cell.1, b.size.1) <= reach
        });
        let mut cells = Vec::new();
        for dy in 0..size.1 {
            for dx in 0..size.0 {
                let cell = (origin.0 + dx, origin.1 + dy);
                // Not on a cell under the shroud.
                let seen = !self.human(house) || !self.map.shrouded_to(house, cell.0, cell.1);
                cells.push((cell, near && seen && self.cell_clear(cell, 0, building, house)));
            }
        }
        cells
    }

    /// The cells a wall placed at a cell would also fill, joining it to walls of
    /// the same kind and owner in line with it.
    // Each of the four straight directions is walked
    // for up to `GuardRange` cells; reaching the house's own wall of the same kind
    // fills every cell between, and a cell where a wall cannot go, or running out
    // of range, leaves that direction empty.
    pub fn wall_line(&self, house: u8, building: u16, origin: (i32, i32)) -> Vec<(i32, i32)> {
        let Some(kind) = self.building_types.get(building as usize) else {
            return Vec::new();
        };
        let Some(overlay) = kind.wall_overlay else {
            return Vec::new();
        };
        let mut filled = Vec::new();
        for side in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let mut run = Vec::new();
            let mut cell = origin;
            loop {
                cell = (cell.0 + side.0, cell.1 + side.1);
                let anchor = self.map.index(cell.0, cell.1).is_some_and(|index| {
                    self.map.wall_kind[index] == overlay && self.map.wall_owner[index] == house
                });
                if anchor {
                    filled.extend(run);
                    break;
                }
                if !self.cell_clear(cell, 0, building, house) {
                    break;
                }
                run.push(cell);
                if run.len() as i32 >= kind.guard_range {
                    break;
                }
            }
        }
        filled
    }

    /// Adds a wall piece the map starts with, joined as the map says it is.
    // The map's stored data is trusted.
    pub fn add_map_wall(&mut self, cell: (i32, i32), overlay: u16, data: u8) {
        self.map.put_wall(cell, overlay, data, u8::MAX);
    }

    /// Whether a house's finished building could be placed with its top cell at a
    /// cell.
    pub fn can_place(&self, house: u8, building: u16, origin: (i32, i32)) -> bool {
        self.placement(house, building, origin).iter().all(|(_, ok)| *ok)
    }

    /// The player's things lift the shroud round themselves.
    // A unit looks out to its `Sight` when it appears, in each new cell, and
    // every 15 frames while it moves; only the local player has a shroud. A
    // ground unit sees 10% further for every `LeptonsPerSightIncrease` (2000)
    // leptons of height, truncated. The veteran `SIGHT` ability multiplies by
    // `VeteranSight`, which the stock rules set to 0.0, meaning no change; it
    // is left out. An aircraft sees its `Sight` with no height bonus, and one
    // cell when it is on the ground.
    fn look(&mut self) {
        if !self.map.shroud_on {
            return;
        }
        // Every person's house has its own shroud.
        for human in 0..self.houses.len().min(32) as u8 {
            if self.houses[human as usize].human {
                self.look_for(human);
            }
        }
    }

    fn look_for(&mut self, human: u8) {
        // With a `SpySat` building
        // (not being sold) everything is revealed; when the last is lost
        // everything goes dark and the player's things look again.
        let has = self.buildings.iter().any(|b| {
            b.owner == human && b.buildup.is_none_or(|buildup| !buildup.selling) && b.buildup.is_none() && self.building_types.get(b.kind as usize).is_some_and(|kind| kind.spy_sat)
        });
        if has != (self.map.satellite & house_bit(human) != 0) {
            self.map.satellite ^= house_bit(human);
            self.map.reveal_all(human, has);
            if !has {
                for unit in self.units.iter_mut().filter(|unit| unit.owner == human) {
                    unit.looked = None;
                }
                for building in self.buildings.iter_mut().filter(|building| building.owner == human) {
                    building.looked = false;
                }
            }
        }
        // A working gap generator of
        // a house that is not the player's ally darkens the cells of its disc
        // ((R+1) cells, round the cell it is drawn over) that none of the
        // player's things is looking at, satellite or no. When one goes,
        // a player with the satellite has its disc lit again.
        // Not original behaviour: the engine counts each cell's viewers and
        // darkens a watched cell as its last viewer leaves; here the disc goes
        // dark every fifteen frames and the player's things look again.
        if self.tick % 15 == 0 {
            let gaps: Vec<((i32, i32), i32)> = self
                .buildings
                .iter()
                .filter(|b| !self.allied(b.owner, human) && b.owner != 0 && b.buildup.is_none() && self.online(b))
                .filter_map(|b| {
                    let radius = self.building_types.get(b.kind as usize).map_or(0, |kind| kind.gap_radius);
                    let cell = (b.cell.0 + b.size.0 / 2, b.cell.1 + b.size.1 / 2);
                    let level = self.map.level(cell.0, cell.1) as i32;
                    (radius > 0).then_some(((cell.0 - level / 2, cell.1 - level / 2), radius))
                })
                .collect();
            if gaps.len() < self.map.gaps[human as usize] as usize && self.map.satellite & house_bit(human) != 0 {
                self.map.reveal_all(human, true);
            }
            self.map.gaps[human as usize] = gaps.len() as u16;
            if !gaps.is_empty() {
                for (centre, radius) in gaps {
                    for dy in -radius - 2..radius + 2 {
                        for dx in -radius - 2..radius + 2 {
                            if dx * dx + dy * dy < (radius + 1) * (radius + 1) {
                                self.map.hide(human, (centre.0 + dx, centre.1 + dy));
                            }
                        }
                    }
                }
                for unit in self.units.iter_mut().filter(|unit| unit.owner == human) {
                    unit.looked = None;
                }
                for building in self.buildings.iter_mut().filter(|building| building.owner == human) {
                    building.looked = false;
                }
            }
        }
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            if unit.owner != human {
                continue;
            }
            let cell = unit.cell();
            if unit.looked == Some(cell) && !(unit.is_moving() && self.tick % 15 == 0) {
                continue;
            }
            let level = self.map.level(cell.0, cell.1) as i32 + self.altitude(unit) / combat::LEVEL_HEIGHT;
            let sight = if unit.plane.is_some() {
                if self.altitude(unit) == 0 { 1 } else { unit.stats.fighter.sight }
            } else {
                let z = self.ground_z(unit.x, unit.y) + self.altitude(unit);
                let bonus = z / self.sight_height_step.max(1) * 10;
                (unit.stats.fighter.sight as f64 * (1.0 + bonus as f64 * 0.01)) as i32
            };
            self.units[index].looked = Some(cell);
            self.map.reveal(human, cell, level, sight);
        }
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            if building.owner != human || building.looked {
                continue;
            }
            let cell = (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2);
            let sight = self.building_types.get(building.kind as usize).map_or(0, |kind| kind.fighter.sight);
            let level = self.map.level(cell.0, cell.1) as i32;
            self.buildings[index].looked = true;
            self.map.reveal(human, cell, level, sight);
        }
    }

    /// Sets a unit's health, for tests.
    #[doc(hidden)]
    pub fn units_mut_for_tests(&mut self, unit: u32, health: i32) {
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == unit) {
            unit.health = health;
        }
    }

    /// Destroys everything a house has, for tests.
    #[doc(hidden)]
    pub fn disguise_for_tests(&mut self, unit: u32, look: (u16, u8)) {
        let tick = self.tick;
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == unit) {
            unit.disguise = Some(look);
            unit.disguised_at = tick;
        }
    }

    pub fn destroy_house_for_tests(&mut self, house: u8) {
        let units: Vec<u32> = self.units.iter().filter(|u| u.owner == house).map(|u| u.id).collect();
        let buildings: Vec<u32> = self.buildings.iter().filter(|b| b.owner == house).map(|b| b.id).collect();
        for aim in units.into_iter().map(combat::Aim::Unit).chain(buildings.into_iter().map(combat::Aim::Building)) {
            self.kill(aim);
        }
    }

    /// Makes a building all but indestructible, for tests.
    #[doc(hidden)]
    pub fn harden_building_for_tests(&mut self, building: u32) {
        if let Some(building) = self.buildings.iter_mut().find(|b| b.id == building) {
            building.health = 50_000_000;
        }
    }

    /// Hands a building to another house with no ceremony, for tests.
    #[doc(hidden)]
    pub fn give_building_for_tests(&mut self, building: u32, house: u8) {
        if let Some(building) = self.buildings.iter_mut().find(|b| b.id == building) {
            building.owner = house;
        }
    }

    /// Halves the health of a house's buildings, for tests.
    #[doc(hidden)]
    pub fn hurt_buildings_for_tests(&mut self, house: u8) {
        for building in self.buildings.iter_mut().filter(|b| b.owner == house) {
            building.health = (building.health / 2).max(1);
        }
    }

    pub fn current_tick(&self) -> u64 {
        self.tick
    }

    /// Applies an order. Orders take effect from the next tick.
    /// Carries out an order given by a house, if it is that house's to give:
    /// the unit or building it is for must be the house's own, and a house
    /// named in it must be that house. Whether it was carried out.
    // Not original behaviour: the engine trusts the house number that comes
    // with each event. Orders that arrive from another machine go through
    // here, so that a changed program cannot command what is not its own.
    pub fn order_as(&mut self, house: u8, order: Order) -> bool {
        let own_unit = order.unit().is_none_or(|id| self.unit(id).is_some_and(|unit| unit.owner == house));
        let own_building = order.building().is_none_or(|id| self.buildings.iter().any(|b| b.id == id && b.owner == house));
        let own_name = order.house().is_none_or(|named| named == house);
        if !(own_unit && own_building && own_name) {
            return false;
        }
        self.order(order);
        true
    }

    pub fn order(&mut self, order: Order) {
        // A soldier still falling starts no new
        // mission; what it is told waits until it is down.
        if let Some(id) = order.unit() {
            if self.unit(id).is_some_and(|unit| unit.falling.is_some()) {
                self.landing_orders.retain(|old| old.unit() != Some(id));
                self.landing_orders.push(order);
                return;
            }
            // Any order ends a guard of the ground round a post; the next
            // place it is left at becomes the post.
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                unit.post = None;
            }
        }
        match order {
            Order::Move { unit, x, y } => {
                // The order only records where to go; a track being driven runs
                // to its end before anything changes. A clicked cell the unit's
                // zone does not reach is swapped for a free cell near it that it
                // does, the choice among equals turning on the frame number; with
                // none, the order comes to nothing.
                let Some(index) = self.units.iter().position(|u| u.id == unit) else {
                    return;
                };
                if self.units[index].tethered {
                    return;
                }
                // A plain move drops the target.
                self.units[index].arms.target = None;
                self.units[index].arms.attacking = false;
                // A person's dug-in
                // soldier takes no destination; a computer's gets up and goes.
                if self.units[index].stats.infantry && self.units[index].stance != combat::Stance::Standing {
                    if self.human(self.units[index].owner) {
                        return;
                    }
                    let frames = self.units[index].stats.fighter.undeploy_frames as u64;
                    if matches!(self.units[index].stance, combat::Stance::Deployed | combat::Stance::Deploying(_)) {
                        self.units[index].stance = combat::Stance::Undeploying(self.tick + frames);
                        self.units[index].undeploy_at = None;
                    }
                }
                // A miner told to move leaves off mining and lets go of its refinery.
                // At the end of the move a human
                // player's miner goes back to mining only if it stands on ore.
                if self.units[index].stats.harvester {
                    self.leave_dock(index);
                    let work = &mut self.units[index].work;
                    work.job = Job::Idle;
                    work.digging = false;
                    work.dumping = false;
                    work.settle = true;
                }
                let (x, y) = {
                    let unit = &self.units[index];
                    let (own, zone) = (unit.cell(), unit.stats.movement_zone as usize);
                    let connected = self.map.zones.as_ref().is_none_or(|zones| {
                        {
                            let _ = own;
                            zones.playable((x, y)) && unit_zones(&self.map, zones, unit).contains(&dest_zone(&self.map, (x, y), zone))
                        }
                    });
                    if connected {
                        (x, y)
                    } else {
                        // The unit's zone, within a
                        // level, picked by frame.
                        match nearby(&self.map, &self.claims, &self.spots, unit, (x, y), Near::new(false, true, None), self.tick) {
                            Some(cell) => cell,
                            None => return,
                        }
                    }
                };
                self.units[index].capture = None;
                self.units[index].enter_area = None;
                self.units[index].boarding = None;
                self.units[index].unloading = None;
                // A person's dug-in soldier does not
                // move; a held or still-arriving unit takes no move orders. A
                // computer's soldier getting up (above) keeps the destination
                // and sets off once it stands; movement waits for that.
                let getting_up = matches!(self.units[index].stance, combat::Stance::Undeploying(_));
                if (self.units[index].stance != combat::Stance::Standing && !getting_up) || self.units[index].frozen.is_some() || self.tick < self.units[index].warp_until || self.tick < self.units[index].paralysed_until {
                    return;
                }
                if self.units[index].plane.is_some() {
                    self.plane_move(index, (x, y));
                    return;
                }
                if self.units[index].jet.is_some() {
                    self.send(index, (x, y));
                    return;
                }
                // A `Teleporter` jumps there.
                if self.units[index].stats.teleporter && !self.units[index].stats.harvester {
                    self.teleport(index, (x, y));
                    return;
                }
                let unit = self.units[index].id;
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == unit) {
                    unit.deploying = false;
                    unit.dest = Some((x, y));
                    unit.drop_path();
                    unit.path_timer = 0;
                    unit.retries = 10;
                    unit.blocked_until = None;
                }
            }
            Order::Deploy { unit } => {
                // The route is dropped and the unit
                // deploys once it has stopped; see `try_deploy`.
                let tick = self.tick;
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == unit) {
                    // A transport with
                    // anyone aboard lets them out.
                    if !unit.cargo.is_empty() {
                        unit.unloading = Some(tick);
                        unit.dest = None;
                        unit.drop_path();
                        return;
                    }
                    // A `Deployer` soldier digs in,
                    // or gets up if it is dug in, and goes nowhere meanwhile.
                    let fighter = unit.stats.fighter;
                    let area_fire = fighter.secondary.and_then(|mount| self.weapons.get(mount.weapon as usize)).is_some_and(|weapon| weapon.area_fire);
                    if unit.stats.infantry && fighter.deployer {
                        match unit.stance {
                            combat::Stance::Standing => {
                                unit.stance = combat::Stance::Deploying(tick + fighter.deploy_frames as u64);
                                unit.dest = None;
                                unit.drop_path();
                                // With an
                                // `AreaFire` second weapon it is aimed at the
                                // soldier's own cell; with an `UndeployDelay`
                                // the soldier gets up again by itself.
                                // Not the Desolator, whose weapon goes
                                // off only when the site under him is
                                // missing or under a third of its level
                                // (`run_radiation`), not every time it is
                                // loaded.
                                let tops_up = fighter.secondary.and_then(|mount| self.weapons.get(mount.weapon as usize)).is_some_and(|weapon| weapon.rad_level > 0);
                                if area_fire && !tops_up {
                                    let cell = unit.cell();
                                    unit.arms.target = Some(combat::Aim::Cell(cell.0, cell.1));
                                    unit.arms.attacking = true;
                                }
                                if fighter.undeploy_delay > -1 {
                                    unit.undeploy_at = Some(tick + fighter.undeploy_delay as u64);
                                }
                            }
                            // One that gets up by
                            // itself cannot be told to.
                            combat::Stance::Deployed if fighter.undeploy_delay <= -1 => unit.stance = combat::Stance::Undeploying(tick + fighter.undeploy_frames as u64),
                            _ => {}
                        }
                    }
                    if unit.stats.deploys_into.is_some() {
                        unit.deploying = true;
                        unit.drop_path();
                        unit.dest = None;
                    }
                }
            }
            Order::Undeploy { building, x, y } => {
                if !self.can_undeploy(building) {
                    return;
                }
                let Some(index) = self.buildings.iter().position(|b| b.id == building) else {
                    return;
                };
                // The building first plays its build-up
                // backwards.
                match self.start_buildup(self.buildings[index].kind, Some((x, y))) {
                    Some(buildup) => self.buildings[index].buildup = Some(buildup),
                    None => self.finish_packing(building, (x, y)),
                }
            }
            Order::Board { unit, transport } => {
                // The Enter mission,
                // with the transport as where to go.
                if !self.can_board(unit, transport) {
                    return;
                }
                let Some(index) = self.units.iter().position(|u| u.id == unit) else {
                    return;
                };
                let Some(cell) = self.unit(transport).map(Unit::cell) else {
                    return;
                };
                // One that
                // teleports jumps to a free cell beside what it was sent to, and
                // goes aboard once it has arrived there.
                if self.units[index].stats.teleporter && !self.units[index].stats.harvester {
                    self.teleport(index, cell);
                } else {
                    self.send(index, cell);
                }
                self.units[index].boarding = Some(transport);
                self.units[index].arms.target = None;
                self.units[index].arms.attacking = false;
            }
            Order::Super { house, index, cell } => self.fire_super(house, index as usize, cell),
            Order::Enter { unit, building } => {
                // The Capture mission, with the
                // building as where to go.
                let Some(index) = self.units.iter().position(|u| u.id == unit) else {
                    return;
                };
                if self.entry_action(unit, building).is_none() {
                    return;
                }
                let Some(target) = self.buildings.iter().find(|b| b.id == building) else {
                    return;
                };
                let from = self.units[index].cell();
                let cell = (
                    from.0.clamp(target.cell.0, target.cell.0 + target.size.0 - 1),
                    from.1.clamp(target.cell.1, target.cell.1 + target.size.1 - 1),
                );
                let area = (target.cell, target.size);
                self.send(index, cell);
                // A new mission: what it was
                // shooting at of its own accord is forgotten.
                self.units[index].arms.target = None;
                self.units[index].arms.attacking = false;
                self.units[index].capture = Some(building);
                self.units[index].enter_area = Some(area);
            }
            Order::Aim { building, target } => {
                // The Attack mission with the target.
                if let Some(building) = self.buildings.iter_mut().find(|b| b.id == building) {
                    building.arms.target = Some(target);
                    building.arms.attacking = true;
                }
            }
            Order::Dock { unit, building } => {
                // A miner sent to a refinery of its house takes the dock if
                // it is free and goes in by its Enter mission (a chrono miner
                // by jumping); with the dock taken it comes back as it would
                // with a full load.
                if let Some(index) = self.units.iter().position(|u| u.id == unit && u.stats.harvester && !u.tethered) {
                    let owner = self.units[index].owner;
                    let Some(at) = self.buildings.iter().position(|b| {
                        b.id == building && b.owner == owner && b.buildup.is_none() && self.building_types.get(b.kind as usize).is_some_and(|kind| kind.refinery)
                    }) else {
                        return;
                    };
                    self.leave_dock(index);
                    let free = self.buildings[at].docked.is_none();
                    if free {
                        self.buildings[at].docked = Some(unit);
                    }
                    let tick = self.tick;
                    let miner = &mut self.units[index];
                    miner.arms.target = None;
                    miner.work.digging = false;
                    miner.work.dumping = false;
                    miner.work.settle = false;
                    miner.work.next = tick + 1;
                    if free {
                        miner.work.dock = Some(building);
                        miner.work.job = Job::Enter;
                        miner.work.status = 0;
                    } else {
                        miner.work.job = Job::Harvest;
                        miner.work.status = 2;
                    }
                    return;
                }
                // The Enter mission with the pad
                // building as where to go.
                if let Some(index) = self.units.iter().position(|u| u.id == unit) {
                    if self.can_dock(unit, building) {
                        self.plane_dock(index, building);
                    }
                }
            }
            Order::Evacuate { building } => {
                // The Unload mission of a
                // building with soldiers in it puts them all out.
                if let Some(index) = self.buildings.iter().position(|b| b.id == building) {
                    self.remove_occupants(index);
                }
            }
            Order::Repair { building } => {
                // The flag is turned over. One at full
                // health keeps it for a moment; the next step clears it.
                if let Some(building) = self.buildings.iter_mut().find(|b| b.id == building && b.buildup.is_none()) {
                    building.repairing = !building.repairing;
                }
            }
            Order::Sell { building } => {
                // Repairs stop; the people in
                // it come out as the build-up starts to run backwards; the money
                // comes at the end. There is no taking it back.
                // Not while a charge is planted in it.
                let Some(index) = self.buildings.iter().position(|b| b.id == building && b.buildup.is_none() && b.c4.is_none()) else {
                    return;
                };
                self.buildings[index].repairing = false;
                self.pad_sold(building);
                self.sale_crew(index);
                let owner = self.buildings[index].owner;
                self.events.push(combat::Event::SaleBegan { owner });
                match self.start_buildup(self.buildings[index].kind, None) {
                    Some(mut buildup) => {
                        buildup.selling = true;
                        self.buildings[index].buildup = Some(buildup);
                    }
                    None => self.finish_sale(building),
                }
            }
            Order::Attack { unit, target } => {
                // A `C4` soldier told to attack a
                // building it can wire walks in instead.
                if let combat::Aim::Building(building) = target {
                    if self.entry_action(unit, building) == Some(combat::Entry::Sabotage) {
                        return self.order(Order::Enter { unit, building });
                    }
                }
                if let Some(index) = self.units.iter().position(|u| u.id == unit && u.plane.is_some()) {
                    self.plane_attack(index, target);
                    return;
                }
                // Not original behaviour: a computer's dug-in soldier told to
                // attack gets up first (the engine's missions bring it to its
                // feet when it has to walk; here it is done at the order).
                let tick = self.tick;
                let human = self.unit(unit).is_some_and(|u| self.human(u.owner));
                if let Some(soldier) = self.units.iter_mut().find(|u| u.id == unit && u.stats.infantry && !human) {
                    if matches!(soldier.stance, combat::Stance::Deployed | combat::Stance::Deploying(_)) {
                        soldier.stance = combat::Stance::Undeploying(tick + soldier.stats.fighter.undeploy_frames as u64);
                        soldier.undeploy_at = None;
                        soldier.busy_at = tick;
                    }
                }
                // The Attack mission with the target.
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == unit && !u.tethered) {
                    unit.arms.target = Some(target);
                    unit.arms.attacking = true;
                    if unit.stats.harvester {
                        unit.work.job = Job::Idle;
                    }
                    // The
                    // order is a new mission with a target and no
                    // destination: where it was going is forgotten. If the
                    // target is out of reach, the approach then sends it
                    // somewhere new; if not (a spy's kit reaches anywhere),
                    // it stops and fires.
                    if unit.dest.is_some() {
                        give_up(unit);
                    }
                }
            }
            Order::Rally { building, x, y } => {
                // The point is the free cell nearest
                // the one asked for, in the factory's own zone.
                let Some(index) = self.buildings.iter().position(|b| b.id == building) else {
                    return;
                };
                if !self.has_rally_point(building) {
                    return;
                }
                let stats = UnitStats { speed_type: SpeedType::Foot, ..UnitStats::default() };
                let probe = Unit::new(u32::MAX - 1, self.buildings[index].owner, self.buildings[index].cell, 0, stats);
                // Any level, picked by frame.
                if let Some(cell) = nearby(&self.map, &self.claims, &self.spots, &probe, (x, y), Near::new(false, false, None), self.tick) {
                    self.buildings[index].rally = Some(cell);
                }
            }
            Order::Primary { building } => {
                // The primary one stops being so, with
                // nothing promoted in its place; any other takes it from the rest
                // of its kind.
                let Some(index) = self.buildings.iter().position(|b| b.id == building) else {
                    return;
                };
                let (owner, kind) = (self.buildings[index].owner, self.buildings[index].kind);
                let Some(wanted) = self.building_types.get(kind as usize).filter(|t| t.factory.is_some()).cloned() else {
                    return;
                };
                if self.buildings[index].primary {
                    self.buildings[index].primary = false;
                    return;
                }
                let types = &self.building_types;
                for other in self.buildings.iter_mut().filter(|b| b.owner == owner) {
                    if types.get(other.kind as usize).is_some_and(|t| t.factory == wanted.factory && t.naval == wanted.naval) {
                        other.primary = other.id == building;
                    }
                }
            }
            Order::Harvest { unit, x, y } => {
                // The Harvest mission with the cell as
                // its destination.
                let Some(index) = self.units.iter().position(|u| u.id == unit) else {
                    return;
                };
                if !self.units[index].stats.harvester || self.units[index].tethered {
                    return;
                }
                self.leave_dock(index);
                self.send(index, (x, y));
                let work = &mut self.units[index].work;
                *work = Work { cargo: work.cargo, job: Job::Harvest, next: self.tick + 1, ..Work::default() };
            }
            Order::Stop { unit } => {
                // Target and destination go.
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == unit) {
                    // Its spawns lose
                    // their target and come home.
                    if let Some(spawner) = unit.spawner.as_mut() {
                        spawner.reset_target();
                    }
                    unit.deploying = false;
                    unit.arms.target = None;
                    unit.arms.attacking = false;
                    give_up(unit);
                }
            }
            Order::Halt { building } => {
                // The target goes;
                // the mission is left as it is.
                if let Some(building) = self.buildings.iter_mut().find(|b| b.id == building) {
                    building.arms.target = None;
                    building.arms.attacking = false;
                    building.charge = None;
                }
            }
            Order::Scatter { unit } => {
                if let Some(cell) = self.unit(unit).map(Unit::cell) {
                    self.scatter(cell);
                }
            }
            // Every soldier of the house that is
            // not busy with something it cannot break off starts its `Cheer`.
            Order::Cheer { house } => {
                let tick = self.tick;
                for unit in self.units.iter_mut().filter(|u| u.owner == house && u.stats.infantry && u.health > 0) {
                    if unit.falling.is_none() && unit.stance == combat::Stance::Standing && unit.prone_at + 2 <= tick {
                        unit.fidget = Some((3, tick));
                    }
                }
            }
            Order::Build { house, building } => {
                // A held production of the same type
                // is resumed; otherwise a new one starts, unless the factory has
                // something in hand (buildings do not queue).
                let combat = self.is_defence(building);
                let rate = self.stage_delay(house, Item::Building(building));
                let tick = self.tick;
                let held = self.houses.get(house as usize).and_then(|h| h.slot(combat));
                if let Some(held) = held {
                    if held.building == building && held.on_hold() {
                        if let Some(production) = self.houses[house as usize].slot_mut(combat) {
                            production.suspended = false;
                            production.rate = rate;
                            production.next = tick + rate as u64;
                        }
                    }
                    return;
                }
                if !self.buildable(house).contains(&building) || self.at_build_limit(house, building) {
                    return;
                }
                let cost = self.building_types[building as usize].cost;
                if let Some(house) = self.houses.get_mut(house as usize) {
                    *house.slot_mut(combat) = Some(Production {
                        building,
                        stage: 0,
                        rate,
                        next: tick + rate as u64,
                        balance: cost,
                        suspended: false,
                        stalled: false,
                    });
                }
            }
            Order::BuildUnit { house, unit } => {
                // A held one of the
                // same type is resumed; with the factory busy the request joins
                // its queue; otherwise production starts.
                let tab = self.unit_slot(unit);
                let item = Item::Unit(unit);
                let rate = self.stage_delay(house, item);
                let tick = self.tick;
                let Some(current) = self.houses.get(house as usize).map(|h| h.tab_slot(tab)) else {
                    return;
                };
                if let Some(held) = current.filter(|p| p.building == unit && p.on_hold()) {
                    *self.houses[house as usize].tab_slot_mut(tab) =
                        Some(Production { suspended: false, rate, next: tick + rate as u64, ..held });
                    return;
                }
                if !self.buildable_units(house).contains(&unit) || self.at_limit(house, item) {
                    return;
                }
                if current.is_some() {
                    let limit = self.queue_limit;
                    let queue = self.houses[house as usize].queue_mut(tab);
                    if queue.len() < limit {
                        queue.push(unit);
                    }
                    return;
                }
                self.start_unit(house, unit);
            }
            Order::SuspendUnit { house, unit } => {
                let tab = self.unit_slot(unit);
                if let Some(production) = self.houses.get_mut(house as usize).and_then(|h| h.tab_slot_mut(tab).as_mut()) {
                    if production.building == unit && !production.suspended {
                        production.suspended = true;
                        production.rate = 0;
                    }
                }
            }
            Order::CancelUnit { house, unit } => {
                // A queued copy goes first; only when
                // none is left is the one in hand abandoned, with what was paid
                // refunded, and the next in the queue started.
                let tab = self.unit_slot(unit);
                let cost = self.unit_types.get(unit as usize).map_or(0, |kind| kind.cost);
                let Some(owner) = self.houses.get_mut(house as usize) else {
                    return;
                };
                if let Some(at) = owner.queue_mut(tab).iter().position(|&queued| queued == unit) {
                    owner.queue_mut(tab).remove(at);
                    return;
                }
                if owner.tab_slot(tab).is_some_and(|p| p.building == unit) {
                    if let Some(production) = owner.tab_slot_mut(tab).take() {
                        owner.credits += cost - production.balance;
                    }
                    self.start_next_queued(house, tab);
                }
            }
            Order::Suspend { house, building } => {
                // The stage and what was paid are kept.
                let combat = self.is_defence(building);
                if let Some(production) = self.houses.get_mut(house as usize).and_then(|h| h.slot_mut(combat).as_mut()) {
                    if production.building == building && !production.suspended {
                        production.suspended = true;
                        production.rate = 0;
                    }
                }
            }
            Order::CancelBuild { house, building } => {
                // The refund is what has been paid.
                let combat = self.is_defence(building);
                let cost = self.building_types.get(building as usize).map_or(0, |kind| kind.cost);
                if let Some(house) = self.houses.get_mut(house as usize) {
                    if house.slot(combat).is_some_and(|p| p.building == building) {
                        if let Some(production) = house.slot_mut(combat).take() {
                            house.credits += cost - production.balance;
                        }
                    }
                }
            }
            Order::Place { house, building, x, y } => {
                let combat = self.is_defence(building);
                let ready = self
                    .houses
                    .get(house as usize)
                    .and_then(|h| h.slot(combat))
                    .filter(|p| p.is_ready() && p.building == building);
                if ready.is_some() && self.can_place(house, building, (x, y)) {
                    *self.houses[house as usize].slot_mut(combat) = None;
                    self.houses[house as usize].score.built += 1;
                    match self.building_types[building as usize].wall_overlay {
                        // A wall never becomes
                        // a building: the piece is written into the cell, then the
                        // lines to other pieces are filled, free.
                        Some(overlay) => {
                            let line = self.wall_line(house, building, (x, y));
                            for cell in std::iter::once((x, y)).chain(line) {
                                self.map.put_wall(cell, overlay, 0, house);
                                self.map.link_walls(cell);
                            }
                            self.map.build_zones();
                        }
                        None => self.add_building(house, building, (x, y)),
                    }
                }
            }
        }
    }

    /// Advances the simulation by one step.
    pub fn tick(&mut self) {
        self.tick += 1;
        // Production advances a tick at a time and is paid for as it goes. When a
        // stage's time is up the stage goes on and the step is paid: the balance
        // over the stages left. A house that cannot pay keeps the stage back and
        // tries again a stage's time later. Running productions follow the
        // house's power as it changes.
        for index in 0..self.houses.len() {
            for tab in 0..6 {
                let Some(mut production) = self.houses[index].tab_slot(tab) else {
                    continue;
                };
                let item = if tab < 2 { Item::Building(production.building) } else { Item::Unit(production.building) };
                if production.is_ready() && tab >= 2 {
                    // A person's finished unit leaves
                    // its factory at once, and if it cannot (the factory busy or
                    // the way shut) it is abandoned and its whole cost handed
                    // back. A computer's factory keeps a unit it is too busy to
                    // let out and tries again every 45 frames
                    // (`PlacementDelay` 0.05 x 900); one that cannot come out at
                    // all is abandoned and refunded.
                    let human = self.houses[index].human;
                    if !human && self.tick < production.next {
                        continue;
                    }
                    match self.exit_unit(index as u8, production.building) {
                        Exit::Busy if !human => {
                            production.next = self.tick + 45;
                            *self.houses[index].tab_slot_mut(tab) = Some(production);
                        }
                        result => {
                            if result != Exit::Out {
                                let cost = self.unit_types.get(production.building as usize).map_or(0, |kind| kind.cost);
                                self.houses[index].credits += cost - production.balance;
                            } else {
                                self.houses[index].score.built += 1;
                                // A unit leaving its
                                // factory has the house look at its production
                                // mode again.
                                if let Some(mode) = self.houses[index].ai.as_ref().map(|brain| brain.mode) {
                                    let next = self.next_production_mode(index as u8, mode, false);
                                    if let Some(brain) = self.houses[index].ai.as_mut() {
                                        brain.mode = next;
                                    }
                                }
                            }
                            *self.houses[index].tab_slot_mut(tab) = None;
                            self.start_next_queued(index as u8, tab);
                        }
                    }
                    continue;
                }
                if production.suspended || production.is_ready() {
                    continue;
                }
                production.rate = self.stage_delay(index as u8, item);
                if self.tick >= production.next {
                    production.stage += 1;
                    production.next = self.tick + production.rate as u64;
                    let left = PRODUCTION_STAGES - production.stage;
                    let step = if left == 0 { production.balance } else { production.balance / left };
                    let step = step.min(production.balance);
                    if step > self.houses[index].credits {
                        production.stalled = true;
                        production.stage -= 1;
                    } else {
                        self.houses[index].credits -= step;
                        production.balance -= step;
                        production.stalled = false;
                    }
                    if production.is_ready() {
                        production.suspended = true;
                        production.rate = 0;
                    }
                }
                *self.houses[index].tab_slot_mut(tab) = Some(production);
            }
        }

        // An idle animated terrain object rolls each
        // tick against its probability to start; a playing one steps an image every
        // `AnimationRate` ticks and goes back to idle on reaching half the art's
        // frame count, having shown each image once.
        for index in 0..self.animated.len() {
            if self.animated[index].playing == 0 {
                let roll = self.next_random() % 1_000_000;
                let object = &mut self.animated[index];
                if (roll as f64) * 1e-6 < object.probability as f64 {
                    object.stage = 0;
                    object.playing = object.rate;
                    object.timer_start = self.tick;
                    object.timer_length = object.rate as u64;
                }
            }
            let object = &mut self.animated[index];
            if object.playing != 0 && self.tick - object.timer_start >= object.timer_length {
                object.stage += 1;
                object.timer_start = self.tick;
                if object.stage >= object.frames {
                    object.stage = 0;
                    object.playing = 0;
                    object.timer_length = 0;
                    // And an ore drill puts ore on
                    // a cell beside it, whatever the growth switches say.
                    let cell = object.cell;
                    self.spread_ore_from(cell, true);
                }
            }
        }

        // Build-up animations step on, and a building that has finished coming down
        // becomes its vehicle.
        let mut packed = Vec::new();
        let mut sold = Vec::new();
        let mut opened = Vec::new();
        for building in &mut self.buildings {
            let Some(buildup) = building.buildup.as_mut() else {
                continue;
            };
            if self.tick >= buildup.next {
                buildup.stage += 1;
                buildup.next = self.tick + buildup.rate as u64;
                if buildup.stage >= buildup.count - 1 {
                    match buildup.packing {
                        Some(dest) => packed.push((building.id, dest)),
                        None if buildup.selling => sold.push(building.id),
                        None => {
                            building.buildup = None;
                            building.opened = self.tick;
                            opened.push(building.id);
                        }
                    }
                }
            }
        }
        for (building, dest) in packed {
            self.finish_packing(building, dest);
        }
        for building in sold {
            self.finish_sale(building);
        }
        self.repair_buildings();
        for building in opened {
            self.grand_opening(building);
        }
        self.run_miners();

        self.unload_factories();
        self.board_and_unload();
        self.occupation();
        self.detonate_charges();
        self.run_abilities();
        self.run_crates();
        self.run_supers();
        self.run_weather();
        self.run_radiation();
        self.run_marks();
        self.run_patients();
        self.run_depots();
        self.run_cash();
        self.run_ore();
        self.run_chargers();
        self.run_defeat();
        self.run_spawns();
        self.run_teams();
        self.think();
        self.look();

        let deploying: Vec<u32> = self.units.iter().filter(|u| u.deploying).map(|u| u.id).collect();
        for unit in deploying {
            self.try_deploy(unit);
        }

        // In id order, each unit moves, seeing the others as they stand at that
        // moment.
        let mut others: Vec<Other> = self.others();
        let mut scatter = Vec::new();
        let mut attack = Vec::new();
        let mut entering = Vec::new();
        {
            let Self { map, tracks, claims, units, buildings, spots, spot_house, random, .. } = self;
            for (index, unit) in units.iter_mut().enumerate() {
                let mut ground = Ground {
                    map,
                    claims,
                    spots,
                    spot_house,
                    random,
                    others: &others,
                    buildings,
                    tracks: tracks.as_ref(),
                    blockage_delay: self.blockage_delay as u64,
                    path_delay: self.path_delay as u64,
                    close_enough: self.close_enough,
                    now: self.tick,
                    scatter: &mut scatter,
                    attack: &mut attack,
                    entering: &mut entering,
                };
                let before = unit.cell();
                // A soldier digging in, dug in or getting up goes nowhere.
                let rooted = unit.stats.infantry && unit.stance != combat::Stance::Standing;
                if unit.frozen.is_none() && !rooted {
                    advance(unit, &mut ground);
                }
                // At a
                // change of cell: onto a bridge cell whose ground is four
                // levels lower, it is on the deck; off bridge cells, it is not.
                let after = unit.cell();
                if after != before {
                    let (old, new) = (ground.map.level(before.0, before.1) as i32, ground.map.level(after.0, after.1) as i32);
                    if new == old - 4 && ground.map.bridged(after.0, after.1) {
                        unit.on_bridge = true;
                    } else if !ground.map.bridged(after.0, after.1) && ground.map.bridged(before.0, before.1) {
                        unit.on_bridge = false;
                    }
                }
                others[index] = Other { allies: others[index].allies, ..Other::of(unit) };
            }
        }
        // A unit held up by a friend at rest asks that cell to clear with no
        // point to go away from, so what is there looks further than its
        // neighbours for a free place.
        for cell in scatter {
            self.scatter_from(cell, Scatter::Nearest);
        }
        for (id, cell) in attack {
            self.attack_blocker(id, cell);
        }
        for (id, cell) in entering {
            if let Some(index) = self.units.iter().position(|u| u.id == id) {
                self.collect_crate(index, cell);
            }
        }
        self.fly_planes();
        self.fight();
        self.run_soldiers();
        // A flyer shot down is gone when
        // it reaches the ground, with its explosion.
        let fallen: Vec<u32> =
            self.units.iter().filter(|u| u.jet.is_some_and(|jet| jet.crashed) || u.plane.is_some_and(|plane| plane.crashed)).map(|u| u.id).collect();
        // Where it comes
        // down an aircraft goes off with its `DeathWeapon`, else its first
        // weapon, else the rules' own (no `Explodes` is asked for); of the
        // hovering kinds only a `BalloonHover` one (the Kirov) does.
        for id in fallen {
            if let Some(unit) = self.unit(id) {
                let blasts = unit.plane.is_some() || unit.stats.jet.is_some_and(|jet| jet.balloon_hover);
                if blasts {
                    let point = (unit.x, unit.y, self.ground_z(unit.x, unit.y));
                    let (own, percent, primary, strength) = (unit.stats.death_weapon, unit.stats.death_percent, unit.stats.fighter.primary.map(|mount| mount.weapon), unit.stats.strength);
                    self.death_blast(point, own, percent, primary, strength, combat::Aim::Unit(id));
                }
            }
            self.kill(combat::Aim::Unit(id));
        }
        // A vehicle just out of its factory, with no
        // rally point to go to, moves off the building.
        // So does an idle foot soldier standing on a
        // building's cell.
        let mut leaving = Vec::new();
        let mut going = Vec::new();
        for unit in &mut self.units {
            let on_building = unit.stats.infantry
                && !unit.is_moving()
                && self.buildings.iter().any(|b| {
                    let cell = unit.cell();
                    (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1)
                });
            let left = std::mem::take(&mut unit.left_factory);
            // A harvester let
            // go by its factory is given the Harvest mission, whoever owns it.
            if left && unit.stats.harvester {
                unit.rally = None;
                unit.work.job = Job::Harvest;
                unit.work.status = 0;
                unit.work.next = self.tick + 1;
                continue;
            }
            if (left || !unit.is_moving()) && !unit.tethered {
                if let Some(rally) = unit.rally.take() {
                    going.push((unit.id, rally));
                    continue;
                }
            }
            if left || on_building {
                leaving.push(unit.id);
            }
        }
        for unit in leaving {
            self.scatter_unit(unit);
        }
        // Every unit is sent to the same cell.
        for (unit, (x, y)) in going {
            self.order(Order::Move { unit, x, y });
        }
    }

    /// Asks whatever vehicle is resting in a cell to move to a neighbouring one.
    // Nothing happens if it is turning or already
    // going somewhere; otherwise it tries the eight directions in turn, starting
    // from its facing nudged by -1, 0 or +1 at random, and goes to the first
    // neighbouring cell it can enter freely.
    fn scatter(&mut self, cell: (i32, i32)) {
        self.scatter_from(cell, Scatter::Facing);
    }

    /// Asks whatever is resting in a cell to move aside, in the given way.
    fn scatter_from(&mut self, cell: (i32, i32), how: Scatter) {
        // Every object in the cell is asked: foot soldiers at rest, any
        // vehicle standing there, and whatever holds the cell (a vehicle
        // leaving a factory holds the cell inside the door while another may
        // still stand on it).
        let claim = self.map.index(cell.0, cell.1).map_or(0, |index| self.claims[index]);
        let mut asked: Vec<u32> = self
            .units
            .iter()
            .filter(|u| u.cell() == cell && u.health > 0 && !(u.stats.infantry && u.is_moving()))
            .map(|u| u.id)
            .collect();
        if let Some(unit) = self.units.iter().find(|u| u.id + 1 == claim).map(|u| u.id) {
            if !asked.contains(&unit) {
                asked.push(unit);
            }
        }
        for unit in asked {
            self.scatter_unit_from(unit, how);
        }
    }

    fn scatter_unit(&mut self, id: u32) {
        self.scatter_unit_from(id, Scatter::Facing);
    }

    /// Every unit as the others see it when moving.
    fn others(&self) -> Vec<Other> {
        self.units
            .iter()
            .map(|unit| {
                let own = 1u32.checked_shl(unit.owner as u32).unwrap_or(0);
                let allies = self.houses.get(unit.owner as usize).map_or(0, |house| house.allies);
                Other { allies: own | allies, ..Other::of(unit) }
            })
            .collect()
    }

    /// A unit with an enemy in the cell in its way fires on it, then goes on.
    // The object in the cell if it is not a friend.
    fn attack_blocker(&mut self, id: u32, cell: (i32, i32)) {
        let Some(index) = self.units.iter().position(|u| u.id == id) else {
            return;
        };
        let owner = self.units[index].owner;
        let enemy = |u: &Unit| u.id != id && u.cell() == cell && u.jet.is_none() && u.plane.is_none() && !self.allied(owner, u.owner);
        let found = self.units.iter().filter(|u| enemy(u)).min_by_key(|u| u.stats.infantry).map(|u| combat::Aim::Unit(u.id));
        let building = || {
            self.buildings
                .iter()
                .find(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1) && !self.allied(owner, b.owner))
                .map(|b| combat::Aim::Building(b.id))
        };
        if let Some(target) = found.or_else(building) {
            let arms = &mut self.units[index].arms;
            arms.target = Some(target);
            arms.attacking = true;
        }
    }

    /// Asks a unit to move aside: to the nearest free place, or away from a
    /// point.
    // With no point given the vehicle is sent to the nearest place it can go; with one, the eight
    // directions are tried starting from the one pointing away from it, nudged by -1, 0 or +1 at
    // random.
    fn scatter_unit_from(&mut self, id: u32, how: Scatter) {
        let Some(index) = self.units.iter().position(|u| u.id == id) else {
            return;
        };
        if self.units[index].is_moving() {
            return;
        }
        // A person's dug-in
        // soldier does not move aside; a computer's gets up and goes. (It was
        // given a destination while still dug in, which it could never walk
        // to, and its team waited on it for ever.)
        if self.units[index].stats.infantry && self.units[index].stance != combat::Stance::Standing {
            if self.human(self.units[index].owner) {
                return;
            }
            if matches!(self.units[index].stance, combat::Stance::Deployed | combat::Stance::Deploying(_)) {
                let frames = self.units[index].stats.fighter.undeploy_frames as u64;
                self.units[index].stance = combat::Stance::Undeploying(self.tick + frames);
                self.units[index].undeploy_at = None;
            }
        }
        if how == Scatter::Nearest {
            let here = self.units[index].cell();
            // Any zone, within a level,
            // picked by frame (not the nearest: a crowd's nearest free cell can
            // be close enough to count as reached before the unit moves).
            let found = nearby(&self.map, &self.claims, &self.spots, &self.units[index], here, Near::new(true, true, None), self.tick).filter(|&to| to != here);
            if let Some(to) = found {
                if self.units[index].jet.is_some() {
                    self.send(index, to);
                } else {
                    let unit = &mut self.units[index];
                    unit.dest = Some(to);
                    unit.drop_path();
                    unit.path_timer = 0;
                    unit.retries = 10;
                }
            }
            return;
        }
        let nudge = (self.next_random() % 3) as usize;
        let start = match how {
            Scatter::Away(point) => {
                let unit = &self.units[index];
                let aim = ((((point.1 - unit.y) as f64).atan2((unit.x - point.0) as f64) - std::f64::consts::FRAC_PI_2) * -10430.06) as i32;
                let away = (((((aim as u16) >> 12) + 1) >> 1) & 7) as usize;
                away + 7 + nudge
            }
            _ => self.units[index].heading() + 7 + nudge,
        };
        let others: Vec<Other> = self.others();
        let mut none = Vec::new();
        let mut no_attack = Vec::new();
        let Self { map, tracks, claims, units, buildings, spots, spot_house, random, .. } = self;
        let ground = Ground {
            map,
            claims,
            spots,
            spot_house,
            random,
            others: &others,
            buildings,
            tracks: tracks.as_ref(),
            blockage_delay: 0,
            path_delay: 0,
            close_enough: 0,
            now: self.tick,
            scatter: &mut none,
            attack: &mut no_attack,
            entering: &mut Vec::new(),
        };
        let unit = &mut units[index];
        let here = unit.cell();
        let mut flies_to = None;
        for turn in 0..8 {
            let dir = (start + turn) & 7;
            let to = step(here, dir);
            if ground.can_enter(unit, to, Some(dir), here) == code::OK {
                unit.dest = Some(to);
                unit.drop_path();
                unit.path_timer = 0;
                unit.retries = 10;
                flies_to = unit.jet.map(|_| to);
                break;
            }
        }
        if let Some(to) = flies_to {
            self.units[index].dest = None;
            self.send(index, to);
        }
    }

    /// Replaces a building with the vehicle it packs up into and sends that off.
    // The vehicle appears one cell down and right of
    // the foundation's top cell, facing the deploy facing, with the building's
    // share of health, and drives to the cell.
    fn finish_packing(&mut self, building: u32, dest: (i32, i32)) {
        let Some(index) = self.buildings.iter().position(|b| b.id == building) else {
            return;
        };
        let packed = self.buildings.remove(index);
        let kind = self.building_types[packed.kind as usize].clone();
        for dy in 0..packed.size.1 {
            for dx in 0..packed.size.0 {
                let cell = (packed.cell.0 + dx, packed.cell.1 + dy);
                let level = self.map.level(cell.0, cell.1);
                self.map.set_cell(cell.0, cell.1, true, level);
            }
        }
        let Some(stats) = kind.undeploys_into else {
            return;
        };
        let id = self.add_unit((packed.cell.0 + 1, packed.cell.1 + 1), DEPLOY_FACING, stats, packed.owner);
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
            let health = packed.health as i64 * stats.strength as i64 / kind.strength.max(1) as i64;
            unit.health = (health as i32).max(1);
        }
        self.unpacked.push((building, id));
        self.order(Order::Move { unit: id, x: dest.0, y: dest.1 });
    }

    /// One step of a unit's deployment: wait until it has stopped, check the ground,
    /// turn to the deploy facing, then become the building.
    // That order of events; the building
    // takes the unit's share of health.
    fn try_deploy(&mut self, id: u32) {
        let Some(index) = self.units.iter().position(|u| u.id == id) else {
            return;
        };
        if self.units[index].is_moving() {
            return;
        }
        if !self.can_deploy(id) {
            self.units[index].deploying = false;
            return;
        }
        let unit = &mut self.units[index];
        if unit.facing != DEPLOY_FACING {
            unit.turn = Turn::start(unit.turn.current(), (DEPLOY_FACING as u16) << 8, unit.stats.rot);
            if unit.turn.remaining > 0 {
                return;
            }
            unit.facing = DEPLOY_FACING;
        }
        let deployed = self.units.remove(index);
        for claim in self.claims.iter_mut().filter(|claim| **claim == deployed.id + 1) {
            *claim = 0;
        }
        if let Some(kind) = deployed.stats.deploys_into {
            self.add_building(deployed.owner, kind, Self::deploy_origin(deployed.cell()));
            let full = self.building_types.get(kind as usize).map_or(1, |kind| kind.strength);
            if let Some(building) = self.buildings.last_mut() {
                let health = deployed.health as i64 * full as i64 / deployed.stats.strength.max(1) as i64;
                building.health = (health as i32).max(1);
            }
        }
    }

    /// A digest of everything that affects future ticks.
    pub fn hash(&self) -> u64 {
        // FNV-1a.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |value: i64| {
            for byte in value.to_le_bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(self.tick as i64);
        mix(self.random as i64);
        for object in &self.animated {
            mix(object.stage as i64);
            mix(object.playing as i64);
            mix(object.timer_start as i64);
        }
        for house in &self.houses {
            mix(house.credits as i64);
            for &queued in house.infantry_queue.iter().chain(&house.vehicle_queue).chain(&house.ship_queue).chain(&house.aircraft_queue) {
                mix(queued as i64);
            }
            for production in [house.production, house.defence, house.infantry, house.vehicles, house.ships, house.aircraft].into_iter().flatten() {
                mix(production.building as i64);
                mix(production.stage as i64);
                mix(production.balance as i64);
                mix(production.suspended as i64);
            }
        }
        for building in &self.buildings {
            mix(building.id as i64);
            mix(building.kind as i64);
            mix(building.owner as i64);
            mix(building.cell.0 as i64);
            mix(building.cell.1 as i64);
            mix(building.buildup.map_or(-1, |b| b.stage as i64));
        }
        for unit in &self.units {
            mix(unit.id as i64);
            mix(unit.owner as i64);
            mix(unit.x as i64);
            mix(unit.y as i64);
            mix(unit.facing as i64);
            mix(unit.speed as i64);
            mix(unit.progress as i64);
            mix(unit.fraction as i64);
            mix(unit.turn.desired as i64);
            mix(unit.turn.remaining as i64);
            mix(unit.deploying as i64);
            mix(unit.head_to.map_or(-1, |head| (head.0 as i64) << 32 | head.1 as i64));
            mix(unit.path_timer as i64);
            mix(unit.retries as i64);
            mix(unit.blocked_until.map_or(-1, |tick| tick as i64));
            if let Some(dest) = unit.dest {
                mix(dest.0 as i64);
                mix(dest.1 as i64);
            }
            if let Some(drive) = unit.drive {
                mix(drive.selector as i64);
                mix(drive.index as i64);
                mix(drive.head.0 as i64);
                mix(drive.head.1 as i64);
            }
            for &step in &unit.path {
                mix(step as i64);
            }
        }
        hash
    }
}

/// What one unit needs to know of another it meets.
#[derive(Clone, Copy)]
struct Other {
    id: u32,
    owner: u8,
    moving: bool,
    /// Whether it is at a standstill, the direction of its next step (or -1), and
    /// the direction it faces.
    stopped: bool,
    next_step: i8,
    heading: usize,
    cell: (i32, i32),
    /// A foot soldier, and whether it is walking.
    infantry: bool,
    walking: bool,
    /// `Crushable`.
    crushable: bool,
    /// On a bridge's deck.
    on_bridge: bool,
    /// The cell a hovering flyer holds.
    holds: Option<(i32, i32)>,
    /// The houses its owner counts as friends, its own among them, a bit each.
    allies: u32,
    cloaked: bool,
    /// Its type, stored speed and path, for the avoid marking of searches.
    kind: u16,
    speed: i32,
    path: [i8; PATH_LENGTH],
}

impl Other {
    fn of(unit: &Unit) -> Self {
        Self {
            id: unit.id,
            owner: unit.owner,
            moving: unit.is_moving(),
            stopped: unit.fraction == 0,
            next_step: unit.path[0],
            heading: unit.heading(),
            cell: unit.cell(),
            infantry: unit.stats.infantry,
            walking: unit.stats.infantry && (unit.head_to.is_some() || unit.dest.is_some()),
            crushable: unit.stats.crushable,
            on_bridge: unit.on_bridge,
            holds: unit.jet.and_then(|jet| jet.holds),
            allies: 1u32.checked_shl(unit.owner as u32).unwrap_or(0),
            cloaked: unit.cloak.state == 2,
            kind: unit.stats.kind,
            speed: unit.stats.speed,
            path: unit.path,
        }
    }
}

/// Whether the house of the unit `me` counts `owner` as a friend. (A unit
/// not among the others, such as a probe, has only its own house.)
fn friend_of(others: &[Other], me: &Unit, owner: u8) -> bool {
    owner == me.owner
        || others.binary_search_by_key(&me.id, |other| other.id).is_ok_and(|at| others[at].allies & 1u32.checked_shl(owner as u32).unwrap_or(0) != 0)
}

/// How a unit asked to move aside chooses where to go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scatter {
    /// To the nearest free place.
    Nearest,
    /// Away from a point (leptons).
    Away((i32, i32)),
    /// Starting from the way it faces (callers not traced).
    Facing,
}

/// What a moving unit can see and change of the world.
struct Ground<'a> {
    map: &'a Map,
    claims: &'a mut Vec<u32>,
    spots: &'a mut Vec<u8>,
    spot_house: &'a mut Vec<u8>,
    random: &'a mut u64,
    /// The other units, in id order.
    others: &'a [Other],
    buildings: &'a [Building],
    tracks: Option<&'a TrackTables>,
    blockage_delay: u64,
    path_delay: u64,
    close_enough: i32,
    now: u64,
    /// Cells whose occupant has been asked to get out of the way.
    scatter: &'a mut Vec<(i32, i32)>,
    /// Units that will fire on the enemy in the cell in their way.
    attack: &'a mut Vec<(u32, (i32, i32))>,
    /// Units setting off into a cell, and the cell (where a crate is picked up).
    entering: &'a mut Vec<(u32, (i32, i32))>,
}

impl Ground<'_> {
    fn holder(&self, cell: (i32, i32), me: u32) -> Option<&Other> {
        holder(self.map, self.claims, self.others, cell, me, false)
    }

    // A bridge cell keeps what is on its
    // deck apart from what is on the ground under it.
    fn hold(&mut self, cell: (i32, i32), unit: &Unit) {
        if let Some(index) = self.map.index(cell.0, cell.1) {
            let deck = self.map.bridged(cell.0, cell.1) && (unit_level(self.map, unit) - self.map.level(cell.0, cell.1) as i32).abs() > 1;
            let half = self.claims.len() / 2;
            self.claims[index + if deck { half } else { 0 }] = unit.id + 1;
        }
    }

    fn release(&mut self, cell: (i32, i32), me: u32) {
        if let Some(index) = self.map.index(cell.0, cell.1) {
            let half = self.claims.len() / 2;
            for at in [index, index + half] {
                if self.claims[at] == me + 1 {
                    self.claims[at] = 0;
                }
            }
        }
    }

    fn can_enter(&self, unit: &Unit, to: (i32, i32), dir: Option<usize>, from: (i32, i32)) -> u8 {
        // A mover following its
        // path is at its cell's level, and four more on a deck.
        // (For the second cell of a curve the step starts from the first, at
        // that cell's level, on its deck if the mover is on one.)
        let level = match dir {
            Some(_) if from == unit.cell() => unit_level(self.map, unit),
            Some(_) => {
                let on_deck = self.map.bridged(from.0, from.1) && (unit.on_bridge || unit_level(self.map, unit) - self.map.level(from.0, from.1) as i32 > 1);
                self.map.level(from.0, from.1) as i32 + if on_deck { 4 } else { 0 }
            }
            None => -1,
        };
        self.can_enter_at(unit, to, dir, from, level)
    }

    fn can_enter_at(&self, unit: &Unit, to: (i32, i32), dir: Option<usize>, from: (i32, i32), level: i32) -> u8 {
        entry_code(self.map, self.claims, (self.spots, self.spot_house), self.others, self.buildings, unit, to, dir, from, level)
    }

    /// What a step into a cell costs the search.
    // For a moving block the search follows the chain
    // of units in the way, each to the cell it is heading for, for up to ten hops;
    // if the chain ends in a free cell the step costs 1, otherwise 4.
    // (Cells marked to be avoided then cost four times as much: `avoid_cells`.)
    fn step_cost(&self, unit: &Unit, entry: u8, cell: (i32, i32), mode: Mode) -> f32 {
        if entry != code::MOVING_BLOCK {
            return path::code_cost(entry);
        }
        match mode {
            Mode::Avoid => 4.0,
            Mode::AvoidAll => 1000.0,
            Mode::Normal => {
                let mut at = cell;
                for _ in 0..10 {
                    let Some(other) = self.holder(at, unit.id) else {
                        return 1.0;
                    };
                    let dir = if other.stopped {
                        if other.next_step < 0 {
                            return 1.0;
                        }
                        other.next_step as usize & 7
                    } else {
                        other.heading
                    };
                    at = step(at, dir);
                }
                4.0
            }
        }
    }

    /// Whether a cell off a route's corridor may be used all the same: one with a
    /// unit, building or terrain object beside it.
    // Units count for the eight
    // cells round them, buildings for their own cells and the ring round them.
    fn beside_something(&self, cell: (i32, i32)) -> bool {
        self.map.index(cell.0, cell.1).is_some_and(|index| self.map.exempt[index])
            || self.others.iter().any(|other| {
                other.cell != cell && (other.cell.0 - cell.0).abs() <= 1 && (other.cell.1 - cell.1).abs() <= 1
            })
            || self.buildings.iter().any(|b| {
                (b.cell.0 - 1..=b.cell.0 + b.size.0).contains(&cell.0)
                    && (b.cell.1 - 1..=b.cell.1 + b.size.1).contains(&cell.1)
            })
    }

    /// Looks for a path from where the unit is to its destination and stores its
    /// first steps. Returns whether one was found.
    // A destination held by a resting friend, or under
    // a building, is first swapped for a free cell near it; on failure the unit
    // may not try again for the path delay, and gives its destination up unless it
    // is next to it.
    // A destination in another zone cannot
    // be reached; otherwise a corridor of subzones is picked and the cell search
    // kept to it, unless either end is outside the playable area or no corridor is
    // found.
    fn avoid_cells(&self, unit: &Unit, mode: Mode) -> (std::collections::HashSet<(i32, i32)>, Mode) {
        let mut marks = std::collections::HashSet::new();
        if mode == Mode::Normal {
            return (marks, mode);
        }
        let mut toggle = |cell: (i32, i32)| {
            if !marks.remove(&cell) {
                marks.insert(cell);
            }
        };
        let here = unit.cell();
        let ahead = step(here, unit.heading());
        let mut there: Vec<&Other> = self.others.iter().filter(|o| o.id != unit.id && o.cell == ahead && o.holds.is_none()).collect();
        if there.is_empty() {
            there.extend(self.holder(ahead, unit.id));
        }
        let mut found = false;
        for other in there {
            let taken = mode == Mode::AvoidAll || (other.kind != unit.stats.kind && unit.stats.speed > other.speed);
            let steps = if other.infantry { 3 } else { 2 };
            if !taken || other.path[..steps].iter().any(|&dir| dir < 0) {
                continue;
            }
            found = true;
            let mut at = other.cell;
            for &dir in other.path.iter().take_while(|&&dir| dir >= 0) {
                at = step(at, dir as usize & 7);
                toggle(at);
            }
        }
        if !found && mode == Mode::Avoid {
            return (std::collections::HashSet::new(), Mode::Normal);
        }
        for dy in -2..=2 {
            for dx in -2..=2 {
                let cell = (ahead.0 + dx, ahead.1 + dy);
                let occupied = self.map.index(cell.0, cell.1).is_some_and(|index| self.claims[index] != 0 || self.spots[index] & SPOT_BITS != 0);
                if occupied && cell != here {
                    toggle(cell);
                }
            }
        }
        toggle(ahead);
        (marks, mode)
    }

    fn find_path(&mut self, unit: &mut Unit, mode: Mode) -> bool {
        unit.drop_path();
        let Some(mut dest) = unit.dest else {
            return false;
        };
        let here = unit.cell();
        let zone = unit.stats.movement_zone as usize;
        let entering = unit.enter_area.is_some_and(|(origin, size)| {
            (origin.0..origin.0 + size.0).contains(&dest.0) && (origin.1..origin.1 + size.1).contains(&dest.1)
        });
        let reachable = self.map.zones.as_ref().is_none_or(|zones| {
            let from = unit_zones(self.map, zones, unit);
            if entering {
                leaving_zones(zones, dest, zone).iter().any(|zone| from.contains(zone))
            } else {
                from.contains(&dest_zone(self.map, dest, zone))
            }
        });
        if !reachable {
            unit.dest = None;
            return false;
        }
        let entry = self.can_enter(unit, dest, None, here);
        let built_on = self.buildings.iter().any(|b| {
            (b.cell.0..b.cell.0 + b.size.0).contains(&dest.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&dest.1)
        }) || self.map.index(dest.0, dest.1).is_some_and(|index| self.map.obstacle[index] == Obstacle::Building as u8);
        if entry == code::TEMP && lepton_distance(unit, dest) > self.close_enough as i64 {
            let nearer = |cell: (i32, i32)| {
                let (cx, cy) = cell_centre(cell);
                let (dx, dy) = cell_centre(dest);
                let apart = (((cx - dx) as f64).powi(2) + ((cy - dy) as f64).powi(2)).sqrt() as i64;
                apart < lepton_distance(unit, dest)
            };
            let found = nearby(self.map, self.claims, self.spots, unit, dest, Near::new(false, true, Some(here)), self.now);
            if let Some(cell) = found.filter(|cell| nearer(*cell)) {
                dest = cell;
                unit.dest = Some(cell);
            }
        } else if entry == code::NO && built_on {
            match nearby(self.map, self.claims, self.spots, unit, dest, Near::new(false, true, Some(here)), self.now) {
                Some(cell) => {
                    dest = cell;
                    unit.dest = Some(cell);
                }
                None => {
                    unit.dest = None;
                    return false;
                }
            }
        }

        let (avoid, mode) = self.avoid_cells(unit, mode);
        let steps = {
            let this = &*self;
            let who = &*unit;
            let zones = this.map.zones.as_ref();
            // On a deck, or with a bridge at the
            // destination, the corridor runs from or to the nearer end of the
            // bridge.
            let (from, to) = (this.map.corridor_cell(here, who.on_bridge), this.map.corridor_cell(dest, this.map.bridged(dest.0, dest.1)));
            let usable = zones.filter(|zones| zones.playable(from) && zones.playable(to));
            let mut barred = zones::Barred::default();
            let mut corridor = usable.and_then(|zones| zones.corridor_barred(from, to, zone, &barred));
            // How far along the bottom level's corridor the search got, and the
            // first cell it reached there.
            let progress = std::cell::Cell::new((0usize, here));
            let run = |corridor: Option<&zones::Corridor>| {
                progress.set((0, here));
                let opened = |cell: (i32, i32)| {
                    if let (Some(corridor), Some(zones)) = (corridor, usable) {
                        let (k, _) = progress.get();
                        if corridor.order[0].get(k + 1) == Some(&zones.subzone(cell)) {
                            progress.set((k + 1, cell));
                        }
                    }
                };
                path::search(
                    &path::Query {
                        width: this.map.width,
                        height: this.map.height,
                        enter: &|to, dir, from, level| this.can_enter_at(who, to, Some(dir), from, level),
                        bridged: &|cell| this.map.bridged(cell.0, cell.1),
                        // The
                        // mover starts four levels up if it is on a deck, and
                        // a destination cell with a bridge means its deck.
                        start_level: unit_level(this.map, who),
                        dest_level: this.map.level(dest.0, dest.1) as i32 + if this.map.deck(dest.0, dest.1) { 4 } else { 0 },
                        cost: &|entry, cell| this.step_cost(who, entry, cell, mode) * if avoid.contains(&cell) { 4.0 } else { 1.0 },
                        level: &|cell| this.map.level(cell.0, cell.1) as i32,
                        allowed: &|cell| match (corridor, usable) {
                            (Some(corridor), Some(zones)) => corridor.on[zones.subzone(cell) as usize] || this.beside_something(cell),
                            _ => true,
                        },
                        opened: &opened,
                    },
                    here,
                    dest,
                )
            };
            // A search kept to a corridor
            // that fails bars links at the subzones where it got furthest and a
            // new corridor is picked, up to five tries in all; once the corridor
            // cannot be used, one search without it is final.
            // The cell entered is judged at its own
            // level, so a step that only the height rule refuses counts as open.
            let blocked = |to: (i32, i32), dir: usize, from: (i32, i32)| this.can_enter_at(who, to, Some(dir), from, this.map.level(to.0, to.1) as i32) != code::OK;
            let mut tries = 0;
            let found = loop {
                let found = run(corridor.as_ref());
                let (Some(zones), Some(confined)) = (usable, corridor.as_ref()) else {
                    break found;
                };
                if found.is_some() {
                    break found;
                }
                tries += 1;
                let still = zones.bar_after_failure(confined, progress.get().1, zone, &blocked, &mut barred);
                // After five failures, or with no corridor left to
                // pick, it gives up; once the corridor can no longer be used,
                // one search without it is final.
                if tries >= 5 {
                    break None;
                }
                if !still {
                    break run(None);
                }
                corridor = zones.corridor_barred(from, to, zone, &barred);
                if corridor.is_none() {
                    break None;
                }
            };
            found
        };
        match steps {
            Some(steps) => {
                for (slot, step) in unit.path.iter_mut().zip(steps.iter().map(|s| *s as i8).chain(std::iter::repeat(-1))) {
                    *slot = step;
                }
                unit.path_timer = self.now;
                true
            }
            None => {
                unit.path_timer = self.now + self.path_delay;
                if (here.0 - dest.0).abs().max((here.1 - dest.1).abs()) > 1 {
                    unit.dest = None;
                }
                false
            }
        }
    }
}

/// The other unit holding a cell, if any.
/// The level a unit is at: its cell's, and four more on a bridge's deck.
fn unit_level(map: &Map, unit: &Unit) -> i32 {
    let cell = unit.cell();
    map.level(cell.0, cell.1) as i32 + if unit.on_bridge { 4 } else { 0 }
}

fn holder<'a>(map: &Map, claims: &[u32], others: &'a [Other], cell: (i32, i32), me: u32, deck: bool) -> Option<&'a Other> {
    let claim = claims[map.index(cell.0, cell.1)? + if deck { claims.len() / 2 } else { 0 }];
    if claim == 0 || claim == me + 1 {
        return None;
    }
    let at = others.binary_search_by_key(&(claim - 1), |other| other.id).ok()?;
    Some(&others[at])
}

/// The entry code for a unit stepping into a cell, in a direction from a
/// neighbouring cell; with no direction, only what is in the cell counts.
// The height rule (level, or one level
// up from a ramp, or one level down onto a ramp); a moving friend in the cell
// is a moving block and a resting one a temporary block; land the unit's speed
// type cannot cross is closed.
// What belongs to a house the mover's counts as
// a friend is in the way as above; an enemy is to be crushed, shot through
// (armed vehicles) or closed (soldiers and unarmed vehicles; see below).
fn entry_code(
map: &Map,
claims: &[u32],
spots: (&[u8], &[u8]),
others: &[Other],
buildings: &[Building],
unit: &Unit,
to: (i32, i32),
dir: Option<usize>,
from: (i32, i32),
level: i32,
) -> u8 {
    if !map.in_playfield(to.0, to.1) {
        return code::NO;
    }
    // The height rule. `level` is
    // the level the mover is at (-1: not known); a bridge's deck is four
    // levels above the ground of its cell, entered only from ground at that
    // height into a cell whose deck can be driven on, and left the same way;
    // along a bridge both cells must have a deck to drive on, or the mover be
    // on the ground under it.
    let (h, from_h) = (map.level(to.0, to.1) as i32, map.level(from.0, from.1) as i32);
    let mut level = level;
    if dir.is_some() {
        let from_bridge = map.bridged(from.0, from.1);
        if level == -1 && from_bridge {
            level = from_h + 4;
            if !map.deck(to.0, to.1) && map.bridged(to.0, to.1) {
                return code::NO;
            }
        }
        let src = if from_bridge { from_h } else if level == -1 { from_h } else { level };
        let allowed = match src - h {
            0 => (map.deck(to.0, to.1) && from_bridge && (level == -1 || level != h || map.deck(from.0, from.1))) || level == -1 || level == h,
            1 => map.ramp(to.0, to.1) != 0,
            -1 => map.ramp(from.0, from.1) != 0,
            // Stepping off the deck onto the high ground at its end.
            -4 => from_bridge && level == from_h + 4,
            // Stepping on from the high ground.
            4 => map.deck(to.0, to.1),
            _ => false,
        };
        if !allowed {
            return code::NO;
        }
        // On a deck, a neighbour with a bridge over it but no deck to drive on
        // is closed.
        if from_bridge && level == from_h + 4 && src - h == 0 && !map.deck(to.0, to.1) {
            return code::NO;
        }
    }
    // The cell is entered on its deck when it has
    // a bridge and the mover's level is not known or is not the ground's.
    let deck = map.bridged(to.0, to.1) && (level == -1 || (level - h).abs() > 1);
    if deck && !map.deck(to.0, to.1) {
        return code::NO;
    }
    // On the missions that
    // go into something, the cells of what the unit was sent into are open to
    // it: a building's foundation, and the cell its transport stands on.
    if let Some((origin, size)) = unit.enter_area {
        if (origin.0..origin.0 + size.0).contains(&to.0) && (origin.1..origin.1 + size.1).contains(&to.1) {
            return code::OK;
        }
    }
    let mut result = code::OK;
    // What is on the other layer
    // of a bridge cell is not in the way.
    let bridge_cell = map.bridged(to.0, to.1);
    if let Some(other) = holder(map, claims, others, to, unit.id, deck) {
        if unit.boarding == Some(other.id) && unit.stats.infantry {
            // Its own transport: the soldier walks on to a spot of the cell.
        } else if !friend_of(others, unit, other.owner) && other.cloaked {
            // A cloaked enemy is a cell the mover does not know is taken.
            result = code::CLOAK;
        } else if !friend_of(others, unit, other.owner) {
            // To a vehicle an
            // enemy is something to crush (no code) if it is a `Crusher`
            // and the enemy `Crushable`; else something to shoot through if
            // it is armed; else closed. The same for an armed soldier.
            if unit.stats.fighter.primary.is_none() {
                return code::NO;
            }
            if unit.stats.infantry || !(unit.stats.crusher && other.crushable) {
                result = code::DESTROYABLE;
            }
        } else {
            // A friend on the move that faces this unit head on, from the very
            // cell this unit faces, within half a cell's width of two, closes
            // the cell, which ends the move. Two units each going for the
            // other's cell would otherwise wait for ever.
            let toward = DIRECTIONS.iter().position(|&step| (unit.cell().0 + step.0, unit.cell().1 + step.1) == other.cell);
            let me = unit.heading();
            if other.moving && toward == Some(me) && (other.heading + 4) % 8 == me {
                return code::NO;
            }
            result = if other.moving { code::MOVING_BLOCK } else { code::TEMP };
        }
    }
    // An enemy building is something an armed
    // vehicle can shoot its way through; anyone else's is closed.
    if !deck && map.index(to.0, to.1).is_some_and(|index| map.obstacle[index] == Obstacle::Building as u8 && !map.passable[index]) {
        let owner = buildings.iter().find(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&to.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&to.1)).map(|b| b.owner);
        return match owner {
            Some(owner) if !friend_of(others, unit, owner) && unit.stats.fighter.primary.is_some() => result.max(code::DESTROYABLE),
            _ => code::NO,
        };
    }
    if map.speed_on(to.0, to.1, unit.stats.speed_type, deck) == 0 {
        return code::NO;
    }
    // Foot soldiers in the cell (on a bridge cell, those of the same layer).
    let index = map.index(to.0, to.1).unwrap_or(0);
    let taken = if bridge_cell {
        let here = others.iter().filter(|other| other.infantry && other.cell == to && other.on_bridge == deck && other.id != unit.id).count();
        if here == 0 { 0 } else if here >= 3 { SPOT_BITS } else { spots.0[index] & SPOT_BITS }
    } else {
        spots.0[index] & SPOT_BITS
    };
    let tree = map.tree_bits[index];
    if taken == 0 {
        return result;
    }
    // A tree's spots and the soldiers' between them fill the cell.
    if unit.stats.infantry && tree != 0 && (taken | tree) == SPOT_BITS && spots.1[index] == unit.owner {
        return result.max(code::TEMP);
    }
    if !friend_of(others, unit, spots.1[index]) {
        // To a `Crusher`
        // vehicle a cell of enemy soldiers that can all be crushed is open; to
        // another armed one, or an armed soldier, they are something to
        // shoot through.
        if unit.stats.infantry {
            return if unit.stats.fighter.primary.is_some() { result.max(code::DESTROYABLE) } else { code::NO };
        }
        let crushes = unit.stats.crusher && others.iter().filter(|other| other.infantry && other.cell == to).all(|other| other.crushable);
        return if crushes {
            result
        } else if unit.stats.fighter.primary.is_some() {
            result.max(code::DESTROYABLE)
        } else {
            code::NO
        };
    }
    let resting = others.iter().filter(|other| other.infantry && other.cell == to && !other.walking).count();
    if unit.stats.infantry {
        // Friends share a cell; a full one is a
        // temporary block if all three are at rest, else a moving one.
        if taken == SPOT_BITS && result < code::MOVING_BLOCK {
            result = if resting == 3 { code::TEMP } else { code::MOVING_BLOCK };
        }
    } else {
        let walking = others.iter().any(|other| other.infantry && other.cell == to && other.walking);
        result = result.max(if walking { code::MOVING_BLOCK } else { code::TEMP });
    }
    result
}

/// How a nearby-cell search is asked: the engine's zone, level and
/// close-to arguments.
#[derive(Clone, Copy, Debug)]
struct Near {
    /// Any zone (-1), not only the ones the unit can reach.
    any_zone: bool,
    /// Within a level of the wanted cell.
    same_level: bool,
    /// The candidate nearest this cell is taken; with none, the frame number
    /// picks.
    close_to: Option<(i32, i32)>,
}

impl Near {
    const fn new(any_zone: bool, same_level: bool, close_to: Option<(i32, i32)>) -> Self {
        Self { any_zone, same_level, close_to }
    }
}

/// A free cell near a cell, for a unit that cannot have the cell itself.
// Square rings outward (top and bottom rows from left
// to right, then the two sides from top to bottom); a cell qualifies if it is in
// the playable area, can be driven on and holds nothing, and as asked is in the
// unit's zone and within a level of the wanted cell. Candidates gather ring by ring until one of
// them is in plain view (not hidden behind higher ground in front of it) or 24
// are held. The ones in view are preferred. With a cell to be close to, the
// candidate nearest it is taken, the earliest on a tie; without, the one the
// frame number picks.
fn nearby(
    map: &Map,
    claims: &[u32],
    spots: &[u8],
    unit: &Unit,
    wanted: (i32, i32),
    near: Near,
    frame: u64,
) -> Option<(i32, i32)> {
    let close_to = near.close_to;
    let zone = unit.stats.movement_zone as usize;
    let fits = |cell: (i32, i32)| {
        map.index(cell.0, cell.1).is_some_and(|index| claims[index] == 0 && spots[index] & SPOT_BITS == 0)
            && map.in_playfield(cell.0, cell.1)
            && (near.any_zone || map.zones.as_ref().is_none_or(|zones| unit_zones(map, zones, unit).contains(&dest_zone(map, cell, zone))))
            && map.speed_on(cell.0, cell.1, unit.stats.speed_type, true) > 0
            && (!near.same_level || (map.level(cell.0, cell.1) as i32 - map.level(wanted.0, wanted.1) as i32).abs() <= 1)
    };
    let rings = (map.width + map.height).min(32);
    let mut found: Vec<(i32, i32)> = Vec::new();
    let mut seen = false;
    'rings: for r in 0..rings {
        let mut ring = Vec::new();
        for i in -r..=r {
            ring.push((wanted.0 + i, wanted.1 - r));
            ring.push((wanted.0 + i, wanted.1 + r));
        }
        for j in -r + 1..r {
            ring.push((wanted.0 - r, wanted.1 + j));
            ring.push((wanted.0 + r, wanted.1 + j));
        }
        for cell in ring {
            if !fits(cell) {
                continue;
            }
            found.push(cell);
            seen |= visible_cell(map, cell) == cell;
            if found.len() == 24 {
                break 'rings;
            }
        }
        if seen {
            break;
        }
    }
    let in_view: Vec<(i32, i32)> = found.iter().copied().filter(|&cell| visible_cell(map, cell) == cell).collect();
    let pool = if in_view.is_empty() { found } else { in_view };
    if pool.is_empty() {
        return None;
    }
    Some(match close_to {
        None => pool[(frame % pool.len() as u64) as usize],
        Some(near) => {
            let apart = |cell: &(i32, i32)| (cell.0 - near.0).pow(2) + (cell.1 - near.1).pow(2);
            let mut best = pool[0];
            for cell in &pool[1..] {
                if apart(cell) < apart(&best) {
                    best = *cell;
                }
            }
            best
        }
    })
}

/// The cell drawn at a cell's place on the screen: itself, or a cell in front
/// of it whose higher ground covers it.
// From six cells down and right of the cell's
// centre, back along the diagonal in steps of 8 leptons; the first cell whose
// ground, raised by its height above the cell's (128 leptons a level in x and
// y), comes to lie no further down or right than the cell is the answer.
fn visible_cell(map: &Map, cell: (i32, i32)) -> (i32, i32) {
    let base = map.level(cell.0, cell.1) as i32;
    let (mut x, mut y) = (cell.0 * LEPTONS_PER_CELL + 0x80 + 0x600, cell.1 * LEPTONS_PER_CELL + 0x80 + 0x600);
    loop {
        x -= 8;
        y -= 8;
        let at = (x / LEPTONS_PER_CELL, y / LEPTONS_PER_CELL);
        let rise = (map.level(at.0, at.1) as i32 - base) * 128;
        if (x - rise) / LEPTONS_PER_CELL <= cell.0 && (y - rise) / LEPTONS_PER_CELL <= cell.1 {
            return at;
        }
        if at == cell {
            return cell;
        }
    }
}

/// Leptons from a unit to the centre of a cell.
fn lepton_distance(unit: &Unit, cell: (i32, i32)) -> i64 {
    let (cx, cy) = cell_centre(cell);
    (((cx - unit.x) as f64).powi(2) + ((cy - unit.y) as f64).powi(2)).sqrt() as i64
}

/// Moves one unit forward by a tick.
// A unit on a track follows it, and starts the next
// one in the same tick if it ends; a unit off a track that is turning only turns;
// otherwise it tries to start a track.
fn advance(unit: &mut Unit, ground: &mut Ground) {
    if unit.plane.is_some() {
        return;
    }
    if unit.jet.is_some() {
        flight::fly(unit, ground);
        return;
    }
    if unit.stats.infantry {
        walk(unit, ground);
        return;
    }
    if unit.drive.is_none() {
        if unit.tethered {
            return;
        }
        if unit.turn.remaining > 0 {
            unit.turn.remaining -= 1;
            unit.facing = (unit.turn.current() >> 8) as u8;
            return;
        }
        if !start_of_move(unit, ground, true, false) {
            unit.fraction = 0;
            unit.progress = 0;
            return;
        }
    }

    // Within the slow-down distance of the destination the speed fraction
    // falls by stored speed times the deceleration, but not below 0.3;
    // otherwise it climbs by the acceleration to the track's target, or falls
    // to it by stored speed times the deceleration. Types that do not
    // accelerate take the target at once.
    let target = unit.drive.map_or(UNIT_FRACTION, |drive| drive.target);
    if unit.stats.accelerates {
        let (gx, gy) = cell_centre(unit.dest.unwrap_or(unit.cell()));
        let (dx, dy) = ((gx - unit.x) as i64, (gy - unit.y) as i64);
        let near = unit.stats.slowdown_distance as i64;
        if dx * dx + dy * dy < near * near {
            let slowed = unit.fraction - unit.speed * unit.stats.deceleration;
            unit.fraction = slowed.max(ARRIVAL_FRACTION);
        } else if unit.fraction < target {
            unit.fraction = (unit.fraction + unit.stats.acceleration).min(target);
        } else if unit.fraction > target {
            unit.fraction = (unit.fraction - unit.speed * unit.stats.deceleration).max(target);
        }
    } else {
        unit.fraction = target;
    }

    // The speed added each tick is the stored speed times the speed fraction,
    // truncated. (The ground counts only through the fraction's target.) The
    // way out of a factory is driven at half speed.
    if unit.tethered {
        unit.fraction = UNIT_FRACTION / 2;
    }
    let speed = unit.speed as i64 * unit.fraction as i64 / UNIT_FRACTION as i64;
    unit.progress += speed as i32;
    while unit.progress > SPEED_PER_TRACK_POINT {
        if unit.drive.is_none() && !start_of_move(unit, ground, true, false) {
            break;
        }
        unit.progress -= SPEED_PER_TRACK_POINT;
        follow_track(unit, ground);
        if unit.tethered && unit.drive.is_none() {
            unit.tethered = false;
            unit.left_factory = true;
            break;
        }
    }
}

/// The zones a unit standing in a cell can set out into: the cell's own, or,
/// where the cell is closed to it (a soldier just made, standing on its
/// barracks), those of the cells round it.
/// The zone of a cell as somewhere to go: its deck's, if it has one.
// A bridge cell as a destination always
// means the deck.
fn dest_zone(map: &Map, cell: (i32, i32), movement_zone: usize) -> u16 {
    map.zone_on(cell, movement_zone, map.deck(cell.0, cell.1)).unwrap_or(1)
}

/// The zones a unit can set out in: its bridge's, on a deck.
fn unit_zones(map: &Map, zones: &zones::Zones, unit: &Unit) -> Vec<u16> {
    let movement_zone = unit.stats.movement_zone as usize;
    if unit.on_bridge {
        return vec![map.zone_on(unit.cell(), movement_zone, true).unwrap_or(1)];
    }
    leaving_zones(zones, unit.cell(), movement_zone)
}

fn leaving_zones(zones: &zones::Zones, here: (i32, i32), movement_zone: usize) -> Vec<u16> {
    let own = zones.zone_of(here, movement_zone);
    if own != 1 {
        return vec![own];
    }
    DIRECTIONS.iter().map(|d| zones.zone_of((here.0 + d.0, here.1 + d.1), movement_zone)).filter(|&zone| zone != 1).collect()
}

/// The last cell inside a war factory's door, where the way-out track ends.
// The eleventh of the foundation's exit cells, one cell back; for the
// 5x3 war factories the fifth cell of the middle row.
fn door_cell(building: &Building) -> (i32, i32) {
    (building.cell.0 + building.size.0 - 1, building.cell.1 + building.size.1 / 2)
}

/// The cells round a foundation that a unit may come out at, in the order tried,
/// from the foundation's top cell.
// The lists for 2x2 and 3x2.
// The order used when a foundation has no list.
fn exit_list((w, h): (i32, i32)) -> Vec<(i32, i32)> {
    let mut cells = Vec::new();
    if (w, h) == (2, 2) || (w, h) == (3, 2) {
        cells.extend((0..w).map(|x| (x, h)));
        cells.extend([(-1, h), (w, h)]);
        for y in (0..h).rev() {
            cells.extend([(-1, y), (w, y)]);
        }
        cells.extend((0..w).map(|x| (x, -1)));
        cells.extend([(-1, -1), (w, -1)]);
        return cells;
    }
    for x in -1..=w {
        cells.extend([(x, h), (x, -1)]);
    }
    for y in -1..=h {
        cells.extend([(w, y), (-1, y)]);
    }
    cells
}

impl Work {
    /// Emptying into a refinery: from the start of the dump until it is let go.
    pub fn dumping(&self) -> bool {
        self.dumping
    }
}

impl World {
    /// An explosion's hit on a wall piece.
    // It takes if it is at least the wall's
    // `Strength`, or by chance: a random number below the strength less than
    // the damage.
    // DATA: `Strength` 300, as the stock walls have.
    pub(crate) fn damage_wall(&mut self, cell: (i32, i32), damage: i32) {
        if !self.map.is_wall(cell) {
            return;
        }
        if damage < 300 && (self.random_below(300) as i32) >= damage {
            return;
        }
        let (cleared, shaken) = self.map.hit_wall(cell);
        for next in shaken {
            self.damage_wall(next, 200);
        }
        if !cleared.is_empty() {
            self.map.build_zones();
        }
    }

    /// A random number below a bound, from the game's own stream.
    pub(crate) fn random_below(&mut self, bound: u32) -> u32 {
        random_step(&mut self.random) % bound.max(1)
    }
}

/// A step of the random stream, for code that holds only its state.
pub(crate) fn random_step(state: &mut u64) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 16) as u32 & 0x7fff_ffff
}

impl Ground<'_> {
    fn mark_spot(&mut self, x: i32, y: i32, owner: u8) {
        let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        if let Some(index) = self.map.index(cell.0, cell.1) {
            self.spots[index] |= 1 << spot_index(x, y);
            self.spot_house[index] = owner;
        }
    }

    fn unmark_spot(&mut self, x: i32, y: i32) {
        let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        if let Some(index) = self.map.index(cell.0, cell.1) {
            self.spots[index] &= !(1 << spot_index(x, y));
            if self.spots[index] & SPOT_BITS == 0 {
                self.spot_house[index] = u8::MAX;
            }
        }
    }

    /// The free spot of a cell nearest in kind to a position in it: the spot the
    /// position is on if that is free, else its alternatives in a fixed order.
    // Wanting spot 2
    // tries 4 then 3; 3 tries 4 then 2; 4 tries 2 then 3; from the middle one of
    // four orders is taken at random. A cell with a vehicle in it has no room.
    fn free_spot(&mut self, x: i32, y: i32, boarding: Option<u32>) -> Option<(i32, i32)> {
        let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        let index = self.map.index(cell.0, cell.1)?;
        // A vehicle's cell has no room for soldiers, unless it is the transport
        // this one is getting into.
        if self.claims[index] != 0 && boarding.is_none_or(|id| self.claims[index] != id + 1) {
            return None;
        }
        let taken = self.spots[index] | self.map.tree_bits[index];
        let free = |spot: usize| taken & (1 << spot) == 0;
        let order: [usize; 3] = match spot_index(x, y) {
            2 => [2, 4, 3],
            3 => [3, 4, 2],
            4 => [4, 2, 3],
            _ => [[2, 3, 4], [2, 3, 4], [3, 4, 2], [4, 2, 3]][(random_step(self.random) % 4) as usize],
        };
        let spot = order.into_iter().find(|&spot| free(spot))?;
        Some((cell.0 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].0, cell.1 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].1))
    }
}

/// Moves one foot soldier forward by a tick.
// A straight walk from spot to spot at the stored speed with no speeding up and no
// ground or slope factor; the facing is turned to the spot at once; within 17
// leptons the soldier is put on the spot, and the next frame it chooses its next
// hop, so each cell costs two frames standing.
fn walk(unit: &mut Unit, ground: &mut Ground) {
    if let Some(head) = unit.head_to {
        let (dx, dy) = (head.0 - unit.x, head.1 - unit.y);
        if (path::engine_sqrt((dx * dx + dy * dy) as f32) as i32) < 17 {
            unit.shift_path(1);
            unit.x = head.0;
            unit.y = head.1;
            unit.head_to = None;
            unit.blocked_until = None;
            if unit.dest.is_none() || unit.dest == Some(unit.cell()) {
                give_up(unit);
                unit.stepping = false;
            }
            return;
        }
        let aim = ((((unit.y - head.1) as f64).atan2((head.0 - unit.x) as f64) - std::f64::consts::FRAC_PI_2) * -10430.06) as i32;
        let facing = aim as u16;
        unit.facing = (facing >> 8) as u8;
        unit.turn = Turn::facing(unit.facing);
        let angle = (facing as i16 as f64 - 16383.0) * -(std::f64::consts::TAU / 65536.0);
        // Lying down a `Crawls` soldier goes at
        // two thirds of its speed, any other at half as much again.
        let speed = match (unit.prone, unit.stats.fighter.crawls) {
            (false, _) => unit.speed,
            (true, true) => unit.speed * 2 / 3,
            (true, false) => unit.speed + unit.speed / 2,
        } as f64;
        unit.fraction = UNIT_FRACTION;
        unit.x = (unit.x as f64 + angle.cos() * speed) as i32;
        unit.y = (unit.y as f64 - angle.sin() * speed) as i32;
        return;
    }
    let Some(dest) = unit.dest else {
        unit.fraction = 0;
        unit.stepping = false;
        return;
    };
    if unit.path[0] < 0 {
        if dest == unit.cell() {
            give_up(unit);
            unit.stepping = false;
            return;
        }
        if ground.now < unit.path_timer {
            unit.stepping = false;
            return;
        }
        if !ground.find_path(unit, Mode::Normal) {
            unit.stepping = false;
            unit.path_timer = ground.now + ground.path_delay;
            match unit.dest {
                Some(dest) if lepton_distance(unit, dest) <= ground.close_enough as i64 => give_up(unit),
                Some(_) if unit.retries < 1 => give_up(unit),
                Some(_) => unit.retries -= 1,
                None => give_up(unit),
            }
            return;
        }
        unit.path_timer = ground.now + ground.path_delay;
        unit.retries = 10;
    }
    next_hop(unit, ground, true);
}

/// Chooses a foot soldier's next spot from the first step of its path.
// By the entry code of the next cell; the
// soldier lets go of its spot and takes the next one as it sets off, not when it
// gets there, and keeps the same spot from cell to cell while that is free.
fn next_hop(unit: &mut Unit, ground: &mut Ground, first: bool) {
    let dir = unit.path[0] as usize & 7;
    let here = unit.cell();
    let ahead = (unit.x + DIRECTIONS[dir].0 * LEPTONS_PER_CELL, unit.y + DIRECTIONS[dir].1 * LEPTONS_PER_CELL);
    let next = step(here, dir);
    let entry = ground.can_enter(unit, next, Some(dir), here);
    if entry == code::OK {
        match ground.free_spot(ahead.0, ahead.1, unit.boarding) {
            Some(spot) => {
                ground.unmark_spot(unit.x, unit.y);
                ground.mark_spot(spot.0, spot.1, unit.owner);
                unit.head_to = Some(spot);
                unit.stepping = true;
                // A soldier picks up a
                // crate as it sets off into the cell.
                ground.entering.push((unit.id, next));
                unit.fraction = UNIT_FRACTION;
            }
            None => unit.fraction = 0,
        }
        return;
    }
    unit.stepping = false;
    let retry = |unit: &mut Unit, ground: &mut Ground| {
        unit.drop_path();
        unit.path_timer = ground.now;
        if ground.find_path(unit, Mode::Normal) {
            unit.path_timer = ground.now + ground.path_delay;
            next_hop(unit, ground, false);
        }
    };
    match entry {
        code::MOVING_BLOCK => {
            let since = *unit.blocked_until.get_or_insert(ground.now + ground.blockage_delay);
            if ground.now < unit.path_timer {
                return;
            }
            let mode = if ground.now >= since { Mode::AvoidAll } else { Mode::Avoid };
            let found = ground.find_path(unit, mode);
            unit.path_timer = ground.now + ground.path_delay;
            stop_if_stuck_close(unit, ground, mode, found);
        }
        code::TEMP if first => retry(unit, ground),
        code::TEMP => {
            if unit.dest.is_some_and(|dest| lepton_distance(unit, dest) <= ground.close_enough as i64) {
                give_up(unit);
            } else {
                ground.scatter.push(next);
            }
        }
        _ if first => retry(unit, ground),
        // An enemy in the way with no way round it: attack it, and wait.
        code::DESTROYABLE => ground.attack.push((unit.id, next)),
        _ => {}
    }
}

/// Ends the move of a unit held up by moving units when no way round them is
/// left and it is as good as there.
// When the search
// that avoids every moving unit finds nothing, the destination is kept only if
// it can be reached (zone connectivity alone). Further than a cell
// from the destination a failed search has already dropped it (see
// `find_path`); next to it the unit is close enough and arrives. Both come to
// this: a failed search ends the move when within `CloseEnough`.
fn stop_if_stuck_close(unit: &mut Unit, ground: &Ground, mode: Mode, found: bool) {
    if mode == Mode::AvoidAll && !found && unit.dest.is_some_and(|dest| lepton_distance(unit, dest) <= ground.close_enough as i64) {
        give_up(unit);
    }
}

/// The unit stops trying to get anywhere.
fn give_up(unit: &mut Unit) {
    unit.dest = None;
    unit.drop_path();
    unit.blocked_until = None;
}

/// Decides what a unit standing at a cell centre does next: look for a path, turn,
/// wait, or start a track. Returns whether it is now on a track.
// Everything here unless marked and.
fn start_of_move(unit: &mut Unit, ground: &mut Ground, first_try: bool, no_curve: bool) -> bool {
    if unit.dest.is_none() {
        return false;
    }
    let here = unit.cell();

    // No path: look for one, no more often than the path delay allows.
    if unit.path[0] < 0 {
        if unit.dest == Some(here) {
            give_up(unit);
            return false;
        }
        if ground.now < unit.path_timer {
            return false;
        }
        // With a new path, or
        // after a failed search, the cell ahead (the path's first, or the one
        // the unit faces) is looked at: a friend at rest there is asked to move
        // aside, unless the unit is close enough to count as arrived.
        let rest_ahead = |unit: &mut Unit, ground: &mut Ground, dir: usize| -> bool {
            let cell = step(here, dir);
            if !ground.map.in_playfield(cell.0, cell.1) || ground.can_enter(unit, cell, Some(dir), here) != code::TEMP {
                return false;
            }
            if unit.dest.is_some_and(|dest| lepton_distance(unit, dest) <= ground.close_enough as i64) {
                give_up(unit);
                return true;
            }
            ground.scatter.push(cell);
            false
        };
        if !ground.find_path(unit, Mode::Normal) {
            unit.path_timer = ground.now + ground.path_delay;
            if unit.dest.is_some() && rest_ahead(unit, ground, unit.heading()) {
                return false;
            }
            match unit.dest {
                Some(dest) if lepton_distance(unit, dest) <= ground.close_enough as i64 => give_up(unit),
                Some(_) if unit.retries < 1 => give_up(unit),
                Some(_) => unit.retries -= 1,
                None => give_up(unit),
            }
            return false;
        }
        unit.path_timer = ground.now + ground.path_delay;
        unit.retries = 10;
        if (0..8).contains(&unit.path[0]) && rest_ahead(unit, ground, unit.path[0] as usize) {
            return false;
        }
    }

    let dir = unit.path[0] as usize & 7;
    let next = step(here, dir);

    // Any difference between the facing and the way to the next cell is turned off
    // on the spot first. The table's short turning tracks are never used.
    let wanted = (dir as u8) << 5;
    if unit.facing != wanted {
        unit.turn = Turn::start(unit.turn.current(), (wanted as u16) << 8, unit.stats.rot);
        unit.facing = (unit.turn.current() >> 8) as u8;
        if unit.turn.remaining > 0 || unit.facing != wanted {
            return false;
        }
    }

    let close = |unit: &Unit, ground: &Ground| {
        unit.dest.is_some_and(|dest| lepton_distance(unit, dest) <= ground.close_enough as i64)
    };
    // What to do about a cell that cannot be entered freely, other than one held
    // by a unit on the move.
    let obstructed = |unit: &mut Unit, ground: &mut Ground, entry: u8, cell: (i32, i32)| -> bool {
        if first_try {
            // Look for another path at once, and try that.
            unit.drop_path();
            unit.path_timer = ground.now;
            return start_of_move(unit, ground, false, false);
        }
        if entry == code::TEMP {
            // A friend at rest: near enough to the destination counts as there;
            // otherwise ask it to move over.
            if close(unit, ground) {
                give_up(unit);
            } else {
                ground.scatter.push(cell);
            }
        } else if entry == code::DESTROYABLE {
            // An enemy in the
            // way, with no way round it: attack it, and wait.
            ground.attack.push((unit.id, cell));
        } else {
            give_up(unit);
        }
        false
    };

    match ground.can_enter(unit, next, Some(dir), here) {
        code::OK => {}
        code::MOVING_BLOCK => {
            // Wait; every path delay look for a way round, giving moving units a
            // wide berth once the blockage delay has passed.
            let since = *unit.blocked_until.get_or_insert(ground.now + ground.blockage_delay);
            if ground.now < unit.path_timer {
                return false;
            }
            let mode = if ground.now >= since { Mode::AvoidAll } else { Mode::Avoid };
            let found = ground.find_path(unit, mode);
            unit.path_timer = ground.now + ground.path_delay;
            stop_if_stuck_close(unit, ground, mode, found);
            return false;
        }
        entry => return obstructed(unit, ground, entry, next),
    }

    // The path is about to run out short of the destination: extend it.
    if unit.path[1] < 0 && unit.dest.is_some_and(|dest| lepton_distance(unit, dest) > 0x200) {
        if !ground.find_path(unit, Mode::Normal) {
            return false;
        }
        if unit.path[0] != dir as i8 {
            return false;
        }
    }

    // A straight track into the next cell, or a curve through it into the one
    // after when the route turns there.
    let mut selector = dir * 8 + dir;
    let mut head = next;
    let mut via = None;
    let second = unit.path[1];
    if let (Some(tracks), false, true) = (ground.tracks, no_curve, second >= 0) {
        let second = second as usize & 7;
        let curve = dir * 8 + second;
        if second != dir && tracks.selector[curve].track != 0 && tracks.selector[curve].two_cells() {
            let far = step(next, second);
            match ground.can_enter(unit, far, Some(second), next) {
                code::OK => {
                    selector = curve;
                    head = far;
                    via = Some(next);
                }
                code::MOVING_BLOCK | code::FRIENDLY_DESTROYABLE | code::DESTROYABLE => {
                    return start_of_move(unit, ground, first_try, true);
                }
                entry => return obstructed(unit, ground, entry, far),
            }
        }
    }
    unit.shift_path(if via.is_some() { 2 } else { 1 });
    ground.hold(head, unit);
    if let Some(via) = via {
        ground.hold(via, unit);
    }
    let on_deck = ground.map.bridged(next.0, next.1) && (unit_level(ground.map, unit) - ground.map.level(next.0, next.1) as i32).abs() > 1;
    // The ground's share is cut
    // to full before the rest; a vehicle (not a soldier) going onto higher
    // ground than it stands on takes its kind's uphill factor, onto lower its
    // downhill one (so it can go over full speed); nothing becomes a half;
    // and at `ConditionYellow` health or less it goes at three quarters.
    // The result is the target the speed fraction
    // climbs or falls to (see `advance`), for every track but the forced ones.
    let mut percent = ground.map.speed_on(next.0, next.1, unit.stats.speed_type, on_deck).min(100) as f64;
    if !unit.stats.infantry && !on_deck && !unit.on_bridge {
        let tracked = unit.stats.speed_type == SpeedType::Track;
        let (here_z, next_z) = (ground.map.level(here.0, here.1) as i32 * combat::LEVEL_HEIGHT + ground.map.slope_z(unit.x, unit.y), ground.map.level(next.0, next.1) as i32 * combat::LEVEL_HEIGHT + ground.map.slope_z(next.0 * LEPTONS_PER_CELL + 128, next.1 * LEPTONS_PER_CELL + 128));
        if next_z > here_z {
            percent *= ground.map.slope_speed[if tracked { 0 } else { 2 }];
        } else if next_z < here_z {
            percent *= ground.map.slope_speed[if tracked { 1 } else { 3 }];
        }
    }
    if percent == 0.0 {
        percent = 50.0;
    }
    if unit.health as f64 <= unit.stats.strength as f64 * ground.map.condition_yellow {
        percent *= 0.75;
    }
    let target = (percent / 100.0 * UNIT_FRACTION as f64) as i32;
    unit.drive = Some(Drive { selector, index: 0, head, via, from: here, target });
    // A crate is
    // picked up as the vehicle sets off for the cell, not on reaching it.
    ground.entering.push((unit.id, head));
    true
}

/// Moves a unit to the next point of its track.
fn follow_track(unit: &mut Unit, ground: &mut Ground) {
    let Some(mut drive) = unit.drive else {
        return;
    };
    // With the first point the unit lets go of the
    // cell it is leaving, and is no longer counted as blocked.
    if drive.index == 0 {
        if drive.from != drive.head && ground.tracks.is_some() {
            ground.release(drive.from, unit.id);
        }
        if unit.tethered {
            ground.hold(drive.head, unit);
        }
        unit.blocked_until = None;
    }
    let Some(tracks) = ground.tracks else {
        // Not original behaviour: without the engine's tracks the unit goes
        // straight from cell to cell, and keeps the cell it leaves until it is
        // out of it.
        step_straight(unit, drive);
        if drive.from != drive.head && unit.cell() != drive.from {
            ground.release(drive.from, unit.id);
        }
        return;
    };
    let entry = tracks.selector[drive.selector];
    let track = &tracks.tracks[entry.track as usize];
    let Some(&point) = track.points.get(drive.index) else {
        unit.drive = None;
        return;
    };
    let (x, y, facing) = entry.place(point);
    let (hx, hy) = cell_centre(drive.head);
    unit.x = hx + x;
    unit.y = hy + y;
    unit.set_facing(facing);
    drive.index += 1;

    // A curve lets go of the cell it passes through
    // at the track's cell-crossing point.
    if let Some(via) = drive.via {
        if track.cell >= 0 && drive.index as i32 > track.cell {
            ground.release(via, unit.id);
            drive.via = None;
        }
    }

    if drive.index >= track.points.len() {
        // The track ends at the centre of its cell, facing the way the entry says.
        unit.x = hx;
        unit.y = hy;
        if drive.selector != tracks::FACTORY_EXIT {
            unit.set_facing(entry.facing);
        }
        if let Some(via) = drive.via {
            ground.release(via, unit.id);
        }
        unit.drive = None;
        return;
    }

    // Joining the next curve part-way, at the track's
    // jump point, is only for `Passive` vehicles (train carriages); everything else
    // runs each track to its end.
    unit.drive = Some(drive);
}

/// Moves a unit one point towards the cell its drive ends in, in a straight line.
/// Used only when the engine's track tables are not available.
fn step_straight(unit: &mut Unit, mut drive: Drive) {
    let (tx, ty) = cell_centre(drive.head);
    let (dx, dy) = (tx - unit.x, ty - unit.y);
    let point = if dx != 0 && dy != 0 { DIAGONAL_POINT } else { STRAIGHT_POINT };
    unit.x += dx.clamp(-point, point);
    unit.y += dy.clamp(-point, point);
    // A diagonal step passes through the corner the four cells share, which
    // counts as the cell below and right; it is stepped over.
    if unit.x & 255 == 0 && unit.y & 255 == 0 && (unit.x, unit.y) != (tx, ty) {
        unit.x += (tx - unit.x).signum();
        unit.y += (ty - unit.y).signum();
    }
    drive.index += 1;
    unit.drive = if (unit.x, unit.y) == (tx, ty) { None } else { Some(drive) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20x20 open field with a wall across x = 10 that has one gap at y = 15.
    fn field() -> Map {
        let mut map = Map::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                map.set_cell(x, y, x != 10 || y == 15, 0);
            }
        }
        map
    }

    #[test]
    fn a_cell_behind_high_ground_is_hidden() {
        let mut map = Map::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                map.set_cell(x, y, true, if x >= 10 && y >= 10 { 4 } else { 0 });
            }
        }
        // Flat ground and the top of the plateau are in view; the cell just
        // up and left of the plateau's corner is drawn over by it.
        assert_eq!(visible_cell(&map, (3, 3)), (3, 3));
        assert_eq!(visible_cell(&map, (12, 12)), (12, 12));
        assert_ne!(visible_cell(&map, (9, 9)), (9, 9));
    }

    #[test]
    fn an_armed_vehicle_attacks_an_enemy_in_its_way() {
        // The wall's one gap is held by an enemy; with no way round it the
        // armed tank takes it on rather than giving its move up.
        let mut world = World::new(field(), None);
        let armed = UnitStats {
            fighter: combat::Fighter { primary: Some(combat::Mount { weapon: 0, flh: (0, 0, 0) }), ..UnitStats::default().fighter },
            ..UnitStats::default()
        };
        let tank = world.add_unit((7, 15), 64, armed, 0);
        let enemy = world.add_unit((10, 15), 64, UnitStats::default(), 1);
        world.order(Order::Move { unit: tank, x: 14, y: 15 });
        let mut seen = false;
        for _ in 0..200 {
            world.tick();
            let unit = world.unit(tank).unwrap();
            seen |= unit.arms.target == Some(combat::Aim::Unit(enemy));
        }
        assert!(seen, "the tank never took the enemy on");
        assert_eq!(world.unit(tank).unwrap().dest, Some((14, 15)), "the move was given up");
    }

    #[test]
    fn a_soldier_sent_elsewhere_mid_step_goes_there() {
        let mut map = Map::new(40, 40);
        for y in 0..40 {
            for x in 0..40 {
                map.set_cell(x, y, true, 0);
            }
        }
        let mut world = World::new(map, None);
        let stats = UnitStats { infantry: true, speed: 4, ..UnitStats::default() };
        let soldier = world.add_infantry((5, 20), 2, 0, stats, 0).unwrap();
        world.order(Order::Move { unit: soldier, x: 35, y: 20 });
        for _ in 0..100 {
            world.tick();
        }
        let turned_at = world.unit(soldier).unwrap().cell().0;
        world.order(Order::Move { unit: soldier, x: 2, y: 20 });
        for _ in 0..200 {
            world.tick();
        }
        let now = world.unit(soldier).unwrap().cell().0;
        assert!(now < turned_at, "it walked on east, from x {turned_at} to {now}");
    }

    fn run(orders: &[(u64, Order)], ticks: u64) -> World {
        let mut world = World::new(field(), None);
        world.add_unit((2, 2), 0, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        world.add_unit((3, 17), 64, UnitStats::default(), 0);
        for tick in 0..ticks {
            for (_, order) in orders.iter().filter(|(at, _)| *at == tick) {
                world.order(*order);
            }
            world.tick();
        }
        world
    }

    #[test]
    fn unit_reaches_its_destination_through_the_gap() {
        let world = run(&[(0, Order::Move { unit: 0, x: 17, y: 3 })], 1500);
        let unit = world.unit(0).unwrap();
        assert_eq!(unit.cell(), (17, 3));
        assert!(!unit.is_moving());
    }

    #[test]
    fn unit_speeds_up_from_rest_and_turns_before_moving() {
        // Facing north, ordered east: it must turn a quarter circle first. At ROT 5
        // that is 16384 / 1280 = 12 ticks.
        let mut world = World::new(field(), None);
        let id = world.add_unit((2, 2), 0, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        world.order(Order::Move { unit: id, x: 8, y: 2 });
        for _ in 0..12 {
            world.tick();
        }
        assert_eq!(world.unit(id).unwrap().cell(), (2, 2));
        assert_eq!(world.unit(id).unwrap().x, 2 * LEPTONS_PER_CELL + 128);
        world.tick();
        assert_eq!(world.unit(id).unwrap().facing, 64);
        // Stored speed 15 at a fraction of 0.03 a tick: nothing moves until the
        // saved-up speed passes 7.
        let start = world.unit(id).unwrap().x;
        for _ in 0..3 {
            world.tick();
        }
        assert_eq!(world.unit(id).unwrap().x, start);
        for _ in 0..40 {
            world.tick();
        }
        assert!(world.unit(id).unwrap().x > start);
    }

    #[test]
    fn a_closed_destination_is_swapped_for_a_free_cell_beside_it() {
        // (10, 3) is in the wall.
        let world = run(&[(0, Order::Move { unit: 0, x: 10, y: 3 })], 1500);
        let unit = world.unit(0).unwrap();
        let (x, y) = unit.cell();
        assert!((x - 10).abs() <= 1 && (y - 3).abs() <= 1 && x != 10, "ended at {:?}", (x, y));
        assert!(!unit.is_moving());
    }

    #[test]
    fn a_destination_in_a_separate_area_with_nothing_near_is_refused() {
        // A lone open cell walled in, far from anything the unit can reach.
        let mut map = Map::new(80, 80);
        for y in 0..80 {
            for x in 0..80 {
                map.set_cell(x, y, x < 10 || (x, y) == (60, 60), 0);
            }
        }
        let mut world = World::new(map, None);
        let id = world.add_unit((2, 2), 0, UnitStats::default(), 0);
        world.order(Order::Move { unit: id, x: 60, y: 60 });
        for _ in 0..50 {
            world.tick();
        }
        assert_eq!(world.unit(id).unwrap().cell(), (2, 2));
        assert!(!world.unit(id).unwrap().is_moving());
    }

    #[test]
    fn same_orders_give_the_same_state_every_tick() {
        let orders = [
            (0, Order::Move { unit: 0, x: 17, y: 3 }),
            (5, Order::Move { unit: 1, x: 15, y: 1 }),
            (40, Order::Move { unit: 0, x: 1, y: 18 }),
        ];
        for ticks in [1, 37, 200, 10_000] {
            assert_eq!(run(&orders, ticks).hash(), run(&orders, ticks).hash());
        }
    }

    #[test]
    fn different_orders_give_different_state() {
        let a = run(&[(0, Order::Move { unit: 0, x: 17, y: 3 })], 30);
        let b = run(&[(0, Order::Move { unit: 0, x: 3, y: 17 })], 30);
        assert_ne!(a.hash(), b.hash());
    }

    /// The engine's own track tables, if the game's executable can be found.
    fn engine_tracks() -> Option<TrackTables> {
        let path = std::env::var("RA2_GAME_EXE")
            .unwrap_or_else(|_| "../../app_2229850/depot_2229851/game.exe".into());
        TrackTables::from_exe(&std::fs::read(path).ok()?)
    }

    #[test]
    fn tracks_carry_a_unit_round_corners_to_exact_cell_centres() {
        let Some(tracks) = engine_tracks() else {
            eprintln!("game.exe not found; skipping");
            return;
        };
        let mut open = Map::new(30, 30);
        for y in 0..30 {
            for x in 0..30 {
                open.set_cell(x, y, true, 0);
            }
        }
        let mut world = World::new(open, Some(tracks));
        let id = world.add_unit((5, 20), 0, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        // North, then a route with a bend in it.
        world.order(Order::Move { unit: id, x: 12, y: 8 });
        let mut furthest_jump = 0;
        let mut last = (world.unit(id).unwrap().x, world.unit(id).unwrap().y);
        for _ in 0..3000 {
            world.tick();
            let unit = world.unit(id).unwrap();
            furthest_jump = furthest_jump.max((unit.x - last.0).abs().max((unit.y - last.1).abs()));
            last = (unit.x, unit.y);
        }
        let unit = world.unit(id).unwrap();
        assert!(!unit.is_moving());
        assert_eq!((unit.x, unit.y), cell_centre((12, 8)));
        // Never more than a few track points in one tick: no teleporting between
        // tracks.
        assert!(furthest_jump < 64, "moved {furthest_jump} leptons in one tick");
    }

    #[test]
    fn straight_track_points_face_and_sit_where_the_selector_says() {
        let Some(tracks) = engine_tracks() else {
            return;
        };
        // Heading east: 245 leptons west of the destination, facing east.
        let east = tracks.selector[2 * 8 + 2];
        assert_eq!(east.place(tracks.tracks[east.track as usize].points[0]), (-245, 0, 64));
        // Heading south-west: north-east of the destination, facing south-west.
        let south_west = tracks.selector[5 * 8 + 5];
        assert_eq!(
            south_west.place(tracks.tracks[south_west.track as usize].points[0]),
            (248, -248, 160)
        );
    }

    fn open_field(size: i32) -> Map {
        let mut map = Map::new(size, size);
        for y in 0..size {
            for x in 0..size {
                map.set_cell(x, y, true, 0);
            }
        }
        map
    }

    #[test]
    fn two_units_never_share_a_cell() {
        // Head-on along a corridor one cell wide, with one passing place.
        for tracks in [None, engine_tracks()] {
        let mut map = Map::new(20, 5);
        for x in 0..20 {
            map.set_cell(x, 2, true, 0);
        }
        map.set_cell(10, 1, true, 0);
        let mut world = World::new(map, tracks);
        let a = world.add_unit((2, 2), 64, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        let b = world.add_unit((17, 2), 192, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        world.order(Order::Move { unit: a, x: 17, y: 2 });
        world.order(Order::Move { unit: b, x: 10, y: 1 });
        for _ in 0..4000 {
            world.tick();
            assert_ne!(world.unit(a).unwrap().cell(), world.unit(b).unwrap().cell());
        }
        // B pulls into the passing place; A gets through to where B started.
        assert_eq!(world.unit(b).unwrap().cell(), (10, 1));
        assert_eq!(world.unit(a).unwrap().cell(), (17, 2));
        }
    }

    #[test]
    fn a_blocked_unit_paths_round_a_parked_one() {
        for tracks in [None, engine_tracks()] {
        let mut world = World::new(open_field(12), tracks);
        let mover = world.add_unit((1, 5), 64, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        let parked = world.add_unit((5, 5), 0, UnitStats::default(), 0);
        world.order(Order::Move { unit: mover, x: 10, y: 5 });
        for _ in 0..3000 {
            world.tick();
            assert_ne!(world.unit(mover).unwrap().cell(), world.unit(parked).unwrap().cell());
        }
        assert_eq!(world.unit(mover).unwrap().cell(), (10, 5));
        assert_eq!(world.unit(parked).unwrap().cell(), (5, 5));
        }
    }

    #[test]
    fn ground_a_speed_type_cannot_cross_is_routed_round_and_slow_ground_is_slower() {
        let mut map = open_field(12);
        let mut lands = LandRules::open();
        lands.0[Land::Water as usize][SpeedType::Track as usize] = 0;
        lands.0[Land::Tiberium as usize][SpeedType::Track as usize] = 50;
        map.lands = lands;
        for y in 0..11 {
            map.set_land(6, y, Land::Water);
        }
        let mut world = World::new(map, None);
        let id = world.add_unit((2, 2), 64, UnitStats { speed: 6, accelerates: false, ..UnitStats::default() }, 0);
        world.order(Order::Move { unit: id, x: 10, y: 2 });
        let mut crossed_water = false;
        for _ in 0..3000 {
            world.tick();
            crossed_water |= world.unit(id).unwrap().cell().0 == 6 && world.unit(id).unwrap().cell().1 < 11;
        }
        assert!(!crossed_water);
        assert_eq!(world.unit(id).unwrap().cell(), (10, 2));

        // The same straight run takes about twice as long over half-speed ground.
        let ticks_to_cross = |land: Land| {
            let mut map = open_field(12);
            let mut lands = LandRules::open();
            lands.0[Land::Tiberium as usize][SpeedType::Track as usize] = 50;
            map.lands = lands;
            for x in 0..12 {
                map.set_land(x, 3, land);
            }
            let mut world = World::new(map, None);
            let id = world.add_unit((1, 3), 64, UnitStats { speed: 6, accelerates: false, ..UnitStats::default() }, 0);
            world.order(Order::Move { unit: id, x: 10, y: 3 });
            (1..5000).find(|_| {
                world.tick();
                !world.unit(id).unwrap().is_moving()
            })
        };
        let (fast, slow) = (ticks_to_cross(Land::Clear).unwrap(), ticks_to_cross(Land::Tiberium).unwrap());
        assert!(slow > fast * 3 / 2, "clear {fast} ticks, slow ground {slow} ticks");
    }

    /// An open field with a construction yard type (0) and a power plant type (1)
    /// that needs it, and a house 1 with 1000 credits.
    fn base_world() -> World {
        let mut world = World::new(open_field(20), None);
        world.building_types = vec![
            BuildingType { cost: 3000, size: (4, 4), adjacent: 2, base_normal: true, owners: 1, tech_level: -1, factory: Some(Factory::Building), ..BuildingType::default() },
            BuildingType {
                cost: 800,
                power: 200,
                size: (2, 2),
                prerequisites: vec![vec![0]],
                adjacent: 2,
                tech_level: 1,
                owners: 1,
                base_normal: true,
                strength: 750,
                ..BuildingType::default()
            },
        ];
        world.houses = vec![House::default(), House { credits: 1000, tech_level: 10, human: true, ..House::default() }];
        world
    }

    #[test]
    fn a_building_is_produced_paid_for_and_placed_next_to_the_base() {
        let mut world = base_world();
        // Nothing can be built without a construction yard.
        assert!(world.buildable(1).is_empty());
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), DEPLOY_FACING, stats, 1);
        world.order(Order::Deploy { unit: mcv });
        world.tick();
        assert_eq!(world.buildable(1), vec![1]);

        world.order(Order::Build { house: 1, building: 1 });
        // 800 x 0.7 x 0.9 = 504 ticks: 54 stages of 9.
        assert_eq!(world.time_to_build(1, 1), 504);
        for _ in 0..54 * 9 - 1 {
            world.tick();
        }
        assert!(!world.houses[1].production.unwrap().is_ready());
        world.tick();
        assert!(world.houses[1].production.unwrap().is_ready());
        assert_eq!(world.houses[1].credits, 200);

        // Too far from the base, then on top of it: both refused.
        world.order(Order::Place { house: 1, building: 1, x: 15, y: 15 });
        world.order(Order::Place { house: 1, building: 1, x: 5, y: 5 });
        assert_eq!(world.buildings().len(), 1);
        // Beside it: accepted, and its power counts.
        world.order(Order::Place { house: 1, building: 1, x: 9, y: 5 });
        assert_eq!(world.buildings().len(), 2);
        assert_eq!(world.power(1), (200, 0));
        assert!(world.houses[1].production.is_none());
    }

    #[test]
    fn production_waits_for_money_and_cancelling_refunds_it() {
        let mut world = base_world();
        world.houses[1].credits = 100;
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), DEPLOY_FACING, stats, 1);
        world.order(Order::Deploy { unit: mcv });
        world.tick();
        world.order(Order::Build { house: 1, building: 1 });
        for _ in 0..5000 {
            world.tick();
        }
        let production = world.houses[1].production.unwrap();
        assert!(!production.is_ready() && production.stalled);
        assert!(world.houses[1].credits < 15);
        world.order(Order::CancelBuild { house: 1, building: 1 });
        assert_eq!(world.houses[1].credits, 100);
        assert!(world.houses[1].production.is_none());
    }

    #[test]
    fn production_goes_on_hold_and_resumes_where_it_left_off() {
        let mut world = base_world();
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), DEPLOY_FACING, stats, 1);
        world.order(Order::Deploy { unit: mcv });
        world.tick();
        world.order(Order::Build { house: 1, building: 1 });
        for _ in 0..100 {
            world.tick();
        }
        world.order(Order::Suspend { house: 1, building: 1 });
        let held = world.houses[1].production.unwrap();
        assert!(held.on_hold() && held.stage == 11);
        let credits = world.houses[1].credits;
        for _ in 0..500 {
            world.tick();
        }
        assert_eq!(world.houses[1].production.unwrap().stage, 11);
        assert_eq!(world.houses[1].credits, credits);
        world.order(Order::Build { house: 1, building: 1 });
        assert!(!world.houses[1].production.unwrap().on_hold());
        for _ in 0..43 * 9 {
            world.tick();
        }
        assert!(world.houses[1].production.unwrap().is_ready());
        assert_eq!(world.houses[1].credits, 200);
    }

    #[test]
    fn a_unit_deploys_into_a_building_that_blocks_its_cells() {
        let mut world = base_world();
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), 0, stats, 1);
        let other = world.add_unit((1, 5), 64, UnitStats { speed: 6, ..UnitStats::default() }, 1);
        assert!(world.can_deploy(mcv));
        world.order(Order::Deploy { unit: mcv });
        // It turns to face south first: half a circle at ROT 5 is 25 ticks.
        for _ in 0..24 {
            world.tick();
        }
        assert!(world.unit(mcv).is_some());
        for _ in 0..3 {
            world.tick();
        }
        assert!(world.unit(mcv).is_none());
        let building = &world.buildings()[0];
        assert_eq!((building.cell, building.size, building.owner), ((4, 4), (4, 4), 1));

        // The other unit has to go round it.
        world.order(Order::Move { unit: other, x: 10, y: 5 });
        for _ in 0..3000 {
            world.tick();
            // A diagonal step may pass exactly over a corner of the foundation, as
            // in the engine, which has no rule against cutting corners.
            let unit = world.unit(other).unwrap();
            let inside = |v: i32| v > 4 * LEPTONS_PER_CELL && v < 8 * LEPTONS_PER_CELL;
            assert!(!(inside(unit.x) && inside(unit.y)), "drove through the building");
        }
        assert_eq!(world.unit(other).unwrap().cell(), (10, 5));
    }

    #[test]
    fn a_unit_cannot_deploy_onto_something_in_the_way() {
        let mut world = base_world();
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), 0, stats, 1);
        world.add_unit((6, 6), 0, UnitStats::default(), 0);
        assert!(!world.can_deploy(mcv));
        world.order(Order::Deploy { unit: mcv });
        assert!(world.unit(mcv).is_some());
        assert!(world.buildings().is_empty());
    }
    #[test]
    fn a_unit_can_deploy_after_it_has_moved() {
        for tracks in [None, engine_tracks()] {
            let mut world = World::new(open_field(30), tracks);
            world.building_types = base_world().building_types;
            world.houses = base_world().houses;
            let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
            let mcv = world.add_unit((5, 5), 0, stats, 1);
            for goal in [(12, 9), (20, 9), (8, 20)] {
                world.order(Order::Move { unit: mcv, x: goal.0, y: goal.1 });
                for _ in 0..3000 {
                    world.tick();
                }
                let unit = world.unit(mcv).unwrap();
                assert_eq!(unit.cell(), goal);
                let held: Vec<usize> = (0..world.claims.len()).filter(|&i| world.claims[i] != 0).collect();
                assert!(world.can_deploy(mcv), "at {goal:?}: moving {} held {held:?} unit {unit:?}", unit.is_moving());
            }
        }
    }
    #[test]
    fn a_building_packs_up_into_its_vehicle_and_drives_off() {
        let mut world = base_world();
        let stats = UnitStats { deploys_into: Some(0), strength: 1000, ..UnitStats::default() };
        world.building_types[0].undeploys_into = Some(stats);
        world.building_types[0].construction_yard = true;
        world.building_types[0].strength = 1000;
        let mcv = world.add_unit((5, 5), DEPLOY_FACING, stats, 1);
        world.order(Order::Deploy { unit: mcv });
        world.tick();
        let yard = world.buildings()[0].id;
        world.mcv_repacks = false;
        world.order(Order::Undeploy { building: yard, x: 12, y: 12 });
        assert_eq!(world.buildings().len(), 1);
        world.mcv_repacks = true;
        world.order(Order::Undeploy { building: yard, x: 12, y: 12 });
        assert!(world.buildings().is_empty());
        let unit = world.units()[0].clone();
        assert_eq!((unit.cell(), unit.facing, unit.owner), ((5, 5), DEPLOY_FACING, 1));
        for _ in 0..3000 {
            world.tick();
        }
        let unit = &world.units()[0];
        assert_eq!(unit.cell(), (12, 12));
        assert!(world.can_deploy(unit.id));
    }
    #[test]
    fn a_route_keeps_to_a_corridor_of_subzones() {
        let world = World::new(field(), None);
        let zones = world.map.zones().unwrap();
        // One area either side of the wall's gap, joined through it.
        assert_eq!(zones.zone_of((2, 2), 0), zones.zone_of((17, 3), 0));
        assert_eq!(zones.zone_of((10, 3), 0), 1);
        let corridor = zones.corridor((2, 2), (17, 3), 0).unwrap();
        let on = corridor.iter().filter(|on| **on).count();
        assert!(on > 8 && on < corridor.len() / 2, "{on} of {} subzones", corridor.len());
        // The gap is on it; the far corner is not.
        assert!(corridor[zones.subzone((10, 15)) as usize]);
        assert!(!corridor[zones.subzone((1, 19)) as usize] || !corridor[zones.subzone((19, 19)) as usize]);
    }
    #[test]
    fn walls_join_their_neighbours_and_fill_lines_between_pieces() {
        let mut world = base_world();
        world.building_types.push(BuildingType {
            cost: 100,
            size: (1, 1),
            adjacent: 8,
            tech_level: 1,
            owners: 1,
            combat: true,
            wall_overlay: Some(2),
            guard_range: 5,
            ..BuildingType::default()
        });
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), DEPLOY_FACING, stats, 1);
        world.order(Order::Deploy { unit: mcv });
        world.tick();
        let wall = |world: &World, cell: (i32, i32)| world.map.walls().find(|(at, ..)| *at == cell).map(|(_, _, data)| data);
        assert!(world.place_now(1, 2, (10, 4)));
        assert_eq!(wall(&world, (10, 4)), Some(0));
        // Beside it: the two join (the new one has a piece at (-1, 0), bit 8).
        assert!(world.place_now(1, 2, (11, 4)));
        assert_eq!((wall(&world, (10, 4)), wall(&world, (11, 4))), (Some(2), Some(8)));
        // Four cells down the column: the three between are filled.
        assert_eq!(world.wall_line(1, 2, (10, 8)), vec![(10, 7), (10, 6), (10, 5)]);
        assert!(world.place_now(1, 2, (10, 8)));
        assert_eq!(wall(&world, (10, 6)), Some(1 | 4));
        assert_eq!(world.map.walls().count(), 6);
        // Too far for a line (more than five cells), and on top of a wall: refused.
        assert!(world.wall_line(1, 2, (10, 14)).is_empty());
        assert!(!world.place_now(1, 2, (10, 6)));
        // A vehicle cannot drive through, and the building tab sees no building.
        assert_eq!(world.buildings().len(), 1);
        assert!(!world.map.passable(10, 6));
    }
    #[test]
    fn a_war_factory_turns_out_vehicles_one_after_another() {
        let mut world = base_world();
        world.houses[1].credits = 5000;
        // Type 2: a war factory needing the yard. Unit type 0: a tank needing it.
        world.building_types.push(BuildingType {
            size: (3, 3),
            adjacent: 2,
            tech_level: 1,
            owners: 1,
            base_normal: true,
            prerequisites: vec![vec![0]],
            factory: Some(Factory::Unit),
            strength: 1000,
            ..BuildingType::default()
        });
        world.unit_types.push(UnitType {
            stats: UnitStats { speed: 6, ..UnitStats::default() },
            cost: 700,
            tech_level: 1,
            owners: 1,
            prerequisites: vec![vec![2]],
            ..UnitType::default()
        });
        let stats = UnitStats { deploys_into: Some(0), ..UnitStats::default() };
        let mcv = world.add_unit((5, 5), DEPLOY_FACING, stats, 1);
        world.order(Order::Deploy { unit: mcv });
        world.tick();
        // No factory, no tanks.
        assert!(world.buildable_units(1).is_empty());
        assert!(world.place_now(1, 2, (9, 4)));
        assert_eq!(world.buildable_units(1), vec![0]);
        for _ in 0..3 {
            world.order(Order::BuildUnit { house: 1, unit: 0 });
        }
        assert_eq!(world.units_in_hand(1, 0), 3);
        // Right click takes a queued one away first.
        world.order(Order::CancelUnit { house: 1, unit: 0 });
        assert_eq!(world.units_in_hand(1, 0), 2);
        for _ in 0..2000 {
            world.tick();
        }
        assert_eq!(world.units().len(), 2);
        assert_eq!(world.units_in_hand(1, 0), 0);
        assert_eq!(world.houses[1].credits, 5000 - 1400);
        // They stand on different cells, outside the factory.
        let cells: Vec<_> = world.units().iter().map(Unit::cell).collect();
        assert_ne!(cells[0], cells[1]);
        assert!(cells.iter().all(|&(x, y)| !((9..12).contains(&x) && (4..7).contains(&y))));
    }
    #[test]
    fn foot_soldiers_walk_spot_to_spot_and_share_a_cell_three_at_most() {
        let mut world = World::new(open_field(20), None);
        let stats = UnitStats { speed: 4, movement_zone: 7, speed_type: SpeedType::Foot, ..UnitStats::default() };
        // Three on one cell, each on a spot of its own; a fourth is refused.
        let squad: Vec<u32> = (0..3).map(|_| world.add_infantry((3, 3), 0, 0, stats, 1).unwrap()).collect();
        assert!(world.add_infantry((3, 3), 0, 0, stats, 1).is_none());
        let spots: Vec<(i32, i32)> = squad.iter().map(|&id| (world.unit(id).unwrap().x & 255, world.unit(id).unwrap().y & 255)).collect();
        assert_eq!(spots, vec![(192, 64), (64, 192), (192, 192)]);
        let extra = world.add_infantry((3, 5), 2, 0, stats, 1).unwrap();
        // Stored speed for `Speed=4` is 10 leptons a tick: a cell east takes 26
        // ticks of walking and two standing.
        world.order(Order::Move { unit: squad[0], x: 4, y: 3 });
        let mut ticks = 0;
        while world.unit(squad[0]).unwrap().is_moving() {
            world.tick();
            ticks += 1;
            assert!(ticks < 100);
        }
        let walker = world.unit(squad[0]).unwrap();
        // The engine's facing sum comes one short of due east.
        assert_eq!((walker.cell(), walker.x & 255, walker.y & 255, walker.facing), ((4, 3), 192, 64, 63));
        assert!((25..=31).contains(&ticks), "took {ticks} ticks");
        // All four sent to one cell: three get in, the fourth stops beside it.
        for &id in squad.iter().chain([&extra]) {
            world.order(Order::Move { unit: id, x: 12, y: 9 });
        }
        for _ in 0..1500 {
            world.tick();
        }
        let cells: Vec<(i32, i32)> = squad.iter().chain([&extra]).map(|&id| world.unit(id).unwrap().cell()).collect();
        assert_eq!(cells.iter().filter(|&&cell| cell == (12, 9)).count(), 3, "{cells:?}");
        assert!(cells.iter().all(|&(x, y)| (x - 12).abs() <= 2 && (y - 9).abs() <= 2), "{cells:?}");
        assert!(squad.iter().chain([&extra]).all(|&id| !world.unit(id).unwrap().is_moving()));
        // No two stand on the same spot.
        let mut places: Vec<(i32, i32)> = squad.iter().chain([&extra]).map(|&id| (world.unit(id).unwrap().x, world.unit(id).unwrap().y)).collect();
        places.sort_unstable();
        places.dedup();
        assert_eq!(places.len(), 4);
    }
}

/// The step from a cell of the sight circle back towards its middle: the
/// offset scaled to its longer side and rounded, negated.
// The engine keeps these in a table; one entry, for (5, -2), steps away from
// the middle instead.
fn sight_step(dx: i32, dy: i32) -> (i32, i32) {
    if (dx, dy) == (5, -2) {
        return (1, 0);
    }
    let longer = dx.abs().max(dy.abs());
    if longer == 0 {
        return (0, 0);
    }
    let round = |v: i32| -((v.abs() * 2 + longer) / (2 * longer)) * v.signum();
    (round(dx), round(dy))
}

#[cfg(test)]
mod sight_step_tests {
    #[test]
    fn steps_lead_back_towards_the_middle() {
        assert_eq!(super::sight_step(1, -1), (-1, 1));
        assert_eq!(super::sight_step(-1, -2), (1, 1));
        assert_eq!(super::sight_step(-1, -3), (0, 1));
        assert_eq!(super::sight_step(3, 1), (-1, 0));
        assert_eq!(super::sight_step(0, 0), (0, 0));
    }
}
