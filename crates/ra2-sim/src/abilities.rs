//! Special weapons whose hit is not plain damage: Ivan's bombs and the kit that
//! disarms them, mind control, and the parasite (the terror drone and the dogs).
//!
//! Follows the original game (bombs)
//! and (mind control, the parasite class, dogs).
//!

use crate::combat::{Aim, Event, FireError, Fighter, Warhead};
use crate::path::code;
use crate::{entry_code, Other, Unit, World, LEPTONS_PER_CELL};

/// The abilities a rank can bring, by the engine's own numbers.
pub mod ability {
    pub const FASTER: u32 = 1 << 0;
    pub const STRONGER: u32 = 1 << 1;
    pub const FIREPOWER: u32 = 1 << 2;
    pub const SCATTER: u32 = 1 << 3;
    pub const ROF: u32 = 1 << 4;
    pub const SIGHT: u32 = 1 << 5;
    pub const SELF_HEAL: u32 = 1 << 9;
    pub const NAMES: [&str; 18] = [
        "FASTER", "STRONGER", "FIREPOWER", "SCATTER", "ROF", "SIGHT", "CLOAK", "TIBERIUM_PROOF", "VEIN_PROOF", "SELF_HEAL", "EXPLODES", "RADAR_INVISIBLE", "SENSORS", "FEARLESS", "C4",
        "TIBERIUM_HEAL", "GUARD_AREA", "CRUSHER",
    ];
}

/// The rules of experience.
// DATA: rules.ini `[General]` `VeteranRatio`, `VeteranCombat`, `VeteranSpeed`,
// `VeteranArmor`, `VeteranROF`, `VeteranCap`, `RepairRate`.
#[derive(Clone, Copy, Debug)]
pub struct Veterancy {
    pub ratio: f64,
    pub combat: f64,
    pub speed: f64,
    pub armor: f64,
    pub rof: f64,
    pub cap: f64,
    /// Frames between a self-healer's points of health.
    pub heal_every: u64,
}

impl Default for Veterancy {
    fn default() -> Self {
        Self { ratio: 3.0, combat: 1.1, speed: 1.2, armor: 1.5, rof: 0.6, cap: 2.0, heal_every: 14 }
    }
}

impl Unit {
    /// 0 elite, 1 veteran, 2 neither.
    pub fn rank(&self) -> u8 {
        if self.veterancy >= 2.0 {
            0
        } else if self.veterancy >= 1.0 {
            1
        } else {
            2
        }
    }

    /// A veteran has its type's veteran
    /// abilities, an elite those and the elite ones.
    pub fn has_ability(&self, bit: u32) -> bool {
        match self.rank() {
            0 => (self.stats.veteran_abilities | self.stats.elite_abilities) & bit != 0,
            1 => self.stats.veteran_abilities & bit != 0,
            _ => false,
        }
    }
}

/// A unit's cloak.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cloak {
    /// 0 seen, 1 cloaking, 2 cloaked, 3 uncloaking.
    pub state: u8,
    pub stage: i32,
    step: i32,
    next: u64,
    /// Not before this frame may it cloak again.
    again: u64,
    cell: (i32, i32),
}

/// `CloakingStages` and `CloakDelay` in frames.
// DATA: rules.ini `[General]`: 9 stages; .02 minutes, 18 frames.
pub const CLOAKING_STAGES: i32 = 9;
pub const CLOAK_DELAY: u64 = 18;

/// A bomb stuck on something.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Bomb {
    /// Who planted it (gone if he has died), and his house.
    pub owner: Option<u32>,
    pub house: u8,
    pub target: Aim,
    pub planted: u64,
    pub goes_off: u64,
}

/// The rules the abilities here read.
/// A biter in the air between firing and landing on its victim.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Leap {
    pub id: u32,
    pub weapon: u16,
    pub house: u8,
    pub position: (f64, f64, f64),
    pub facing: u8,
    speed: f64,
    target: u32,
    biter: Unit,
}

/// The house of a disguise that is a tree, not a unit.
pub const TREE_DISGUISE: u8 = u8::MAX;

#[derive(Clone, Debug)]
pub struct Rules {
    /// `[CombatDamage] DeathWeapon`: what goes off for an exploding type with
    /// no weapon of its own.
    pub death_weapon: Option<u16>,
    /// `AlliedDisguise` and `SovietDisguise`: unit kinds (as a unit's `kind`).
    pub disguises: [Option<u16>; 2],
    /// How many `DefaultMirageDisguises` there are to pick from, and
    /// `InfantryBlinkDisguiseTime`.
    pub mirage_looks: u16,
    pub infantry_blink: u32,
    /// `C4Warhead`: the warhead of "absolute damage".
    pub c4_warhead: Option<u16>,
    /// `SpyPowerBlackout` and `SpyMoneyStealPercent`.
    pub spy_blackout: u32,
    pub spy_steal: f64,
    /// `BuildTech`: the laboratories, by building type.
    pub build_tech: Vec<u16>,
    /// `PrismType`, `PrismSupportMax`, `PrismSupportModifier` (as a share) and
    /// `PrismSupportDelay`.
    pub prism_type: Option<u16>,
    pub prism_max: u32,
    pub prism_modifier: f64,
    pub prism_delay: u32,
    /// `IvanDamage`, `IvanWarhead` and `IvanTimedDelay`.
    pub ivan_damage: i32,
    pub ivan_warhead: u16,
    pub ivan_delay: u32,
}

impl Default for Rules {
    fn default() -> Self {
        Self { death_weapon: None, disguises: [None; 2], mirage_looks: 0, infantry_blink: 20, c4_warhead: None, spy_blackout: 1000, spy_steal: 0.5, build_tech: Vec::new(), prism_type: None, prism_max: 8, prism_modifier: 1.5, prism_delay: 60, ivan_damage: 400, ivan_warhead: 0, ivan_delay: 450 }
    }
}

impl World {
    /// The bomb on something, if it has one.
    pub fn bomb_on(&self, who: Aim) -> Option<&Bomb> {
        self.bombs.iter().find(|bomb| bomb.target == who)
    }

    /// The fire test's extra refusals for the weapons here.
    // A bomb cannot go on what
    // already has one; the disarming kit needs a bomb to disarm; mind control
    // cannot take what is immune to it or already held; a parasite needs a
    // vehicle or soldier that can be infested and is not already.
    pub(crate) fn special_error(&self, warhead: &Warhead, target: Aim) -> Option<FireError> {
        if warhead.ivan_bomb && (self.bomb_on(target).is_some() || matches!(target, Aim::Cell(..))) {
            return Some(FireError::Illegal);
        }
        if warhead.bomb_disarm && self.bomb_on(target).is_none() {
            return Some(FireError::Illegal);
        }
        if warhead.mind_control {
            let free = match target {
                Aim::Unit(id) => self.unit(id).is_some_and(|unit| !unit.stats.fighter.immune_to_psionics && unit.controlled_by.is_none()),
                _ => false,
            };
            if !free {
                return Some(FireError::Illegal);
            }
        }
        if warhead.parasite {
            let open = match target {
                Aim::Unit(id) => self.unit(id).is_some_and(|unit| unit.stats.fighter.parasiteable && unit.parasite.is_empty() && self.altitude(unit) == 0),
                _ => false,
            };
            if !open {
                return Some(FireError::Illegal);
            }
        }
        None
    }

    /// What a hit by one of these weapons does instead of damage. Returns whether
    /// it was one of them.
    pub(crate) fn special_hit(&mut self, warhead: &Warhead, by: Option<Aim>, house: u8, target: Aim) -> bool {
        if warhead.ivan_bomb {
            // Planted only by a foot soldier, on a
            // thing with no bomb yet; it goes off `IvanTimedDelay` later.
            let planter = match by {
                Some(Aim::Unit(id)) => self.unit(id).filter(|unit| unit.stats.infantry).map(|unit| unit.id),
                _ => None,
            };
            if planter.is_some() && !matches!(target, Aim::Cell(..)) && self.alive(target) && self.bomb_on(target).is_none() {
                self.bombs.push(Bomb { owner: planter, house, target, planted: self.tick, goes_off: self.tick + self.ability_rules.ivan_delay as u64 });
                self.events.push(Event::BombPlanted { target, house });
            }
            return true;
        }
        if warhead.bomb_disarm {
            // The bomb is gone, with no explosion,
            // whoever's it was.
            self.bombs.retain(|bomb| bomb.target != target);
            return true;
        }
        if warhead.makes_disguise {
            // The firer takes the look of
            // a foot soldier it hits: what that one shows if it is disguised
            // itself, else its own type and house. No damage.
            if let (Some(Aim::Unit(firer)), Aim::Unit(victim)) = (by, target) {
                let look = self.unit(victim).filter(|unit| unit.stats.infantry).map(|unit| unit.disguise.unwrap_or((unit.stats.kind, unit.owner)));
                if let (Some(look), Some(unit)) = (look, self.units.iter_mut().find(|u| u.id == firer)) {
                    unit.disguise = Some(look);
                    unit.disguised_at = self.tick;
                }
            }
            return true;
        }
        if warhead.mind_control {
            if let (Some(Aim::Unit(controller)), Aim::Unit(victim)) = (by, target) {
                self.capture_mind(controller, victim);
            }
            return true;
        }
        if warhead.electric_assault {
            // A shot from a foot soldier
            // that lands on a building puts him on its list of chargers, once,
            // at the end. Neither's house nor the building's type is asked
            // about. No damage is done: the branch leaves before the blast.
            if let (Some(Aim::Unit(charger)), Aim::Building(id)) = (by, target) {
                let soldier = self.unit(charger).is_some_and(|unit| unit.stats.infantry);
                if let Some(building) = self.buildings.iter_mut().find(|b| b.id == id).filter(|_| soldier) {
                    building.chargers.retain(|other| *other != charger);
                    building.chargers.push(charger);
                }
            }
            return true;
        }
        false
    }

    /// Whether a house's psychic sensors show what a unit is about: where it is
    /// going or what it is after.
    // Not for an ally's unit; for each of the house's sensors with power,
    // the unit's target (a thing or a cell), or a foot unit's destination,
    // within the type's `PsychicDetectionRadius` of the sensor, measured in
    // three dimensions. Where the unit itself is does not matter.
    pub fn sensor_reveals(&self, viewer: u8, unit: &Unit) -> bool {
        if self.allied(viewer, unit.owner) {
            return false;
        }
        let half = LEPTONS_PER_CELL / 2;
        let centre = |cell: (i32, i32)| (cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, self.map.level(cell.0, cell.1) as i32 * crate::combat::LEVEL_HEIGHT);
        let points = [unit.arms.target.and_then(|target| self.aim_point(target)), unit.destination().map(centre)];
        self.buildings.iter().any(|sensor| {
            let radius = self.building_types.get(sensor.kind as usize).map_or(0, |kind| kind.psychic_radius) as i64 * LEPTONS_PER_CELL as i64;
            if radius == 0 || sensor.owner != viewer || sensor.buildup.is_some() || !self.online(sensor) {
                return false;
            }
            let at = centre(sensor.cell);
            points.iter().flatten().any(|p| {
                let (dx, dy, dz) = ((p.0 - at.0) as i64, (p.1 - at.1) as i64, (p.2 - at.2) as i64);
                dx * dx + dy * dy + dz * dz <= radius * radius
            })
        })
    }

    /// Keeps each building's list of chargers and what comes of it.
    // Each
    // frame an `Overpowerable` building drops the chargers no longer aiming at
    // it; it is charged with three, or with one while its house has all the
    // power it needs.
    pub(crate) fn run_chargers(&mut self) {
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            if !self.building_types.get(building.kind as usize).is_some_and(|kind| kind.overpowerable) {
                continue;
            }
            let me = Some(Aim::Building(building.id));
            let kept: Vec<u32> = building.chargers.iter().copied().filter(|id| self.unit(*id).is_some_and(|unit| unit.health > 0 && unit.arms.target == me)).collect();
            let charged = kept.len() >= 3 || (!kept.is_empty() && !self.low_power(building.owner));
            let building = &mut self.buildings[index];
            building.chargers = kept;
            building.charged = charged;
        }
    }

    /// Whether something would be charging, not attacking, a building.
    // The target is a building of an allied house,
    // its type is `Overpowerable`, and the second weapon's warhead has
    // `ElectricAssault`.
    pub(crate) fn charges(&self, house: u8, fighter: &Fighter, target: Aim) -> bool {
        let Aim::Building(id) = target else {
            return false;
        };
        let assault = fighter.secondary.and_then(|mount| self.weapons.get(mount.weapon as usize)).and_then(|weapon| self.warheads.get(weapon.warhead as usize)).is_some_and(|warhead| warhead.electric_assault);
        assault
            && self.buildings.iter().find(|b| b.id == id).is_some_and(|building| {
                self.allied(house, building.owner) && self.building_types.get(building.kind as usize).is_some_and(|kind| kind.overpowerable)
            })
    }

    /// The `Overpowerable` building of a house in one of the eight cells round
    /// a cell, looking north first and then clockwise.
    // The same owner, not merely an ally.
    pub(crate) fn coil_beside(&self, cell: (i32, i32), house: u8) -> Option<u32> {
        crate::path::DIRECTIONS.iter().find_map(|step| {
            let building = &self.buildings[self.building_on((cell.0 + step.0, cell.1 + step.1))?];
            (building.owner == house && building.buildup.is_none() && self.building_types.get(building.kind as usize).is_some_and(|kind| kind.overpowerable)).then_some(building.id)
        })
    }

    /// Whether one of a house's buildings that sees through disguises covers a
    /// cell.
    fn disguise_detected(&self, cell: (i32, i32), house: u8) -> bool {
        // Only buildings with `DetectDisguise`
        // mark cells, out to `DetectDisguiseRange`.
        self.buildings.iter().any(|building| {
            building.owner == house
                && self.building_types.get(building.kind as usize).is_some_and(|kind| {
                    let reach = kind.detect_disguise as i64;
                    let (dx, dy) = ((building.cell.0 - cell.0) as i64, (building.cell.1 - cell.1) as i64);
                    reach > 0 && dx * dx + dy * dy <= reach * reach
                })
        })
    }

    /// A Mirage takes the look of a tree while it stands still, and loses it
    /// when it moves or an enemy soldier comes beside it; then it waits
    /// `InfantryBlinkDisguiseTime` before taking it again.
    fn run_mirages(&mut self) {
        let (looks, blink) = (self.ability_rules.mirage_looks, self.ability_rules.infantry_blink as u64);
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            if !unit.stats.fighter.disguise_when_still || unit.health <= 0 {
                continue;
            }
            if unit.is_moving() {
                self.units[index].disguise = None;
                continue;
            }
            let (cell, owner) = (unit.cell(), unit.owner);
            if self.tick % 8 != 0 {
                let enemy_beside = crate::path::DIRECTIONS.iter().any(|step| {
                    let next = (cell.0 + step.0, cell.1 + step.1);
                    self.units.iter().any(|other| other.stats.infantry && other.health > 0 && other.cell() == next && !self.allied(other.owner, owner))
                });
                if enemy_beside {
                    let unit = &mut self.units[index];
                    unit.disguise = None;
                    unit.undisguised_until = self.tick + blink;
                    continue;
                }
            }
            if self.units[index].disguise.is_none() && self.tick >= self.units[index].undisguised_until && looks > 0 {
                let look = (self.next_random() % looks as u32) as u16;
                let unit = &mut self.units[index];
                unit.disguise = Some((look, TREE_DISGUISE));
                unit.disguised_at = self.tick;
            }
        }
    }

    /// Whether a unit's disguise takes in a house.
    // Disguised; its owner no ally of the house;
    // no detector of the house's covering its cell; and the house is the one
    // imitated or an ally of it.
    pub fn disguised_to(&self, unit: &Unit, house: u8) -> bool {
        let Some((_, as_house)) = unit.disguise else {
            return false;
        };
        if as_house == TREE_DISGUISE {
            return !self.allied(unit.owner, house) && !self.disguise_detected(unit.cell(), house);
        }
        if self.allied(unit.owner, house) || !(as_house == house || self.allied(as_house, house)) {
            return false;
        }
        // Only buildings with `DetectDisguise`
        // mark cells, out to `DetectDisguiseRange`.
        let cell = unit.cell();
        !self.buildings.iter().any(|building| {
            building.owner == house
                && self.building_types.get(building.kind as usize).is_some_and(|kind| {
                    let reach = kind.detect_disguise as i64;
                    let (dx, dy) = ((building.cell.0 - cell.0) as i64, (building.cell.1 - cell.1) as i64);
                    reach > 0 && dx * dx + dy * dy <= reach * reach
                })
        })
    }

    /// Whether a house's player sees a unit as what it is.
    // Its owner sees the disguise for 48 frames of
    // every 256; anyone else sees the disguise unless one of their detectors
    // covers its cell (there is no test of alliance here).
    pub fn shows_true_self(&self, unit: &Unit, house: u8) -> bool {
        if unit.disguise.is_none() || self.tick < unit.blinks_until {
            return true;
        }
        if unit.owner == house {
            let phase = (self.tick.wrapping_sub(unit.disguised_at) + 0x40) % 256;
            return !(0x48..=0x77).contains(&phase);
        }
        let cell = unit.cell();
        self.buildings.iter().any(|building| {
            building.owner == house
                && self.building_types.get(building.kind as usize).is_some_and(|kind| {
                    let reach = kind.detect_disguise as i64;
                    let (dx, dy) = ((building.cell.0 - cell.0) as i64, (building.cell.1 - cell.1) as i64);
                    reach > 0 && dx * dx + dy * dy <= reach * reach
                })
        })
    }

    /// What a spy does to a building it gets into.
    // The first that applies,
    // in this order: a radar loses its owner the map it has seen; a power
    // plant, its power for a while; a super weapon's building, the charge; a
    // laboratory gives its technology; a store of money, a share of it; a
    // vehicle factory or a barracks makes the spy's side's new ones veterans.
    pub(crate) fn infiltrate(&mut self, at: usize, spy: u8) {
        let building = &self.buildings[at];
        let (id, victim) = (building.id, building.owner);
        let Some(kind) = self.building_types.get(building.kind as usize).cloned() else {
            return;
        };
        let which = building.kind;
        let rules = self.ability_rules.clone();
        let effect = if kind.radar {
            if self.human(victim) {
                self.map.reshroud();
                // Everything looks about itself afresh.
                for unit in &mut self.units {
                    unit.looked = None;
                }
            }
            1
        } else if kind.power > 0 {
            if let Some(house) = self.houses.get_mut(victim as usize) {
                house.blackout_until = self.tick + rules.spy_blackout as u64;
            }
            2
        } else if let Some(index) = kind.super_weapon {
            let recharge = self.super_types.get(index as usize).map_or(0, |t| t.recharge);
            if let Some(state) = self.houses.get_mut(victim as usize).and_then(|h| h.supers.get_mut(index as usize)).filter(|s| s.granted) {
                state.ready = false;
                state.left = recharge;
            }
            3
        } else if rules.build_tech.contains(&which) {
            let side = self.houses.get(victim as usize).map_or(0, |h| h.side);
            if let Some(house) = self.houses.get_mut(spy as usize) {
                house.stolen[if side == 0 { 1 } else { 0 }] = true;
            }
            4
        } else if kind.storage > 0 {
            let take = (self.houses.get(victim as usize).map_or(0, |h| h.credits.max(0)) as f64 * rules.spy_steal) as i32;
            if let Some(house) = self.houses.get_mut(victim as usize) {
                house.credits -= take;
            }
            if let Some(house) = self.houses.get_mut(spy as usize) {
                house.credits += take;
            }
            5
        } else if kind.factory == Some(crate::Factory::Unit) {
            if let Some(house) = self.houses.get_mut(spy as usize) {
                house.veterans[1] = true;
            }
            6
        } else if kind.factory == Some(crate::Factory::Infantry) {
            if let Some(house) = self.houses.get_mut(spy as usize) {
                house.veterans[0] = true;
            }
            7
        } else {
            0
        };
        self.events.push(Event::Infiltrated { building: id, victim, by: spy, effect });
    }

    /// A controller takes a victim.
    // The one it held before is
    // let go first, always; the victim joins the controller's house, forgets
    // where it was going and what it was shooting, and stands guard.
    fn capture_mind(&mut self, controller: u32, victim: u32) {
        let Some(owner) = self.unit(controller).map(|unit| unit.owner) else {
            return;
        };
        self.release_mind(controller);
        let free = self.unit(victim).is_some_and(|unit| unit.owner != owner && unit.controlled_by.is_none() && !unit.stats.fighter.immune_to_psionics);
        if !free {
            return;
        }
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == victim) {
            unit.controlled_by = Some((controller, unit.owner));
            unit.owner = owner;
        }
        self.changed_sides(victim);
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == controller) {
            unit.controls = Some(victim);
        }
        self.events.push(Event::MindControlled { controller, victim });
    }

    /// A controller lets go of whatever it holds, which goes back to its house.
    pub(crate) fn release_mind(&mut self, controller: u32) {
        let Some(victim) = self.units.iter_mut().find(|u| u.id == controller).and_then(|unit| unit.controls.take()) else {
            return;
        };
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == victim) {
            if let Some((_, home)) = unit.controlled_by.take() {
                unit.owner = home;
            }
        }
        self.changed_sides(victim);
    }

    /// What goes with a unit passing to another house.
    // Its target and destination go and it falls idle; everything that
    // was aiming at it lets go; taken or let go, the same.
    pub(crate) fn changed_sides(&mut self, id: u32) {
        let me = Some(Aim::Unit(id));
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
            unit.arms.target = None;
            unit.arms.attacking = false;
            unit.arms.firing = 0;
            unit.dest = None;
            unit.drop_path();
            unit.post = None;
            unit.charging = false;
            unit.capture = None;
            unit.boarding = None;
            if !unit.stats.harvester {
                unit.work = Default::default();
            }
        }
        for unit in &mut self.units {
            if unit.arms.target == me {
                unit.arms.target = None;
                unit.arms.attacking = false;
            }
        }
        for building in &mut self.buildings {
            if building.arms.target == me {
                building.arms.target = None;
            }
        }
        for team in &mut self.teams {
            team.members.retain(|member| *member != id);
        }
    }

    /// A weapon that sends its firer into the target takes hold.
    // The firer goes off the map as it fires and its shot stands in for it;
    // when the shot lands on a vehicle or soldier that is alive, able to be
    // infested and not so already, the firer is inside it; otherwise it is
    // back where it stood, or lost if it cannot stand there. A soldier is
    // killed outright by the first bite, a vehicle is hurt by the weapon's
    // damage every `ROF`.
    pub(crate) fn infest(&mut self, firer: u32, target: Aim, weapon: u16) -> bool {
        let Aim::Unit(victim) = target else {
            return false;
        };
        if !self.can_infest(victim) {
            return false;
        }
        let Some(index) = self.units.iter().position(|u| u.id == firer) else {
            return false;
        };
        let height = self.altitude(&self.units[index]) + self.ground_z(self.units[index].x, self.units[index].y);
        let mut biter = self.lift(index);
        biter.arms.target = None;
        biter.arms.attacking = false;
        biter.dest = None;
        biter.head_to = None;
        biter.drop_path();
        let speed = self.weapons.get(weapon as usize).map_or(0, |w| w.speed).max(1) as f64;
        self.leaps.push(Leap { id: biter.id, weapon, house: biter.owner, position: (biter.x as f64, biter.y as f64, height as f64), facing: biter.facing, speed, target: victim, biter });
        true
    }

    fn can_infest(&self, victim: u32) -> bool {
        self.unit(victim).is_some_and(|unit| unit.stats.fighter.parasiteable && unit.parasite.is_empty() && unit.health > 0)
    }

    /// The leaps in the air.
    pub fn leaps(&self) -> &[Leap] {
        &self.leaps
    }

    /// Leaping biters fly on, and take hold or come back.
    pub(crate) fn run_leaps(&mut self) {
        let mut flying = Vec::new();
        for mut leap in std::mem::take(&mut self.leaps) {
            let to = Some(leap.target).filter(|&victim| self.can_infest(victim)).and_then(|victim| self.aim_point(Aim::Unit(victim)));
            let Some(to) = to else {
                // Missed: back to where it stood.
                let cell = leap.biter.cell();
                self.set_down(leap.biter, cell);
                continue;
            };
            let d = (to.0 as f64 - leap.position.0, to.1 as f64 - leap.position.1, to.2 as f64 - leap.position.2);
            let far = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
            if far <= leap.speed {
                leap.biter.bite = Some((leap.weapon, self.tick));
                if let Some(host) = self.units.iter_mut().find(|u| u.id == leap.target) {
                    host.parasite.push(leap.biter);
                }
                continue;
            }
            let step = leap.speed / far;
            leap.position = (leap.position.0 + d.0 * step, leap.position.1 + d.1 * step, leap.position.2 + d.2 * step);
            leap.facing = (crate::combat::facing_of((-d.1).atan2(d.0)) as u16 >> 8) as u8;
            flying.push(leap);
        }
        self.leaps = flying;
    }

    /// Puts something that was off the map on a cell, if the cell will have it.
    pub(crate) fn set_down(&mut self, mut unit: Unit, cell: (i32, i32)) -> bool {
        let others: Vec<Other> = self.others();
        let free = entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &unit, cell, None, cell, -1) == code::OK;
        let Some(at) = self.map.index(cell.0, cell.1).filter(|_| free) else {
            return false;
        };
        unit.bite = None;
        if unit.stats.infantry {
            let Some(spot) = (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0) else {
                return false;
            };
            unit.x = cell.0 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].0;
            unit.y = cell.1 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].1;
            self.spots[at] |= 1 << spot;
            self.spot_house[at] = unit.owner;
        } else {
            unit.x = cell.0 * LEPTONS_PER_CELL + 128;
            unit.y = cell.1 * LEPTONS_PER_CELL + 128;
            self.claims[at] = unit.id + 1;
        }
        self.put_back(unit);
        true
    }

    /// Whether a house's sensors cover a cell.
    // Every unit with `SensorsSight` marks the
    // cells within that radius of its own (the far row and column left out, as
    // the engine's loop has it).
    pub fn sensed_by(&self, cell: (i32, i32), house: u8) -> bool {
        self.units.iter().any(|unit| {
            let r = unit.stats.sensors_sight;
            if r <= 0 || unit.owner != house {
                return false;
            }
            let at = unit.cell();
            let (dx, dy) = (cell.0 - at.0, cell.1 - at.1);
            (-r..r).contains(&dx) && (-r..r).contains(&dy) && dx * dx + dy * dy < r * r
        })
    }

    /// How a unit looks to a house's player: 0 plain, 1 a quarter see-through,
    /// 2 to 4 half see-through, 5 not there.
    // Cloaked, its owner (and anyone whose
    // sensors cover its cell) sees it half see-through and nobody else sees
    // it; between, by how far the cloak has come.
    pub fn visual_character(&self, unit: &Unit, house: u8) -> u8 {
        let mine = unit.owner == house;
        match unit.cloak.state {
            0 => 0,
            2 => {
                if mine || self.sensed_by(unit.cell(), house) {
                    3
                } else {
                    5
                }
            }
            _ => {
                if unit.cloak.stage <= 0 {
                    return 0;
                }
                let v = (unit.cloak.stage as f64 / CLOAKING_STAGES as f64 * 256.0) as i32;
                match v {
                    ..0x40 => 1,
                    0x40..0x80 => 2,
                    0x80..0xc0 => 3,
                    _ if mine => 3,
                    0xff.. => 5,
                    _ => 4,
                }
            }
        }
    }

    /// Whether a unit cannot be seen, pointed at or shot at by a house.
    pub fn hidden_from(&self, unit: &Unit, house: u8) -> bool {
        unit.cloak.state == 2 && unit.owner != house && !self.sensed_by(unit.cell(), house)
    }

    /// Vehicles that crush run over what is under them.
    // Every `Crushable`
    // thing of another side on the crusher's cell within 128 leptons of it,
    // and not falling, is gone, with no damage, warhead or animation: only its
    // type's `CrushSound` and the kill to the crusher's credit.
    // Not original structure: looked at every frame, not on entering a cell.
    fn run_crush(&mut self) {
        // A crusher starting
        // into a cell warns the soldiers on it; those of a computer house, and
        // those whose rank brings SCATTER, step aside if they are standing.
        let warned: Vec<(i32, i32)> = self
            .units
            .iter()
            .filter(|u| u.stats.crusher && !u.stats.infantry && u.health > 0)
            .filter_map(|u| u.head_to.map(|(x, y)| (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL))).filter(|cell| *cell != u.cell()))
            .collect();
        for cell in warned {
            let movers: Vec<u32> = self
                .units
                .iter()
                .filter(|u| u.stats.infantry && u.cell() == cell && !u.is_moving() && u.stance == crate::combat::Stance::Standing && u.falling.is_none())
                .filter(|u| !self.human(u.owner) || u.has_ability(ability::SCATTER))
                .map(|u| u.id)
                .collect();
            for id in movers {
                self.scatter_unit(id);
            }
        }
        let crushers: Vec<(u32, u8, (i32, i32), (i32, i32))> =
            self.units.iter().filter(|u| u.stats.crusher && !u.stats.infantry && u.health > 0 && self.altitude(u) == 0).map(|u| (u.id, u.owner, u.cell(), (u.x, u.y))).collect();
        for (crusher, owner, cell, at) in crushers {
            let victims: Vec<(u32, u16, (i32, i32, i32))> = self
                .units
                .iter()
                .filter(|v| {
                    let (dx, dy) = ((v.x - at.0) as i64, (v.y - at.1) as i64);
                    v.stats.crushable && v.id != crusher && v.cell() == cell && !self.allied(owner, v.owner) && v.falling.is_none() && self.altitude(v) == 0 && dx * dx + dy * dy < 0x4000
                })
                .map(|v| (v.id, v.stats.kind, (v.x, v.y, self.ground_z(v.x, v.y))))
                .collect();
            for (victim, kind, point) in victims {
                self.events.push(Event::Crushed { kind, at: point });
                self.record_kill(Aim::Unit(victim), Some(Aim::Unit(crusher)));
                self.score_kill(Aim::Unit(victim), Some(Aim::Unit(crusher)));
                self.vanish(victim);
            }
        }
    }

    /// The cloaks' turn.
    // A `Cloakable` unit with no target in range, loaded, and not just
    // uncloaked, cloaks over `CloakingStages` steps of `CloakingSpeed` frames
    // (in the red only on a 4 in 100 chance each frame); it uncloaks the same
    // way; a cloaked unit that comes to a new cell beside an enemy with
    // `Sensors` uncloaks.
    fn run_cloaks(&mut self) {
        let now = self.tick;
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            if !unit.stats.cloakable {
                continue;
            }
            let mut cloak = unit.cloak;
            let speed = unit.stats.cloaking_speed.max(1) as u64;
            let (id, here, owner) = (unit.id, unit.cell(), unit.owner);
            let mut sound = false;
            match cloak.state {
                0 => {
                    let busy = unit.frozen.is_some() || now < unit.warp_until || unit.arms.target.is_some() || now < cloak.again || unit.tethered;
                    if !busy {
                        let red = unit.health * 4 <= unit.stats.strength;
                        if !red || self.random_below(100) < 4 {
                            cloak = Cloak { state: 1, stage: 0, step: 1, next: now + speed, again: 0, cell: here };
                            sound = true;
                        }
                    }
                }
                1 | 3 => {
                    if now >= cloak.next {
                        cloak.stage += cloak.step;
                        cloak.next = now + speed;
                    }
                    if cloak.state == 1 && cloak.stage >= CLOAKING_STAGES {
                        cloak.state = 2;
                        cloak.cell = here;
                    } else if cloak.state == 3 && cloak.stage <= 0 {
                        cloak = Cloak { state: 0, stage: 0, step: 0, next: 0, again: now + CLOAK_DELAY, cell: here };
                    }
                }
                _ => {
                    let found = cloak.cell != here && {
                        self.units.iter().any(|other| {
                            let at = other.cell();
                            other.stats.sensors && !self.allied(owner, other.owner) && other.id != id && (at.0 - here.0).abs() <= 1 && (at.1 - here.1).abs() <= 1
                        })
                    };
                    cloak.cell = here;
                    if found || self.units[index].frozen.is_some() {
                        cloak = Cloak { state: 3, stage: CLOAKING_STAGES - 1, step: -1, next: now + speed, again: 0, cell: here };
                        sound = true;
                    }
                }
            }
            self.units[index].cloak = cloak;
            if sound {
                self.events.push(Event::Cloaked { unit: id });
            }
        }
    }

    /// A cloaked unit that must fire comes out first. Returns whether it may
    /// fire now.
    // The fire test answers
    // "cloaked" and the unit uncloaks.
    pub(crate) fn uncloak_to_fire(&mut self, index: usize) -> bool {
        let unit = &mut self.units[index];
        match unit.cloak.state {
            0 => true,
            3 => false,
            _ => {
                let speed = unit.stats.cloaking_speed.max(1) as u64;
                unit.cloak = Cloak { state: 3, stage: (CLOAKING_STAGES - 1).min(unit.cloak.stage.max(1)), step: -1, next: self.tick + speed, again: 0, cell: unit.cell() };
                let id = unit.id;
                self.events.push(Event::Cloaked { unit: id });
                false
            }
        }
    }

    /// The blast of something that explodes as it dies.
    // Its
    // `DeathWeapon` at `DeathWeaponDamageModifier` times that weapon's damage;
    // else its first weapon; else the rules' `DeathWeapon` with half the type's
    // strength as the damage. Buildings go the same way as units.
    pub(crate) fn death_blast(&mut self, point: (i32, i32, i32), own: Option<u16>, percent: i32, primary: Option<u16>, strength: i32, source: Aim) {
        let blast = match (own, primary, self.ability_rules.death_weapon) {
            (Some(weapon), ..) => self.weapons.get(weapon as usize).map(|w| ((w.damage as i64 * percent as i64 / 100) as i32, w.warhead)),
            (None, Some(weapon), _) => self.weapons.get(weapon as usize).map(|w| (w.damage, w.warhead)),
            (None, None, Some(weapon)) => self.weapons.get(weapon as usize).map(|w| (strength / 2, w.warhead)),
            _ => None,
        };
        if let Some((damage, warhead)) = blast {
            self.explode(point, damage, warhead, Some(source));
            self.blast_event(warhead, damage, point, false);
        }
    }

    /// The killer of something gains experience from it.
    // The victim's cost, doubled for a veteran and
    // tripled for an elite, nothing for an ally's; a `Trainable` killer gains
    // that over its own cost times `VeteranRatio`, up to `VeteranCap`; a
    // missile's launcher gains the same.
    pub(crate) fn record_kill(&mut self, victim: Aim, killer: Option<Aim>) {
        let Some(Aim::Unit(killer)) = killer else {
            return;
        };
        let cost_of = |world: &World, kind: u16| (kind as usize).checked_sub(1).and_then(|k| world.unit_types.get(k)).map_or(0, |t| t.cost);
        let (owner, mut points) = match victim {
            Aim::Unit(id) => match self.unit(id) {
                Some(unit) => (unit.owner, cost_of(self, unit.stats.kind) * [3, 2, 1][unit.rank() as usize]),
                None => return,
            },
            Aim::Building(id) => match self.buildings.iter().find(|b| b.id == id) {
                Some(building) => (building.owner, self.building_types.get(building.kind as usize).map_or(0, |t| t.cost)),
                None => return,
            },
            Aim::Cell(..) => return,
        };
        let Some(by) = self.unit(killer) else {
            return;
        };
        if self.allied(by.owner, owner) {
            points = 0;
        }
        let launcher = by.spawn_owner.filter(|_| by.stats.missile_spawn);
        let rules = self.veterancy;
        for id in [Some(killer), launcher].into_iter().flatten() {
            let Some(index) = self.units.iter().position(|u| u.id == id) else {
                continue;
            };
            let own = cost_of(self, self.units[index].stats.kind).max(1) as f64;
            let unit = &mut self.units[index];
            if unit.stats.trainable {
                unit.veterancy = (unit.veterancy + points as f64 / (own * rules.ratio)).min(rules.cap);
            }
        }
    }

    /// Ranks gained, and the slow mending of those that heal themselves.
    // A change of rank is noticed each frame; the
    // player hears of a promotion. A
    // `SelfHealing` type, or one whose rank brings SELF_HEAL, gains one point of
    // health every `RepairRate` (14 frames) while it is short of full.
    // Not original structure: an elite's `ElitePrimary` and `EliteSecondary`
    // replace its weapons when it is promoted (the engine picks by rank each
    // time), and FASTER is applied to its speed then.
    fn run_veterancy(&mut self) {
        let rules = self.veterancy;
        let heal = rules.heal_every > 0 && self.tick % rules.heal_every == 0;
        for index in 0..self.units.len() {
            let unit = &mut self.units[index];
            let rank = unit.rank();
            if rank != unit.shown_rank {
                let was = unit.shown_rank;
                unit.shown_rank = rank;
                if rank == 0 {
                    if let Some(mount) = unit.stats.elite_primary {
                        unit.stats.fighter.primary = Some(mount);
                    }
                    if let Some(mount) = unit.stats.elite_secondary {
                        unit.stats.fighter.secondary = Some(mount);
                    }
                }
                if unit.has_ability(ability::FASTER) && !unit.sped_up {
                    unit.sped_up = true;
                    unit.speed = (unit.speed as f64 * rules.speed) as i32;
                }
                if rank < was {
                    let (id, owner) = (unit.id, unit.owner);
                    self.events.push(Event::Promoted { unit: id, owner, elite: rank == 0 });
                }
            }
            let unit = &mut self.units[index];
            if heal && unit.health > 0 && unit.health < unit.stats.strength && (unit.stats.self_healing || unit.has_ability(ability::SELF_HEAL)) {
                unit.health += 1;
            }
        }
    }

    /// Bombs go off, parasites bite, and controllers whose victims have gone
    /// forget them.
    pub(crate) fn run_abilities(&mut self) {
        self.run_veterancy();
        self.run_cloaks();
        self.run_crush();
        self.run_mirages();
        // A `PermaDisguise` type starts out looking
        // like its side's ordinary soldier, of its own house.
        let disguises = self.ability_rules.disguises;
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            if unit.disguise.is_none() && unit.stats.fighter.can_disguise && unit.stats.fighter.perma_disguise {
                let side = self.houses.get(unit.owner as usize).map_or(0, |h| h.side);
                if let Some(kind) = disguises[if side == 0 { 0 } else { 1 }] {
                    self.units[index].disguise = Some((kind, unit.owner));
                    self.units[index].disguised_at = self.tick;
                }
            }
        }

        // A bomb whose time is up is one
        // ordinary explosion of `IvanDamage` with `IvanWarhead` where its
        // carrier stands; the carrier is hurt like anything else near.
        let due: Vec<Bomb> = self.bombs.iter().copied().filter(|bomb| self.tick > bomb.goes_off || !self.alive(bomb.target)).collect();
        let keep: Vec<Bomb> = self.bombs.iter().copied().filter(|bomb| self.tick <= bomb.goes_off && self.alive(bomb.target)).collect();
        self.bombs = keep;
        for bomb in due {
            if self.alive(bomb.target) {
                self.detonate_bomb(bomb);
            }
        }

        // Bites come every `ROF` of the
        // weapon; a soldier loses its whole health, a vehicle the weapon's
        // damage by its armour.
        for index in 0..self.units.len() {
            let Some(biter) = self.units[index].parasite.first() else {
                continue;
            };
            let Some((weapon_index, due)) = biter.bite else {
                continue;
            };
            if self.tick < due {
                continue;
            }
            let Some(weapon) = self.weapons.get(weapon_index as usize).cloned() else {
                continue;
            };
            let (host, on_foot, health, source) = (self.units[index].id, self.units[index].stats.infantry, self.units[index].health, Aim::Unit(biter.id));
            self.units[index].parasite[0].bite = Some((weapon_index, self.tick + weapon.rof.max(1) as u64));
            let damage = if on_foot { health } else { weapon.damage };
            // A vehicle sparks, shows the
            // weapon's animation for its direction, and is rocked from a point
            // two leptons to one side or the other.
            if !on_foot {
                let side = if self.random_below(2) == 1 { -2 } else { 2 };
                self.events.push(Event::Bitten { host, weapon: weapon_index, side });
            }
            self.receive_damage(Aim::Unit(host), damage, 0, weapon.warhead, Some(source));
        }

        // A victim that has gone is forgotten; a controller that has gone frees.
        let gone: Vec<u32> = self.units.iter().filter(|u| u.controls.is_some_and(|victim| self.unit(victim).is_none())).map(|u| u.id).collect();
        for id in gone {
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                unit.controls = None;
            }
        }
        let orphans: Vec<u32> = self.units.iter().filter(|u| u.controlled_by.is_some_and(|(controller, _)| self.unit(controller).is_none())).map(|u| u.id).collect();
        for id in orphans {
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                if let Some((_, home)) = unit.controlled_by.take() {
                    unit.owner = home;
                    unit.arms.target = None;
                }
            }
        }
    }

    fn detonate_bomb(&mut self, bomb: Bomb) {
        let Some(at) = self.aim_point(bomb.target) else {
            return;
        };
        let (damage, warhead) = (self.ability_rules.ivan_damage, self.ability_rules.ivan_warhead);
        let source = bomb.owner.filter(|&id| self.unit(id).is_some()).map(Aim::Unit);
        self.explode(at, damage, warhead, source);
        self.blast_event(warhead, damage, at, false);
    }

    /// What is done with those inside a unit as it dies: a bomb on it goes off,
    /// its parasite comes out, and its passengers step out if they can.
    // The bomb. The
    // parasite comes out where its host stood. Passengers of a vehicle that does not crash come out on
    // its cell if they could stand there, and scatter; otherwise they die.
    pub(crate) fn empty_the_dead(&mut self, mut dead: Unit, wet: bool) {
        let cell = dead.cell();
        if let Some(at) = self.bombs.iter().position(|bomb| bomb.target == Aim::Unit(dead.id)) {
            let bomb = self.bombs.remove(at);
            let point = (dead.x, dead.y, self.ground_z(dead.x, dead.y));
            let (damage, warhead) = (self.ability_rules.ivan_damage, self.ability_rules.ivan_warhead);
            self.explode(point, damage, warhead, bomb.owner.filter(|&id| self.unit(id).is_some()).map(Aim::Unit));
            self.blast_event(warhead, damage, point, false);
        }
        if let Some(controller) = dead.controlled_by.map(|(controller, _)| controller) {
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == controller) {
                unit.controls = None;
            }
        }
        if dead.controls.is_some() {
            let id = dead.id;
            // The dead unit is already off the list: free its victim directly.
            if let Some(unit) = self.units.iter_mut().find(|u| u.controlled_by.is_some_and(|(controller, _)| controller == id)) {
                if let Some((_, home)) = unit.controlled_by.take() {
                    unit.owner = home;
                    unit.arms.target = None;
                }
            }
        }
        if let Some(spawner) = dead.spawner.take() {
            self.orphan_spawns(&spawner);
        }
        // A type
        // with `Explodes`, a rank with EXPLODES, or one killed by its own
        // `Suicide` weapon takes its passengers with it and goes off: its
        // `DeathWeapon`, else its first weapon, at `DeathWeaponDamageModifier`
        // times the weapon's damage, where it stood.
        // None of this in the water.
        if !wet && (dead.stats.explodes || dead.has_ability(1 << 10) || dead.died_by_suicide) {
            dead.cargo.clear();
            let point = (dead.x, dead.y, self.ground_z(dead.x, dead.y));
            self.death_blast(point, dead.stats.death_weapon, dead.stats.death_percent, dead.stats.fighter.primary.map(|mount| mount.weapon), dead.stats.strength, Aim::Unit(dead.id));
        }
        let crashes = dead.stats.jet.is_some_and(|jet| jet.crashable) || dead.plane.is_some();
        // A parasite whose
        // host was lately hurt by somebody else goes with it.
        let tick = self.tick;
        dead.parasite.retain(|biter| tick >= biter.suppressed_until || biter.curtain_until > tick);
        let riders: Vec<Unit> = dead.parasite.drain(..).chain(if crashes { Vec::new() } else { std::mem::take(&mut dead.cargo) }).collect();
        let mut out = Vec::new();
        for mut rider in riders {
            let others: Vec<Other> = self.others();
            let free = entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &rider, cell, None, cell, -1) == code::OK;
            let Some(at) = self.map.index(cell.0, cell.1).filter(|_| free) else {
                continue;
            };
            rider.bite = None;
            if rider.stats.infantry {
                let Some(spot) = (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0) else {
                    continue;
                };
                rider.x = cell.0 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].0;
                rider.y = cell.1 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].1;
                self.spots[at] |= 1 << spot;
                self.spot_house[at] = rider.owner;
            } else {
                (rider.x, rider.y) = (dead.x, dead.y);
                self.claims[at] = rider.id + 1;
            }
            out.push(rider.id);
            self.put_back(rider);
        }
        for id in out {
            self.scatter_unit(id);
        }
    }
}
