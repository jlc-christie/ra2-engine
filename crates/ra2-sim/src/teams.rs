//! The computer player's attacks: AI triggers choose team types; a team gathers
//! its members (taking idle units, and having the rest built), then runs its
//! script; a house chooses its enemy and answers attacks on its base.
//!
//! Follows the original game and
//! (2.2 base centre, 2.3 enemy choice, 5.1 the base-attack response, part B
//! target choice).
//!
//! Not the original's structure: the engine's units carry missions (Move,
//! Attack, Guard, Area Guard, Rescue...) that the team code queues; here a
//! member is given the order that mission would amount to.

use crate::combat::{Aim, FireError};
use crate::{Factory, Order, Unit, World, LEPTONS_PER_CELL};

/// What an AI trigger counts: a building type or a unit type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thing {
    Building(u16),
    Unit(u16),
}

/// How an AI trigger has done in a game: its weight now, and its teams
/// that succeeded and that ended.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Score {
    pub weight: f64,
    pub successes: u32,
    pub completed: u32,
}

/// An `[AITriggerTypes]` entry.
#[derive(Clone, Debug)]
pub struct Trigger {
    pub team1: Option<usize>,
    pub team2: Option<usize>,
    /// `None`: never; `Some(None)`: any country; else that country.
    pub owner: Option<Option<u8>>,
    pub condition: i32,
    pub object: Option<Thing>,
    pub operand: i32,
    pub operator: i32,
    pub weight: f64,
    pub min_weight: f64,
    pub max_weight: f64,
    pub skirmish: bool,
    pub side: i32,
    /// Enabled for easy, medium, hard.
    pub enabled: [bool; 3],
    pub successes: u32,
    pub completed: u32,
}

/// A `[TeamTypes]` entry.
#[derive(Clone, Debug, Default)]
pub struct TeamType {
    pub name: String,
    pub max: i32,
    pub priority: i32,
    pub base_defense: bool,
    pub reinforce: bool,
    pub annoyance: bool,
    pub recruiter: bool,
    pub aggressive: bool,
    pub suicide: bool,
    pub only_target_house_enemy: bool,
    pub members_recruitable: bool,
    /// `TransportsReturnOnUnload`: a transport goes back to where it loaded
    /// once it has let its passengers out, and leaves the team.
    pub transports_return: bool,
    pub script: usize,
    pub task_force: usize,
    pub autocreate: bool,
}

/// Everything read from ai.ini, and the rules' numbers for it.
#[derive(Clone, Debug)]
pub struct Data {
    pub triggers: Vec<Trigger>,
    pub team_types: Vec<TeamType>,
    /// Each task force: how many of which unit type.
    pub task_forces: Vec<Vec<(u32, u16)>>,
    /// Each script: its lines, an action and its argument.
    pub scripts: Vec<Vec<(i32, i32)>>,
    /// By difficulty, hardest first.
    pub team_delays: [u32; 3],
    pub min_defensive: [i32; 3],
    pub max_defensive: [i32; 3],
    pub team_cap: [i32; 3],
    pub use_min_defense: bool,
    pub dissolve_delay: u32,
    pub success_delta: f64,
    pub failure_delta: f64,
    pub track_record: f64,
    /// `AISafeDistance` in cells; `Stray`, `RelaxedStray`, `CloseEnough` in leptons.
    pub safe_distance: i32,
    pub stray: i32,
    pub relaxed_stray: i32,
    pub close_enough: i32,
    pub enemy_bonus: f64,
    /// `ComputerBaseDefenseResponse`, and `BaseDefenseDelay` in frames.
    pub defense_response: i32,
    pub defense_delay: u32,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            triggers: Vec::new(),
            team_types: Vec::new(),
            task_forces: Vec::new(),
            scripts: Vec::new(),
            team_delays: [2000, 2500, 3500],
            min_defensive: [1; 3],
            max_defensive: [2; 3],
            team_cap: [30; 3],
            use_min_defense: true,
            dissolve_delay: 5000,
            success_delta: 20.0,
            failure_delta: -50.0,
            track_record: 1.0,
            safe_distance: 20,
            stray: 512,
            relaxed_stray: 768,
            close_enough: 576,
            enemy_bonus: 400.0,
            defense_response: 3,
            defense_delay: 225,
        }
    }
}

/// A team: units of one house gathered to run a script.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Team {
    pub id: u32,
    pub kind: usize,
    pub house: u8,
    /// The script line it is on; -1 before the first.
    pub line: i32,
    /// Its members, the newest first.
    pub members: Vec<u32>,
    pub created: u64,
    zone: Option<(i32, i32)>,
    pub target: Option<Aim>,
    focus: Option<Aim>,
    guard_until: u64,
    has_been: bool,
    full: bool,
    under: bool,
    reforming: bool,
    dirty: bool,
    pub moving: bool,
    pub(crate) next_mission: bool,
    success: bool,
}

impl Team {
    fn new(id: u32, kind: usize, house: u8, created: u64) -> Self {
        Self {
            id,
            kind,
            house,
            line: -1,
            members: Vec::new(),
            created,
            zone: None,
            target: None,
            focus: None,
            guard_until: 0,
            has_been: false,
            full: false,
            under: true,
            reforming: false,
            dirty: true,
            moving: false,
            next_mission: true,
            success: false,
        }
    }

    /// Where the team is sent while it is not running its script.
    pub fn focus(&self) -> Option<Aim> {
        self.focus
    }
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i64 {
    let (dx, dy) = ((a.0 - b.0) as f64, (a.1 - b.1) as f64);
    dx.hypot(dy) as i64
}

fn centre_of(cell: (i32, i32)) -> (i32, i32) {
    (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128)
}

impl World {
    fn class_of(unit: &crate::UnitStats) -> u8 {
        if unit.infantry {
            0
        } else if unit.plane.is_some() {
            2
        } else {
            1
        }
    }

    fn team_point(&self, who: Aim) -> Option<(i32, i32)> {
        self.aim_point(who).map(|point| (point.0, point.1))
    }

    /// A house with nothing left.
    pub fn defeated(&self, house: u8) -> bool {
        !self.buildings.iter().any(|b| b.owner == house) && !self.units.iter().any(|u| u.owner == house)
    }

    // ---------------------------------------------------------------- houses

    /// The centre of a house's base.
    // The middle of its buildings,
    // each weighted by a thousandth of its cost plus one. Not original
    // behaviour: worked out when read, not at each placement and loss.
    pub(crate) fn recalc_centres(&mut self) {
        for house in 0..self.houses.len() {
            let (mut total, mut sx, mut sy) = (0i64, 0i64, 0i64);
            for building in self.buildings.iter().filter(|b| b.owner as usize == house && b.health > 0) {
                let weight = self.building_types.get(building.kind as usize).map_or(0, |kind| kind.cost) as i64 / 1000 + 1;
                total += weight;
                sx += weight * (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128) as i64;
                sy += weight * (building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128) as i64;
            }
            if total > 0 {
                let cell = ((sx / total) as i32 / LEPTONS_PER_CELL, (sy / total) as i32 / LEPTONS_PER_CELL);
                self.houses[house].base_centre = centre_of(cell);
            }
        }
    }

    /// Adds to a house's anger at another and picks its enemy again.
    // The angriest entry above nothing whose house
    // is still in the game and no ally; the earlier house on a tie.
    pub(crate) fn adjust_anger(&mut self, house: u8, delta: i32, at: u8) {
        let count = self.houses.len();
        let Some(mut brain) = self.houses.get(house as usize).and_then(|h| h.ai.clone()) else {
            return;
        };
        brain.anger.resize(count, 0);
        if let Some(entry) = brain.anger.get_mut(at as usize) {
            *entry += delta;
        }
        let mut best = 0;
        brain.enemy = None;
        for other in 0..count as u8 {
            let anger = brain.anger[other as usize];
            if anger > best && other != house && !self.defeated(other) && !self.allied(house, other) {
                best = anger;
                brain.enemy = Some(other);
            }
        }
        self.houses[house as usize].ai = Some(brain);
    }

    /// What a hit does to the victim's house: it grows angrier at the attacker,
    /// and if a building was hit its defenders answer.
    // The anger added is the share of the victim's
    // strength taken, times its cost.
    pub(crate) fn note_attack(&mut self, victim: Aim, damage: i32, source: Option<Aim>) {
        let Some(attacker) = source else {
            return;
        };
        let (Some(owner), Some(by)) = (self.owner_of(victim), self.owner_of(attacker)) else {
            return;
        };
        if self.houses.get(owner as usize).is_none_or(|h| h.ai.is_none()) {
            return;
        }
        let (strength, cost) = match victim {
            Aim::Unit(id) => self.unit(id).map_or((1, 0), |unit| {
                (unit.stats.strength.max(1), (unit.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).map_or(0, |t| t.cost))
            }),
            Aim::Building(id) => self.buildings.iter().find(|b| b.id == id).and_then(|b| self.building_types.get(b.kind as usize)).map_or((1, 0), |kind| (kind.strength.max(1), kind.cost)),
            Aim::Cell(..) => return,
        };
        if by != owner {
            self.adjust_anger(owner, (damage as f64 / strength as f64 * cost as f64) as i32, by);
        }
        if let (Aim::Building(building), Aim::Unit(unit)) = (victim, attacker) {
            if let Some(brain) = self.houses[owner as usize].ai.as_mut() {
                brain.last_attack = self.tick;
            }
            self.base_attacked(building, unit);
        }
    }

    /// The house's units answer an attack on one of its buildings.
    // Behaves as the original does, including its list of six.
    fn base_attacked(&mut self, building: u32, attacker: u32) {
        let Some(victim) = self.buildings.iter().find(|b| b.id == building).cloned() else {
            return;
        };
        let Some(a) = self.unit(attacker).cloned() else {
            return;
        };
        let house = victim.owner;
        if self.allied(house, a.owner) || self.human(house) || a.plane.is_some() || a.rescue_until > self.tick {
            return;
        }
        if self.building_types.get(victim.kind as usize).is_some_and(|kind| kind.fighter.insignificant) {
            return;
        }
        let threat = |unit: &Unit| unit.stats.fighter.threat_posed;
        let mut budget = threat(&a) * self.team_data.defense_response;
        let rating = |world: &World, unit: &Unit| -> i32 {
            if unit.arms.target == Some(Aim::Unit(attacker)) {
                return -threat(unit);
            }
            if unit.arms.target.is_some_and(|target| world.fighter(target).is_some_and(|f| f.primary.is_some())) {
                return 0;
            }
            if unit.team.is_some_and(|team| !world.team_type_of(team).is_some_and(|t| t.base_defense)) {
                return 0;
            }
            if unit.stats.harvester || threat(unit) == 0 {
                return 0;
            }
            let range = unit.stats.fighter.primary.and_then(|m| world.weapons.get(m.weapon as usize)).map_or(0, |w| w.range) as i64;
            let d = distance((unit.x, unit.y), (a.x, a.y)) - range;
            let mut r = threat(unit) as i64 * 1024;
            if d > 0 {
                let time = d / (unit.speed.max(1) as i64);
                r = (r / time.max(1)).max(1);
            }
            r as i32
        };
        let armour = a.stats.fighter.armour.min(10) as usize;
        let mut slots: Vec<(u32, i32)> = Vec::new();
        let mut least = 0;
        for class in [0u8, 1] {
            if budget <= 0 {
                break;
            }
            let candidates: Vec<Unit> = self.units.iter().filter(|u| u.owner == house && Self::class_of(&u.stats) == class).cloned().collect();
            for unit in candidates {
                if class == 0 && budget <= 0 {
                    break;
                }
                let free = unit.team.is_none_or(|team| self.team_type_of(team).is_some_and(|t| t.base_defense));
                let fighter = unit.stats.fighter;
                let which = self.select_weapon(&fighter, Aim::Unit(attacker));
                let hurts = Self::mount(&fighter, which)
                    .and_then(|m| self.weapons.get(m.weapon as usize))
                    .and_then(|w| self.warheads.get(w.warhead as usize))
                    .is_some_and(|w| w.verses[armour] != 0.0);
                if !(free && unit.recruitable && fighter.primary.is_some() && hurts && unit.health > 0) {
                    continue;
                }
                let r = rating(self, &unit);
                if r == 0 {
                    continue;
                }
                if r < 0 {
                    budget += r;
                } else if slots.len() < 6 {
                    slots.push((unit.id, r));
                } else if r > least {
                    let mut new_least = r;
                    for slot in &mut slots {
                        if slot.1 == least {
                            *slot = (unit.id, r);
                        } else if slot.1 < new_least {
                            new_least = slot.1;
                        }
                    }
                    least = new_least;
                }
            }
        }
        if budget <= 0 {
            return;
        }
        for i in 0..slots.len() {
            for j in i + 1..slots.len() {
                if slots[i].1 < slots[j].1 {
                    slots.swap(i, j);
                }
            }
        }
        let mut answered = 0;
        for (id, _) in slots {
            // The draw decides Rescue or Area Guard: both go for the attacker.
            self.random_below(100);
            self.order(Order::Attack { unit: id, target: Aim::Unit(attacker) });
            answered += self.unit(id).map_or(0, threat);
            if answered > budget {
                break;
            }
        }
        if answered > budget {
            let until = self.tick + self.team_data.defense_delay as u64;
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == attacker) {
                unit.rescue_until = until;
            }
        }
    }

    /// The house's periodic look at how things stand.
    // With no enemy it takes the
    // house whose base is nearest; one whose enemy is beaten forgets it. It
    // comes round every 106 to 112 frames.
    pub(crate) fn expert_ai(&mut self, house: u8) {
        let h = house as usize;
        let Some(brain) = self.houses[h].ai.clone() else {
            return;
        };
        if self.tick < brain.expert_at {
            return;
        }
        if brain.enemy.is_none() && self.houses[h].base_centre != (0, 0) {
            let mine = self.houses[h].base_centre;
            // Not original behaviour: the neutral house (0) stands for every
            // passive one, and houses with no base yet are skipped.
            let nearest = (1..self.houses.len() as u8)
                .filter(|&other| other != house && !self.allied(house, other) && !self.defeated(other) && self.houses[other as usize].base_centre != (0, 0))
                .min_by_key(|&other| distance(mine, self.houses[other as usize].base_centre));
            if let Some(other) = nearest {
                self.adjust_anger(house, 1, other);
            }
        }
        if let Some(enemy) = self.houses[h].ai.as_ref().and_then(|b| b.enemy) {
            if self.defeated(enemy) {
                let anger = self.houses[h].ai.as_ref().and_then(|b| b.anger.get(enemy as usize).copied()).unwrap_or(0);
                self.adjust_anger(house, -anger, enemy);
            }
        }
        // A
        // house that still has buildings, none of them a factory of any kind,
        // and has not been hit for 900 frames sells everything and sends all it
        // has to hunt.
        let mine: Vec<(u32, bool)> = self.buildings.iter().filter(|b| b.owner == house && b.health > 0).map(|b| (b.id, self.building_types.get(b.kind as usize).is_some_and(|k| k.factory.is_some()))).collect();
        let quiet = brain.last_attack + 900 < self.tick;
        if (quiet && !mine.is_empty() && !mine.iter().any(|&(_, factory)| factory)) || brain.all_hunt {
            for &(building, _) in &mine {
                self.order(Order::Sell { building });
            }
            if let Some(brain) = self.houses[h].ai.as_mut() {
                brain.all_hunt = true;
            }
            self.all_to_hunt(house);
        }
        if self.check_raise_money(house) {
            self.raise_money(house);
        }
        self.super_weapon_ai(house);
        let wait = self.random_below(7) + 1 + 105;
        if let Some(brain) = self.houses[h].ai.as_mut() {
            brain.expert_at = self.tick + wait as u64;
        }
    }

    /// A computer house fires the super weapons it has ready.
    // On its
    // turn to take stock, each ready one in list order: the missile and the
    // storm (one storm at a time) at the most valuable thing of its enemy's on
    // the ground, ties broken at random; the paradrops near the enemy's base,
    // or its own with no enemy; the curtain and the chronosphere are left to
    // the teams' scripts.
    fn super_weapon_ai(&mut self, house: u8) {
        let h = house as usize;
        let Some((enemy, difficulty)) = self.houses[h].ai.as_ref().map(|brain| (brain.enemy, brain.difficulty.min(2))) else {
            return;
        };
        for index in 0..self.super_types.len() {
            if !self.houses[h].supers.get(index).is_some_and(|state| state.can_fire()) {
                continue;
            }
            let cell = match self.super_types[index].kind {
                0 | 2 => {
                    if self.super_types[index].kind == 2 && self.storm.as_ref().is_some_and(|storm| storm.started.is_some()) {
                        continue;
                    }
                    let Some(enemy) = enemy else {
                        continue;
                    };
                    let values = self.ai_rules.target_values.map(|list| list[difficulty]);
                    let mut found: Vec<(i32, (i32, i32))> = Vec::new();
                    for building in self.buildings.iter().filter(|b| b.owner == enemy && b.health > 0) {
                        let Some(kind) = self.building_types.get(building.kind as usize) else {
                            continue;
                        };
                        let value = if kind.construction_yard {
                            values[0]
                        } else if kind.factory == Some(Factory::Unit) {
                            values[1]
                        } else if kind.power > 0 {
                            values[2]
                        } else if self.is_defence(building.kind) {
                            values[8]
                        } else {
                            4
                        };
                        found.push((value, (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2)));
                    }
                    for unit in self.units.iter().filter(|u| u.owner == enemy && u.health > 0 && self.altitude(u) == 0) {
                        let passengers = self.unit_types.get((unit.stats.kind as usize).wrapping_sub(1)).map_or(0, |t| t.passengers);
                        let value = if unit.plane.is_some() {
                            1
                        } else if unit.stats.infantry {
                            if unit.stats.fighter.engineer { values[3] } else { 2 }
                        } else if unit.stats.harvester {
                            values[5]
                        } else if unit.stats.deploys_into.is_some_and(|kind| self.ai_rules.build_const.contains(&kind)) {
                            values[6]
                        } else if passengers > 0 {
                            values[7]
                        } else {
                            2
                        };
                        found.push((value, unit.cell()));
                    }
                    let Some(best) = found.iter().map(|(value, _)| *value).max() else {
                        continue;
                    };
                    let ties: Vec<(i32, i32)> = found.iter().filter(|(value, _)| *value == best).map(|(_, cell)| *cell).collect();
                    let pick = if ties.len() > 1 { self.random_below(ties.len() as u32) as usize } else { 0 };
                    ties[pick]
                }
                5 | 6 => {
                    let centre = self.houses[enemy.unwrap_or(house) as usize].base_centre;
                    let near = self.near_ground((centre.0 / LEPTONS_PER_CELL, centre.1 / LEPTONS_PER_CELL));
                    (near.0 + 2, near.1 + 2)
                }
                _ => continue,
            };
            self.fire_super(house, index, cell);
        }
    }

    /// The harvester a house's country builds, and the refinery it would build.
    fn economy_types(&self, house: u8) -> (Option<u16>, Option<u16>) {
        let bit = 1u32 << self.houses[house as usize].country.min(31);
        let harvester = self.ai_rules.harvesters.iter().copied().find(|&unit| self.unit_types.get(unit as usize).is_some_and(|t| t.owners & bit != 0));
        let refinery = self.ai_rules.build_refinery.iter().copied().find(|&kind| self.building_types.get(kind as usize).is_some_and(|t| t.owners & bit != 0));
        (harvester, refinery)
    }

    fn has_economy(&self, house: u8) -> bool {
        let money = self.houses[house as usize].credits;
        let (harvester, refinery) = self.economy_types(house);
        let cost_of = |kind: Option<u16>| kind.and_then(|k| self.building_types.get(k as usize)).map_or(0, |t| t.cost);
        let harvester_cost = harvester.and_then(|k| self.unit_types.get(k as usize)).map_or(0, |t| t.cost);
        let owns = |list: &[u16]| self.buildings.iter().any(|b| b.owner == house && list.contains(&b.kind));
        if !owns(&self.ai_rules.build_refinery) {
            return money > cost_of(refinery);
        }
        if self.units.iter().any(|u| u.owner == house && u.stats.harvester) {
            return true;
        }
        let weapons_cost = cost_of(self.ai_rules.build_weapons.first().copied());
        (owns(&self.ai_rules.build_weapons) && money >= harvester_cost) || money >= harvester_cost + weapons_cost || money >= cost_of(refinery)
    }

    // With no economy, money must
    // be raised unless what would restore it is already in hand and affordable.
    // Not original behaviour: nothing more is sold while a sale is under way
    // (the engine's own guard against selling twice was not found).
    fn check_raise_money(&self, house: u8) -> bool {
        if self.has_economy(house) || self.buildings.iter().any(|b| b.owner == house && b.buildup.is_some_and(|up| up.selling)) {
            return false;
        }
        let h = &self.houses[house as usize];
        let Some(brain) = h.ai.as_ref() else {
            return false;
        };
        let (harvester, _) = self.economy_types(house);
        let refineries: Vec<&crate::Building> = self.buildings.iter().filter(|b| b.owner == house && self.ai_rules.build_refinery.contains(&b.kind)).collect();
        if !refineries.is_empty() {
            if refineries.iter().any(|b| b.buildup.is_some()) {
                return false;
            }
            // (Here the choice is handed to the factory at once, so a
            // harvester in the factory counts as the choice.)
            let chosen = brain.build_unit == harvester || h.vehicles.is_some_and(|p| Some(p.building) == harvester);
            if !chosen || harvester.is_none() {
                return h.credits < harvester.and_then(|k| self.unit_types.get(k as usize)).map_or(0, |t| t.cost);
            }
            h.vehicles.filter(|p| Some(p.building) == harvester).is_none_or(|p| h.credits < p.balance)
        } else {
            let wanted = brain.build_structure.filter(|kind| self.ai_rules.build_refinery.contains(kind));
            let Some(kind) = wanted else {
                return h.credits < self.ai_rules.build_refinery.first().and_then(|&k| self.building_types.get(k as usize)).map_or(0, |t| t.cost);
            };
            h.production.filter(|p| p.building == kind).is_none_or(|p| h.credits < p.balance)
        }
    }

    // Buildings are sold from the
    // end of the base plan backwards until what they bring covers a harvester
    // (with a refinery and a war factory standing) or a refinery; then every
    // production is dropped and the harvester, or a refinery put first among
    // what is still to build, is all that is asked for. With nothing left to
    // sell, everything goes to hunt.
    fn raise_money(&mut self, house: u8) {
        let h = house as usize;
        let Some(brain) = self.houses[h].ai.clone() else {
            return;
        };
        let (harvester, refinery) = self.economy_types(house);
        let owns = |world: &World, list: &[u16]| world.buildings.iter().any(|b| b.owner == house && list.contains(&b.kind));
        let need_harvester = owns(self, &self.ai_rules.build_refinery) && owns(self, &self.ai_rules.build_weapons.iter().copied().take(2).collect::<Vec<_>>());
        let goal = if need_harvester {
            harvester.and_then(|k| self.unit_types.get(k as usize)).map_or(0, |t| t.cost)
        } else {
            refinery.and_then(|k| self.building_types.get(k as usize)).map_or(0, |t| t.cost)
        };
        let mut raised = 0;
        let mut reached = false;
        for node in brain.nodes.iter().rev() {
            let standing = match (node.kind, node.cell) {
                (Some(kind), Some(cell)) => self.buildings.iter().find(|b| b.owner == house && b.kind == kind && b.cell == cell && b.buildup.is_none()).map(|b| (b.id, kind)),
                _ => None,
            };
            if let Some((building, kind)) = standing {
                raised += self.building_types.get(kind as usize).map_or(0, |t| (t.cost as f64 * self.refund_percent) as i32);
                self.order(Order::Sell { building });
            }
            if self.houses[h].credits + raised > goal {
                reached = true;
                break;
            }
        }
        if !reached {
            if let Some(brain) = self.houses[h].ai.as_mut() {
                brain.all_hunt = true;
            }
            self.all_to_hunt(house);
            return;
        }
        // What was in hand is abandoned, and what was paid for it comes back.
        for tab in 0..6 {
            if let Some(production) = self.houses[h].tab_slot(tab) {
                let cost = if tab < 2 { self.building_types.get(production.building as usize).map_or(0, |t| t.cost) } else { self.unit_types.get(production.building as usize).map_or(0, |t| t.cost) };
                self.houses[h].credits += (cost - production.balance).max(0);
                *self.houses[h].tab_slot_mut(tab) = None;
            }
        }
        let house_state = &mut self.houses[h];
        house_state.infantry_queue.clear();
        house_state.vehicle_queue.clear();
        house_state.ship_queue.clear();
        house_state.aircraft_queue.clear();
        let built: Vec<bool> = brain.nodes.iter().map(|node| matches!((node.kind, node.cell), (Some(kind), Some(cell)) if self.buildings.iter().any(|b| b.owner == house && b.kind == kind && b.cell == cell))).collect();
        let Some(brain) = self.houses[h].ai.as_mut() else {
            return;
        };
        brain.build_structure = None;
        brain.build_unit = None;
        brain.build_infantry = None;
        brain.build_aircraft = None;
        if need_harvester {
            brain.build_unit = harvester;
            brain.mode = 2;
        } else if let Some(refinery) = refinery {
            brain.mode = 1;
            let at = built.iter().position(|done| !done).unwrap_or(built.len()).max(1);
            let mut index = 0;
            brain.nodes.retain(|node| {
                index += 1;
                index <= at || node.kind != Some(refinery)
            });
            brain.nodes.insert(at.min(brain.nodes.len()), crate::ai::Node::wanted(refinery));
        }
    }

    /// Everything a house has leaves its team and goes after the enemy.
    // Not original behaviour: the engine gives each the Hunt mission; here one
    // without a target is sent against the nearest thing of its house's enemy
    // (or of anyone else), again each time the house takes stock.
    fn all_to_hunt(&mut self, house: u8) {
        let enemy = self.houses[house as usize].ai.as_ref().and_then(|b| b.enemy);
        let hunters: Vec<(u32, (i32, i32))> = self
            .units
            .iter()
            .filter(|u| u.owner == house && u.health > 0 && u.stats.fighter.primary.is_some() && !u.stats.harvester && u.arms.target.is_none())
            .map(|u| (u.id, (u.x, u.y)))
            .collect();
        for unit in self.units.iter_mut().filter(|u| u.owner == house) {
            unit.team = None;
        }
        let hostile = |owner: u8| owner != 0 && owner != house && enemy.is_none_or(|enemy| owner == enemy);
        let quarry: Vec<(Aim, (i32, i32))> = self
            .buildings
            .iter()
            .filter(|b| hostile(b.owner) && b.health > 0)
            .map(|b| (Aim::Building(b.id), (b.cell.0 * LEPTONS_PER_CELL, b.cell.1 * LEPTONS_PER_CELL)))
            .chain(self.units.iter().filter(|u| hostile(u.owner) && u.health > 0 && self.altitude(u) == 0).map(|u| (Aim::Unit(u.id), (u.x, u.y))))
            .collect();
        for (unit, at) in hunters {
            if let Some(&(target, _)) = quarry.iter().min_by_key(|(_, point)| distance(at, *point)) {
                self.order(Order::Attack { unit, target });
            }
        }
    }

    // -------------------------------------------------------------- triggers

    fn count_of(&self, house: u8, thing: Option<Thing>) -> i32 {
        match thing {
            Some(Thing::Building(kind)) => self.buildings.iter().filter(|b| b.owner == house && b.kind == kind).count() as i32,
            Some(Thing::Unit(kind)) => self.units.iter().filter(|u| u.owner == house && u.stats.kind == kind + 1).count() as i32,
            None => 0,
        }
    }

    fn team_type_of(&self, team: u32) -> Option<&TeamType> {
        self.teams.iter().find(|t| t.id == team).and_then(|t| self.team_data.team_types.get(t.kind))
    }

    fn count_teams(&self, house: u8, kind: usize) -> i32 {
        self.teams.iter().filter(|t| t.house == house && t.kind == kind).count() as i32
    }

    // The highest `TechLevel` among the members;
    // one that cannot be built at all asks for 11.
    fn task_force_tech(&self, kind: Option<usize>) -> i32 {
        let Some(force) = kind.and_then(|k| self.team_data.team_types.get(k)).and_then(|t| self.team_data.task_forces.get(t.task_force)) else {
            return 0;
        };
        let mut tech = 0;
        for &(_, unit) in force {
            let level = self.unit_types.get(unit as usize).map_or(-1, |t| t.tech_level);
            if level > tech {
                tech = level;
            } else if level == -1 {
                tech = 11;
            }
        }
        tech
    }

    // Every member type needs a factory of the
    // house's that can build it.
    fn can_build_team(&self, house: u8, kind: usize, buildable: &[u16]) -> bool {
        let _ = house;
        self.team_data.team_types.get(kind).and_then(|t| self.team_data.task_forces.get(t.task_force)).is_some_and(|force| force.iter().all(|(_, unit)| buildable.contains(unit)))
    }

    // A team that is not for defence
    // needs its base and the enemy's joined by ground.
    fn team_zone_ok(&self, house: u8, enemy: Option<u8>, kind: usize) -> bool {
        let Some(team) = self.team_data.team_types.get(kind) else {
            return false;
        };
        let Some(force) = self.team_data.task_forces.get(team.task_force) else {
            return false;
        };
        let naval = force.iter().any(|(_, unit)| self.unit_types.get(*unit as usize).is_some_and(|t| t.naval));
        let flies = force.iter().all(|(_, unit)| self.unit_types.get(*unit as usize).is_some_and(|t| t.stats.plane.is_some() || t.stats.jet.is_some()));
        let (Some(enemy), false) = (enemy, team.base_defense) else {
            return true;
        };
        if naval {
            return false;
        }
        if flies {
            return true;
        }
        let Some(zones) = self.map.zones() else {
            return true;
        };
        let cell = |h: u8| (self.houses[h as usize].base_centre.0 / LEPTONS_PER_CELL, self.houses[h as usize].base_centre.1 / LEPTONS_PER_CELL);
        let (mine, theirs) = (self.near_ground(cell(house)), self.near_ground(cell(enemy)));
        zones.zone_of(mine, 0) == zones.zone_of(theirs, 0)
    }

    /// The nearest cell to one that ground vehicles can stand on.
    fn near_ground(&self, cell: (i32, i32)) -> (i32, i32) {
        for ring in 0..12i32 {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    let at = (cell.0 + dx, cell.1 + dy);
                    if dx.abs().max(dy.abs()) == ring
                        && self.map.in_playfield(at.0, at.1)
                        && self.map.speed_percent(at.0, at.1, crate::SpeedType::Track) > 0
                        && self.building_on(at).is_none()
                    {
                        return at;
                    }
                }
            }
        }
        cell
    }

    // In this order.
    fn trigger_eligible(&self, index: usize, house: u8, enemy: Option<u8>, no_defense: bool, buildable: &[u16]) -> bool {
        let data = &self.team_data;
        let trigger = &data.triggers[index];
        let Some(team1) = trigger.team1 else {
            return false;
        };
        let brain_difficulty = self.houses[house as usize].ai.as_ref().map_or(2, |b| b.difficulty).min(2);
        let is_def = |kind: Option<usize>| kind.and_then(|k| data.team_types.get(k)).is_some_and(|t| t.base_defense);
        let defensive = is_def(Some(team1)) && (trigger.team2.is_none() || is_def(trigger.team2));
        let have = self.teams.iter().filter(|t| t.house == house && data.team_types.get(t.kind).is_some_and(|k| k.base_defense)).count() as i32;
        let need_defense = enemy.is_none() || (data.use_min_defense && have < data.min_defensive[brain_difficulty]);
        if need_defense {
            if !defensive || no_defense {
                return false;
            }
        } else if defensive && no_defense {
            return false;
        }
        // Difficulty 0 is the hardest: it needs the "hard" flag.
        if !trigger.skirmish || !trigger.enabled[2 - brain_difficulty] {
            return false;
        }
        let owner = &self.houses[house as usize];
        match trigger.owner {
            None => return false,
            Some(Some(country)) if country != owner.country => return false,
            _ => {}
        }
        if (trigger.side == 1 && owner.side != 0) || (trigger.side == 2 && owner.side != 1) {
            return false;
        }
        if owner.tech_level < self.task_force_tech(Some(team1)).max(self.task_force_tech(trigger.team2)).max(0) {
            return false;
        }
        let compare = |count: i32| match trigger.operator {
            0 => count < trigger.operand,
            1 => count <= trigger.operand,
            2 => count == trigger.operand,
            3 => count >= trigger.operand,
            4 => count > trigger.operand,
            5 => count != trigger.operand,
            _ => false,
        };
        let power_gap = |h: u8| {
            let (output, drain) = self.power(h);
            output - drain
        };
        let met = match (trigger.condition, enemy) {
            (-1, _) => true,
            (1, _) => compare(self.count_of(house, trigger.object)),
            (_, None) => false,
            (0, Some(enemy)) => compare(self.count_of(enemy, trigger.object)),
            (2, Some(enemy)) => power_gap(enemy) < 100,
            (3, Some(enemy)) => power_gap(enemy) < 0,
            (4, Some(enemy)) => compare(self.houses[enemy as usize].credits),
            // Iron curtain or chronosphere nearly ready.
            (5 | 6, _) => {
                let wanted = if trigger.condition == 5 { 1 } else { 3 };
                self.super_types.iter().position(|t| t.kind == wanted).is_some_and(|at| {
                    owner.supers.get(at).is_some_and(|state| state.granted && (state.ready || state.left as f64 / self.super_types[at].recharge.max(1) as f64 <= 0.3))
                })
            }
            (7, _) => compare(self.count_of(0, trigger.object)),
            _ => false,
        };
        if !met {
            return false;
        }
        for kind in [Some(team1), trigger.team2].into_iter().flatten() {
            if !self.team_zone_ok(house, enemy, kind) || !self.can_build_team(house, kind, buildable) {
                return false;
            }
            let max = data.team_types.get(kind).map_or(-1, |t| t.max);
            if max >= 0 && self.count_teams(house, kind) >= max {
                return false;
            }
        }
        true
    }

    /// The house's team check: perhaps a trigger starts a team or two.
    pub(crate) fn team_check(&mut self, house: u8) {
        let h = house as usize;
        let Some(brain) = self.houses[h].ai.clone() else {
            return;
        };
        let difficulty = brain.difficulty.min(2);
        let delay = self.team_data.team_delays[difficulty] as u64;
        // The timer is first
        // set to `TeamDelays` and 175 frames more for each house before it.
        if !brain.timer_armed {
            if let Some(brain) = self.houses[h].ai.as_mut() {
                brain.timer_armed = true;
                brain.team_timer = self.tick + delay + 175 * (h as u64).saturating_sub(1);
            }
            return;
        }
        if self.tick < brain.team_timer {
            return;
        }
        if let Some(brain) = self.houses[h].ai.as_mut() {
            brain.team_timer = self.tick + delay;
        }
        let roll = self.random_below(100) + 1;
        let mut out: Vec<usize> = Vec::new();
        if roll <= 100 && brain.triggers_active {
            let is_def = |world: &World, team: &Team| world.team_data.team_types.get(team.kind).is_some_and(|t| t.base_defense);
            let mut total = self.teams.iter().filter(|t| t.house == house).count() as i32;
            let defensive = self.teams.iter().filter(|t| t.house == house && is_def(self, t)).count() as i32;
            let cap = self.team_data.team_cap[difficulty];
            let mut no_defense = false;
            if total >= cap && defensive >= total / 2 {
                let oldest = self.teams.iter().filter(|t| t.house == house && is_def(self, t)).min_by_key(|t| t.created).map(|t| t.id);
                if let Some(id) = oldest {
                    total -= 1;
                    no_defense = true;
                    self.delete_team(id);
                }
            } else if defensive > self.team_data.max_defensive[difficulty] {
                no_defense = true;
            }
            if total < cap {
                let buildable = self.buildable_units(house);
                self.start_scores();
                let mut candidates: Vec<(usize, i32)> = Vec::new();
                let (mut sum, mut infinite) = (0, false);
                for index in 0..self.team_data.triggers.len() {
                    if !self.trigger_eligible(index, house, brain.enemy, no_defense, &buildable) {
                        continue;
                    }
                    let weight = self.trigger_scores[index].weight as i32;
                    if weight == 5000 {
                        if !infinite {
                            infinite = true;
                            candidates.clear();
                            sum = 0;
                        }
                    } else if infinite {
                        continue;
                    }
                    candidates.push((index, weight));
                    sum += weight;
                }
                if sum >= 1 && !candidates.is_empty() {
                    let pick = self.random_below(sum as u32) as i32 + 1;
                    let mut acc = 0;
                    for (index, weight) in candidates {
                        acc += weight;
                        if acc >= pick {
                            let trigger = &self.team_data.triggers[index];
                            out.extend(trigger.team1);
                            out.extend(trigger.team2);
                            break;
                        }
                    }
                }
            }
        }
        // A type still forming is not started again.
        if self.teams.iter().any(|t| t.house == house && (t.reforming || !t.moving) && out.contains(&t.kind)) {
            return;
        }
        for kind in out {
            if let Some(team) = self.team_data.team_types.get_mut(kind) {
                team.autocreate = true;
            }
            let max = self.team_data.team_types.get(kind).map_or(-1, |t| t.max);
            if max >= 0 && self.count_teams(house, kind) >= max {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            self.teams.push(Team::new(id, kind, house, self.tick));
        }
    }

    /// Gives each AI trigger its starting weight, the first time it is asked.
    fn start_scores(&mut self) {
        if self.trigger_scores.len() != self.team_data.triggers.len() {
            self.trigger_scores = self.team_data.triggers.iter().map(|t| Score { weight: t.weight, successes: t.successes, completed: t.completed }).collect();
        }
    }

    /// Ends a team: its members are free again and its triggers are scored.
    fn delete_team(&mut self, id: u32) {
        let Some(at) = self.teams.iter().position(|t| t.id == id) else {
            return;
        };
        let team = self.teams.remove(at);
        for unit in self.units.iter_mut().filter(|u| u.team == Some(id)) {
            unit.team = None;
        }
        self.start_scores();
        let data = &self.team_data;
        for (trigger, score) in data.triggers.iter().zip(self.trigger_scores.iter_mut()).filter(|(t, _)| t.team1 == Some(team.kind)) {
            let (min_weight, max_weight) = (trigger.min_weight, trigger.max_weight);
            let trigger = score;
            let (s, n) = (trigger.successes as f64, trigger.completed as f64);
            let record = if n > 0.0 { n * (s / n - 0.5) } else { 0.0 };
            if team.success {
                trigger.weight += data.success_delta + record.max(0.0);
                trigger.successes += 1;
            } else {
                trigger.weight += data.failure_delta + (record * data.track_record).min(0.0);
            }
            if trigger.weight < min_weight {
                trigger.weight = min_weight;
            }
            if trigger.weight > max_weight {
                trigger.weight = max_weight;
            }
            trigger.completed += 1;
        }
    }

    // ------------------------------------------------------------ production

    /// What each of the house's forming teams still lacks, by unit type, with
    /// the age of the oldest team wanting it.
    fn team_needs(&self, house: u8, class: u8) -> Vec<(u16, i32, u64)> {
        let mut needs: Vec<(u16, i32, u64)> = Vec::new();
        for team in self.teams.iter().filter(|t| t.house == house) {
            let Some(kind) = self.team_data.team_types.get(team.kind) else {
                continue;
            };
            if !((kind.reinforce && !team.full) || !team.has_been) {
                continue;
            }
            let Some(force) = self.team_data.task_forces.get(kind.task_force) else {
                continue;
            };
            for &(count, unit) in force {
                if self.unit_types.get(unit as usize).is_none_or(|t| Self::class_of(&t.stats) != class) {
                    continue;
                }
                let held = team.members.iter().filter(|&&id| self.unit(id).is_some_and(|u| u.stats.kind == unit + 1)).count() as i32;
                let missing = count as i32 - held;
                if missing <= 0 {
                    continue;
                }
                match needs.iter_mut().find(|need| need.0 == unit) {
                    Some(need) => {
                        need.1 += missing;
                        need.2 = need.2.min(team.created);
                    }
                    None => needs.push((unit, missing, team.created)),
                }
            }
        }
        // Idle units of the type, in no team, are as good as built.
        for unit in self.units.iter().filter(|u| u.owner == house && u.team.is_none() && self.recruit_ready(u)) {
            if let Some(need) = needs.iter_mut().find(|need| need.0 + 1 == unit.stats.kind) {
                need.1 -= 1;
            }
        }
        needs.retain(|need| need.1 > 0);
        needs
    }

    /// The unit type the house should build next of a class (0 soldiers, 1
    /// vehicles, 2 aircraft) for its teams: a missing member of the oldest
    /// unfilled team, the lowest type first.
    // With the stock
    // `FillEarliestTeamProbability` of 100; the random number is drawn anyway.
    pub(crate) fn team_build_choice(&mut self, house: u8, class: u8) -> Option<u16> {
        let buildable = self.buildable_units(house);
        let credits = self.houses[house as usize].credits;
        let mut needs = self.team_needs(house, class);
        needs.retain(|need| buildable.contains(&need.0) && self.unit_types.get(need.0 as usize).is_some_and(|t| t.cost <= credits));
        needs.sort_by_key(|need| need.0);
        self.random_below(0x7fff_ffff);
        let mut pick: Option<(u16, u64)> = None;
        for (unit, _, created) in needs {
            if pick.is_none_or(|(_, earliest)| created < earliest) {
                pick = Some((unit, created));
            }
        }
        pick.map(|(unit, _)| unit)
    }

    // --------------------------------------------------------------- members

    /// Doing nothing a team could not interrupt.
    // A free recruit is on a mission
    // that lets it be taken, which a unit still being let out of its factory
    // is not (it is under the factory's radio contact). Recruiting it put the
    // team's centre in the factory's door, and the team then kept the door
    // crowded for good.
    fn recruit_ready(&self, unit: &Unit) -> bool {
        !unit.tethered
            && unit.health > 0
            && unit.recruitable
            && unit.work.job == crate::Job::Idle
            && unit.capture.is_none()
            && unit.boarding.is_none()
            && unit.spawn_owner.is_none()
            && unit.falling.is_none()
            && !unit.stats.harvester
    }

    fn can_add(&self, team: &Team, unit: &Unit, slot_unit: u16) -> bool {
        let Some(kind) = self.team_data.team_types.get(team.kind) else {
            return false;
        };
        let flies = unit.plane.is_some();
        unit.owner == team.house
            && unit.stats.kind == slot_unit + 1
            && !team.members.contains(&unit.id)
            && unit.health > 0
            && (self.recruit_ready(unit) || (flies && unit.recruitable))
            && (unit.recruitable || !kind.autocreate || flies)
            && unit.team.is_none_or(|other| self.team_type_of(other).is_some_and(|t| t.priority < kind.priority))
            && !(flies && unit.stats.fighter.primary.is_some() && unit.plane.is_some_and(|plane| plane.ammo <= 0))
    }

    fn held(&self, team: &Team, unit: u16) -> u32 {
        team.members.iter().filter(|&&id| self.unit(id).is_some_and(|u| u.stats.kind == unit + 1)).count() as u32
    }

    // The nearest free unit of the type
    // joins, at the front of the list; the first to join is initiated.
    fn recruit(&mut self, team: &mut Team, unit_kind: u16, count: u32) {
        if self.held(team, unit_kind) >= count {
            return;
        }
        let origin = team.zone.map_or((0, 0), centre_of);
        let best = self.units.iter().filter(|unit| self.can_add(team, unit, unit_kind)).min_by_key(|unit| distance((unit.x, unit.y), origin)).map(|unit| unit.id);
        let Some(id) = best else {
            return;
        };
        let recruitable = self.team_data.team_types.get(team.kind).is_some_and(|t| t.members_recruitable);
        let first = team.members.is_empty();
        let old = self.unit(id).and_then(|unit| unit.team);
        if let Some(old) = old {
            if let Some(other) = self.teams.iter_mut().find(|t| t.id == old) {
                other.members.retain(|&member| member != id);
                other.dirty = true;
            }
        }
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
            unit.team = Some(team.id);
            unit.initiated = first;
            unit.recruitable = recruitable;
            unit.arms.target = None;
            unit.arms.attacking = false;
        }
        team.members.insert(0, id);
        team.dirty = true;
        if team.zone.is_none() {
            self.calc_centre(team);
        }
    }

    // The middle of the initiated members (and
    // aircraft).
    fn calc_centre(&self, team: &mut Team) {
        let (mut n, mut sx, mut sy) = (0i64, 0i64, 0i64);
        for unit in team.members.iter().filter_map(|&id| self.unit(id)).filter(|u| u.initiated || u.plane.is_some()) {
            n += 1;
            sx += unit.x as i64;
            sy += unit.y as i64;
        }
        if n > 0 {
            team.zone = Some(((sx / n) as i32 / LEPTONS_PER_CELL, (sy / n) as i32 / LEPTONS_PER_CELL));
        }
    }

    // The member with the greatest `LeadershipRating`
    // among those initiated, the first found on a tie; else the first member.
    fn leader(&self, team: &Team) -> Option<u32> {
        let mut best: Option<(u32, i32)> = None;
        for unit in team.members.iter().filter_map(|&id| self.unit(id)).filter(|u| u.health > 0 && (u.initiated || u.plane.is_some())) {
            if best.is_none_or(|(_, rating)| unit.stats.leadership > rating) {
                best = Some((unit.id, unit.stats.leadership));
            }
        }
        best.map(|(id, _)| id).or(team.members.first().copied())
    }

    /// How many passengers a unit's type carries.
    fn carries(&self, unit: &Unit) -> i32 {
        self.unit_types.get((unit.stats.kind as usize).wrapping_sub(1)).map_or(0, |t| t.passengers)
    }

    /// Whether a task force has an aircraft that carries passengers.
    fn air_transported(&self, force: &[(u32, u16)]) -> bool {
        force.iter().any(|&(_, unit)| self.unit_types.get(unit as usize).is_some_and(|t| t.passengers > 0 && t.stats.plane.is_some()))
    }

    fn has_destination(unit: &Unit) -> bool {
        unit.plane.map_or(unit.dest.is_some(), |plane| plane.is_moving())
    }

    fn send_member(&mut self, id: u32, to: (i32, i32)) {
        let cell = (to.0.div_euclid(LEPTONS_PER_CELL), to.1.div_euclid(LEPTONS_PER_CELL));
        self.order(Order::Move { unit: id, x: cell.0, y: cell.1 });
    }

    // A member not yet initiated and
    // farther than the stray distance from the centre is sent there; inside it
    // it becomes initiated. Returns whether it takes part.
    fn initiate(&mut self, team: &Team, id: u32, stray: i32) -> bool {
        let Some(unit) = self.unit(id) else {
            return false;
        };
        let flies = unit.plane.is_some();
        if unit.initiated {
            return true;
        }
        let Some(zone) = team.zone.map(centre_of) else {
            return flies;
        };
        if distance((unit.x, unit.y), zone) > stray as i64 {
            if !Self::has_destination(unit) {
                self.send_member(id, zone);
            }
            return flies;
        }
        if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
            unit.initiated = true;
        }
        true
    }

    fn regroup(&mut self, team: &Team, stray: i32) -> bool {
        let Some(zone) = team.zone.map(centre_of) else {
            return true;
        };
        let mut together = true;
        for &id in &team.members {
            let Some(unit) = self.unit(id) else {
                continue;
            };
            if !(unit.initiated || unit.plane.is_some()) {
                continue;
            }
            if distance((unit.x, unit.y), zone) > stray as i64 && unit.arms.target.is_none() && !Self::has_destination(unit) {
                self.send_member(id, zone);
                together = false;
            }
        }
        together
    }

    fn coordinate_move(&mut self, team: &mut Team, stray: i32) {
        if team.members.is_empty() {
            return;
        }
        if team.focus.is_none() {
            team.focus = team.target;
        }
        let Some(focus) = team.focus.and_then(|focus| self.team_point(focus)) else {
            return;
        };
        let aggressive = self.team_data.team_types.get(team.kind).is_some_and(|t| t.aggressive);
        let close = self.team_data.close_enough as i64;
        let (mut arrived, mut any) = (true, false);
        for id in team.members.clone() {
            // One riding in a transport is off the map and plays no part.
            if self.unit(id).is_none() {
                continue;
            }
            if !self.initiate(team, id, stray) {
                arrived = false;
                continue;
            }
            let Some(unit) = self.unit(id) else {
                continue;
            };
            any = true;
            let reach = if self.altitude(unit) > 0 { stray as i64 * 2 } else { stray as i64 };
            let d = distance((unit.x, unit.y), focus);
            if d > reach {
                if aggressive && unit.arms.target.is_some() {
                    continue;
                }
                if !Self::has_destination(unit) {
                    self.send_member(id, focus);
                }
                arrived = false;
            } else if Self::has_destination(unit) && d > close {
                arrived = false;
            }
        }
        if any && arrived && team.moving {
            team.next_mission = true;
        }
    }

    /// Takes a team off its script and back to forming, sent beside the
    /// nearest of its house's unarmed buildings (a repair depot counting at
    /// half its distance).
    // Each building's distance is multiplied by one more than the threat at
    // its cell, which is always nothing. With no building
    // found, or no free cell round the one found, the team gathers on its
    // own centre.
    fn retreat(&mut self, team: &mut Team) {
        team.moving = false;
        team.line = -1;
        if team.members.is_empty() {
            team.zone = None;
            return;
        }
        self.calc_centre(team);
        let leader = self.leader(team);
        if team.zone.is_none() {
            team.zone = leader.and_then(|id| self.unit(id)).map(|unit| unit.cell());
        }
        let (Some(zone), Some(leader)) = (team.zone, leader) else {
            return;
        };
        let height = |world: &World, (x, y): (i32, i32)| world.ground_z(x, y) as i64;
        let from = centre_of(zone);
        let mut best = (i64::MAX, zone);
        for building in self.buildings.iter().filter(|b| b.owner == team.house) {
            let Some(kind) = self.building_types.get(building.kind as usize) else {
                continue;
            };
            if kind.fighter.primary.is_some() || building.occupants.iter().any(|o| o.stats.fighter.primary.is_some()) {
                continue;
            }
            let middle = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128);
            let (dx, dy, dz) = ((middle.0 - from.0) as f64, (middle.1 - from.1) as f64, (height(self, middle) - height(self, from)) as f64);
            let d = (dx * dx + dy * dy + dz * dz).sqrt() as i64;
            let score = if kind.unit_repair { d / 2 } else { d };
            if score >= best.0 {
                continue;
            }
            let cell = (middle.0.div_euclid(LEPTONS_PER_CELL), middle.1.div_euclid(LEPTONS_PER_CELL));
            if let Some(near) = self.cell_toward(leader, zone, cell, 2, 4) {
                best = (score, near);
            }
        }
        team.focus = Some(Aim::Cell(best.1 .0, best.1 .1));
        self.coordinate_move(team, self.team_data.stray);
    }

    /// A cell the unit could enter at `min` to `max - 1` cells from `to`, on
    /// the side facing `from`.
    // Ring by ring: from the corner or edge
    // middle three eighths round from the direction `from` to `to`, along the
    // part of the ring that faces `from` (two sides' worth from a corner, a
    // side's half and the next side's half from an edge middle); the first
    // cell that may be entered.
    fn cell_toward(&self, unit: u32, from: (i32, i32), to: (i32, i32), min: i32, max: i32) -> Option<(i32, i32)> {
        use crate::path::DIRECTIONS;
        let unit = self.unit(unit)?;
        let (dx, dy) = ((to.0 - from.0) as f64, (to.1 - from.1) as f64);
        let facing = (dx.atan2(-dy) / std::f64::consts::TAU * 65536.0) as i32 & 0xffff;
        let toward = (((facing >> 12) + 1) >> 1) as usize & 7;
        let start = (toward + 3) & 7;
        let others = self.others();
        let open = |cell: (i32, i32)| {
            self.map.in_playfield(cell.0, cell.1) && crate::entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, unit, cell, None, cell, -1) == crate::path::code::OK
        };
        for r in min..max {
            let mut at = (to.0 + DIRECTIONS[start].0 * r, to.1 + DIRECTIONS[start].1 * r);
            let legs: &[(usize, i32)] = if start & 1 == 1 { &[((start + 3) & 7, 2 * r)] } else { &[((start + 2) & 7, r), ((start + 4) & 7, r)] };
            for &(dir, steps) in legs {
                for _ in 0..steps {
                    at = (at.0 + DIRECTIONS[dir].0, at.1 + DIRECTIONS[dir].1);
                    if open(at) {
                        return Some(at);
                    }
                }
            }
        }
        None
    }

    /// A team's target is set to nothing.
    // Members aiming at, or going to, the old
    // target are put on guard and let go of it; the focus follows the target
    // when it was the same (or none).
    pub(crate) fn clear_team_target(&mut self, team: u32) {
        let Some(at) = self.teams.iter().position(|t| t.id == team) else {
            return;
        };
        let Some(old) = self.teams[at].target else {
            return;
        };
        let point = match old {
            Aim::Cell(x, y) => Some((x, y)),
            _ => None,
        };
        for id in self.teams[at].members.clone() {
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                let aiming = unit.arms.target == Some(old);
                let going = point.is_some() && unit.dest == point;
                if aiming {
                    unit.arms.target = None;
                    unit.arms.attacking = false;
                }
                if going {
                    unit.dest = None;
                    unit.drop_path();
                }
            }
        }
        let team = &mut self.teams[at];
        if team.focus == team.target || team.focus.is_none() {
            team.focus = None;
        }
        team.target = None;
    }

    fn coordinate_attack(&mut self, team: &mut Team, stray: i32) {
        if team.focus.is_none() {
            team.focus = team.target;
        }
        let Some(focus) = team.focus.filter(|focus| self.alive(*focus)) else {
            team.next_mission = true;
            return;
        };
        let mut fighting = false;
        for id in team.members.clone() {
            if !self.initiate(team, id, stray) {
                continue;
            }
            let Some(unit) = self.unit(id) else {
                continue;
            };
            let armed = unit.stats.fighter.primary.is_some();
            let spent = unit.plane.is_some_and(|plane| armed && plane.ammo <= 0);
            // A soldier whose way
            // of dealing with a building is to walk into it (an engineer
            // taking it, a C4 soldier wiring it) does that when its team is
            // set on the building; it has no weapon to be told to fire.
            let walks_in = match focus {
                Aim::Building(building) => matches!(self.entry_action(id, building), Some(crate::combat::Entry::Capture | crate::combat::Entry::Sabotage)),
                _ => false,
            };
            let busy = unit.dest.is_some() || unit.work.job != crate::Job::Idle;
            // A member's own target is kept.
            if walks_in {
                if let (Aim::Building(building), false) = (focus, busy) {
                    self.order(Order::Enter { unit: id, building });
                }
            } else if unit.arms.target.is_none() && armed && !spent {
                self.order(Order::Attack { unit: id, target: focus });
            }
            if !spent {
                fighting = true;
            }
        }
        if !fighting {
            team.next_mission = true;
        }
    }

    // --------------------------------------------------------------- targets

    /// What a team's leader would most like to attack, of a category.
    // The whole
    // map is searched; the rejections kept are those that can arise here.
    fn team_threat(&self, leader: u32, quarry: i32, only_enemy: bool) -> Option<Aim> {
        let me = self.unit(leader)?;
        let fighter = me.stats.fighter;
        let house = me.owner;
        let enemy = self.houses[house as usize].ai.as_ref().and_then(|b| b.enemy);
        let mut mask: u32 = match quarry {
            2 => 0x20,
            3 => 0x40,
            4 => 0x08,
            5 => 0x10,
            6 => 0x1000,
            7 => 0x2000,
            9 => 0x800,
            10 => 0x8000,
            11 => 0x10000,
            _ => 0,
        };
        if mask & 0x1b978 == 0 {
            for mount in [fighter.primary, fighter.secondary].into_iter().flatten() {
                if let Some(projectile) = self.weapons.get(mount.weapon as usize).and_then(|w| self.projectiles.get(w.projectile as usize)) {
                    if projectile.aa {
                        mask |= 4;
                    }
                    if projectile.ag {
                        mask |= 0xb8;
                    }
                }
            }
        }
        let zone_of = |cell: (i32, i32)| self.map.zones().map(|zones| zones.zone_of(cell, me.stats.movement_zone as usize));
        let my_zone = zone_of(me.cell());
        let grounded = me.plane.is_none() && me.stats.jet.is_none();
        let blank = crate::combat::Arms::default();
        let at = (me.x, me.y, self.ground_z(me.x, me.y) + self.altitude(me));
        let mut candidates: Vec<(u32, Aim, u8)> = Vec::new();
        for unit in &self.units {
            let class = Self::class_of(&unit.stats);
            let wanted = match class {
                0 => mask & 8 != 0,
                1 => mask & 0x50 != 0,
                _ => mask & 4 != 0 || mask & 0x10 != 0,
            };
            if wanted && unit.id != leader {
                candidates.push((unit.id, Aim::Unit(unit.id), class));
            }
        }
        if mask & 0x1ba60 != 0 {
            candidates.extend(self.buildings.iter().map(|b| (b.id, Aim::Building(b.id), 6)));
        }
        candidates.sort_by_key(|candidate| candidate.0);
        let mut best: Option<(Aim, i64)> = None;
        for (_, who, class) in candidates {
            let Some(owner) = self.owner_of(who) else {
                continue;
            };
            // Neutral things are no targets for a team (rules 17, 18).
            if owner == 0 || self.allied(house, owner) || (only_enemy && Some(owner) != enemy) {
                continue;
            }
            let Some(theirs) = self.fighter(who) else {
                continue;
            };
            if !self.alive(who) || !theirs.legal_target || (theirs.insignificant && class != 6) {
                continue;
            }
            let index = self.select_weapon(&fighter, who);
            let weapon = Self::mount(&fighter, index).and_then(|m| self.weapons.get(m.weapon as usize));
            let verses = weapon.and_then(|w| self.warheads.get(w.warhead as usize)).map(|w| w.verses[theirs.armour.min(10) as usize]);
            if mask & 0x18200 == 0 && self.fire_error(at, &fighter, &blank, who, index) == FireError::Illegal {
                continue;
            }
            if verses.is_some_and(|v| (v * 100.0) as i32 == 1) {
                continue;
            }
            let Some(point) = self.team_point(who) else {
                continue;
            };
            if let Aim::Unit(id) = who {
                let Some(unit) = self.unit(id) else {
                    continue;
                };
                // Rule 9: ground things must be in the leader's zone.
                if grounded && self.altitude(unit) == 0 && zone_of(unit.cell()) != my_zone {
                    continue;
                }
                // Rule 5: cloaked and not sensed.
                if self.hidden_from(unit, house) {
                    continue;
                }
                // Rule 20: a disguise that takes in my house.
                if !fighter.detect_disguise && self.disguised_to(unit, house) {
                    continue;
                }
                // Rule 24: harvesters only.
                if mask & 0x40 != 0 && !unit.stats.harvester {
                    continue;
                }
            }
            // The threat value, with the types' default coefficients.
            let mut v = 0.0;
            let their_index = self.select_weapon(&theirs, Aim::Unit(leader));
            if let Some(warhead) = Self::mount(&theirs, their_index).and_then(|m| self.weapons.get(m.weapon as usize)).and_then(|w| self.warheads.get(w.warhead as usize)) {
                v = -200.0 * warhead.verses[fighter.armour.min(10) as usize];
                let at_me = match who {
                    Aim::Unit(id) => self.unit(id).is_some_and(|u| u.arms.target == Some(Aim::Unit(leader))),
                    Aim::Building(id) => self.buildings.iter().any(|b| b.id == id && b.arms.target == Some(Aim::Unit(leader))),
                    Aim::Cell(..) => false,
                };
                if at_me {
                    v = -v;
                }
            }
            if Some(owner) == enemy {
                v += self.team_data.enemy_bonus;
            }
            if let Some(verses) = verses {
                v += 200.0 * verses;
            }
            let (health, strength) = match who {
                Aim::Unit(id) => self.unit(id).map_or((0, 1), |u| (u.health, u.stats.strength.max(1))),
                Aim::Building(id) => self.buildings.iter().find(|b| b.id == id).map_or((0, 1), |b| (b.health, self.building_types.get(b.kind as usize).map_or(1, |k| k.strength.max(1)))),
                Aim::Cell(..) => (0, 1),
            };
            v += -200.0 * health as f64 / strength as f64;
            let range_cells = weapon.map_or(fighter.guard_range, |w| w.range) as i64 / 256;
            let d = distance((me.x, me.y), point);
            let mut value = (100_000.0 + v + -10.0 * (d - range_cells).max(0) as f64) as i64;
            // The category rules.
            if let Aim::Building(id) = who {
                let Some(kind) = self.buildings.iter().find(|b| b.id == id).and_then(|b| self.building_types.get(b.kind as usize)) else {
                    continue;
                };
                if mask & 0x800 != 0 {
                    value = if kind.power > 0 { value + kind.power as i64 * 1000 } else { 0 };
                }
                if mask & 0x1000 != 0 && kind.factory.is_none() {
                    value = 0;
                }
                if mask & 0x2000 != 0 {
                    let empty = kind.can_be_occupied && self.buildings.iter().any(|b| b.id == id && b.occupants.is_empty());
                    if fighter.primary.is_none() || empty {
                        value = 0;
                    }
                }
                if mask & 0x18000 != 0 {
                    continue;
                }
            } else if mask & 0x2000 != 0 {
                value = 0;
            }
            if value == 0 {
                continue;
            }
            let value = value.max(1);
            if best.is_none_or(|(_, top)| value > top) {
                best = Some((who, value));
            }
        }
        best.map(|(who, _)| who)
    }

    // The building of a type, of the enemy's (or with `own` of the
    // house's own), by one of four rules. The threat at a cell is always
    // nothing, so the two threat rules take the first found.
    fn find_building(&self, leader: u32, kind: u16, mode: i32, own: bool, only_enemy: bool) -> Option<u32> {
        let me = self.unit(leader)?;
        let enemy = self.houses[me.owner as usize].ai.as_ref().and_then(|b| b.enemy);
        let mut best: Option<(u32, i64)> = None;
        for building in self.buildings.iter().filter(|b| b.kind == kind) {
            let theirs = if own { building.owner == me.owner } else { !self.allied(me.owner, building.owner) && (!only_enemy || Some(building.owner) == enemy) };
            if !theirs {
                continue;
            }
            let at = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128);
            let d = distance((me.x, me.y), at);
            let threat = 0;
            let value = match mode {
                0 => i64::MAX - threat,
                1 => threat,
                2 => i64::MAX - d,
                _ => d,
            };
            if best.is_none_or(|(_, top)| value > top) {
                best = Some((building.id, value));
            }
        }
        best.map(|(id, _)| id)
    }

    // ---------------------------------------------------------------- script

    /// Every team's turn.
    pub(crate) fn run_teams(&mut self) {
        let ids: Vec<u32> = self.teams.iter().map(|t| t.id).collect();
        for id in ids {
            let Some(mut team) = self.teams.iter().find(|t| t.id == id).cloned() else {
                continue;
            };
            let keep = self.team_ai(&mut team);
            match (keep, self.teams.iter_mut().find(|t| t.id == id)) {
                (true, Some(slot)) => *slot = team,
                (false, Some(_)) => {
                    if let Some(slot) = self.teams.iter_mut().find(|t| t.id == id) {
                        *slot = team;
                    }
                    self.delete_team(id);
                }
                _ => {}
            }
        }
    }

    fn team_ai(&mut self, team: &mut Team) -> bool {
        let Some(kind) = self.team_data.team_types.get(team.kind).cloned() else {
            return false;
        };
        let force = self.team_data.task_forces.get(kind.task_force).cloned().unwrap_or_default();
        let script = self.team_data.scripts.get(kind.script).cloned().unwrap_or_default();
        // Members that have died or been taken away.
        let before = team.members.len();
        // (One riding in a transport is off the map but still a member.)
        team.members.retain(|&id| {
            self.units.iter().any(|u| (u.id == id && u.team == Some(team.id) && u.health > 0) || u.cargo.iter().any(|rider| rider.id == id && rider.team == Some(team.id)))
        });
        if team.members.len() != before {
            team.dirty = true;
            if !team.members.iter().any(|&id| self.unit(id).is_some_and(|u| u.initiated)) {
                if let Some(unit) = team.members.first().and_then(|&id| self.units.iter_mut().find(|u| u.id == id)) {
                    unit.initiated = true;
                }
                team.zone = None;
            }
        }
        if team.target.is_some_and(|target| !matches!(target, Aim::Cell(..)) && !self.alive(target)) {
            team.target = None;
            team.focus = None;
        }
        // 2: the status.
        if team.dirty {
            let was = team.under;
            let want: u32 = force.iter().map(|entry| entry.0).sum();
            let n = team.members.len() as u32;
            if n < 1 {
                team.under = true;
                team.full = false;
                team.zone = None;
                if team.has_been {
                    return false;
                }
            } else {
                team.full = n == want;
                if team.full {
                    team.has_been = true;
                }
                team.under = if !kind.reinforce {
                    !team.has_been
                } else if want < 3 {
                    n < want
                } else {
                    n <= want / 3
                };
                team.dirty = false;
            }
            if was != team.under {
                team.reforming = true;
            }
        }
        // 3: a team on its way that has fallen under strength (only a
        // `Reinforce` team can) goes back to forming, near one of its house's
        // buildings.
        if team.moving && team.under {
            self.retreat(team);
        }
        // 4: it sets off.
        if !team.moving && team.full {
            team.moving = true;
            team.has_been = true;
            team.under = false;
            if team.reforming {
                for &id in &team.members {
                    if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                        unit.initiated = true;
                    }
                }
            }
            team.line = -1;
            team.next_mission = true;
        }
        // 5: the centre.
        if team.reforming || team.moving || team.zone.is_none() {
            self.calc_centre(team);
        }
        // 6: recruiting.
        if !team.moving || (!team.full && kind.reinforce) {
            for &(count, unit) in &force {
                self.recruit(team, unit, count);
            }
        }
        // 7: an empty team.
        if team.members.is_empty() {
            return !(team.has_been || self.tick.saturating_sub(team.created) > self.team_data.dissolve_delay as u64);
        }
        // 8: while forming, members wait where they are (or, after a retreat,
        // go where it sent them).
        if !team.moving {
            self.coordinate_move(team, self.team_data.stray);
            return true;
        }
        let (action, argument) = script.get(team.line.max(0) as usize).copied().unwrap_or((-1, 0));
        let stray = if !team.next_mission && matches!(action, 53 | 54) { self.team_data.relaxed_stray } else { self.team_data.stray };
        // 9: getting back together.
        if team.reforming || team.under {
            let together = self.regroup(team, stray);
            team.reforming = !together;
            return true;
        }
        // 10: the next line.
        let mut first = false;
        if team.next_mission {
            first = true;
            team.next_mission = false;
            team.line += 1;
            // Every member forgets where it was to go back to, but a transport
            // that is to return there.
            for &id in &team.members {
                let keeps = kind.transports_return && self.unit(id).is_some_and(|u| self.carries(u) > 0);
                if let (false, Some(unit)) = (keeps, self.units.iter_mut().find(|u| u.id == id && !u.stats.harvester)) {
                    unit.work.archive = None;
                }
            }
            if team.line as usize >= script.len() {
                return false;
            }
            team.target = None;
            team.focus = None;
        } else if team.focus.is_none() {
            team.focus = team.target;
        }
        let (action, argument) = if first { script[team.line as usize] } else { (action, argument) };
        let stray = if matches!(action, 53 | 54) { self.team_data.relaxed_stray } else { self.team_data.stray };
        let leader = self.leader(team);
        // 11: the action.
        match action {
            // Attack a category of thing.
            0 => {
                if team.target.is_none() {
                    let found = leader.and_then(|leader| self.team_threat(leader, argument, kind.only_target_house_enemy));
                    team.target = found;
                    team.focus = found;
                    if found.is_none() {
                        team.next_mission = true;
                        return true;
                    }
                }
                let spent = team.members.iter().filter_map(|&id| self.unit(id)).all(|u| u.plane.is_some_and(|plane| u.stats.plane.is_some_and(|s| s.ammo >= 1) && plane.ammo <= 0));
                if spent {
                    team.next_mission = true;
                    return true;
                }
                self.coordinate_attack(team, stray);
            }
            // Move to a cell.
            4 => {
                if first {
                    team.target = Some(Aim::Cell(argument % 128, argument / 128));
                    team.focus = team.target;
                }
                self.coordinate_move(team, stray);
            }
            // Guard for a while: fifteen frames for each unit of the argument.
            5 => {
                if first {
                    team.guard_until = self.tick + argument.max(0) as u64 * 15;
                }
                self.regroup(team, stray);
                if self.tick >= team.guard_until {
                    team.next_mission = true;
                }
            }
            // Jump to a line (counted from one).
            6 => {
                team.line = argument - 2;
                team.next_mission = true;
            }
            // Do this: the members keep to their own devices, for good.
            11 => {
                let Some(zone) = team.zone.map(centre_of) else {
                    return true;
                };
                for id in team.members.clone() {
                    if !self.initiate(team, id, stray) {
                        continue;
                    }
                    let Some(unit) = self.unit(id) else {
                        continue;
                    };
                    if unit.arms.target.is_none() && !Self::has_destination(unit) && distance((unit.x, unit.y), zone) > 2 * stray as i64 && argument != 11 {
                        self.send_member(id, zone);
                    }
                }
            }
            // Scatter.
            21 => {
                for id in team.members.clone() {
                    self.scatter_unit(id);
                }
                team.next_mission = true;
            }
            // Attack, or move to, an enemy building of a type; or move to one
            // of the house's own.
            46 | 47 | 58 => {
                if (first || action != 46) && team.target.is_none() {
                    let seeker = if action == 58 { leader } else { team.members.first().copied() };
                    let found = seeker.and_then(|seeker| self.find_building(seeker, (argument & 0xffff) as u16, argument >> 16, action == 58, kind.only_target_house_enemy));
                    // A move goes to a free cell in the seeker's zone nearest
                    // the building's own cell, picked by frame.
                    team.target = match (action, found) {
                        (46, Some(id)) => Some(Aim::Building(id)),
                        (_, Some(id)) => {
                            let cell = self.buildings.iter().find(|b| b.id == id).map(|b| b.cell);
                            let unit = seeker.and_then(|seeker| self.unit(seeker));
                            match (cell, unit) {
                                (Some(cell), Some(unit)) => crate::nearby(&self.map, &self.claims, &self.spots, unit, cell, crate::Near::new(false, false, None), self.tick).map(|near| Aim::Cell(near.0, near.1)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    team.focus = team.target;
                }
                if team.target.is_none() {
                    team.next_mission = true;
                } else if action == 46 {
                    self.coordinate_attack(team, stray);
                } else {
                    self.coordinate_move(team, stray);
                }
            }
            // Register success.
            49 => {
                team.success = true;
                team.next_mission = true;
            }
            // Gather a safe distance from the enemy's base, towards one's own;
            // or from one's own, towards the enemy.
            53 | 54 => {
                if first {
                    let Some(owner) = leader.and_then(|leader| self.unit(leader)).map(|unit| unit.owner) else {
                        team.next_mission = true;
                        return true;
                    };
                    let mine = self.houses[owner as usize].base_centre;
                    let enemy = self.houses[owner as usize].ai.as_ref().and_then(|b| b.enemy).map(|enemy| self.houses[enemy as usize].base_centre).filter(|&centre| centre != (0, 0));
                    let (from, towards) = match (action, enemy) {
                        (53, Some(enemy)) => (enemy, mine),
                        (53, None) => {
                            team.next_mission = true;
                            return true;
                        }
                        (_, Some(enemy)) => (mine, enemy),
                        (_, None) => {
                            let facing = self.random_below(256) as f64 / 256.0 * std::f64::consts::TAU;
                            (mine, (mine.0 + (facing.sin() * 1024.0) as i32, mine.1 - (facing.cos() * 1024.0) as i32))
                        }
                    };
                    let (dx, dy) = ((towards.0 - from.0) as f64, (towards.1 - from.1) as f64);
                    let length = dx.hypot(dy).max(1.0);
                    let reach = (self.team_data.safe_distance * LEPTONS_PER_CELL) as f64;
                    let point = (from.0 + (dx / length * reach) as i32, from.1 + (dy / length * reach) as i32);
                    let cell = self.near_ground((point.0 / LEPTONS_PER_CELL, point.1 / LEPTONS_PER_CELL));
                    team.target = Some(Aim::Cell(cell.0, cell.1));
                    team.focus = team.target;
                }
                self.coordinate_move(team, stray);
            }
            // Load onto the transport.
            // The transport is the
            // first member that carries and is not full, an aircraft if the task
            // force has an aircraft that carries; one member a frame is sent
            // aboard; with nobody left to send the line is done.
            // With `TransportsReturnOnUnload` the transport is to come back to
            // the cell it loaded on.
            14 => {
                let air = self.air_transported(&force);
                let transport = team.members.iter().filter_map(|&id| self.unit(id)).find(|u| self.carries(u) > 0 && (u.cargo.len() as i32) < self.carries(u) && u.plane.is_some() == air).map(|u| u.id);
                let Some(transport) = transport else {
                    team.next_mission = true;
                    return true;
                };
                let next = team.members.iter().copied().find(|&id| id != transport && self.unit(id).is_some_and(|u| u.boarding != Some(transport)) && self.can_board(id, transport));
                match next {
                    Some(unit) => self.order(Order::Board { unit, transport }),
                    None => {
                        if kind.transports_return {
                            if let Some(unit) = self.units.iter_mut().find(|u| u.id == transport) {
                                unit.work.archive = Some(unit.cell());
                            }
                        }
                        team.next_mission = true;
                    }
                }
            }
            // Wait until every transport is full.
            43 => {
                let waiting = team.members.iter().filter_map(|&id| self.unit(id)).any(|u| (u.cargo.len() as i32) < self.carries(u));
                if !waiting {
                    team.next_mission = true;
                }
            }
            // Unload.
            // Members carrying anyone, not on a
            // building's cell and not already at it, let them out; once all are
            // empty, by the argument: 1 or 3 lets the passengers go from the
            // team, 2 or 3 the transports.
            8 => {
                let loaded: Vec<u32> = team.members.iter().filter_map(|&id| self.unit(id)).filter(|u| !u.cargo.is_empty()).map(|u| u.id).collect();
                if !loaded.is_empty() {
                    for unit in loaded {
                        let ready = self.unit(unit).is_some_and(|u| u.unloading.is_none() && self.building_on(u.cell()).is_none());
                        if ready {
                            self.order(Order::Deploy { unit });
                        }
                    }
                    return true;
                }
                // A transport that is to return leaves the team whatever the
                // argument and drives back to where it loaded. (When the task
                // force has a transport aircraft, only aircraft count as
                // transports.)
                let air = self.air_transported(&force);
                for id in team.members.clone() {
                    let carrier = self.unit(id).is_some_and(|u| self.carries(u) > 0 && (!air || u.plane.is_some()));
                    let back = if carrier && kind.transports_return { self.units.iter_mut().find(|u| u.id == id).map(|u| u.work.archive.take()) } else { None };
                    let goes = back.is_some() || if carrier { matches!(argument, 2 | 3) } else { matches!(argument, 1 | 3) };
                    if goes {
                        team.members.retain(|&member| member != id);
                        if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                            unit.team = None;
                        }
                    }
                    if let Some(Some(cell)) = back {
                        self.order(Order::Move { unit: id, x: cell.0, y: cell.1 });
                    }
                }
                team.next_mission = true;
            }
            // Iron curtain me; chrono shift to a quarry.
            // Ready, the curtain comes down on the team's own place (or
            // the sphere takes it from there to its leader's greatest threat
            // of the kind, which becomes its target); not yet granted, or
            // charged less than `AIMinorSuperReadyPercent`, the line is given
            // up; otherwise the team waits.
            55 | 57 => {
                let h = team.house as usize;
                let wanted = if action == 55 { 1 } else { 3 };
                let found = if action == 55 { self.super_types.iter().position(|t| t.kind == wanted) } else { self.super_types.iter().rposition(|t| t.kind == wanted) };
                let second = self.super_types.iter().rposition(|t| t.kind == 4);
                let state = found.and_then(|index| self.houses[h].supers.get(index).copied().map(|state| (index, state)));
                let (Some((index, state)), Some(zone)) = (state, team.zone) else {
                    team.next_mission = true;
                    return true;
                };
                if action == 57 && second.is_none() {
                    team.next_mission = true;
                    return true;
                }
                if state.can_fire() {
                    if action == 55 {
                        self.fire_super(team.house, index, zone);
                    } else if let Some(target) = leader.and_then(|leader| self.team_threat(leader, argument, kind.only_target_house_enemy)) {
                        let to = self.aim_point(target).map(|p| (p.0.div_euclid(LEPTONS_PER_CELL), p.1.div_euclid(LEPTONS_PER_CELL)));
                        if let (Some(to), Some(second)) = (to, second) {
                            self.fire_super(team.house, index, zone);
                            self.fire_super(team.house, second, to);
                            team.target = Some(target);
                            team.focus = Some(target);
                            for id in team.members.clone() {
                                self.order(Order::Attack { unit: id, target });
                            }
                        }
                    }
                    team.next_mission = true;
                } else {
                    let total = self.super_types[index].recharge.max(1) as f32;
                    let waits = state.granted && state.left as f32 / total <= 1.0 - self.ai_rules.minor_super_ready as f32;
                    if !waits {
                        team.next_mission = true;
                    }
                }
            }
            // Not built (see the top of the file): the line is passed over.
            _ => team.next_mission = true,
        }
        true
    }
}

impl World {
    /// Starts a team of a type for a house, as an AI trigger would, for tests.
    #[doc(hidden)]
    pub fn start_team_for_tests(&mut self, house: u8, kind: usize) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.teams.push(Team::new(id, kind, house, self.tick));
        id
    }

    /// Kills a unit, for tests.
    #[doc(hidden)]
    pub fn kill_unit_for_tests(&mut self, unit: u32) {
        self.kill(Aim::Unit(unit));
    }

    /// The factory a unit type comes from, for the computer's choice of what
    /// to build.
    pub(crate) fn factory_of_class(class: u8) -> Factory {
        match class {
            0 => Factory::Infantry,
            2 => Factory::Aircraft,
            _ => Factory::Unit,
        }
    }
}
