//! The computer player.
//!
//! Follows the original game. Built so far: the start of a skirmish and the computer's construction vehicle deploying by its
//! own missions (1.3).
//! Then the base: the plan made once the yard stands, what to
//! build next with the power test (4.3), where to put it (5.1 to 5.3), and
//! harvesters (6).

use crate::path::code;
use crate::{cell_centre, entry_code, Other, Unit, UnitStats, World, LEPTONS_PER_CELL};

/// The rules lists the computer plans from.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    /// `[AI]` `BuildConst`, `BuildPower`, `BuildBarracks`, `BuildWeapons` and
    /// `BuildRefinery`, as building types.
    pub build_const: Vec<u16>,
    pub build_power: Vec<u16>,
    pub build_barracks: Vec<u16>,
    pub build_weapons: Vec<u16>,
    pub build_refinery: Vec<u16>,
    /// `[General]` `GDIPowerPlant` and `NodRegularPower`.
    pub allied_power: Option<u16>,
    pub soviet_power: Option<u16>,
    /// `[General] HarvesterUnit`, as unit types, and `HarvestersPerRefinery`.
    pub harvesters: Vec<u16>,
    pub harvesters_per_refinery: i32,
    /// `AlliedBaseDefenseCounts` and `SovietBaseDefenseCounts`, hardest first.
    pub defence_counts: ([i32; 3], [i32; 3]),
    /// `[AI] AIBaseSpacing`.
    pub base_spacing: i32,
    /// By difficulty, hardest first: the `ROF` of `[Easy]`, `[Normal]` and
    /// `[Difficult]` (the hardest computer reads `[Easy]`); `MultiplayerAICM`;
    /// `AIVirtualPurifiers`. And `PurifierBonus`.
    pub rof: [f64; 3],
    pub credit_bonus: [i32; 3],
    pub virtual_purifiers: [i32; 3],
    pub purifier_bonus: f64,
    /// `RepairDelay` of the difficulty records, hardest first, in minutes;
    /// and `[General] CreditReserve`.
    pub repair_delay: [f64; 3],
    pub credit_reserve: i32,
    /// `AIAlternateProductionCreditCutoff`.
    pub credit_cutoff: i32,
    /// The `AIIonCannon...Value` lists, hardest first: construction yard, war
    /// factory, power, engineer, thief, harvester, MCV, APC, base defence,
    /// plug, helipad, temple.
    pub target_values: [[i32; 3]; 12],
    /// `AIMinorSuperReadyPercent`.
    pub minor_super_ready: f64,
    /// `AIAutoDeployFrameDelay`, hardest first.
    pub auto_deploy_delay: [i32; 3],
    /// `[AI] ConcreteWalls`, as building types.
    pub concrete_walls: Vec<u16>,
    /// `AIPickWallDefensePercent`, hardest first: the chance a defence's place
    /// becomes a ring of walls.
    pub wall_percent: [i32; 3],
}

/// A place in a computer house's base plan: a building type, or a defence still
/// to be chosen, and where it stands once it has been placed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Node {
    pub kind: Option<u16>,
    pub cell: Option<(i32, i32)>,
    fails: u32,
    /// For a defence: what it was chosen against (0 aircraft, 1 armour, 2
    /// soldiers) and the quarter of the base that was weakest.
    cover: Option<(u8, u8)>,
}

impl Node {
    /// A place in the plan for a building type, still to be built.
    pub(crate) fn wanted(kind: u16) -> Self {
        Self { kind: Some(kind), cell: None, fails: 0, cover: None }
    }
}

/// A computer house's state.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Brain {
    /// The base plan, the yard's cell, and the building it has decided on next.
    pub nodes: Vec<Node>,
    pub base_cell: Option<(i32, i32)>,
    pub build_structure: Option<u16>,
    pub build_unit: Option<u16>,
    /// The cells its base is taken to cover, those of them on its edge in the
    /// order they were found, and the buildings already counted.
    covered: std::collections::BTreeSet<(i32, i32)>,
    edge: Vec<(i32, i32)>,
    counted: Vec<u32>,
    /// 0 hardest to 2 easiest, as the rules' lists are ordered.
    pub difficulty: usize,
    /// Set at the end of the house's first frame: production is on and its
    /// construction vehicle may deploy.
    pub awake: bool,
    /// When its construction vehicle's mission next runs, and whether that
    /// mission is now Hunt (looking for somewhere to deploy).
    mission_at: u64,
    hunting: bool,
    /// When it next looks at its AI triggers; whether they are on (its
    /// construction vehicle has deployed); its enemy and its anger at each
    /// house; when it next takes stock; when a building of its was last hit.
    pub team_timer: u64,
    pub timer_armed: bool,
    /// Selling up and sending everything to hunt; when it may next start a
    /// repair.
    pub all_hunt: bool,
    pub repair_at: u64,
    /// The production mode: 0 everything at once, 1 a building
    /// first, 2 units first; and, in mode 2, whether the units had nothing to
    /// ask for last turn, so that a building may be chosen.
    pub mode: u8,
    units_stuck: bool,
    pub triggers_active: bool,
    pub enemy: Option<u8>,
    pub anger: Vec<i32>,
    pub expert_at: u64,
    pub last_attack: u64,
    /// The soldier and aircraft types it has decided to build next.
    pub build_infantry: Option<u16>,
    pub build_aircraft: Option<u16>,
}

/// The cells a blocked construction vehicle tries, in order.
// North up to four cells, then south,
// then west, then east.
const DEPLOY_OFFSETS: [(i32, i32); 36] = [
    (0, -1), (0, -2), (1, -2), (-1, -2), (0, -3), (1, -3), (-1, -3), (2, -3), (-2, -3), (0, -4), (1, -4), (-1, -4), (2, -4), (-2, -4),
    (0, 1), (0, 2), (1, 2), (-1, 2), (0, 3), (1, 3), (-1, 3), (2, 3), (-2, 3), (0, 4), (1, 4), (-1, 4), (2, 4), (-2, 4),
    (-1, 0), (-2, 0), (-3, 0), (-4, 0), (1, 0), (2, 0), (3, 0), (4, 0),
];

/// The eight directions of the placement search, north first, clockwise.
const AROUND: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

impl Brain {
    /// A computer house of a difficulty: 0 hardest to 2 easiest.
    pub fn of(difficulty: usize) -> Self {
        Self { difficulty, ..Self::default() }
    }
}

impl World {
    fn draw(&mut self, low: i32, high: i32) -> i32 {
        low + (self.next_random() % (high - low + 1).max(1) as u32) as i32
    }

    /// Whether a new thing of a kind could stand on a cell.
    // Nothing there, or a soldier joining soldiers.
    fn can_start_on(&self, stats: &UnitStats, owner: u8, cell: (i32, i32)) -> bool {
        if !self.map.in_playfield(cell.0, cell.1) {
            return false;
        }
        let probe = Unit::new(u32::MAX - 1, owner, cell, 0, *stats);
        let others: Vec<Other> = self.others();
        let taken = self.units.iter().any(|u| u.cell() == cell && !(u.stats.infantry && stats.infantry));
        !taken && entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, &probe, cell, None, cell, -1) == code::OK
    }

    /// Puts a new unit on a cell or the nearest free one round it.
    // The cell itself, then rings from the starting
    // radius out to 31: eight directions from a random one, first as they stand,
    // then each nudged by up to a cell at random.
    fn place_near(&mut self, stats: UnitStats, owner: u8, cell: (i32, i32), start_radius: i32) -> bool {
        let put = |world: &mut World, at: (i32, i32)| -> bool {
            if !world.can_start_on(&stats, owner, at) {
                return false;
            }
            if stats.infantry {
                world.add_infantry(at, 2, 0, stats, owner).is_some()
            } else {
                world.add_unit(at, 0, stats, owner);
                true
            }
        };
        if put(self, cell) {
            return true;
        }
        for radius in start_radius..32 {
            let mut direction = self.draw(0, 7) as usize;
            for pass in 0..2 {
                for _ in 0..8 {
                    let mut at = (cell.0 + radius * AROUND[direction].0, cell.1 + radius * AROUND[direction].1);
                    if pass == 1 {
                        let a = self.draw(0, 1);
                        at.0 += if self.draw(0, 99) < 50 { a } else { -a };
                        let b = self.draw(0, 1);
                        at.1 += if self.draw(0, 99) < 50 { b } else { -b };
                    }
                    if at != cell && put(self, at) {
                        return true;
                    }
                    direction = (direction + 1) & 7;
                }
            }
        }
        false
    }

    /// Gives every playing house its starting army.
    // The budget is the
    // unit count times the rounded average cost of every type that may start a
    // game for a country in play (not above the tech level, not a base unit);
    // each house buys random vehicles of its own until a third is left, then foot
    // soldiers, giving up after twenty failures to place one; what is left is
    // paid to it.
    pub fn create_starting_units(&mut self, unit_count: i32) {
        if unit_count <= 0 {
            return;
        }
        let playing: Vec<usize> = (1..self.houses.len()).collect();
        let mask = playing.iter().fold(0u32, |mask, &h| mask | 1 << self.houses[h].country.min(31));
        let may_start = |kind: &crate::UnitType, countries: u32, tech: i32| {
            kind.stats.allowed_to_start && kind.owners & countries != 0 && kind.tech_level <= tech && !kind.stats.base_unit && kind.stats.plane.is_none() && !kind.stats.fighter.aircraft
        };
        let all: Vec<i32> = self.unit_types.iter().filter(|kind| may_start(kind, mask, 10)).map(|kind| kind.cost).collect();
        let count = (all.len() as i32).max(1);
        let budget = ((count / 2 + all.iter().sum::<i32>()) / count) * unit_count;
        for house in playing {
            let (country, tech, centre) = (1u32 << self.houses[house].country.min(31), self.houses[house].tech_level, self.houses[house].base_centre);
            let cell = (centre.0.div_euclid(LEPTONS_PER_CELL), centre.1.div_euclid(LEPTONS_PER_CELL));
            let pick = |infantry: bool, world: &World| -> Vec<usize> {
                (0..world.unit_types.len())
                    .filter(|&index| world.unit_types[index].infantry == infantry && may_start(&world.unit_types[index], country, tech))
                    .collect()
            };
            let (vehicles, soldiers) = (pick(false, self), pick(true, self));
            if vehicles.is_empty() && soldiers.is_empty() {
                continue;
            }
            let (mut left, mut tries) = (budget, 20);
            while left > 0 && tries > 0 {
                let list = if left > budget / 3 && !vehicles.is_empty() { &vehicles } else { &soldiers };
                if list.is_empty() {
                    break;
                }
                let index = list[self.draw(0, list.len() as i32 - 1) as usize];
                let stats = UnitStats { kind: index as u16 + 1, ..self.unit_types[index].stats };
                if !self.place_near(stats, house as u8, cell, 4) {
                    tries -= 1;
                    continue;
                }
                left -= self.unit_types[index].cost;
            }
            if left > 0 {
                self.houses[house].credits += left;
            }
        }
        self.draw(0, 0xffff);
    }

    /// The computer houses' turn, after everything else in the frame.
    // Houses run last.
    pub(crate) fn think(&mut self) {
        self.recalc_centres();
        for house in 0..self.houses.len() {
            if self.houses[house].ai.is_none() {
                continue;
            }
            self.base_unit_mission(house as u8);
            self.auto_deploy(house as u8);
            // The team check comes before the
            // house's production.
            self.expert_ai(house as u8);
            self.team_check(house as u8);
            self.run_base(house as u8);
            if let Some(brain) = self.houses[house].ai.as_mut() {
                brain.awake = true;
            }
        }
    }

    /// A computer house's soldiers that deploy to fire dig in when they have
    /// stood a while with nowhere to go.
    // `Deployer` and `DeployFire`
    // types with no `UndeployDelay`, not immune to radiation, with no
    // destination, once `AIAutoDeployFrameDelay` frames have gone by.
    // It runs only
    // from the soldier's Guard and Area Guard missions; the frames count from
    // the mission's start; the place it was sent to (ArchiveTarget) must be none
    // or its own cell.
    // Not original behaviour: there are no missions here, so the frames count
    // from when the soldier last stood still with nowhere to go, and the
    // sent-to test is stood in for by "not on a building's cell" below.
    fn auto_deploy(&mut self, house: u8) {
        let Some(difficulty) = self.houses[house as usize].ai.as_ref().map(|brain| brain.difficulty.min(2)) else {
            return;
        };
        let delay = self.ai_rules.auto_deploy_delay[difficulty].max(0) as u64;
        let tick = self.tick;
        let mut ready = Vec::new();
        let buildings: Vec<((i32, i32), (i32, i32))> = self.buildings.iter().map(|b| (b.cell, b.size)).collect();
        for unit in self.units.iter_mut().filter(|u| u.owner == house && u.stats.infantry) {
            let fighter = unit.stats.fighter;
            if !(fighter.deployer && fighter.deploy_fire && fighter.undeploy_delay < 0 && !fighter.immune_to_radiation) {
                continue;
            }
            // An idle soldier on a building's cell
            // is walking off it, so it is not at rest. (Without this one fresh
            // from the barracks dug in on the barracks' own cell, never left,
            // and no soldier could come out after it.)
            // (The engine's soldier there has been sent off the building, so
            // has a destination and is not on Guard.)
            let cell = unit.cell();
            let on_building = buildings.iter().any(|&(at, size)| (at.0..at.0 + size.0).contains(&cell.0) && (at.1..at.1 + size.1).contains(&cell.1));
            if on_building || unit.dest.is_some() || unit.is_moving() || unit.stance != crate::combat::Stance::Standing || unit.falling.is_some() {
                unit.busy_at = tick;
            } else if tick > unit.busy_at + delay {
                ready.push(unit.id);
            }
        }
        for unit in ready {
            self.order(crate::Order::Deploy { unit });
        }
    }

    /// The construction vehicle of a computer house deploys of its own accord.
    // On its Guard turn, once its house is awake, it goes to Unload and tries to
    // deploy where it stands; refused, it Hunts: it looks at 36 cells round it
    // in a fixed order for one where the yard could stand and drives to the
    // first, or with none is asked to scatter, and tries again on its next turn.
    fn base_unit_mission(&mut self, house: u8) {
        let Some(brain) = self.houses[house as usize].ai.clone() else {
            return;
        };
        if !brain.awake || self.tick < brain.mission_at {
            return;
        }
        let vehicle = self.units.iter().find(|u| {
            u.owner == house
                && u.stats.deploys_into.is_some_and(|kind| self.building_types.get(kind as usize).is_some_and(|kind| kind.construction_yard))
        });
        let Some((id, cell, moving, kind)) = vehicle.map(|u| (u.id, u.cell(), u.is_moving() || u.deploying, u.stats.deploys_into.unwrap_or(0))) else {
            return;
        };
        let mut hunting = brain.hunting;
        let wait = if hunting { 14 } else { 27 } + self.draw(0, 2) as u64;
        if !moving {
            if self.can_deploy(id) {
                if let Some(unit) = self.unit_at(id).map(|at| &mut self.units[at]) {
                    unit.deploying = true;
                }
            } else {
                hunting = true;
                let spot = DEPLOY_OFFSETS
                    .iter()
                    .map(|off| (cell.0 + off.0, cell.1 + off.1))
                    .find(|&at| self.foundation_clear(Self::deploy_origin(at), kind, id + 1));
                match spot {
                    Some(at) => self.order(crate::Order::Move { unit: id, x: at.0, y: at.1 }),
                    None => self.scatter_unit(id),
                }
            }
        }
        if let Some(brain) = self.houses[house as usize].ai.as_mut() {
            brain.mission_at = self.tick + wait;
            brain.hunting = hunting;
        }
        let _ = cell_centre;
    }
}

/// The eight directions as cell steps, north first, clockwise.
const STEPS: [(i32, i32); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

impl World {
    /// The first entry of a rules list that a house's country may build.
    // By `Owner`, `RequiredHouses`,
    // `ForbiddenHouses` and `AIBasePlanningSide`.
    fn pick(&self, house: u8, list: &[u16]) -> Option<u16> {
        let h = self.houses.get(house as usize)?;
        let bit = 1u32 << h.country.min(31);
        list.iter().copied().find(|&kind| {
            self.building_types.get(kind as usize).is_some_and(|t| {
                t.owners & bit != 0
                    && t.required_houses.is_none_or(|mask| mask & bit != 0)
                    && t.forbidden_houses.is_none_or(|mask| mask & bit == 0)
                    && (t.planning_side == -1 || t.planning_side == h.side)
            })
        })
    }

    /// Makes a computer house's base plan.
    // The yard, a power plant, then every building the house may plan
    // (`AIBuildThis`, its side, its tech level) as its prerequisites come to be
    // in the list, the barracks and war factory looked at first, the plug only
    // when nothing else can go; then extra refineries and the places for
    // defences at random points.
    fn generate_base_list(&mut self, house: u8) -> Vec<Node> {
        let Some(h) = self.houses.get(house as usize) else {
            return Vec::new();
        };
        let (bit, side, tech, difficulty) = (1u32 << h.country.min(31), h.side, h.tech_level, h.ai.as_ref().map_or(2, |brain| brain.difficulty));
        let mut candidates: Vec<u16> = (0..self.building_types.len() as u16)
            .filter(|&kind| {
                let t = &self.building_types[kind as usize];
                t.owners & bit != 0
                    && (t.planning_side == side || t.planning_side == -1)
                    && t.ai_build_this
                    && t.tech_level <= tech
                    && t.required_houses.is_none_or(|mask| mask & bit != 0)
                    && t.forbidden_houses.is_none_or(|mask| mask & bit == 0)
            })
            .collect();
        let mut done = vec![false; candidates.len()];
        let rules = self.ai_rules.clone();
        let mut order: Vec<Option<u16>> = Vec::new();
        if let Some(yard) = self.pick(house, &rules.build_const)
            && let Some(at) = candidates.iter().position(|&c| c == yard) {
                done[at] = true;
                order.push(Some(yard));
            }
        order.push(self.pick(house, &rules.build_power));
        for (slot, list) in [(0usize, &rules.build_barracks), (1, &rules.build_weapons)] {
            if let Some(at) = self.pick(house, list).and_then(|kind| candidates.iter().position(|&c| c == kind))
                && slot < candidates.len() {
                    candidates.swap(slot, at);
                    done[at] = done[slot];
                    done[slot] = false;
                }
        }
        let met = |world: &World, kind: u16, list: &[Option<u16>]| {
            world.building_types[kind as usize].prerequisites.iter().all(|group| group.iter().any(|needed| list.contains(&Some(*needed))))
        };
        let mut left = candidates.len() as i32 - 1;
        while left > 0 {
            let (mut progressed, mut plug) = (false, None);
            let known = order.clone();
            for i in 0..candidates.len() {
                if done[i] {
                    continue;
                }
                if self.building_types[candidates[i] as usize].plug {
                    plug = Some(i);
                } else if met(self, candidates[i], &known) {
                    done[i] = true;
                    order.push(Some(candidates[i]));
                    left -= 1;
                    progressed = true;
                }
            }
            if !progressed {
                let Some(plug) = plug else {
                    break;
                };
                done[plug] = true;
                order.push(Some(candidates[plug]));
                left -= 1;
            }
        }
        // Extra refineries: two for the hardest, one, none.
        if let Some(refinery) = self.pick(house, &rules.build_refinery)
            && let Some(first) = order[..order.len().saturating_sub(1)].iter().position(|&kind| kind == Some(refinery)) {
                for _ in 0..(2 - difficulty.min(2)) {
                    let at = self.draw(first as i32, order.len() as i32 - 1) as usize;
                    order.insert(at + 1, Some(refinery));
                }
            }
        let counts = if side == 1 { rules.defence_counts.1 } else { rules.defence_counts.0 };
        for _ in 0..counts[difficulty.min(2)] {
            if order.len() > 3 {
                let at = self.draw(3, order.len() as i32 - 1) as usize;
                order.insert(at + 1, None);
            }
        }
        order.into_iter().map(|kind| Node { kind, cell: None, fails: 0, cover: None }).collect()
    }

    /// Adds a building's ground to what its house's base covers.
    // The foundation grown by `AIBaseSpacing` each way is marked; then round it
    // the cells that are marked and have both a marked and an unmarked neighbour
    // are kept as the base's edge, in the order found.
    fn mark_base_region(&mut self, house: u8, cell: (i32, i32), size: (i32, i32)) {
        let s = self.ai_rules.base_spacing;
        let Some(brain) = self.houses[house as usize].ai.as_mut() else {
            return;
        };
        let (x0, y0, x1, y1) = (cell.0 - s, cell.1 - s, cell.0 + size.0 + s, cell.1 + size.1 + s);
        for y in y0..y1 {
            for x in x0..x1 {
                brain.covered.insert((x, y));
            }
        }
        for y in y0 - 1..=y1 + 1 {
            for x in x0 - 1..=x1 + 1 {
                if !brain.covered.contains(&(x, y)) {
                    continue;
                }
                let around = STEPS.iter().filter(|d| brain.covered.contains(&(x + d.0, y + d.1))).count();
                let at = brain.edge.iter().position(|&c| c == (x, y));
                if around == 8 {
                    if let Some(at) = at {
                        brain.edge.remove(at);
                    }
                } else if around > 0 && at.is_none() {
                    brain.edge.push((x, y));
                }
            }
        }
    }

    /// Where a computer house puts a building.
    // The base's edge cells nearest the yard first; at each, the foundation is
    // tried just outside the base on the side facing away from it, three places
    // along the edge, then one cell closer with less room; the ground must be
    // free and flat with nothing of the base on it, within three levels of the
    // yard's, and still beside the base.
    fn find_build_location(&self, house: u8, kind: u16) -> Option<(i32, i32)> {
        let brain = self.houses.get(house as usize)?.ai.as_ref()?;
        let base = brain.base_cell?;
        let t = self.building_types.get(kind as usize)?;
        let (w, h) = t.size;
        let spacing = self.ai_rules.base_spacing;
        let s = spacing + i32::from(t.extra_space);
        // A defence goes by the
        // edge cell least covered against what it was chosen for, then the one
        // best in line with the base's weakest quarter; anything else by the
        // cell nearest the base.
        let cover = brain.nodes.iter().find(|node| node.kind == Some(kind) && node.cell.is_none()).and_then(|node| node.cover);
        let mut cells: Vec<(i32, (i32, i32))> = brain
            .edge
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let key = match cover {
                    Some((against, quarter)) => {
                        let g = self.coverage(house, base, c, against as usize);
                        let (dx, dy) = ((c.0 - base.0) as f64, (c.1 - base.1) as f64);
                        let facing = (dx.atan2(-dy) / std::f64::consts::TAU * 65536.0) as i32 & 0xffff;
                        let f = (((facing >> 7) + 1) >> 1) & 0xff;
                        let mut a = (quarter as i32 * 2 * 32 - f).abs();
                        if a >= 128 {
                            a = 255 - a;
                        }
                        g * 128 + a
                    }
                    None => (c.0 - base.0).abs().max((c.1 - base.1).abs()),
                };
                (i as i32 + 1000 * key, c)
            })
            .collect();
        cells.sort_by_key(|(key, _)| *key);
        let level0 = self.map.level(base.0, base.1) as i32;
        let free = |x0: i32, y0: i32, rw: i32, rh: i32| {
            (y0..y0 + rh).all(|y| {
                (x0..x0 + rw).all(|x| {
                    self.map.in_playfield(x, y)
                        && !brain.covered.contains(&(x, y))
                        && self.map.buildable(x, y)
                        && self.map.ramp(x, y) == 0
                        && self.building_on((x, y)).is_none()
                })
            })
        };
        let beside = |p: (i32, i32)| {
            (p.1 - spacing - 1..=p.1 + h + 2 * spacing).any(|y| (p.0 - spacing - 1..=p.0 + w + 2 * spacing).any(|x| brain.covered.contains(&(x, y))))
        };
        for &(_, c) in &cells {
            let v = STEPS.iter().filter(|d| brain.covered.contains(&(c.0 + d.0, c.1 + d.1))).fold((0, 0), |sum, d| (sum.0 + d.0, sum.1 + d.1));
            if v == (0, 0) {
                continue;
            }
            // The direction pointing away from the base, 0 north, clockwise.
            let angle = ((v.0 as f64).atan2(-(v.1 as f64)) + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU);
            let dir = ((angle / (std::f64::consts::TAU / 8.0)).round() as usize) & 7;
            let off_x = if (1..=3).contains(&dir) { s } else if (5..=7).contains(&dir) { 1 - w - s } else { 0 };
            let off_y = if (3..=5).contains(&dir) { s + 1 } else if matches!(dir, 7 | 0 | 1) { 1 - h - s } else { 0 };
            let (a, b) = (STEPS[(dir + 6) & 7], STEPS[(dir + 2) & 7]);
            for attempt in 0..2 {
                let mut p = (c.0 + off_x + a.0, c.1 + off_y + a.1);
                let mut m = s;
                if attempt == 1 {
                    let back = STEPS[(dir + 4) & 7];
                    p = (p.0 + back.0, p.1 + back.1);
                    m = s - spacing;
                }
                for _ in 0..3 {
                    if free(p.0 - m, p.1 - m, w + 2 * m, h + 2 * m)
                        && self.foundation_clear(p, kind, 0)
                        && (level0 - self.map.level(p.0, p.1) as i32).abs() < 3
                        && beside(p)
                    {
                        return Some(p);
                    }
                    p = (p.0 + b.0, p.1 + b.1);
                }
            }
        }
        None
    }

    /// Puts a ring of wall places round the latest walled building in the plan
    /// before `at` that has none yet. Returns whether it did.
    // The wall type is the first of
    // `ConcreteWalls` for the house's side (or any side); the building must be
    // built, have `ProtectWithWall`, and not be followed in the plan by that
    // wall already; the places go in just after it: the rows above and below,
    // the columns left and right, then the corners, in reverse order.
    fn add_wall_nodes(&self, house: u8, nodes: &mut Vec<Node>, at: usize) -> bool {
        let side = self.houses[house as usize].side;
        let Some(wall) = self.ai_rules.concrete_walls.iter().copied().find(|&w| self.building_types.get(w as usize).is_some_and(|t| t.planning_side == side || t.planning_side == -1)) else {
            return false;
        };
        for i in (0..at.min(nodes.len())).rev() {
            let node = &nodes[i];
            let walled = node.kind.and_then(|kind| self.building_types.get(kind as usize)).is_some_and(|t| t.protect_with_wall);
            if !walled || !self.node_built(house, node) || nodes.get(i + 1).is_some_and(|next| next.kind == Some(wall)) {
                continue;
            }
            let (Some(kind), Some((x0, y0))) = (node.kind, node.cell) else {
                continue;
            };
            let (w, h) = self.building_types[kind as usize].size;
            let mut cells = Vec::new();
            for k in 1..=w {
                cells.push((x0 - 1 + k, y0 - 1));
                cells.push((x0 - 1 + k, y0 + h));
            }
            for k in 1..=h {
                cells.push((x0 - 1, y0 - 1 + k));
                cells.push((x0 + w, y0 - 1 + k));
            }
            cells.extend([(x0 - 1, y0 - 1), (x0 + w, y0 - 1), (x0 - 1, y0 + h), (x0 + w, y0 + h)]);
            for cell in cells {
                nodes.insert(i + 1, Node { kind: Some(wall), cell: Some(cell), fails: 0, cover: None });
            }
            return true;
        }
        false
    }

    /// Whether a place in the plan has its building standing.
    fn node_built(&self, house: u8, node: &Node) -> bool {
        // A wall's place is built when its piece of wall is there, or any
        // building stands on the cell.
        if let (Some(overlay), Some(cell)) = (node.kind.and_then(|kind| self.building_types.get(kind as usize)).and_then(|t| t.wall_overlay), node.cell) {
            return self.map.index(cell.0, cell.1).is_some_and(|index| self.map.wall_kind[index] == overlay) || self.building_on(cell).is_some();
        }
        match (node.kind, node.cell) {
            (Some(kind), Some(cell)) => self.buildings.iter().any(|b| b.owner == house && b.kind == kind && b.cell == cell),
            _ => false,
        }
    }

    /// What a base-defence building type is worth against aircraft, armour and
    /// soldiers.
    // From its first weapon: damage over `ROF`
    // times 0.025, times the warhead's share against the fourth, fifth and first
    // armour, each capped at `MaximumBaseDefenseValue`.
    // DATA: the cap is rules.ini `[AI] MaximumBaseDefenseValue` (60).
    fn defense_values(&self, kind: u16) -> [i32; 3] {
        let Some(weapon) = self.building_types.get(kind as usize).filter(|_| self.is_defence(kind)).and_then(|t| t.fighter.primary).and_then(|m| self.weapons.get(m.weapon as usize)) else {
            return [0; 3];
        };
        let (Some(warhead), Some(projectile)) = (self.warheads.get(weapon.warhead as usize), self.projectiles.get(weapon.projectile as usize)) else {
            return [0; 3];
        };
        let base = (weapon.damage as f64 / (weapon.rof.max(1) as f64 * 0.025)) as i32 as f64;
        let value = |armour: usize, on: bool| if on { (base * warhead.verses[armour]).min(60.0) as i32 } else { 0 };
        [value(3, projectile.aa), value(4, projectile.ag), value(0, projectile.ag)]
    }

    /// The defence a computer house builds next.
    // The quarter of the base with
    // the least defence is found; of anti-air, anti-armour and anti-infantry
    // the kind that quarter is shortest of (each wants a third) is chosen; and
    // of the types of that kind the house can build, one at random, weighted
    // by its value plus 10000 over its cost.
    fn choose_defense(&mut self, house: u8, base: Option<(i32, i32)>) -> Option<(u16, (u8, u8))> {
        let base = base?;
        let mut sums = [[0i32; 3]; 4];
        for building in self.buildings.iter().filter(|b| b.owner == house && b.cell != base) {
            let values = self.defense_values(building.kind);
            if values.iter().sum::<i32>() <= 0 {
                continue;
            }
            let (dx, dy) = ((building.cell.0 - base.0) as f64, (building.cell.1 - base.1) as f64);
            let facing = ((dx.atan2(-dy) / std::f64::consts::TAU * 65536.0) as i32 & 0xffff) as u32;
            let quarter = ((((facing >> 13) + 1) >> 1) & 3) as usize;
            for (sum, value) in sums[quarter].iter_mut().zip(values) {
                *sum += value;
            }
        }
        let weakest = (0..4).min_by_key(|&q| sums[q].iter().sum::<i32>())?;
        let total = sums[weakest].iter().sum::<i32>();
        let share = |kind: usize| if total == 0 { 0.0 } else { sums[weakest][kind] as f64 / total as f64 };
        let (aa, armour, infantry) = (0.33 - share(0), 0.33 - share(1), 0.33 - share(2));
        let wanted = if aa > armour && aa > infantry {
            0
        } else if armour > infantry {
            1
        } else {
            2
        };
        let buildable = self.buildable(house);
        let list = |world: &Self, kind: usize| -> Vec<(u16, i32)> {
            buildable.iter().map(|&t| (t, world.defense_values(t)[kind])).filter(|&(t, value)| value > 0 && !world.building_types[t as usize].naval && !world.building_types[t as usize].water_bound).collect()
        };
        let (wanted, candidates) = [wanted, 1, 2, 0].into_iter().map(|kind| (kind, list(self, kind))).find(|(_, found)| !found.is_empty())?;
        let cover = (wanted as u8, weakest as u8);
        if candidates.len() == 1 {
            return Some((candidates[0].0, cover));
        }
        let weights: Vec<i32> = candidates.iter().map(|&(t, value)| value + 10000 / self.building_types[t as usize].cost.max(1)).collect();
        let roll = self.draw(1, weights.iter().sum::<i32>().max(1));
        let mut acc = 0;
        for (&(kind, _), weight) in candidates.iter().zip(weights) {
            acc += weight;
            if acc >= roll {
                return Some((kind, cover));
            }
        }
        candidates.last().map(|&(kind, _)| (kind, cover))
    }

    /// How well a cell is already covered by a house's defences against one
    /// kind of thing.
    // Each defence within six cells
    // adds its value, less with distance: over (max(d, 1) - 1) x 0.1 + 1, the
    // sum cut to a whole number at each addition.
    fn coverage(&self, house: u8, base: (i32, i32), cell: (i32, i32), kind: usize) -> i32 {
        let mut total = 0;
        for building in self.buildings.iter().filter(|b| b.owner == house && b.cell != base) {
            let values = self.defense_values(building.kind);
            if values.iter().sum::<i32>() <= 0 {
                continue;
            }
            let (dx, dy) = ((cell.0 - building.cell.0) as f64, (cell.1 - building.cell.1) as f64);
            let d = dx.hypot(dy);
            if dx.abs() > 6.0 || dy.abs() > 6.0 || d >= 6.0 {
                continue;
            }
            total = (total as f64 + values[kind].max(1) as f64 / ((d.max(1.0) - 1.0) * 0.1 + 1.0)) as i32;
        }
        total
    }

    /// A computer house's base: its plan, what to build next, and building it.
    fn run_base(&mut self, house: u8) {
        let h = house as usize;
        let Some(mut brain) = self.houses[h].ai.clone() else {
            return;
        };
        if !brain.awake {
            return;
        }
        // When the yard stands the
        // base is centred on its cell, the plan is made, and its first place is
        // the yard's.
        let yard = self.buildings.iter().find(|b| b.owner == house && self.building_types.get(b.kind as usize).is_some_and(|t| t.construction_yard)).map(|b| (b.kind, b.cell));
        if brain.nodes.is_empty() {
            let Some((kind, cell)) = yard else {
                return;
            };
            brain.nodes = self.generate_base_list(house);
            if let Some(first) = brain.nodes.iter_mut().find(|node| node.kind == Some(kind)) {
                first.cell = Some(cell);
            }
            brain.base_cell = Some(cell);
            // The deployed yard turns the
            // house's AI triggers on.
            brain.triggers_active = true;
            self.houses[h].base_centre = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
        }
        self.houses[h].ai = Some(brain.clone());
        // Buildings new to the house are added to its base's ground.
        let fresh: Vec<(u32, (i32, i32), (i32, i32))> =
            self.buildings.iter().filter(|b| b.owner == house && !brain.counted.contains(&b.id)).map(|b| (b.id, b.cell, b.size)).collect();
        for (id, cell, size) in fresh {
            self.mark_base_region(house, cell, size);
            if let Some(brain) = self.houses[h].ai.as_mut() {
                brain.counted.push(id);
            }
        }
        let Some(mut brain) = self.houses[h].ai.clone() else {
            return;
        };

        // With nothing decided and a
        // yard standing, the first place in the plan not yet built is next; a
        // power plant goes in before it if it would take the drain past the
        // output.
        // In mode 2 a building is
        // chosen only when the unit factories have nothing to ask for.
        // Not original behaviour: that is judged from the turn before.
        let building_turn = brain.mode != 2 || brain.units_stuck;
        if brain.build_structure.is_none() && yard.is_some() && building_turn {
            while let Some(at) = brain.nodes.iter().position(|node| !self.node_built(house, node)) {
                if brain.nodes[at].kind.is_none() {
                    // First, by the difficulty's `AIPickWallDefensePercent`, the
                    // place may go to a ring of walls instead (the number is
                    // drawn every time).
                    let roll = self.random_below(100) as i32;
                    if roll < self.ai_rules.wall_percent[brain.difficulty.min(2)] && self.add_wall_nodes(house, &mut brain.nodes, at) {
                        brain.nodes.remove(at);
                        break;
                    }
                    // A defence's place is given
                    // its type when its turn comes; with none to be had the
                    // place is dropped.
                    match self.choose_defense(house, brain.base_cell) {
                        Some((kind, cover)) => {
                            brain.nodes[at].kind = Some(kind);
                            brain.nodes[at].cover = Some(cover);
                        }
                        None => {
                            brain.nodes.remove(at);
                            continue;
                        }
                    }
                }
                let kind = brain.nodes[at].kind.unwrap_or(0);
                let t = &self.building_types[kind as usize];
                // Naval yards are left out where one
                // cannot be placed. Not original behaviour: left out always.
                if t.naval || t.water_bound {
                    brain.nodes.remove(at);
                    continue;
                }
                let (output, drain) = self.power(house);
                if t.power < 0 && drain - t.power > output && !self.ai_rules.build_const.contains(&kind) {
                    let plant = if self.houses[h].side == 0 { self.ai_rules.allied_power } else { self.ai_rules.soviet_power };
                    if let Some(plant) = plant {
                        brain.nodes.insert(at, Node { kind: Some(plant), cell: None, fails: 0, cover: None });
                        break;
                    }
                }
                // A wall's place keeps its cell; anything else is placed
                // afresh.
                if t.wall_overlay.is_none() {
                    brain.nodes[at].cell = None;
                }
                brain.build_structure = Some(kind);
                break;
            }
        }

        // The yard starts what has
        // been decided on, with at least 11 credits in hand, and places it when
        // it is done.
        if let Some(kind) = brain.build_structure {
            let combat = self.is_defence(kind);
            match self.houses[h].slot(combat) {
                None => {
                    if self.houses[h].credits >= 11 {
                        self.order(crate::Order::Build { house, building: kind });
                        if self.houses[h].slot(combat).is_none() {
                            // It cannot be built (yet): the place is given up.
                            if let Some(at) = brain.nodes.iter().position(|node| node.kind == Some(kind) && !self.node_built(house, node)) {
                                brain.nodes[at].fails += 1;
                                if brain.nodes[at].fails > 3 {
                                    brain.nodes.remove(at);
                                }
                            }
                            brain.build_structure = None;
                        }
                    }
                }
                Some(production) if production.is_ready() && production.building == kind => {
                    self.houses[h].ai = Some(brain.clone());
                    // A wall goes on its place's cell if it may stand there;
                    // otherwise the place is dropped.
                    if let Some(overlay) = self.building_types[kind as usize].wall_overlay {
                        let at = brain.nodes.iter().position(|node| node.kind == Some(kind) && node.cell.is_some() && !self.node_built(house, node));
                        *self.houses[h].slot_mut(combat) = None;
                        match at.and_then(|at| brain.nodes[at].cell.map(|cell| (at, cell))) {
                            Some((_, cell)) if self.can_place(house, kind, cell) => {
                                let line = self.wall_line(house, kind, cell);
                                for cell in std::iter::once(cell).chain(line) {
                                    self.map.put_wall(cell, overlay, 0, house);
                                    self.map.link_walls(cell);
                                }
                                self.map.build_zones();
                                brain.mode = self.next_production_mode(house, brain.mode, true);
                            }
                            other => {
                                self.houses[h].credits += self.building_types[kind as usize].cost - production.balance;
                                if let Some((at, _)) = other {
                                    brain.nodes.remove(at);
                                }
                            }
                        }
                    } else {
                        match self.find_build_location(house, kind) {
                            Some(cell) => {
                                *self.houses[h].slot_mut(combat) = None;
                                self.add_building(house, kind, cell);
                                self.map.build_zones();
                                if let Some(node) = brain.nodes.iter_mut().find(|node| node.kind == Some(kind) && node.cell.is_none()) {
                                    node.cell = Some(cell);
                                }
                                brain.mode = self.next_production_mode(house, brain.mode, true);
                            }
                            None => {
                                // With nowhere to put it the building is abandoned
                                // and what was paid handed back. Its place stays
                                // first in the plan, so the same building is
                                // started again, for as long as there is no room.
                                let paid = self.building_types[kind as usize].cost - production.balance;
                                self.houses[h].credits += paid;
                                *self.houses[h].slot_mut(combat) = None;
                            }
                        }
                    }
                    brain.build_structure = None;
                }
                Some(_) => {}
            }
        }

        // In mode 1 the unit
        // factories wait for the building in hand; in mode 2 a harvester asked
        // for keeps soldiers and aircraft waiting.
        if brain.mode == 1 && brain.build_structure.is_some() {
            self.finish_base_turn(h, brain);
            return;
        }
        // A harvester whenever the
        // house has fewer than `HarvestersPerRefinery` for each refinery.
        let harvester_wanted = brain.build_unit;
        if brain.build_unit.is_none() {
            let bit = 1u32 << self.houses[h].country.min(31);
            let harvester = self.ai_rules.harvesters.iter().copied().find(|&unit| self.unit_types.get(unit as usize).is_some_and(|t| t.owners & bit != 0));
            let refineries = self.buildings.iter().filter(|b| b.owner == house && self.ai_rules.build_refinery.contains(&b.kind)).count() as i32;
            if let Some(harvester) = harvester {
                let owned = self.units.iter().filter(|u| u.owner == house && u.stats.kind == harvester + 1).count() as i32;
                if self.ai_rules.harvesters_per_refinery * refineries > owned && !self.houses[house as usize].no_ore {
                    brain.build_unit = Some(harvester);
                }
            }
        }
        // With
        // nothing decided for a kind of factory, a member one of its forming
        // teams lacks.
        self.houses[h].ai = Some(brain.clone());
        if brain.build_unit.is_none() {
            brain.build_unit = self.team_build_choice(house, 1);
        }
        let miner_first = brain.mode == 2 && (harvester_wanted.is_some() || brain.build_unit.is_some_and(|unit| self.ai_rules.harvesters.contains(&unit)));
        if brain.build_infantry.is_none() && !miner_first {
            brain.build_infantry = self.team_build_choice(house, 0);
        }
        if brain.build_aircraft.is_none() && !miner_first {
            brain.build_aircraft = self.team_build_choice(house, 2);
        }
        brain.units_stuck = brain.build_unit.is_none() && brain.build_infantry.is_none() && brain.build_aircraft.is_none() && (2..6).all(|tab| self.houses[h].tab_slot(tab).is_none());
        for choice in [&mut brain.build_unit, &mut brain.build_infantry, &mut brain.build_aircraft] {
            if let Some(unit) = *choice {
                let tab = self.unit_slot(unit);
                if self.houses[h].tab_slot(tab).is_none() {
                    if self.houses[h].credits >= 11 {
                        self.order(crate::Order::BuildUnit { house, unit });
                    }
                    // Asked for once: the factory's own turn clears the choice.
                    *choice = None;
                }
            }
        }
        self.finish_base_turn(h, brain);
    }

    /// The production mode after something has left a factory.
    // With
    // `AIAlternateProductionCreditCutoff` or more everything is built at once;
    // with less the house takes a building, then units, in turn.
    pub(crate) fn next_production_mode(&mut self, house: u8, mode: u8, building: bool) -> u8 {
        let money = self.houses[house as usize].credits;
        let cut = self.ai_rules.credit_cutoff;
        match mode {
            0 if money < cut => if building { 2 } else { 1 },
            1 if money >= cut => 0,
            1 if building => 2,
            2 if money >= cut => 0,
            2 => {
                let owns = |list: &[u16]| self.buildings.iter().any(|b| b.owner == house && list.contains(&b.kind));
                let (output, drain) = self.power(house);
                if owns(&self.ai_rules.build_weapons) && owns(&self.ai_rules.build_barracks) && drain <= output && self.random_below(2) != 0 {
                    2
                } else if !building {
                    1
                } else {
                    2
                }
            }
            3 if money > cut => 0,
            other => other,
        }
    }

    fn finish_base_turn(&mut self, h: usize, mut brain: Brain) {
        if let Some(now) = self.houses[h].ai.as_ref() {
            // The team check and the hits taken meanwhile have their say.
            brain.team_timer = now.team_timer;
            brain.timer_armed = now.timer_armed;
            brain.all_hunt = now.all_hunt;
            brain.repair_at = now.repair_at;
            brain.enemy = now.enemy;
            brain.anger = now.anger.clone();
            brain.expert_at = now.expert_at;
            brain.last_attack = now.last_attack;
        }
        self.houses[h].ai = Some(brain);
    }
}
