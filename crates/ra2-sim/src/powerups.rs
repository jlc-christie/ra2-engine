//! Crates in skirmish: how many, where, when they move, who picks them up,
//! and what each holds.
//!
//! Follows the original game

use crate::combat::Event;
use crate::{cell_centre, Land, Near, Unit, UnitStats, World};

/// What a crate can hold, in the engine's order.
pub const NAMES: [&str; 19] = [
    "Money", "Unit", "HealBase", "Cloak", "Explosion", "Napalm", "Squad", "Darkness", "Reveal", "Armor", "Speed", "Firepower", "ICBM",
    "Invulnerability", "Veteran", "IonStorm", "Gas", "Tiberium", "Pod",
];
pub const MONEY: u8 = 0;
pub const UNIT: u8 = 1;
pub const HEAL_BASE: u8 = 2;
pub const CLOAK: u8 = 3;
pub const SQUAD: u8 = 6;
pub const REVEAL: u8 = 8;
pub const ARMOR: u8 = 9;
pub const SPEED: u8 = 10;
pub const FIREPOWER: u8 = 11;
pub const VETERAN: u8 = 14;

/// One `[Powerups]` entry: its share of the draw, its animation's name, whether
/// a crate on water may hold it, and its argument.
#[derive(Clone, Debug, Default)]
pub struct Powerup {
    pub share: i32,
    pub anim: Option<String>,
    pub water: bool,
    pub argument: f64,
}

/// The rules for crates.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    pub powerups: Vec<Powerup>,
    /// `CrateMinimum`, `CrateMaximum`.
    pub minimum: i32,
    pub maximum: i32,
    /// `CrateRadius`, in leptons.
    pub radius: i32,
    /// `CrateRegen`, in minutes.
    pub regen: f64,
    /// `UnitCrateType`, as a unit type.
    pub unit_type: Option<u16>,
    /// The map's `[Basic] TruckCrate`: `CarriesCrate` vehicles leave crates.
    pub truck_crate: bool,
    /// `[General] BaseUnit`, as unit types.
    pub base_units: Vec<u16>,
    /// Unit types with `CrateGoodie=yes`.
    pub goodies: Vec<u16>,
}

/// A crate on the map.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Crate {
    pub cell: (i32, i32),
    /// On water (the `WaterCrateImg` picture), else on land.
    pub water: bool,
    /// What it holds: a powerup, or `None` to draw when it is picked up.
    pub content: Option<u8>,
    /// The tick it moves elsewhere if nobody has picked it up.
    pub moves_at: u64,
}

/// The engine keeps 256 crate slots.
const SLOTS: usize = 256;

impl World {
    /// Puts the match's crates down at the start.
    // With "Crates Appear", the larger of
    // `CrateMinimum` and `players`, but no more than `CrateMaximum`.
    // `players` is the session's count, which only the network code sets, to the number of people
    // playing: 0 in skirmish, so one crate with the stock rules.
    pub fn start_crates(&mut self, players: i32) {
        let rules = &self.crate_rules;
        let n = players.max(rules.minimum).min(rules.maximum);
        self.crates_on = true;
        for _ in 0..n {
            self.place_random_crate();
        }
    }

    /// The crates on the map.
    pub fn crates(&self) -> &[Crate] {
        &self.crates
    }

    /// Destroys a building, as if shot, for tests.
    #[doc(hidden)]
    pub fn kill_building_for_tests(&mut self, building: u32) {
        self.kill(crate::combat::Aim::Building(building));
    }

    /// A crate with chosen contents at a cell, for tests.
    #[doc(hidden)]
    pub fn put_crate_for_tests(&mut self, cell: (i32, i32), content: u8) {
        self.crates_on = true;
        let water = self.map.index(cell.0, cell.1).is_some_and(|i| self.map.land[i] == Land::Water as u8);
        self.crates.push(Crate { cell, water, content: ((content as usize) < NAMES.len()).then_some(content), moves_at: u64::MAX });
    }

    /// A crate at a random place.
    // A free slot; up to 1000 tries of a random cell (x drawn first) in
    // the square (1, 1)-(W+H-1, W+H-1) round the whole diamond map (W and H
    // the map's `Size`), so many tries fall off the map; moved to a free cell
    // nearby (by water rules on water); it must be in the playable area with
    // no overlay. It moves on after 450 R to 1800 R frames, R = `CrateRegen`.
    fn place_random_crate(&mut self) -> bool {
        if self.crates.len() >= SLOTS {
            return false;
        }
        let (width, height) = self.map.size();
        let side = self.map.diamond_side();
        for _ in 0..1000 {
            let (x, y) = match side {
                Some(side) => (1 + self.random_below((side - 1).max(1) as u32) as i32, 1 + self.random_below((side - 1).max(1) as u32) as i32),
                None => (self.random_below(width.max(1) as u32) as i32, self.random_below(height.max(1) as u32) as i32),
            };
            if self.place_crate_near((x, y), None) {
                return true;
            }
        }
        false
    }

    /// The crates a destroyed thing leaves.
    // A `CarriesCrate` vehicle, when the map's `[Basic] TruckCrate` is set
    // and crates are on, leaves one of any kind near where it died.
    pub(crate) fn crate_from_unit(&mut self, kind: u16, cell: (i32, i32)) {
        let carries = self.unit_types.get((kind as usize).wrapping_sub(1)).is_some_and(|kind| kind.carries_crate);
        if carries && self.crate_rules.truck_crate && self.crates_on {
            self.place_crate_near(cell, None);
        }
    }

    // A `CrateBeneath`
    // building destroyed (not sold) leaves one at its centre, money if
    // `CrateBeneathIsMoney`, whether or not crates are on.
    pub(crate) fn crate_from_building(&mut self, kind: u16, centre: (i32, i32)) {
        if let Some((beneath, money)) = self.building_types.get(kind as usize).map(|kind| (kind.crate_beneath, kind.crate_beneath_money))
            && beneath {
                self.place_crate_near(centre, money.then_some(MONEY));
            }
    }

    /// A crate at or near a cell, holding something chosen or not.
    fn place_crate_near(&mut self, cell: (i32, i32), content: Option<u8>) -> bool {
        let Some(index) = self.map.index(cell.0, cell.1) else {
            return false;
        };
        let water = self.map.land[index] == Land::Water as u8;
        let stats = UnitStats { speed_type: if water { crate::SpeedType::Float } else { crate::SpeedType::Track }, ..UnitStats::default() };
        let probe = Unit::new(u32::MAX - 2, 0, cell, 0, stats);
        let Some(at) = crate::nearby(&self.map, &self.claims, &self.spots, &probe, cell, Near::new(true, false, None), self.tick) else {
            return false;
        };
        let Some(index) = self.map.index(at.0, at.1) else {
            return false;
        };
        if !self.map.in_playfield(at.0, at.1) || self.map.has_overlay(at.0, at.1) || self.crates.iter().any(|c| c.cell == at) || self.building_on(at).is_some() {
            return false;
        }
        let water = self.map.land[index] == Land::Water as u8;
        let regen = self.crate_rules.regen;
        let span = (1800.0 * regen - 450.0 * regen).max(0.0);
        let r = self.random_below(0x7fff_ffff) as f64 / 2147483648.0;
        let life = (450.0 * regen + r * span) as u64;
        self.crates.push(Crate { cell: at, water, content, moves_at: self.tick + life });
        true
    }

    /// Crates nobody picked up move elsewhere.
    // Each frame, every crate whose time has run
    // out goes, and one is put down at random in its place.
    pub(crate) fn run_crates(&mut self) {
        if !self.crates_on {
            return;
        }
        let tick = self.tick;
        let due = self.crates.iter().filter(|c| tick >= c.moves_at).count();
        if due == 0 {
            return;
        }
        self.crates.retain(|c| tick < c.moves_at);
        for _ in 0..due {
            self.place_random_crate();
        }
    }

    /// A unit coming into a cell picks up the crate there.
    // Soldiers,
    // vehicles and ships, as they set off into the cell (the walking, driving and
    // sailing code calls it; flying and jumping do not); not one of a passive
    // house. The content is drawn by the shares if it was not set; a house with
    // no buildings, no construction vehicle and more than 1500 credits gets one
    // (bases are always on in skirmish); what would be no use to the one who
    // picked it up is paid as money. A new crate goes down at random at once.
    // The unit is setting off into `cell` (see the movement code).
    pub(crate) fn collect_crate(&mut self, index: usize, cell: (i32, i32)) {
        let (owner, id) = {
            let unit = &self.units[index];
            (unit.owner, unit.id)
        };
        let Some(at) = self.crates.iter().position(|c| c.cell == cell) else {
            return;
        };
        if self.units[index].jet.is_some() || self.units[index].plane.is_some() || self.units[index].health <= 0 {
            return;
        }
        // The neutral house's things drive over crates.
        if owner == 0 {
            return;
        }
        let found = self.crates.remove(at);
        let shares: Vec<i32> = self.crate_rules.powerups.iter().map(|p| p.share.max(0)).collect();
        let mut kind = match found.content {
            Some(kind) if (kind as usize) < NAMES.len() => kind,
            _ => {
                let total: i32 = shares.iter().sum();
                let mut pick = MONEY;
                if total > 0 {
                    let r = 1 + self.random_below(total as u32) as i32;
                    let mut sum = 0;
                    for (i, share) in shares.iter().enumerate() {
                        sum += share;
                        if r <= sum {
                            pick = i as u8;
                            break;
                        }
                    }
                }
                pick
            }
        };
        let house = owner as usize;
        let buildings = self.buildings.iter().filter(|b| b.owner == owner).count();
        let has_base_unit = self.units.iter().any(|u| u.owner == owner && self.crate_rules.base_units.contains(&(u.stats.kind.saturating_sub(1))));
        let credits = self.houses.get(house).map_or(0, |h| h.credits);
        let force_mcv = buildings == 0 && credits > 1500 && !has_base_unit;
        if force_mcv {
            kind = UNIT;
        }
        let land = self.map.index(cell.0, cell.1).map_or(0, |i| self.map.land[i]);
        let wet = land == Land::Water as u8 || land == Land::Beach as u8;
        let unit = &self.units[index];
        let useless = match kind {
            UNIT => self.units.iter().filter(|u| u.owner == owner).count() > 50 || wet,
            SQUAD => wet,
            ARMOR => unit.crate_armour != 1.0,
            SPEED => unit.crate_speed != 1.0,
            FIREPOWER => unit.crate_firepower != 1.0 || unit.stats.fighter.primary.is_none(),
            VETERAN => !unit.stats.trainable || unit.rank() == 0,
            _ => false,
        };
        if useless {
            kind = MONEY;
        }
        if land == Land::Water as u8 && !self.crate_rules.powerups.get(kind as usize).is_some_and(|p| p.water) {
            kind = MONEY;
        }
        // A replacement goes down at once.
        self.place_random_crate();
        if kind == SQUAD {
            kind = MONEY;
        }
        let argument = self.crate_rules.powerups.get(kind as usize).map_or(0.0, |p| p.argument);
        let centre = cell_centre(cell);
        let radius = self.crate_rules.radius as i64;
        let near: Vec<usize> = (0..self.units.len())
            .filter(|&i| {
                let u = &self.units[i];
                let (dx, dy, dz) = ((u.x - centre.0) as i64, (u.y - centre.1) as i64, self.altitude(u) as i64);
                u.health > 0 && dx * dx + dy * dy + dz * dz < radius * radius
            })
            .collect();
        let shown = match kind {
            UNIT if self.crate_unit(index, force_mcv, cell) => UNIT,
            UNIT => {
                // A unit that cannot be put down
                // pays as Money with the Unit entry's argument (Oddity 9).
                self.pay_crate(owner, argument);
                MONEY
            }
            HEAL_BASE => {
                // Everything of the house back to
                // full strength.
                for u in self.units.iter_mut().filter(|u| u.owner == owner && u.health > 0) {
                    u.health = u.stats.strength;
                }
                let strengths: Vec<i32> = self.buildings.iter().map(|b| self.building_types.get(b.kind as usize).map_or(b.health, |t| t.strength)).collect();
                for (b, strength) in self.buildings.iter_mut().zip(strengths) {
                    if b.owner == owner && b.health > 0 {
                        b.health = strength;
                    }
                }
                HEAL_BASE
            }
            REVEAL => {
                // Only a person's house sees the map.
                if self.houses.get(house).is_some_and(|h| h.human) {
                    self.map.reveal_all(owner, true);
                }
                REVEAL
            }
            ARMOR | SPEED | FIREPOWER => {
                // Everything in
                // the radius, whoever's, not yet bettered that way.
                for i in near {
                    let u = &mut self.units[i];
                    match kind {
                        ARMOR if u.crate_armour == 1.0 => u.crate_armour = argument as f32,
                        FIREPOWER if u.crate_firepower == 1.0 => u.crate_firepower = argument as f32,
                        SPEED if u.crate_speed == 1.0 && u.plane.is_none() => {
                            u.crate_speed = argument as f32;
                            u.speed = (u.speed as f64 * argument) as i32;
                        }
                        _ => {}
                    }
                }
                kind
            }
            VETERAN => {
                // Everything trainable in the
                // radius goes up a rank, `argument` times.
                for i in near {
                    let u = &mut self.units[i];
                    if !u.stats.trainable {
                        continue;
                    }
                    for _ in 0..(argument as i32).max(1) {
                        u.veterancy = if u.veterancy >= 1.0 { 2.0 } else { 1.0 };
                    }
                }
                VETERAN
            }
            _ => {
                self.pay_crate(owner, self.crate_rules.powerups.get(MONEY as usize).map_or(2000.0, |p| p.argument));
                MONEY
            }
        };
        self.events.push(Event::CrateCollected { unit: id, house: owner, cell, kind: shown });
    }

    /// Money from a crate: its argument and up to 900 more.
    // `(arg, arg + 900)`.
    fn pay_crate(&mut self, owner: u8, argument: f64) {
        let low = argument as i32;
        let amount = low + self.random_below(901) as i32;
        if let Some(house) = self.houses.get_mut(owner as usize) {
            house.credits += amount;
        }
    }

    /// A unit from a crate, at the crate's cell or near it. Returns whether
    /// one was put down.
    // A construction vehicle when forced; else a miner for a house with a
    // refinery and none; `UnitCrateType` over both; else any `CrateGoodie`
    // type at random, a computer's never a base unit.
    fn crate_unit(&mut self, index: usize, force_mcv: bool, cell: (i32, i32)) -> bool {
        let owner = self.units[index].owner;
        let country = self.houses.get(owner as usize).map_or(0, |h| h.country);
        let human = self.houses.get(owner as usize).is_some_and(|h| h.human);
        let mine = 1u32.checked_shl(country as u32).unwrap_or(0);
        let of_side = |world: &World, list: &[u16]| list.iter().copied().find(|&kind| world.unit_types.get(kind as usize).is_some_and(|t| t.owners & mine != 0)).or(list.first().copied());
        let mut kind = None;
        if force_mcv {
            kind = of_side(self, &self.crate_rules.base_units.clone());
        }
        let harvesters = self.ai_rules.harvesters.clone();
        let refineries = self.ai_rules.build_refinery.clone();
        if kind.is_none() && self.buildings.iter().any(|b| b.owner == owner && refineries.contains(&b.kind)) && !self.units.iter().any(|u| u.owner == owner && harvesters.contains(&u.stats.kind.saturating_sub(1))) {
            kind = of_side(self, &harvesters);
        }
        if let Some(fixed) = self.crate_rules.unit_type {
            kind = Some(fixed);
        }
        if kind.is_none() {
            let goodies: Vec<u16> = self.crate_rules.goodies.iter().copied().filter(|g| human || !self.crate_rules.base_units.contains(g)).collect();
            if !goodies.is_empty() {
                kind = Some(goodies[self.random_below(goodies.len() as u32) as usize]);
            }
        }
        let Some(kind) = kind else {
            return false;
        };
        let Some(stats) = self.unit_types.get(kind as usize).map(|t| UnitStats { kind: kind + 1, ..t.stats }) else {
            return false;
        };
        let stats_for_probe = stats;
        let probe = Unit::new(u32::MAX - 2, owner, cell, 0, stats_for_probe);
        let at = crate::nearby(&self.map, &self.claims, &self.spots, &probe, cell, Near::new(false, false, None), self.tick).unwrap_or(cell);
        let id = self.add_unit(at, 0x40, stats, owner);
        self.units.iter().any(|u| u.id == id)
    }
}
