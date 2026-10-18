//! Super weapons: gaining and losing them, charging, and what firing one does.
//!
//! Follows the original game
//!

use crate::aircraft::{Mission, Plane};
use crate::combat::Event;
use crate::{Land, Unit, UnitStats, World, LEPTONS_PER_CELL};

/// A `[SuperWeaponTypes]` entry.
#[derive(Clone, Copy, Debug)]
pub struct SuperType {
    /// `RechargeTime` in frames.
    pub recharge: u32,
    /// `Type`: 0 nuclear missile, 1 iron curtain, 2 lightning storm, 3
    /// chronosphere, 4 its second click, 5 paradrop, 6 American paradrop.
    pub kind: i32,
    pub powered: bool,
    pub show_timer: bool,
    /// `DisableableFromShell`: its building cannot be built with the
    /// skirmish screen's super weapons option off.
    pub disableable: bool,
}

/// One house's hold on one type.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Super {
    pub granted: bool,
    pub ready: bool,
    pub hold: bool,
    /// Frames still to charge.
    pub left: u32,
}

/// The rules the paradrops read: the plane, and each list of soldier types (by
/// place among the unit types) with how many of each.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    pub plane: Option<u16>,
    pub american: Vec<(u16, u32)>,
    pub allied: Vec<(u16, u32)>,
    pub soviet: Vec<(u16, u32)>,
    /// `ParadropRadius` in leptons.
    pub radius: i32,
    /// `ParachuteMaxFallRate`, as a speed downward.
    pub fall_rate: i32,
    /// `IronCurtainDuration` in frames.
    pub curtain_duration: u32,
    /// `LightningDeferment`, `LightningStormDuration`, `LightningDamage`,
    /// `LightningWarhead`, `LightningHitDelay`, `LightningScatterDelay`,
    /// `LightningCellSpread`, `LightningSeparation`.
    pub storm: StormRules,
    /// The `NukePayload` weapon.
    pub nuke_weapon: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StormRules {
    pub deferment: u32,
    pub duration: u32,
    pub damage: i32,
    pub warhead: u16,
    pub hit_delay: u32,
    pub scatter_delay: u32,
    pub spread: i32,
    pub separation: i32,
}

/// A lightning storm on its way or raging.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Storm {
    pub cell: (i32, i32),
    pub house: u8,
    /// Frames until it starts; then the frame it started.
    pub deferment: u32,
    pub started: Option<u64>,
    /// Clouds that have yet to strike: where, and when.
    clouds: Vec<((i32, i32), u64)>,
}

/// A nuclear missile on its way: whose, where, and when it lands.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Nuke {
    pub house: u8,
    pub cell: (i32, i32),
    pub lands: u64,
}

/// The cells an iron curtain or a chronosphere covers, from its middle.
const AROUND: [(i32, i32); 9] = [(0, 0), (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1), (0, 1), (1, 1)];

impl Super {
    pub fn can_fire(&self) -> bool {
        self.granted && self.ready && !self.hold
    }
}

impl World {
    /// The stage of a super weapon's clock, of 54.
    pub fn super_stage(&self, house: u8, index: usize) -> u32 {
        let Some((state, kind)) = self.houses.get(house as usize).and_then(|h| h.supers.get(index)).zip(self.super_types.get(index)) else {
            return 0;
        };
        if !state.granted {
            0
        } else if state.ready {
            54
        } else {
            let total = kind.recharge.max(1);
            (((total.saturating_sub(state.left)) as f64 / total as f64 * 54.0) as u32).min(53)
        }
    }

    /// Grants what a house's buildings newly provide, takes away what none
    /// provides any more, and charges the rest.
    // A building provides from
    // the moment it is placed; a powered type is held while the house is short
    // of power; the charge runs down a frame at a time and then it is ready.
    pub(crate) fn run_supers(&mut self) {
        let types = self.super_types.clone();
        for house in 0..self.houses.len() {
            if self.houses[house].supers.len() != types.len() {
                self.houses[house].supers = vec![Super::default(); types.len()];
            }
            let low = self.low_power(house as u8);
            for (index, kind) in types.iter().enumerate() {
                let provided = self.buildings.iter().any(|building| {
                    building.owner as usize == house
                        && self.building_types.get(building.kind as usize).is_some_and(|t| t.super_weapon == Some(index as u16) || t.super_weapon2 == Some(index as u16))
                });
                let state = &mut self.houses[house].supers[index];
                if !state.granted {
                    if provided {
                        *state = Super { granted: true, ready: false, hold: false, left: kind.recharge };
                        self.events.push(Event::SuperGranted { house: house as u8, index: index as u16 });
                    }
                    continue;
                }
                if !provided {
                    *state = Super::default();
                    self.events.push(Event::SuperLost { house: house as u8, index: index as u16 });
                    continue;
                }
                state.hold = kind.powered && low;
                if state.ready || state.hold {
                    continue;
                }
                state.left = state.left.saturating_sub(1);
                if state.left == 0 {
                    state.ready = true;
                    self.events.push(Event::SuperReady { house: house as u8, index: index as u16 });
                }
            }
        }

        // Soldiers under parachutes come down.
        // The height changes by the fall rate, which
        // then grows by one a frame up to `ParachuteMaxFallRate`; on the ground
        // the fall is over.
        let (most, tick) = (self.super_rules.fall_rate.max(1), self.tick);
        let mut landed = Vec::new();
        for unit in &mut self.units {
            if let Some(height) = unit.falling {
                // With no
                // parachute the fall gathers 1.4 a frame, up to 100.
                let rate = if unit.doomed { ((tick.saturating_sub(unit.dropped_at) as f64 * 1.4) as i32).clamp(1, 100) } else { (tick.saturating_sub(unit.dropped_at) as i32).min(most) };
                unit.falling = (height > rate).then(|| height - rate);
                if unit.falling.is_none() {
                    // On the
                    // ground he looks round himself afresh.
                    unit.looked = None;
                    unit.landed_at = tick;
                    landed.push(unit.id);
                }
            }
        }
        // One that fell with a bridge dies where
        // it lands, on land or water.
        let doomed: Vec<u32> = landed.iter().copied().filter(|&id| self.unit(id).is_some_and(|unit| unit.doomed)).collect();
        for id in doomed {
            self.kill(crate::combat::Aim::Unit(id));
        }
        // What they were told on the way down starts now.
        for id in landed {
            if let Some(at) = self.landing_orders.iter().position(|order| order.unit() == Some(id)) {
                let order = self.landing_orders.remove(at);
                self.order(order);
            }
        }
    }

    /// Fires a house's super weapon at a cell.
    // It must be granted, ready and not
    // held; once launched it is no longer ready and its charge starts again.
    pub(crate) fn fire_super(&mut self, house: u8, index: usize, cell: (i32, i32)) {
        let Some(kind) = self.super_types.get(index).copied() else {
            return;
        };
        // The chronosphere's second click needs
        // neither a grant nor a charge, only the first click before it.
        if kind.kind == 4 {
            let Some(from) = self.houses.get_mut(house as usize).and_then(|h| h.chrono_source.take()) else {
                return;
            };
            self.chrono_shift(house, from, cell);
            if let Some(first) = self.super_types.iter().position(|t| t.kind == 3) {
                let recharge = self.super_types[first].recharge;
                let state = &mut self.houses[house as usize].supers[first];
                state.ready = false;
                state.left = recharge;
            }
            self.events.push(Event::SuperFired { house, kind: 4, cell, from: Some(from) });
            return;
        }
        if !self.houses.get(house as usize).and_then(|h| h.supers.get(index)).is_some_and(Super::can_fire) {
            return;
        }
        match kind.kind {
            5 | 6 => self.launch_paradrop(house, kind.kind == 6, cell),
            // With no silo nothing is launched but
            // the charge starts again all the same.
            0 => {
                let silo = self.buildings.iter().any(|b| b.owner == house && self.building_types.get(b.kind as usize).is_some_and(|t| t.super_weapon == Some(index as u16) || t.super_weapon2 == Some(index as u16)));
                if silo {
                    self.nukes.push(Nuke { house, cell, lands: self.tick + 150 + 200 });
                }
            }
            1 => self.iron_curtain(cell),
            // A storm is put off by
            // `LightningDeferment`; while one rages another only moves the
            // target for the next.
            2 => {
                let deferment = self.super_rules.storm.deferment;
                match self.storm.as_mut() {
                    Some(storm) if storm.started.is_some() => {}
                    Some(storm) => {
                        storm.cell = cell;
                        storm.house = house;
                        storm.deferment = storm.deferment.min(deferment);
                    }
                    None => self.storm = Some(Storm { cell, house, deferment, started: None, clouds: Vec::new() }),
                }
            }
            // The first click only remembers the
            // cell; the charge is not spent until the second.
            3 => {
                self.houses[house as usize].chrono_source = Some(cell);
                self.events.push(Event::SuperFired { house, kind: 3, cell, from: None });
                return;
            }
            _ => return,
        }
        self.events.push(Event::SuperFired { house, kind: kind.kind, cell, from: None });
        let state = &mut self.houses[house as usize].supers[index];
        state.ready = false;
        state.left = kind.recharge;
    }

    /// The iron curtain comes down on the nine cells round a cell.
    // Soldiers and
    // `Organic` things are killed; vehicles and buildings can not be hurt for
    // `IronCurtainDuration` frames.
    fn iron_curtain(&mut self, cell: (i32, i32)) {
        let until = self.tick + self.super_rules.curtain_duration as u64;
        for (dx, dy) in AROUND {
            let at = (cell.0 + dx, cell.1 + dy);
            let here: Vec<(u32, bool)> = self.units.iter().filter(|u| u.cell() == at && self.altitude(u) == 0).map(|u| (u.id, u.stats.infantry || u.stats.fighter.organic)).collect();
            for (id, dies) in here {
                if dies {
                    self.kill(crate::combat::Aim::Unit(id));
                } else if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                    unit.curtain_until = until;
                }
            }
            if let Some(index) = self.building_on(at) {
                self.buildings[index].curtain_until = until;
            }
        }
    }

    /// The chronosphere moves what stands on the nine cells round one cell to
    /// those round another.
    // Things in the air are left; `Organic` things
    // that do not teleport of themselves are killed; those under the iron
    // curtain are left; each keeps its place in the square.
    fn chrono_shift(&mut self, _house: u8, from: (i32, i32), to: (i32, i32)) {
        for (dx, dy) in AROUND {
            let (source, dest) = ((from.0 + dx, from.1 + dy), (to.0 + dx, to.1 + dy));
            let here: Vec<u32> = self.units.iter().filter(|u| u.cell() == source && self.altitude(u) == 0).map(|u| u.id).collect();
            for id in here {
                let Some(index) = self.units.iter().position(|u| u.id == id) else {
                    continue;
                };
                let unit = &self.units[index];
                if (unit.stats.infantry || unit.stats.fighter.organic) && !unit.stats.teleporter {
                    self.kill(crate::combat::Aim::Unit(id));
                    continue;
                }
                if unit.curtain_until > self.tick {
                    continue;
                }
                let mut moved = self.lift(index);
                moved.dest = None;
                moved.head_to = None;
                moved.drop_path();
                let back = moved.clone();
                if !self.set_down(moved, dest) {
                    self.set_down(back, source);
                }
            }
        }
    }

    /// Storms and missiles in the air.
    pub(crate) fn run_weather(&mut self) {
        // The payload goes off
        // at the target with the `NukePayload` weapon's damage and warhead.
        let tick = self.tick;
        let landed: Vec<Nuke> = self.nukes.iter().copied().filter(|nuke| nuke.lands <= tick).collect();
        self.nukes.retain(|nuke| nuke.lands > tick);
        for nuke in landed {
            let Some(weapon) = self.super_rules.nuke_weapon.and_then(|index| self.weapons.get(index as usize).cloned()) else {
                continue;
            };
            let (x, y) = (nuke.cell.0 * LEPTONS_PER_CELL + 128, nuke.cell.1 * LEPTONS_PER_CELL + 128);
            let point = (x, y, self.ground_z(x, y));
            self.explode(point, weapon.damage, weapon.warhead, None);
            self.irradiate(point, weapon.rad_level, weapon.warhead);
            self.blast_event(weapon.warhead, weapon.damage, point, false);
        }
        // While the storm rages a cloud forms over
        // the target every `LightningHitDelay` frames, and every
        // `LightningScatterDelay` one at a random cell within half
        // `LightningCellSpread` of it that no cloud is within
        // `LightningSeparation` cells of (three tries); a cloud strikes half-way
        // through its life, with `LightningDamage` and nobody to blame.
        let Some(mut storm) = self.storm.take() else {
            return;
        };
        let rules = self.super_rules.storm;
        match storm.started {
            None => {
                storm.deferment = storm.deferment.saturating_sub(1);
                if storm.deferment == 0 {
                    storm.started = Some(tick);
                    self.events.push(Event::StormStarted { cell: storm.cell });
                }
            }
            Some(started) => {
                const STRIKE_AFTER: u64 = 20;
                if started + rules.duration as u64 >= tick {
                    let mut cloud = |storm: &mut Storm, events: &mut Vec<Event>, cell: (i32, i32)| {
                        storm.clouds.push((cell, tick + STRIKE_AFTER));
                        events.push(Event::Cloud { cell });
                    };
                    if rules.hit_delay > 0 && tick % rules.hit_delay as u64 == 0 {
                        let centre = storm.cell;
                        cloud(&mut storm, &mut self.events, centre);
                    }
                    if rules.scatter_delay > 0 && tick % rules.scatter_delay as u64 == 0 {
                        for _ in 0..3 {
                            let span = rules.spread.max(1) as u32;
                            let cell = (storm.cell.0 + self.random_below(span) as i32 - rules.spread / 2, storm.cell.1 + self.random_below(span) as i32 - rules.spread / 2);
                            let apart = storm.clouds.iter().all(|(other, _)| (other.0 - cell.0).abs() + (other.1 - cell.1).abs() >= rules.separation);
                            if self.map.in_playfield(cell.0, cell.1) && apart {
                                cloud(&mut storm, &mut self.events, cell);
                                break;
                            }
                        }
                    }
                }
                let due: Vec<(i32, i32)> = storm.clouds.iter().filter(|(_, at)| *at <= tick).map(|(cell, _)| *cell).collect();
                storm.clouds.retain(|(_, at)| *at > tick);
                for cell in due {
                    let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                    let point = (x, y, self.ground_z(x, y));
                    self.events.push(Event::Bolt { cell });
                    self.explode(point, rules.damage, rules.warhead, None);
                    self.blast_event(rules.warhead, rules.damage, point, false);
                }
                if started + (rules.duration as u64) < tick && storm.clouds.is_empty() {
                    return;
                }
            }
        }
        self.storm = Some(storm);
    }

    // A target on water is moved to dry
    // ground nearby if there is any; the American drop has its own list, the
    // other takes the Allied or the Soviet list by the house's side; one plane
    // for each entry of the list.
    fn launch_paradrop(&mut self, house: u8, american: bool, cell: (i32, i32)) {
        let dry = |world: &World, cell: (i32, i32)| world.map.index(cell.0, cell.1).is_some_and(|index| world.map.land[index] != Land::Water as u8);
        let mut target = cell;
        if !dry(self, target) {
            'search: for ring in 1..12i32 {
                for dy in -ring..=ring {
                    for dx in -ring..=ring {
                        let at = (cell.0 + dx, cell.1 + dy);
                        if dx.abs().max(dy.abs()) == ring && self.map.in_playfield(at.0, at.1) && dry(self, at) {
                            target = at;
                            break 'search;
                        }
                    }
                }
            }
        }
        let rules = self.super_rules.clone();
        let list = if american {
            rules.american
        } else if self.houses[house as usize].side == 0 {
            rules.allied
        } else {
            rules.soviet
        };
        for (soldier, count) in list {
            self.create_paradrop(house, target, soldier, count);
        }
    }

    // The plane is made at a cell on the house's edge of the map on the ground,
    // with the soldiers inside and the drop cell as its target, on the approach
    // mission.
    fn create_paradrop(&mut self, house: u8, target: (i32, i32), soldier: u16, count: u32) {
        let Some(stats) = self.super_rules.plane.and_then(|kind| self.unit_types.get(kind as usize).map(|t| UnitStats { kind: kind + 1, ..t.stats })) else {
            return;
        };
        let Some(plane_stats) = stats.plane else {
            return;
        };
        let Some(soldier_stats) = self.unit_types.get(soldier as usize).map(|t| UnitStats { kind: soldier + 1, infantry: true, ..t.stats }) else {
            return;
        };
        let entry = self.edge_cell();
        let id = self.next_id;
        self.next_id += 1;
        let mut unit = Unit::new(id, house, entry, 0, stats);
        let mut plane = Plane::parked(&plane_stats);
        // It is born at flight level, facing north.
        plane.z = plane_stats.flight_level;
        plane.mission = Mission::ParadropApproach;
        plane.drop = Some(target);
        plane.passes = 5;
        unit.plane = Some(plane);
        for _ in 0..count {
            let id = self.next_id;
            self.next_id += 1;
            unit.cargo.push(Unit::new(id, house, entry, 0x80, soldier_stats));
        }
        self.units.push(unit);
    }

    /// A cell on the top edge of the playing area, anywhere along it.
    // The edge and a random place along it.
    pub(crate) fn edge_cell(&mut self) -> (i32, i32) {
        let (width, height) = self.map.size();
        let Some((w, [lx, ly, lw, _])) = self.map.playfield else {
            return (self.random_below(width.max(1) as u32) as i32, 0);
        };
        let sum = w + 2 * ly + 2;
        let (low, high) = (2 * lx - w + 2, 2 * (lx + lw) - w - 2);
        let mut across = low + self.random_below((high - low).max(1) as u32) as i32;
        if (sum + across) % 2 != 0 {
            across += 1;
        }
        (((sum + across) / 2).clamp(0, width - 1), ((sum - across) / 2).clamp(0, height - 1))
    }

    /// The paradrop plane's missions. Returns the frames until the next.
    pub(crate) fn paradrop_mission(&mut self, index: usize, plane: &mut Plane) -> u64 {
        let radius = self.super_rules.radius as i64;
        let (id, here, at, loaded) = {
            let unit = &self.units[index];
            (unit.id, unit.cell(), (unit.x, unit.y), !unit.cargo.is_empty())
        };
        let far = |cell: (i32, i32)| {
            let (dx, dy) = ((cell.0 * LEPTONS_PER_CELL + 128 - at.0) as i64, (cell.1 * LEPTONS_PER_CELL + 128 - at.1) as i64);
            dx * dx + dy * dy > radius * radius
        };
        match plane.mission {
            Mission::ParadropApproach => {
                if plane.drop.is_none() {
                    self.plane_retreat(index, plane);
                    return 3;
                }
                if plane.drop.is_some_and(|target| !far(target)) {
                    plane.mission = Mission::ParadropOverfly;
                    plane.passes = plane.passes.saturating_sub(1);
                }
                3
            }
            Mission::ParadropOverfly => {
                match plane.drop.filter(|_| loaded) {
                    Some(target) if far(target) => {
                        if plane.passes > 0 {
                            plane.mission = Mission::ParadropApproach;
                        } else {
                            self.plane_retreat(index, plane);
                        }
                    }
                    Some(_) => {
                        if self.map.in_playfield(here.0, here.1) {
                            self.drop_one(index, plane);
                        }
                    }
                    None => self.plane_retreat(index, plane),
                }
                5
            }
            // It flies for a cell on the
            // edge and is gone on the first frame it is off the playing area
            // (`paradrop_fly`).
            _ => {
                let _ = id;
                5
            }
        }
    }

    /// One frame of the cargo plane's flight. Returns whether it has left the map.
    // It keeps
    // `FlightLevel` above the ground under it and never comes down; 38 leptons
    // a frame; it turns towards where it is going by its `ROT` (two of 256 a
    // frame), but while dropping it flies dead straight; on Retreat it is
    // removed when it is off the playing area.
    pub(crate) fn paradrop_fly(&mut self, index: usize, plane: &mut Plane) -> bool {
        let level = self.units[index].stats.plane.map_or(1500, |stats| stats.flight_level);
        plane.z = level;
        let goal = match plane.mission {
            Mission::ParadropApproach => plane.drop,
            Mission::Retreat => plane.retreat,
            _ => None,
        };
        let unit = &mut self.units[index];
        if let (Mission::Retreat, Some(goal)) = (plane.mission, goal) {
            let (dx, dy) = ((goal.0 * LEPTONS_PER_CELL + 128 - unit.x) as f64, (goal.1 * LEPTONS_PER_CELL + 128 - unit.y) as f64);
            if dx.hypot(dy) < LEPTONS_PER_CELL as f64 {
                plane.retreat = None;
            }
        }
        let goal = if plane.mission == Mission::Retreat { plane.retreat } else { goal };
        if let Some(goal) = goal {
            let (dx, dy) = ((goal.0 * LEPTONS_PER_CELL + 128 - unit.x) as f64, (goal.1 * LEPTONS_PER_CELL + 128 - unit.y) as f64);
            let wanted = (dx.atan2(-dy) / std::f64::consts::TAU * 256.0).rem_euclid(256.0) as i32;
            let turn = ((wanted - unit.facing as i32 + 128).rem_euclid(256)) - 128;
            unit.facing = ((unit.facing as i32 + turn.clamp(-2, 2)).rem_euclid(256)) as u8;
            // At full speed, so above any `PitchSpeed` under 1, it leans into
            // the turn.
            let fast = unit.stats.plane.is_some_and(|stats| stats.pitch_speed < 1000);
            plane.bank = if fast { turn.signum() as i8 } else { 0 };
        } else {
            plane.bank = 0;
        }
        let angle = unit.facing as f64 / 256.0 * std::f64::consts::TAU;
        unit.x += (angle.sin() * 38.0).round() as i32;
        unit.y -= (angle.cos() * 38.0).round() as i32;
        let cell = unit.cell();
        plane.mission == Mission::Retreat && !self.map.in_playfield(cell.0, cell.1)
    }

    fn plane_retreat(&mut self, _index: usize, plane: &mut Plane) {
        let edge = self.edge_cell();
        plane.drop = None;
        plane.retreat = Some(edge);
        plane.mission = Mission::Retreat;
    }

    // The first passenger goes out 128 leptons to
    // one side of the plane, sides taking turns, at the plane's height; if the
    // cell there will not have it, it stays aboard. Each one out sets the
    // passes left to five. It faces
    // south, falling under a parachute, and stands guard.
    fn drop_one(&mut self, index: usize, plane: &mut Plane) {
        if self.units[index].cargo.is_empty() {
            return;
        }
        plane.drops = plane.drops.wrapping_sub(1);
        let unit = &self.units[index];
        let turn = if plane.drops & 1 != 0 { -64i32 } else { 64 };
        let angle = ((unit.facing as i32 + turn) & 0xff) as f64 / 256.0 * std::f64::consts::TAU;
        let (x, y) = (unit.x + (angle.sin() * 128.0) as i32, unit.y - (angle.cos() * 128.0) as i32);
        let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        let (height, carrier) = (plane.z, unit.id);
        let open = self.map.index(cell.0, cell.1).filter(|&at| {
            self.map.in_playfield(cell.0, cell.1)
                && self.map.speed_percent(cell.0, cell.1, self.units[index].cargo[0].stats.speed_type) > 0
                && self.claims[at] == 0
                && self.building_on(cell).is_none()
        });
        let spot = open.and_then(|at| (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0).map(|spot| (at, spot)));
        let Some((at, spot)) = spot else {
            plane.drops = plane.drops.wrapping_add(1);
            return;
        };
        let mut soldier = self.units[index].cargo.remove(0);
        soldier.x = cell.0 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].0;
        soldier.y = cell.1 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].1;
        soldier.facing = 0x80;
        soldier.falling = Some(height.max(1));
        soldier.dropped_at = self.tick;
        self.spots[at] |= 1 << spot;
        self.spot_house[at] = soldier.owner;
        self.events.push(Event::Parachuted { unit: soldier.id, from: carrier });
        self.put_back(soldier);
        plane.passes = 5;
    }
}
