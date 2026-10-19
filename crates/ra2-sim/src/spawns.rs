//! Units that fight by sending out aircraft or missiles of their own: the
//! aircraft carrier's Hornets, the destroyer's Osprey, the V3's rocket and the
//! dreadnought's missiles; and the flight of those rockets.
//!
//! Follows the original game.
//!

use crate::aircraft::{Mission, Plane};
use crate::combat::Aim;
use crate::{Unit, UnitStats, World, LEPTONS_PER_CELL};

/// `Spawns`, `SpawnsNumber`, `SpawnRegenRate`, `SpawnReloadRate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpawnStats {
    /// The spawned type, by its place among the unit types.
    pub kind: u16,
    pub number: u32,
    pub regen: u32,
    pub reload: u32,
}

/// One kind of rocket's flight, from the rules.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RocketRules {
    /// The unit type it applies to.
    pub kind: Option<u16>,
    pub pause_frames: u32,
    pub tilt_frames: u32,
    pub pitch_initial: f32,
    pub pitch_final: f32,
    pub turn_rate: f32,
    pub acceleration: f64,
    pub altitude: i32,
    pub damage: i32,
    pub body_length: i32,
    pub lazy_curve: bool,
    pub warhead: u16,
}

/// A rocket in flight.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Rocket {
    /// 1 waiting, 2 tilting up, 3 climbing, 4 cruising, 5 diving.
    pub state: u8,
    since: u64,
    /// Radians above level.
    pub pitch: f32,
    speed: f64,
    /// Height above the map's zero.
    pub z: f64,
    position: (f64, f64),
    destination: (i32, i32, i32),
    /// How far off the target was over the ground when the cruise began.
    cruise: f64,
    rules: RocketRules,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Node {
    /// The spawn while it is aboard, and its id while it is out.
    aboard: Option<Unit>,
    out: Option<u32>,
    /// 0 aboard, 1 a rocket getting ready, 2 out and waiting, 3 attacking, 4
    /// coming back, 6 reloading, 7 lost and being replaced.
    state: u8,
    until: u64,
    rocket: bool,
}

/// What a unit with `Spawns` keeps of them.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Spawner {
    nodes: Vec<Node>,
    think_at: u64,
    launch_at: u64,
    target: Option<Aim>,
    pending: Option<Aim>,
    /// 0 idle, 1 gathering, 2 attacking.
    status: u8,
}

impl Spawner {
    /// How many are aboard (ready or reloading).
    pub fn docked(&self) -> usize {
        self.nodes.iter().filter(|node| matches!(node.state, 0 | 6)).count()
    }

    /// Those not lost.
    pub fn alive(&self) -> usize {
        self.nodes.iter().filter(|node| node.state != 7).count()
    }

    /// Rockets getting ready.
    pub fn preparing(&self) -> usize {
        self.nodes.iter().filter(|node| node.state == 1).count()
    }

    /// A new target waits to be taken up.
    pub(crate) fn set_target(&mut self, target: Aim) {
        if self.target != Some(target) {
            self.pending = Some(target);
        }
    }

    /// Target, waiting target and status cleared.
    pub(crate) fn reset_target(&mut self) {
        self.target = None;
        self.pending = None;
        self.status = 0;
    }

    fn adopt(&mut self) {
        if let Some(target) = self.pending.take() {
            self.target = Some(target);
        }
    }
}

impl World {
    fn rocket_rules(&self, kind: u16) -> Option<RocketRules> {
        // The V3's numbers for the V3's rocket
        // type, the dreadnought's for any other.
        let index = kind.checked_sub(1)?;
        self.unit_types.get(index as usize).filter(|t| t.stats.rocket)?;
        Some(if self.rockets[0].kind == Some(index) { self.rockets[0] } else { self.rockets[1] })
    }

    fn new_spawn(&mut self, owner: &Unit, stats: SpawnStats) -> Option<Unit> {
        let made = self.unit_types.get(stats.kind as usize).map(|t| UnitStats { kind: stats.kind + 1, ..t.stats })?;
        let id = self.next_id;
        self.next_id += 1;
        let mut unit = Unit::new(id, owner.owner, owner.cell(), owner.facing, made);
        unit.spawn_owner = Some(owner.id);
        unit.plane = made.plane.as_ref().map(Plane::parked);
        Some(unit)
    }

    /// Runs every spawner, and tidies up after owners and spawns that died.
    // Each thinks every ten frames (twenty before
    // the first).
    pub(crate) fn run_spawns(&mut self) {
        let owners: Vec<u32> = self.units.iter().filter(|unit| unit.stats.spawns.is_some()).map(|unit| unit.id).collect();
        for id in owners {
            let Some(index) = self.unit_at(id) else {
                continue;
            };
            let Some(stats) = self.units[index].stats.spawns else {
                continue;
            };
            let mut spawner = match self.units[index].spawner.take() {
                Some(spawner) => spawner,
                None => {
                    // The spawns are made at once
                    // and kept off the map.
                    let owner = self.units[index].clone();
                    let rocket = self.rocket_rules(stats.kind + 1).is_some();
                    let nodes = (0..stats.number).filter_map(|_| self.new_spawn(&owner, stats)).map(|unit| Node { aboard: Some(unit), out: None, state: 0, until: 0, rocket }).collect();
                    Spawner { nodes, think_at: self.tick + 20, launch_at: 0, target: None, pending: None, status: 0 }
                }
            };
            if self.tick >= spawner.think_at {
                spawner.think_at = self.tick + 10;
                self.think_spawner(id, stats, &mut spawner);
            }
            if let Some(unit) = self.unit_at(id).map(|at| &mut self.units[at]) {
                unit.spawner = Some(spawner);
            }
        }
        self.fly_rockets();
    }

    fn think_spawner(&mut self, id: u32, stats: SpawnStats, spawner: &mut Spawner) {
        let now = self.tick;
        if spawner.target.is_some_and(|target| !self.alive(target)) {
            spawner.target = None;
        }
        if spawner.pending.is_some_and(|target| !self.alive(target)) {
            spawner.pending = None;
        }
        for at in 0..spawner.nodes.len() {
            let Some(owner) = self.unit(id).cloned() else {
                return;
            };
            let home = owner.cell();
            let mut node = spawner.nodes[at].clone();
            // One that has died is replaced after
            // `SpawnRegenRate`.
            if let Some(out) = node.out
                && !self.unit(out).is_some_and(|unit| unit.health > 0 && !unit.plane.is_some_and(|plane| plane.crashing)) {
                    node = Node { aboard: None, out: None, state: 7, until: now + stats.regen as u64, rocket: node.rocket };
                }
            match node.state {
                // With a target and the launch timer run out, and for
                // a rocket only while the owner stands still, the spawn is put
                // on the map at the owner, facing as it does; a rocket is given
                // the target, an aircraft the cell north of the owner.
                0 => {
                    let ready = spawner.target.is_some() && now >= spawner.launch_at && spawner.status != 2 && !(node.rocket && owner.is_moving());
                    if let Some(mut unit) = node.aboard.take_if(|_| ready) {
                        spawner.launch_at = now + 20;
                        node.state = 2;
                        (unit.x, unit.y, unit.facing) = (owner.x, owner.y, owner.facing);
                        unit.turn = crate::Turn::facing(owner.facing);
                        let spawn = unit.id;
                        if node.rocket {
                            spawner.adopt();
                            let destination = spawner.target.and_then(|target| self.aim_point(target));
                            let base = self.ground_z(owner.x, owner.y) + self.altitude(&owner) + 10;
                            unit.rocket = destination.zip(self.rocket_rules(unit.stats.kind)).map(|(destination, rules)| Rocket {
                                state: if rules.pause_frames != 0 { 1 } else { 2 },
                                since: now,
                                pitch: rules.pitch_initial * std::f32::consts::FRAC_PI_2,
                                speed: 0.0,
                                z: base as f64,
                                position: (owner.x as f64, owner.y as f64),
                                destination,
                                cruise: 0.0,
                                rules,
                            });
                            self.put_back(unit);
                        } else {
                            self.put_back(unit);
                            if let Some(index) = self.unit_at(spawn) {
                                self.plane_move(index, (home.0, home.1 - 1));
                            }
                        }
                        node.out = Some(spawn);
                    }
                }
                // A rocket that has left is counted as lost.
                1 => {
                    if now >= node.until {
                        node = Node { aboard: None, out: None, state: 7, until: now + stats.regen as u64, rocket: true };
                    }
                }
                2 if !node.rocket => {
                    spawner.adopt();
                    if let Some(index) = node.out.and_then(|out| self.unit_at(out)) {
                        if spawner.target.is_none() {
                            self.spawn_home(index, home);
                            node.state = 4;
                        } else {
                            self.spawn_move(index, (home.0, home.1 + 1));
                        }
                    }
                }
                // Out of ammunition or with nothing to attack it is
                // sent home; otherwise it is told to attack again.
                3 => {
                    spawner.adopt();
                    if let Some(index) = node.out.and_then(|out| self.unit_at(out)) {
                        let spent = self.units[index].plane.is_some_and(|plane| plane.ammo == 0);
                        match spawner.target.filter(|_| !spent) {
                            Some(target) => self.spawn_attack(index, target),
                            None => {
                                self.spawn_home(index, home);
                                node.state = 4;
                            }
                        }
                    }
                }
                // Over the owner's cell and less than 20 above it, it
                // is taken aboard to reload.
                4 => {
                    spawner.adopt();
                    if let Some(index) = node.out.and_then(|out| self.unit_at(out)) {
                        let loaded = self.units[index].plane.is_some_and(|plane| plane.ammo > 0);
                        if let Some(target) = spawner.target.filter(|_| loaded) {
                            node.state = 3;
                            self.spawn_attack(index, target);
                        } else if self.units[index].cell() == home && self.altitude(&self.units[index]) - self.altitude(&owner) < 20 {
                            let mut unit = self.lift(index);
                            unit.arms.target = None;
                            node.aboard = Some(unit);
                            node.out = None;
                            node.state = 6;
                            node.until = now + stats.reload as u64;
                        } else {
                            self.spawn_home(index, home);
                        }
                    }
                }
                // Reloaded, it has all its ammunition and strength.
                6 => {
                    if now >= node.until {
                        node.state = 0;
                        if let Some(unit) = node.aboard.as_mut() {
                            unit.health = unit.stats.strength.max(1);
                            if let (Some(plane), Some(stats)) = (unit.plane.as_mut(), unit.stats.plane) {
                                plane.ammo = stats.ammo;
                            }
                        }
                    }
                }
                7
                    if now >= node.until => {
                        node.aboard = self.new_spawn(&owner, stats);
                        node.state = if node.aboard.is_some() { 0 } else { 7 };
                    }
                _ => {}
            }
            spawner.nodes[at] = node;
        }

        // A sortie starts only against a target in range; once every
        // spawn is out (or lost) they are let go together, and for missiles the
        // target is then forgotten: one order, one salvo.
        match spawner.status {
            0 => {
                spawner.adopt();
                if let Some(target) = spawner.target {
                    if self.spawner_in_range(id, target) {
                        spawner.status = 1;
                    } else {
                        self.reset_spawner(spawner, stats);
                    }
                }
            }
            1 => {
                let Some(target) = spawner.target else {
                    self.reset_spawner(spawner, stats);
                    return;
                };
                if !spawner.nodes.iter().all(|node| matches!(node.state, 2 | 7)) {
                    return;
                }
                let mut missiles = false;
                for at in 0..spawner.nodes.len() {
                    if spawner.nodes[at].state != 2 {
                        continue;
                    }
                    if spawner.nodes[at].rocket {
                        let wait = spawner.nodes[at].out.and_then(|out| self.unit(out)).and_then(|unit| unit.rocket).map_or(0, |rocket| rocket.rules.pause_frames + rocket.rules.tilt_frames);
                        spawner.nodes[at].state = 1;
                        spawner.nodes[at].until = self.tick + wait as u64;
                        missiles = true;
                    } else {
                        spawner.nodes[at].state = 3;
                        if let Some(index) = spawner.nodes[at].out.and_then(|out| self.unit_at(out)) {
                            self.spawn_attack(index, target);
                        }
                    }
                }
                if missiles {
                    spawner.target = None;
                    spawner.pending = None;
                }
                spawner.status = 2;
            }
            _ => {
                if !spawner.nodes.iter().any(|node| matches!(node.state, 3 | 4)) {
                    spawner.status = 0;
                }
            }
        }
    }

    fn reset_spawner(&mut self, spawner: &mut Spawner, stats: SpawnStats) {
        for node in &mut spawner.nodes {
            if node.state == 2 && node.rocket {
                *node = Node { aboard: None, out: None, state: 7, until: self.tick + stats.regen as u64, rocket: true };
            }
        }
        spawner.status = 0;
        spawner.target = None;
        spawner.pending = None;
    }

    fn spawner_in_range(&self, owner: u32, target: Aim) -> bool {
        let Some((unit, at)) = self.unit(owner).zip(self.aim_point(target)) else {
            return false;
        };
        let range = unit.stats.fighter.primary.and_then(|mount| self.weapons.get(mount.weapon as usize)).map_or(0, |weapon| weapon.range) as i64;
        let (dx, dy) = ((at.0 - unit.x) as i64, (at.1 - unit.y) as i64);
        range < 0 || dx * dx + dy * dy <= range * range
    }

    /// The orders the manager gives its aircraft, given again only when they
    /// would change something (the engine queues the same mission each time,
    /// which leaves one already running alone).
    fn spawn_move(&mut self, index: usize, cell: (i32, i32)) {
        let unit = &self.units[index];
        if unit.cell() != cell && !unit.plane.is_some_and(|plane| plane.mission == Mission::Move && plane.heading_for(cell)) {
            self.plane_move(index, cell);
        }
    }

    fn spawn_home(&mut self, index: usize, cell: (i32, i32)) {
        self.units[index].arms.target = None;
        let plane = self.units[index].plane;
        if !plane.is_some_and(|plane| plane.mission == Mission::Move && plane.heading_for(cell)) {
            self.plane_move(index, cell);
        }
    }

    fn spawn_attack(&mut self, index: usize, target: Aim) {
        let unit = &self.units[index];
        if unit.arms.target != Some(target) || !unit.plane.is_some_and(|plane| plane.mission == Mission::Attack) {
            self.plane_attack(index, target);
        }
    }

    /// What becomes of a dead unit's spawns.
    // Those aboard go with it; aircraft in the air
    // fall; missiles fly on.
    pub(crate) fn orphan_spawns(&mut self, spawner: &Spawner) {
        for node in &spawner.nodes {
            if node.rocket {
                continue;
            }
            if let Some(unit) = node.out.and_then(|out| self.unit_at(out).map(|at| &mut self.units[at])) {
                unit.health = 0;
                if let Some(plane) = unit.plane.as_mut() {
                    plane.crashing = true;
                }
            }
        }
    }

    /// Flies the rockets.
    fn fly_rockets(&mut self) {
        let now = self.tick;
        let mut burst: Vec<(u32, (i32, i32, i32), RocketRules)> = Vec::new();
        for unit in &mut self.units {
            let Some(mut rocket) = unit.rocket else {
                continue;
            };
            let rules = rocket.rules;
            let right = std::f32::consts::FRAC_PI_2;
            let (dx, dy) = (rocket.destination.0 as f64 - rocket.position.0, rocket.destination.1 as f64 - rocket.position.1);
            let flat = dx.hypot(dy);
            let down = if flat < 1.0 { -right } else { ((rocket.destination.2 as f64 - rocket.z) / flat).atan() as f32 };
            let hit = |rocket: &Rocket| rocket.z + (rocket.pitch.sin() as f64) * rules.body_length as f64 <= rocket.destination.2 as f64;
            let mut explode = unit.health <= 0;
            match rocket.state {
                1 => {
                    if now >= rocket.since + rules.pause_frames as u64 {
                        rocket.state = 2;
                        rocket.since = now;
                    }
                }
                2 => {
                    let share = (now - rocket.since) as f32 / rules.tilt_frames.max(1) as f32;
                    rocket.pitch = (rules.pitch_initial + (rules.pitch_final - rules.pitch_initial) * share.min(1.0)) * right;
                    if share >= 1.0 {
                        rocket.state = 3;
                    }
                }
                3 => {
                    rocket.speed = (rocket.speed + rules.acceleration).min(unit.speed.max(1) as f64);
                    let ground = self.map.level(unit.x.div_euclid(LEPTONS_PER_CELL), unit.y.div_euclid(LEPTONS_PER_CELL)) as i32 * crate::combat::LEVEL_HEIGHT;
                    if rocket.z as i32 - ground >= rules.altitude {
                        rocket.state = 4;
                        rocket.cruise = flat;
                    }
                }
                4 => {
                    rocket.speed = (rocket.speed + rules.acceleration).min(unit.speed.max(1) as f64);
                    if rules.lazy_curve && rocket.cruise != 0.0 {
                        explode |= hit(&rocket);
                        let share = (flat / rocket.cruise) as f32;
                        rocket.pitch = down * (1.0 - share) + rules.pitch_final * right * share;
                    } else {
                        if rocket.pitch > 0.0 {
                            rocket.pitch = (rocket.pitch - rules.turn_rate).max(0.0);
                        }
                        if flat <= rocket.z - rocket.destination.2 as f64 {
                            rocket.state = 5;
                        }
                    }
                }
                _ => {
                    explode |= hit(&rocket);
                    rocket.pitch += (down - rocket.pitch).clamp(-rules.turn_rate, rules.turn_rate);
                }
            }
            if rocket.speed > 0.0 && !explode {
                let level = rocket.speed * rocket.pitch.cos() as f64;
                if flat >= 1.0 {
                    rocket.position.0 += dx / flat * level;
                    rocket.position.1 += dy / flat * level;
                    unit.facing = (((dx.atan2(-dy) / std::f64::consts::TAU * 256.0).round() as i32) & 0xff) as u8;
                }
                rocket.z += rocket.speed * rocket.pitch.sin() as f64;
                (unit.x, unit.y) = (rocket.position.0 as i32, rocket.position.1 as i32);
            }
            if explode {
                // The point is a body's length ahead along its heading and pitch.
                let reach = rules.body_length as f64;
                let (ux, uy) = if flat >= 1.0 { (dx / flat, dy / flat) } else { (0.0, 0.0) };
                let level = reach * rocket.pitch.cos() as f64;
                let point = ((rocket.position.0 + ux * level) as i32, (rocket.position.1 + uy * level) as i32, (rocket.z + reach * rocket.pitch.sin() as f64).max(rocket.destination.2 as f64) as i32);
                burst.push((unit.id, point, rules));
            }
            unit.rocket = Some(rocket);
        }
        for (id, point, rules) in burst {
            self.explode(point, rules.damage, rules.warhead, Some(Aim::Unit(id)));
            self.blast_event(rules.warhead, rules.damage, point, false);
            self.vanish(id);
        }
    }
}
