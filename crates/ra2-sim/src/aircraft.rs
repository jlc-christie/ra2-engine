//! Aircraft: the Fly locomotor and the aircraft's own missions.
//!
//! Follows the original game, sections 3a and 3b (the missions, docks, ammunition and
//! production). What is here is what the two aircraft a player can build need (the
//! Harrier and the Black Eagle: `Fighter`, `AirportBound`, one shot a sortie).

use crate::combat::{Aim, FireError, LEVEL_HEIGHT};
use crate::{cell_centre, Turn, Unit, World, LEPTONS_PER_CELL};

/// What makes a type an aircraft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlaneStats {
    /// The type's `FlightLevel`, or the rules' own (1500).
    pub flight_level: i32,
    pub fighter: bool,
    /// `AirportBound`: it can only come down on a pad, and falls without one.
    pub airport_bound: bool,
    pub landable: bool,
    pub ammo: i32,
    /// `SlowdownDistance` (500 unless given).
    pub slowdown: i32,
    /// `PitchSpeed` in thousandths of full speed (250 unless given): above it
    /// the model leans into its turns.
    pub pitch_speed: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mission {
    Guard,
    Move,
    Attack,
    Enter,
    /// On the pad, being reloaded.
    Sleep,
    /// The paradrop plane: flying to the drop, over it, and away.
    ParadropApproach,
    ParadropOverfly,
    Retreat,
}

/// Where an aircraft has been sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Goal {
    Cell(i32, i32),
    Building(u32),
}

/// An aircraft's flight and mission.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Plane {
    /// Height above the ground.
    pub z: i32,
    /// The locomotor's own destination, and whether it has been told to move.
    moving_to: Option<(i32, i32, i32)>,
    has_order: bool,
    /// The height being held.
    flight_level: i32,
    /// Speed wanted and speed now, as shares of full speed.
    target_speed: f64,
    cur_speed: f64,
    taking_off: bool,
    pub landing: bool,
    /// Stay up over the destination (an attack run).
    elevating: bool,
    fall_speed: i32,
    pub ammo: i32,
    /// A shot has been fired and its ammunition not yet taken off.
    pending: bool,
    /// On its firing pass: it neither turns nor slows.
    locked: bool,
    pub mission: Mission,
    state: u8,
    wake: u64,
    goal: Option<Goal>,
    /// The pad it last used, and the building it is linked with now.
    home: Option<u32>,
    pub contact: Option<u32>,
    pub crashing: bool,
    pub crashed: bool,
    /// A paradrop plane's drop cell, passes left, count of drops (its low bit
    /// is the side the next goes out of), and the edge cell it leaves by.
    pub(crate) drop: Option<(i32, i32)>,
    pub(crate) passes: u8,
    pub(crate) drops: u8,
    pub(crate) retreat: Option<(i32, i32)>,
    /// The landing sound has been heard on this way down.
    landing_heard: bool,
    /// Leaning into a turn: 1 turning clockwise, -1 the other way, 0 level.
    pub bank: i8,
}

impl Plane {
    pub(crate) fn parked(stats: &PlaneStats) -> Self {
        Self {
            z: 0,
            moving_to: None,
            has_order: false,
            flight_level: 0,
            target_speed: 0.0,
            cur_speed: 0.0,
            taking_off: false,
            landing: false,
            elevating: false,
            fall_speed: 0,
            ammo: stats.ammo,
            pending: false,
            locked: false,
            mission: Mission::Guard,
            state: 0,
            wake: 0,
            goal: None,
            home: None,
            contact: None,
            crashing: false,
            crashed: false,
            drop: None,
            passes: 5,
            drops: 0,
            retreat: None,
            landing_heard: false,
            bank: 0,
        }
    }

    /// Whether it has been sent to a cell.
    pub(crate) fn heading_for(&self, cell: (i32, i32)) -> bool {
        self.goal == Some(Goal::Cell(cell.0, cell.1))
    }

    /// Told to move, or still going.
    pub fn is_moving(&self) -> bool {
        self.has_order || self.cur_speed > 0.0
    }

    /// Whether the player's orders can have it attack: it has something to fire.
    pub fn has_ammo(&self) -> bool {
        self.ammo != 0
    }
}

fn cell_of(x: i32, y: i32) -> (i32, i32) {
    (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL))
}

/// The direction from one point to another as a 16-bit facing.
fn direction(from: (i32, i32), to: (i32, i32)) -> u16 {
    if from == to {
        return 0x4000;
    }
    ((((from.1 - to.1) as f64).atan2((to.0 - from.0) as f64) - std::f64::consts::FRAC_PI_2) * -10430.06004) as i32 as u16
}

fn angle_of(facing: u16) -> f64 {
    (facing as i16 as f64 - 16383.0) * (-std::f64::consts::PI / 32768.0)
}

impl World {
    /// The total of `NumberOfDocks` over a house's pad buildings.
    pub fn dock_count(&self, house: u8) -> usize {
        self.buildings.iter().filter(|b| b.owner == house).map(|b| b.docks.len()).sum()
    }

    /// Where an aircraft sits on a pad building.
    // The building's own point plus the docking offset of the slot the aircraft
    // holds; without a slot, the building's point.
    pub fn dock_point(&self, building: u32, plane: u32) -> Option<(i32, i32, i32)> {
        let building = self.buildings.iter().find(|b| b.id == building)?;
        let kind = self.building_types.get(building.kind as usize)?;
        let (x, y) = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128);
        let slot = building.docks.iter().position(|slot| *slot == Some(plane));
        let offset = slot.and_then(|slot| kind.docking_offsets.get(slot)).copied().unwrap_or((0, 0));
        Some((x + offset.0, y + offset.1, 0))
    }

    /// The nearest pad building of the types an aircraft docks at that has room
    /// for it (or already holds it).
    // The owner's buildings of each
    // `Dock` type with a free slot, the nearest first; an `AirportBound` one asks
    // the pad it last used first.
    fn find_pad(&self, unit: &Unit, home: Option<u32>) -> Option<u32> {
        let docks = &self.unit_types.get((unit.stats.kind as usize).wrapping_sub(1))?.dock;
        let free = |b: &crate::Building| {
            b.owner == unit.owner && b.buildup.is_none() && docks.contains(&b.kind) && b.docks.iter().any(|slot| slot.is_none() || *slot == Some(unit.id))
        };
        if let Some(home) = home.filter(|_| unit.stats.plane.is_some_and(|p| p.airport_bound))
            && self.buildings.iter().any(|b| b.id == home && free(b)) {
                return Some(home);
            }
        self.buildings
            .iter()
            .filter(|b| free(b))
            .min_by_key(|b| {
                let (dx, dy) = ((b.cell.0 * LEPTONS_PER_CELL - unit.x) as i64, (b.cell.1 * LEPTONS_PER_CELL - unit.y) as i64);
                dx * dx + dy * dy
            })
            .map(|b| b.id)
    }

    /// Links an aircraft with a pad building, taking its first free slot.
    // The lowest empty slot; none, and it is refused.
    fn link(&mut self, plane: &mut Plane, id: u32, building: u32) -> bool {
        let Some(b) = self.buildings.iter_mut().find(|b| b.id == building) else {
            return false;
        };
        if !b.docks.contains(&Some(id)) {
            match b.docks.iter().position(Option::is_none) {
                Some(slot) => b.docks[slot] = Some(id),
                None => return false,
            }
        }
        plane.contact = Some(building);
        plane.home = Some(building);
        true
    }

    fn unlink(&mut self, plane: &mut Plane, id: u32) {
        if let Some(building) = plane.contact.take()
            && let Some(b) = self.buildings.iter_mut().find(|b| b.id == building) {
                for slot in &mut b.docks {
                    if *slot == Some(id) {
                        *slot = None;
                    }
                }
            }
    }

    fn floor(&self, x: i32, y: i32) -> i32 {
        let cell = cell_of(x, y);
        self.map.level(cell.0, cell.1) as i32 * LEVEL_HEIGHT
    }

    /// Puts a new aircraft on a pad of one of its owner's buildings.
    // Only where a pad has a free slot; it appears on the slot's point, on the
    // ground, linked to the building, which becomes its home.
    pub(crate) fn exit_aircraft(&mut self, house: u8, stats: crate::UnitStats) -> bool {
        let Some(plane_stats) = stats.plane else {
            return false;
        };
        let probe = Unit::new(u32::MAX - 1, house, (0, 0), 0, stats);
        let Some(building) = self.find_pad(&probe, None) else {
            return false;
        };
        let id = self.next_id;
        self.next_id += 1;
        let mut unit = Unit::new(id, house, (0, 0), 0x40, stats);
        let mut plane = Plane::parked(&plane_stats);
        if !self.link(&mut plane, id, building) {
            return false;
        }
        if let Some((x, y, _)) = self.dock_point(building, id) {
            unit.x = x;
            unit.y = y;
        }
        // On the pad its mission is to be loaded; it is made full.
        plane.mission = Mission::Guard;
        unit.plane = Some(plane);
        self.units.push(unit);
        true
    }

    /// Sends an aircraft somewhere (its Destination), and tells its locomotor.
    // Given anywhere but its pad, a parked aircraft
    // lets go of the pad's slot.
    fn set_goal(&mut self, index: usize, plane: &mut Plane, goal: Option<Goal>) {
        let id = self.units[index].id;
        if let Some(contact) = plane.contact
            && goal.is_some() && goal != Some(Goal::Building(contact)) {
                self.unlink(plane, id);
            }
        plane.goal = goal;
        let coord = match goal {
            None => None,
            Some(Goal::Cell(x, y)) => {
                let (cx, cy) = cell_centre((x, y));
                Some((cx, cy, self.floor(cx, cy)))
            }
            Some(Goal::Building(building)) => self.dock_point(building, id).map(|(x, y, _)| (x, y, self.floor(x, y))),
        };
        self.move_to(index, plane, coord);
    }

    /// The locomotor's move order.
    fn move_to(&mut self, index: usize, plane: &mut Plane, coord: Option<(i32, i32, i32)>) {
        let unit = &self.units[index];
        let Some(stats) = unit.stats.plane else {
            return;
        };
        let on_run = unit.arms.target.is_some() && plane.ammo != 0;
        let Some(mut coord) = coord else {
            if (plane.z > 0 || plane.taking_off) && !plane.landing {
                plane.moving_to = Some((unit.x, unit.y, self.floor(unit.x, unit.y)));
                self.begin_landing(index, plane);
            } else {
                plane.moving_to = None;
            }
            plane.elevating = false;
            return;
        };
        if plane.landing && plane.moving_to.is_some_and(|at| cell_of(at.0, at.1) == cell_of(coord.0, coord.1)) {
            return;
        }
        let floor = self.floor(coord.0, coord.1);
        if on_run {
            coord.2 = floor + stats.flight_level;
        }
        plane.moving_to = Some(coord);
        plane.has_order = true;
        if !plane.taking_off && (plane.landing || plane.z <= 0) {
            self.begin_takeoff(index, plane);
        }
        plane.elevating = coord.2 > floor + 120 || on_run || !stats.landable;
    }

    fn begin_takeoff(&mut self, index: usize, plane: &mut Plane) {
        let unit = &self.units[index];
        plane.landing = false;
        plane.taking_off = true;
        plane.flight_level = unit.stats.plane.map_or(1500, |stats| stats.flight_level);
        if plane.z == 0 {
            self.events.push(crate::combat::Event::TookOff { unit: unit.id });
        }
    }

    /// An `AirportBound` aircraft over anything but
    /// its own pad does not come down: it goes idle (and so looks for a pad).
    fn begin_landing(&mut self, index: usize, plane: &mut Plane) {
        let unit = &self.units[index];
        let bound = unit.stats.plane.is_some_and(|stats| stats.airport_bound);
        let under = self.building_on(unit.cell()).map(|i| self.buildings[i].id);
        if bound && (under.is_none() || under != plane.contact) {
            self.plane_idle(index, plane);
            return;
        }
        plane.taking_off = false;
        plane.landing = true;
        plane.landing_heard = false;
        plane.flight_level = 0;
    }

    /// Brings an aircraft down out of control.
    // Its health goes to nothing, its link is
    // dropped, and the locomotor lets it fall.
    fn crash(&mut self, index: usize, plane: &mut Plane) {
        let id = self.units[index].id;
        self.unlink(plane, id);
        self.units[index].health = 0;
        plane.crashing = true;
    }

    /// What an aircraft does when it has nothing to do: in the air, go back to a
    /// pad.
    fn plane_idle(&mut self, index: usize, plane: &mut Plane) {
        let unit = &self.units[index];
        let Some(stats) = unit.stats.plane else {
            return;
        };
        let (id, armed) = (unit.id, unit.stats.fighter.primary.is_some());
        let mut mission = Mission::Guard;
        if plane.z > 0 {
            if plane.ammo != 0 && unit.arms.target.is_some() && plane.mission == Mission::Attack {
                mission = Mission::Attack;
            } else {
                let dock = self.find_pad(unit, plane.home);
                match dock.filter(|&dock| self.link(plane, id, dock)) {
                    Some(dock) => {
                        self.set_goal(index, plane, Some(Goal::Building(dock)));
                        mission = Mission::Enter;
                    }
                    None if stats.airport_bound => {
                        self.crash(index, plane);
                        return;
                    }
                    None => {
                        let cell = self.units[index].cell();
                        self.set_goal(index, plane, Some(Goal::Cell(cell.0, cell.1)));
                        mission = Mission::Move;
                    }
                }
            }
        } else {
            plane.goal = None;
            self.units[index].arms.target = None;
        }
        if plane.ammo == 0 && armed && plane.contact.is_none() {
            let unit = &self.units[index];
            match self.find_pad(unit, plane.home).filter(|&dock| self.link(plane, id, dock)) {
                Some(dock) => {
                    self.set_goal(index, plane, Some(Goal::Building(dock)));
                    self.units[index].arms.target = None;
                    mission = Mission::Enter;
                }
                None if stats.airport_bound && plane.z > 0 => {
                    self.crash(index, plane);
                    return;
                }
                None => {}
            }
        }
        if plane.contact.is_some() && mission != Mission::Attack {
            mission = Mission::Enter;
        }
        plane.mission = mission;
        plane.state = 0;
        plane.wake = self.tick;
    }

    /// A cell to attack from.
    // Rings in from a cell short of the weapon's
    // range, sixteen points round each; the nearest to the aircraft and the one
    // before it are kept, and one of the two is taken at random.
    fn attack_position(&mut self, index: usize, target: Aim) -> Option<(i32, i32)> {
        let unit = &self.units[index];
        let aim = self.aim_point(target)?;
        let weapon = unit.stats.fighter.primary.and_then(|mount| self.weapons.get(mount.weapon as usize))?;
        let mut reach = weapon.range - 0x100;
        let (ux, uy) = (unit.x, unit.y);
        while reach > 0x100 {
            let mut best: Option<((i32, i32), i64)> = None;
            let mut previous = None;
            for step in 0..16 {
                let angle = angle_of((step as u16) << 12);
                let point = (aim.0 + (angle.cos() * reach as f64) as i32, aim.1 - (angle.sin() * reach as f64) as i32);
                let cell = cell_of(point.0, point.1);
                if !self.map.in_playfield(cell.0, cell.1) {
                    continue;
                }
                let score = (((point.0 - ux) as f64).hypot((point.1 - uy) as f64)) as i64;
                if best.is_none_or(|(_, held)| score < held) {
                    previous = best.map(|(cell, _)| cell);
                    best = Some((cell, score));
                }
            }
            if let Some((cell, _)) = best {
                let other = previous.unwrap_or(cell);
                return Some(if self.next_random() % 100 < 50 { cell } else { other });
            }
            reach -= 0x100;
        }
        None
    }

    /// One turn of an aircraft's mission; returns the wait before the next.
    fn plane_mission(&mut self, index: usize, plane: &mut Plane) -> u64 {
        let Some(stats) = self.units[index].stats.plane else {
            return 15;
        };
        let normal = 14 + (self.next_random() % 3) as u64;
        let id = self.units[index].id;
        match plane.mission {
            Mission::ParadropApproach | Mission::ParadropOverfly | Mission::Retreat => self.paradrop_mission(index, plane),
            Mission::Guard => {
                if plane.z == stats.flight_level && plane.z > 0 {
                    self.plane_idle(index, plane);
                    return 1;
                }
                let unit = &self.units[index];
                let armed = unit.stats.fighter.primary.is_some();
                if armed && plane.ammo < stats.ammo && plane.contact.is_none()
                    && let Some(dock) = self.find_pad(unit, plane.home).filter(|&dock| self.link(plane, id, dock)) {
                        plane.mission = Mission::Enter;
                        plane.state = 0;
                        self.set_goal(index, plane, Some(Goal::Building(dock)));
                        self.units[index].arms.target = None;
                        return 1;
                    }
                // Settled on its pad short of
                // ammunition or health, the pad puts it to sleep to see to it.
                let unit = &self.units[index];
                if plane.z == 0 && plane.contact.is_some() && !plane.is_moving() && unit.arms.target.is_none() && (plane.ammo < stats.ammo || unit.health < unit.stats.strength) {
                    plane.mission = Mission::Sleep;
                    return self.reload_delay as u64;
                }
                if plane.ammo < stats.ammo / 2 && plane.contact.is_some() {
                    return 1;
                }
                if self.units[index].arms.target.is_some() {
                    plane.mission = Mission::Attack;
                    plane.state = 0;
                    return 1;
                }
                normal
            }
            // Give the locomotor the destination,
            // then wait for it to stop.
            Mission::Move => match plane.state {
                0 => {
                    if plane.goal.is_none() {
                        self.plane_idle(index, plane);
                    } else {
                        let goal = plane.goal;
                        self.set_goal(index, plane, goal);
                        plane.state = 2;
                    }
                    1
                }
                _ => {
                    if !plane.is_moving() {
                        self.plane_idle(index, plane);
                    }
                    1
                }
            },
            // The states a `Fighter` goes through.
            Mission::Attack => {
                let target = self.units[index].arms.target.filter(|target| self.alive(*target));
                if target.is_none() {
                    self.units[index].arms.target = None;
                }
                if self.is_strafer(index) && matches!(plane.state, 1 | 3 | 4 | 6..=9) {
                    return self.strafe(index, plane, target);
                }
                match plane.state {
                    0 => {
                        plane.locked = false;
                        plane.state = if target.is_some() { 1 } else { 10 };
                        1
                    }
                    1 => {
                        plane.locked = false;
                        if std::mem::take(&mut plane.pending) {
                            plane.ammo -= 1;
                        }
                        match target.filter(|_| plane.ammo != 0) {
                            Some(target) => match self.attack_position(index, target) {
                                Some(cell) => {
                                    self.set_goal(index, plane, Some(Goal::Cell(cell.0, cell.1)));
                                    plane.state = 3;
                                }
                                None => plane.state = 10,
                            },
                            None => plane.state = 10,
                        }
                        normal
                    }
                    3 => {
                        if std::mem::take(&mut plane.pending) {
                            plane.ammo -= 1;
                        }
                        plane.locked = false;
                        // A fighter does not wait to arrive.
                        plane.state = if target.is_none() || plane.ammo == 0 { 10 } else { 4 };
                        1
                    }
                    4 | 5 => {
                        let Some(target) = target.filter(|_| plane.ammo != 0) else {
                            plane.state = 10;
                            return 1;
                        };
                        let unit = &self.units[index];
                        let fighter = unit.stats.fighter;
                        let which = self.select_weapon(&fighter, target);
                        let at = (unit.x, unit.y, self.floor(unit.x, unit.y) + plane.z);
                        let (owner, body) = (unit.owner, unit.facing);
                        let mut arms = unit.arms;
                        // A fighter's shot does not
                        // wait on its facing.
                        match self.fire_error(at, &fighter, &arms, target, which) {
                            FireError::Ok if plane.z > 0 => {
                                let facing = self.direction_to((at.0, at.1), target).unwrap_or((body as u16) << 8);
                                let rof = Self::mount(&fighter, which).and_then(|m| self.weapons.get(m.weapon as usize)).map_or(1, |w| w.rof.max(1)) as u64;
                                if self.fire(Aim::Unit(id), at, body, facing, owner, &fighter, &mut arms, target, which) {
                                    self.units[index].arms = arms;
                                    plane.pending = true;
                                    plane.locked = true;
                                    plane.state = if plane.ammo > 0 { 1 } else { 10 };
                                    return rof;
                                }
                                1
                            }
                            FireError::Rearm => 1,
                            // Out of range, or not yet off the ground: pick a
                            // place again shortly.
                            _ => {
                                plane.state = 1;
                                normal
                            }
                        }
                    }
                    _ => {
                        // Break off.
                        plane.locked = false;
                        if std::mem::take(&mut plane.pending) && plane.ammo > 0 {
                            plane.ammo -= 1;
                        }
                        if plane.ammo != 0 && target.is_some() {
                            plane.state = 1;
                            return 1;
                        }
                        // A person's aircraft with
                        // nothing left to fire forgets its target.
                        if plane.ammo == 0 {
                            self.units[index].arms.target = None;
                        }
                        plane.goal = None;
                        self.plane_idle(index, plane);
                        1
                    }
                }
            }
            // Fly to the pad, come down sliding on to
            // the slot's point, and be taken in.
            Mission::Enter => {
                if plane.contact.is_none() {
                    if plane.landing {
                        return 1;
                    }
                    self.plane_idle(index, plane);
                    return 1;
                }
                if plane.landing {
                    if let Some(point) = plane.contact.and_then(|contact| self.dock_point(contact, id)) {
                        let unit = &mut self.units[index];
                        unit.x += (point.0 - unit.x).clamp(-5, 5);
                        unit.y += (point.1 - unit.y).clamp(-5, 5);
                    }
                    return 1;
                }
                if plane.z == 0 {
                    // On the pad it
                    // sleeps while the building loads it.
                    if plane.goal.is_some_and(|goal| Some(goal) != plane.contact.map(Goal::Building)) {
                        // Sent elsewhere while still down.
                        plane.mission = Mission::Move;
                        plane.state = 0;
                        return 1;
                    }
                    plane.goal = None;
                    plane.mission = Mission::Sleep;
                    return self.reload_delay as u64;
                }
                // Still on the way: make sure the locomotor is heading there.
                if !plane.is_moving() {
                    let goal = plane.contact.map(Goal::Building);
                    self.set_goal(index, plane, goal);
                }
                3
            }
            // Every `ReloadRate` the pad gives it a point
            // of ammunition or, full, one step of repair; full and whole, or with
            // a repair refused or just finished, it is let go and stands guard.
            Mission::Sleep => {
                let unit = &self.units[index];
                let whole = unit.health >= unit.stats.strength;
                if plane.ammo >= stats.ammo && unit.arms.target.is_none() && whole {
                    plane.mission = Mission::Guard;
                    plane.state = 0;
                    return 1;
                }
                if plane.ammo < stats.ammo {
                    plane.ammo += 1;
                    return self.reload_delay as u64;
                }
                if self.pad_repair(index) {
                    return self.reload_delay as u64;
                }
                plane.mission = Mission::Guard;
                plane.state = 0;
                1
            }
        }
    }

    /// One step of repair for an aircraft on a pad, paid by its owner. Whether
    /// it is still short of full health after it.
    // `RepairStep` points for the type's cost over its
    // strength in steps (both whole numbers) times `RepairPercent`, at least 1.
    // Already whole, or with too little money, nothing is done.
    fn pad_repair(&mut self, index: usize) -> bool {
        let unit = &self.units[index];
        let (health, strength, owner) = (unit.health, unit.stats.strength, unit.owner as usize);
        if health >= strength {
            return false;
        }
        let cost = (unit.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).map_or(0, |t| t.cost);
        let steps = (strength / self.repair_step.max(1)).max(1);
        let price = (((cost / steps) as f64 * self.repair_percent) as i32).max(1);
        let Some(house) = self.houses.get_mut(owner).filter(|house| house.credits >= price) else {
            return false;
        };
        house.credits -= price;
        let unit = &mut self.units[index];
        unit.health = (health + self.repair_step.max(1)).min(strength);
        unit.health < strength
    }

    /// Where an aircraft with nowhere to set down makes for.
    // Near the nearest of its owner's buildings
    // (a building of the first type it docks at counted four times nearer),
    // or near itself when that is within a cell; a type that docks nowhere goes
    // near the nearest other unit of its side; failing all, its own cell.
    fn landing_zone(&self, index: usize, height: i32) -> (i32, i32) {
        let unit = &self.units[index];
        let here = unit.cell();
        let at = (unit.x as i64, unit.y as i64, (self.floor(unit.x, unit.y) + height) as i64);
        let length = |x: i32, y: i32, z: i32| {
            let (dx, dy, dz) = (x as i64 - at.0, y as i64 - at.1, z as i64 - at.2);
            ((dx * dx + dy * dy + dz * dz) as f64).sqrt() as i64
        };
        let near = |cell: (i32, i32)| crate::nearby(&self.map, &self.claims, &self.spots, unit, cell, crate::Near::new(true, false, None), self.tick).unwrap_or(cell);
        let docks = self.unit_types.get((unit.stats.kind as usize).wrapping_sub(1)).map(|kind| kind.dock.clone()).unwrap_or_default();
        if let Some(&first) = docks.first() {
            let best = self
                .buildings
                .iter()
                .filter(|b| b.owner == unit.owner)
                .map(|b| {
                    let (x, y) = (b.cell.0 * LEPTONS_PER_CELL + b.size.0 * 128, b.cell.1 * LEPTONS_PER_CELL + b.size.1 * 128);
                    let d = length(x, y, self.floor(x, y));
                    (if b.kind == first { d / 4 } else { d }, b.cell)
                })
                .min_by_key(|(d, _)| *d);
            if let Some((d, cell)) = best {
                return near(if d < 0x100 { here } else { cell });
            }
        }
        let friend = self
            .units
            .iter()
            .filter(|u| u.owner == unit.owner && u.id != unit.id && u.health > 0)
            .map(|u| (length(u.x, u.y, self.floor(u.x, u.y)), u.cell()))
            .min_by_key(|(d, _)| *d);
        match friend {
            Some((_, cell)) => near(cell),
            None => here,
        }
    }

    /// A pad building being sold sends away every aircraft linked with it: each
    /// takes off for somewhere to set down, and from there looks for another pad.
    pub(crate) fn pad_sold(&mut self, building: u32) {
        for index in 0..self.units.len() {
            let Some(mut plane) = self.units[index].plane.filter(|plane| plane.contact == Some(building)) else {
                continue;
            };
            if matches!(plane.mission, Mission::ParadropApproach | Mission::ParadropOverfly | Mission::Retreat) {
                continue;
            }
            let id = self.units[index].id;
            self.unlink(&mut plane, id);
            let cell = self.landing_zone(index, plane.z);
            plane.goal = Some(Goal::Cell(cell.0, cell.1));
            plane.mission = Mission::Move;
            plane.state = 0;
            plane.wake = self.tick;
            self.units[index].plane = Some(plane);
        }
    }

    /// The locomotor's turn.
    fn plane_process(&mut self, index: usize, plane: &mut Plane) {
        let Some(stats) = self.units[index].stats.plane else {
            return;
        };
        let alive = self.units[index].health > 0 && !plane.crashing;
        if !plane.landing && !plane.taking_off && plane.target_speed >= 1.0 && plane.flight_level == 0 {
            plane.flight_level = stats.flight_level;
        }

        // Movement.
        if !alive && plane.z > 0 {
            plane.fall_speed += 1;
            plane.z -= plane.fall_speed;
            if plane.z < 1 {
                plane.z = 0;
                plane.crashed = true;
            }
        } else if !alive {
            plane.crashed = true;
        }
        if alive && plane.is_moving() {
            let on_run = self.units[index].arms.target.is_some() && plane.ammo != 0;
            let may_slow = !plane.locked && !(!plane.landing && plane.elevating && stats.fighter && plane.ammo != 0);
            let unit = &mut self.units[index];
            // Forward along the body's facing.
            let speed = (unit.speed as f64 * plane.cur_speed) as i32;
            if speed > 0 {
                let angle = angle_of(unit.turn.current());
                let (x, y) = ((angle.cos() * speed as f64 + unit.x as f64) as i32, (unit.y as f64 - angle.sin() * speed as f64) as i32);
                let cell = cell_of(x, y);
                if cell.0 >= 0 && cell.1 >= 0 && cell.0 < self.map.width && cell.1 < self.map.height {
                    unit.x = x;
                    unit.y = y;
                }
            }
            // Height.
            let to = plane.moving_to;
            let distance = to.map_or(0, |to| (((to.0 - unit.x) as f64).hypot((to.1 - unit.y) as f64)) as i32);
            let h = plane.z;
            if h < plane.flight_level {
                plane.z = h + 20.min(plane.flight_level - h);
            }
            if h > plane.flight_level {
                let d = h - plane.flight_level;
                let step = (d / 20).min(d).clamp(20, 50);
                plane.z = (h - step).max(0);
                let onto_pad = plane.mission == Mission::Enter && matches!(plane.goal, Some(Goal::Building(_)));
                if let Some(to) = to.filter(|_| !onto_pad) {
                    // Creep to the middle of the destination cell on the way down.
                    let centre = cell_centre(cell_of(to.0, to.1));
                    let angle = angle_of(direction((unit.x, unit.y), centre));
                    unit.x = (unit.x as f64 + angle.cos() * 5.0) as i32;
                    unit.y = (unit.y as f64 - angle.sin() * 5.0) as i32;
                }
            }
            // The speed wanted.
            if !plane.landing && (!plane.taking_off || plane.z >= plane.flight_level / 2) && to.is_some() {
                if !may_slow {
                    plane.target_speed = 1.0;
                } else {
                    plane.target_speed = (distance as f64 / stats.slowdown.max(1) as f64).min(1.0);
                    if plane.target_speed < 0.1 {
                        if distance > 85 {
                            plane.target_speed = 0.1;
                        } else {
                            plane.target_speed = 0.0;
                            plane.cur_speed *= 0.5;
                        }
                    }
                    if (distance as f64) < plane.cur_speed {
                        plane.cur_speed = distance as f64;
                    }
                    if plane.target_speed == 0.0 && plane.cur_speed == 0.0 && distance > 0 {
                        plane.cur_speed = 0.05;
                    }
                }
            }
            // Over the destination: come down, unless on an attack run.
            let arrived = to.is_some_and(|to| cell_of(to.0, to.1) == unit.cell());
            if !plane.elevating && !plane.landing && !plane.taking_off && arrived && plane.target_speed == 0.0 && !on_run {
                self.begin_landing(index, plane);
            }
            // A tenth of full speed a tick, either way.
            if plane.cur_speed < plane.target_speed {
                plane.cur_speed = (plane.cur_speed + 0.1).min(plane.target_speed);
            } else if plane.cur_speed > plane.target_speed {
                plane.cur_speed = (plane.cur_speed - 0.1).max(plane.target_speed);
            }
        }
        if !alive {
            plane.bank = 0;
            return;
        }

        // Steering.
        if let Some(to) = plane.moving_to.filter(|_| !plane.landing && !plane.taking_off && plane.z > 0) {
            let may_slow = !plane.locked && !(plane.elevating && stats.fighter && plane.ammo != 0);
            let unit = &mut self.units[index];
            let distance = (((to.0 - unit.x) as f64).hypot((to.1 - unit.y) as f64)) as i32;
            if !plane.locked {
                let want = direction((unit.x, unit.y), (to.0, to.1));
                if unit.turn.desired != want {
                    unit.turn = Turn::start(unit.turn.current(), want, unit.stats.rot);
                }
            }
            let floor = self.floor(to.0, to.1);
            plane.flight_level = if !plane.elevating || distance >= 0x300 { stats.flight_level } else { to.2 - floor };
            if may_slow {
                if distance < 0x80 {
                    plane.target_speed = 0.0;
                    if !plane.elevating && plane.cur_speed < 0.05 {
                        self.begin_landing(index, plane);
                    }
                } else if distance < 0x200 {
                    plane.target_speed = 0.5;
                } else if distance < 0x300 {
                    plane.target_speed = 0.75;
                }
            }
        }
        let unit = &mut self.units[index];
        if unit.turn.remaining > 0 {
            unit.turn.remaining -= 1;
        }
        unit.facing = (unit.turn.current() >> 8) as u8;
        // In the air and faster than `PitchSpeed`, it leans
        // the way it is still turning.
        let turning = unit.turn.steps > 0 && unit.turn.remaining > 0;
        plane.bank = if plane.z > 0 && turning && plane.cur_speed * 1000.0 > stats.pitch_speed as f64 { unit.turn.difference.signum() as i8 } else { 0 };

        // Taking off and landing.
        if plane.taking_off {
            // A trigger for one tick; the climb is
            // the height step above.
            plane.taking_off = false;
            plane.landing = false;
        } else if plane.landing {
            plane.target_speed = 0.0;
            let unit = &self.units[index];
            let over = plane.moving_to.and_then(|to| self.building_on(cell_of(to.0, to.1))).map(|i| self.buildings[i].id);
            let allowed = if stats.airport_bound { over.is_some() && over == plane.contact } else { true };
            // Once a landing, under 300 leptons up.
            if allowed && !plane.landing_heard && plane.z < 300 {
                plane.landing_heard = true;
                self.events.push(crate::combat::Event::Landing { unit: unit.id });
            }
            if !allowed {
                self.begin_takeoff(index, plane);
                self.plane_idle(index, plane);
            } else if plane.z <= 0 && plane.cur_speed <= 0.0 {
                plane.z = 0;
                plane.landing = false;
                plane.cur_speed = 0.0;
                plane.has_order = false;
                plane.moving_to = None;
                plane.elevating = false;
                self.events.push(crate::combat::Event::Landed { unit: unit.id });
            }
        }
    }

    /// Every aircraft takes its turn: its mission, then its flight.
    pub(crate) fn fly_planes(&mut self) {
        for index in 0..self.units.len() {
            let Some(mut plane) = self.units[index].plane else {
                continue;
            };
            if plane.crashed {
                continue;
            }
            // Outside an attack
            // the pass is over and a fired shot is counted; a target that has
            // taken to the air is dropped.
            if plane.mission != Mission::Attack {
                plane.locked = false;
                if std::mem::take(&mut plane.pending) && plane.ammo > 0 {
                    plane.ammo -= 1;
                }
            }
            if let Some(target) = self.units[index].arms.target
                && (!self.alive(target) || self.in_air(target)) {
                    self.units[index].arms.target = None;
                }
            if !plane.crashing && self.units[index].health > 0 && self.tick >= plane.wake {
                let wait = self.plane_mission(index, &mut plane);
                plane.wake = self.tick + wait;
            }
            // A cargo plane that has been shot down falls as any aircraft does;
            // only one still flying keeps its course.
            if !plane.crashing && matches!(plane.mission, Mission::ParadropApproach | Mission::ParadropOverfly | Mission::Retreat) {
                if self.paradrop_fly(index, &mut plane) {
                    self.units[index].plane = Some(plane);
                    let id = self.units[index].id;
                    self.vanish(id);
                    continue;
                }
            } else {
                self.plane_process(index, &mut plane);
            }
            self.units[index].plane = Some(plane);
        }
    }

    /// An aircraft strafes when its first weapon's
    /// shot turns at 1 or less and can be seen.
    fn is_strafer(&self, index: usize) -> bool {
        self.units[index]
            .stats
            .fighter
            .primary
            .and_then(|mount| self.weapons.get(mount.weapon as usize))
            .and_then(|weapon| self.projectiles.get(weapon.projectile as usize))
            .is_some_and(|projectile| projectile.rot <= 1 && !projectile.inviso)
    }

    /// The attack mission of a strafing aircraft (the Hornet, the Osprey).
    // It flies at the
    // target itself; inside its weapon's range it fires five times, `ROF`
    // apart, held on its course from the first, whatever its facing; the pass
    // costs one point of ammunition, taken when it comes round again.
    fn strafe(&mut self, index: usize, plane: &mut Plane, target: Option<Aim>) -> u64 {
        let normal = 14 + (self.next_random() % 3) as u64;
        let unit = &self.units[index];
        let (id, owner, body, fighter, speed) = (unit.id, unit.owner, unit.facing, unit.stats.fighter, unit.speed.max(1));
        let at = (unit.x, unit.y, self.floor(unit.x, unit.y) + plane.z);
        let weapon = fighter.primary.and_then(|mount| self.weapons.get(mount.weapon as usize)).cloned();
        let (rof, range) = weapon.map_or((1, 0), |weapon| (weapon.rof.max(1) as u64, weapon.range));
        let cell_of_target = |world: &Self, target: Aim| world.aim_point(target).map(|point| (point.0.div_euclid(crate::LEPTONS_PER_CELL), point.1.div_euclid(crate::LEPTONS_PER_CELL)));
        let shoot = |world: &mut Self, plane: &mut Plane, target: Aim| {
            let mut arms = world.units[index].arms;
            let facing = world.direction_to((at.0, at.1), target).unwrap_or((body as u16) << 8);
            let which = world.select_weapon(&fighter, target);
            world.fire(Aim::Unit(id), at, body, facing, owner, &fighter, &mut arms, target, which);
            world.units[index].arms = arms;
            let _ = plane;
        };
        match plane.state {
            1 => {
                plane.locked = false;
                if std::mem::take(&mut plane.pending) {
                    plane.ammo -= 1;
                }
                match target.filter(|_| plane.ammo != 0).and_then(|target| cell_of_target(self, target)) {
                    Some(cell) => {
                        self.set_goal(index, plane, Some(Goal::Cell(cell.0, cell.1)));
                        plane.state = 3;
                    }
                    None => plane.state = 10,
                }
                normal
            }
            3 => {
                if std::mem::take(&mut plane.pending) {
                    plane.ammo -= 1;
                }
                plane.locked = false;
                let Some(target) = target.filter(|_| plane.ammo != 0) else {
                    plane.state = 10;
                    return 1;
                };
                let near = self.aim_point(target).is_some_and(|point| {
                    let (dx, dy) = ((point.0 - at.0) as i64, (point.1 - at.1) as i64);
                    dx * dx + dy * dy < (range as i64) * (range as i64)
                });
                if near {
                    plane.state = 4;
                } else if let Some(cell) = cell_of_target(self, target)
                    && (!plane.heading_for(cell) || !plane.is_moving()) {
                        self.set_goal(index, plane, Some(Goal::Cell(cell.0, cell.1)));
                    }
                1
            }
            4 => {
                let Some(target) = target.filter(|_| plane.ammo != 0) else {
                    plane.state = 10;
                    return 1;
                };
                let arms = self.units[index].arms;
                let which = self.select_weapon(&fighter, target);
                match self.fire_error(at, &fighter, &arms, target, which) {
                    FireError::Ok => {
                        plane.pending = true;
                        shoot(self, plane, target);
                        plane.state = 6;
                        plane.locked = true;
                        rof
                    }
                    FireError::Facing => {
                        plane.state = 1;
                        45
                    }
                    _ => 1,
                }
            }
            6..=9 => {
                let Some(target) = target else {
                    plane.state = 10;
                    return 1;
                };
                let arms = self.units[index].arms;
                let which = self.select_weapon(&fighter, target);
                match self.fire_error(at, &fighter, &arms, target, which) {
                    FireError::Ok | FireError::Facing | FireError::Range => {
                        shoot(self, plane, target);
                        if plane.state == 9 {
                            plane.state = 3;
                            return ((range + 0x400) / speed) as u64;
                        }
                        if let Some(cell) = cell_of_target(self, target) {
                            self.set_goal(index, plane, Some(Goal::Cell(cell.0, cell.1)));
                        }
                        plane.state += 1;
                        rof
                    }
                    _ => {
                        if plane.ammo == 0 {
                            plane.state = 10;
                            plane.locked = false;
                        }
                        1
                    }
                }
            }
            _ => 1,
        }
    }

    /// The player's order to an aircraft to go somewhere.
    pub(crate) fn plane_move(&mut self, index: usize, cell: (i32, i32)) {
        let Some(mut plane) = self.units[index].plane else {
            return;
        };
        self.units[index].arms.target = None;
        plane.goal = Some(Goal::Cell(cell.0, cell.1));
        plane.mission = Mission::Move;
        plane.state = 0;
        plane.wake = self.tick;
        self.units[index].plane = Some(plane);
    }

    /// The player's order to an aircraft to attack something.
    pub(crate) fn plane_attack(&mut self, index: usize, target: Aim) {
        let Some(mut plane) = self.units[index].plane else {
            return;
        };
        self.units[index].arms.target = Some(target);
        plane.mission = Mission::Attack;
        plane.state = 0;
        plane.wake = self.tick;
        self.units[index].plane = Some(plane);
    }

    /// The player's order to an aircraft to go to a pad building.
    pub(crate) fn plane_dock(&mut self, index: usize, building: u32) {
        let Some(mut plane) = self.units[index].plane else {
            return;
        };
        let id = self.units[index].id;
        if plane.contact != Some(building) {
            self.unlink(&mut plane, id);
            if !self.link(&mut plane, id, building) {
                return;
            }
        }
        self.units[index].arms.target = None;
        plane.mission = Mission::Enter;
        plane.state = 0;
        plane.wake = self.tick;
        self.set_goal(index, &mut plane, Some(Goal::Building(building)));
        self.units[index].plane = Some(plane);
    }

    /// Whether an aircraft could be sent to a pad building now.
    // One of its own side's pad buildings
    // with a slot free (or its own).
    pub fn can_dock(&self, unit: u32, building: u32) -> bool {
        let Some(unit) = self.unit(unit).filter(|unit| unit.plane.is_some()) else {
            return false;
        };
        let docks = self.unit_types.get((unit.stats.kind as usize).wrapping_sub(1)).map(|kind| &kind.dock);
        self.buildings.iter().any(|b| {
            b.id == building
                && b.owner == unit.owner
                && b.buildup.is_none()
                && docks.is_some_and(|docks| docks.contains(&b.kind))
                && b.docks.iter().any(|slot| slot.is_none() || *slot == Some(unit.id))
        })
    }
}
