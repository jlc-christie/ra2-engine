//! Yuri's slave miners. A miner owns a fixed set of worker slots; moving and
//! deployed forms share those slots. Workers walk, harvest into their own
//! storage, return to the deployed miner, and unload after its reload delay.
//!
//! Scan distances, harvest/reload/regeneration delays and workforce sizes come
//! from rulesmd.ini. The deterministic choice among equally good cells is ours;
//! this does not reproduce the original engine's mission scheduling byte for byte.

use crate::combat::{Aim, Arms};
use crate::{Order, Other, Unit, UnitStats, Work, World};
use crate::path::code;

#[derive(Clone, Copy, Debug)]
pub struct SpawnerRules {
    pub worker: u16,
    pub count: usize,
    pub regeneration: u32,
    pub reload: u32,
    pub resource_gatherer: bool,
    pub resource_destination: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WorkerRules {
    pub harvest_rate: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Rules {
    pub short_scan: i32,
    pub worker_scan: i32,
    pub long_scan: i32,
    pub scan_correction: i32,
    pub kick_delay: u32,
}

impl Default for Rules {
    fn default() -> Self {
        Self { short_scan: 8, worker_scan: 14, long_scan: 48, scan_correction: 3, kick_delay: 150 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Host {
    Unit(u32),
    Building(u32),
}

impl Host {
    fn aim(self) -> Aim { match self { Self::Unit(id) => Aim::Unit(id), Self::Building(id) => Aim::Building(id) } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Phase {
    Seeking,
    Going((i32, i32)),
    Digging((i32, i32)),
    Returning,
    Reloading,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Slot {
    worker: Option<u32>,
    due: u64,
    phase: Phase,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Pool {
    host: Host,
    owner: u8,
    slots: Vec<Slot>,
    spawn_at: u64,
    scan_at: u64,
    /// A dying carrier can eject a live miner. Keep its killer until the
    /// passenger is either back on the map or actually lost with the carrier.
    liberator: Option<u8>,
}

#[derive(Clone, Copy)]
struct Site {
    owner: u8,
    cell: (i32, i32),
    size: (i32, i32),
    active: bool,
    rules: SpawnerRules,
}

impl Site {
    fn centre(self) -> (i32, i32) { (self.cell.0 + self.size.0 / 2, self.cell.1 + self.size.1 / 2) }
    fn adjacent(self, cell: (i32, i32)) -> bool {
        (self.cell.0 - 1..=self.cell.0 + self.size.0).contains(&cell.0)
            && (self.cell.1 - 1..=self.cell.1 + self.size.1).contains(&cell.1)
            && !((self.cell.0..self.cell.0 + self.size.0).contains(&cell.0) && (self.cell.1..self.cell.1 + self.size.1).contains(&cell.1))
    }
    fn perimeter(self) -> Vec<(i32, i32)> {
        (-1..=self.size.0).map(|x| (self.cell.0 + x, self.cell.1 - 1))
            .chain((-1..=self.size.0).map(|x| (self.cell.0 + x, self.cell.1 + self.size.1)))
            .chain((0..self.size.1).flat_map(|y| [(self.cell.0 - 1, self.cell.1 + y), (self.cell.0 + self.size.0, self.cell.1 + y)]))
            .collect()
    }
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i32 { (a.0 - b.0).abs().max((a.1 - b.1).abs()) }

fn contains_host(unit: &Unit, id: u32) -> bool {
    (unit.id == id && unit.health > 0)
        || unit.cargo.iter().chain(&unit.parasite).any(|child| contains_host(child, id))
}

impl World {
    /// Enslaved workers are automatic. Once liberated, these same units may be
    /// selected and ordered like other infantry.
    pub fn is_enslaved(&self, id: u32) -> bool {
        self.slave_pools.iter().any(|pool| pool.slots.iter().any(|slot| slot.worker == Some(id)))
    }

    pub fn is_slave_miner(&self, id: u32) -> bool {
        self.unit(id).and_then(|unit| self.kind_of(unit)).and_then(|kind| kind.slave_spawner).is_some_and(|rules| rules.resource_gatherer)
    }

    pub fn slave_workers(&self, host: Host) -> Vec<u32> {
        self.slave_pools.iter().find(|pool| pool.host == host).map(|pool| pool.slots.iter().filter_map(|slot| slot.worker).collect()).unwrap_or_default()
    }

    fn slave_site(&self, host: Host) -> Option<Site> {
        match host {
            Host::Building(id) => {
                let building = self.buildings.iter().find(|building| building.id == id && building.health > 0)?;
                let rules = self.building_types.get(building.kind as usize)?.slave_spawner?;
                Some(Site { owner: building.owner, cell: building.cell, size: building.size,
                    active: building.buildup.is_none() && self.online(building), rules })
            }
            Host::Unit(id) => {
                let unit = self.unit(id).filter(|unit| unit.health > 0)?;
                let rules = self.kind_of(unit)?.slave_spawner?;
                Some(Site { owner: unit.owner, cell: unit.cell(), size: (1, 1), active: false, rules })
            }
        }
    }

    /// Being a passenger is temporary absence from the map, not a death.
    /// Include a dying carrier until its ordinary passenger evacuation runs.
    fn stored_slave_host(&self, host: Host) -> bool {
        let Host::Unit(id) = host else { return false; };
        self.units.iter().any(|unit| contains_host(unit, id))
            || self.buildings.iter().any(|building| building.occupants.iter()
                .chain(building.patients.iter().map(|(_, unit)| unit)).any(|unit| contains_host(unit, id)))
    }

    pub(crate) fn transfer_slave_host(&mut self, from: Host, to: Host) {
        if let Some(pool) = self.slave_pools.iter_mut().find(|pool| pool.host == from) {
            pool.host = to;
            pool.scan_at = self.tick + self.slave_rules.kick_delay as u64;
        }
    }

    /// Called before a killed host disappears, while the killer's house is
    /// still known. Script removal without a killer liberates to the neutral house.
    pub(crate) fn slave_host_died(&mut self, victim: Aim, killer: Option<u8>) {
        // Passengers may survive this death. Record the source without freeing
        // their workforce until normal death handling has settled their fate.
        let carried: Vec<_> = self.slave_pools.iter().filter_map(|pool| {
            let Host::Unit(id) = pool.host else { return None; };
            let inside = match victim {
                Aim::Unit(carrier) => self.unit(carrier).is_some_and(|unit| unit.cargo.iter().chain(&unit.parasite).any(|unit| contains_host(unit, id))),
                Aim::Building(carrier) => self.buildings.iter().find(|building| building.id == carrier).is_some_and(|building| building.occupants.iter()
                    .chain(building.patients.iter().map(|(_, unit)| unit)).any(|unit| contains_host(unit, id))),
                Aim::Cell(..) => false,
            };
            inside.then_some(pool.host)
        }).collect();
        for pool in &mut self.slave_pools {
            if carried.contains(&pool.host) { pool.liberator = Some(killer.unwrap_or(0)); }
        }
        if let Some(index) = self.slave_pools.iter().position(|pool| pool.host.aim() == victim) {
            let pool = self.slave_pools.remove(index);
            self.liberate_workers(pool, killer.unwrap_or(0));
        }
    }

    fn pause_slave_pool(&mut self, pool: &mut Pool) {
        pool.spawn_at = pool.spawn_at.saturating_add(1);
        pool.scan_at = pool.scan_at.saturating_add(1);
        for slot in &mut pool.slots {
            slot.due = slot.due.saturating_add(1);
            if let Some(worker) = slot.worker.and_then(|id| self.units.iter_mut().find(|unit| unit.id == id)) {
                worker.work.digging = false;
                crate::give_up(worker);
            }
        }
    }

    fn liberate_workers(&mut self, pool: Pool, owner: u8) {
        for id in pool.slots.into_iter().filter_map(|slot| slot.worker) {
            let Some(index) = self.units.iter().position(|unit| unit.id == id) else { continue; };
            let original = self.kind_of(&self.units[index]).map(|kind| kind.stats);
            let worker = &mut self.units[index];
            worker.owner = owner;
            worker.work = Work::default();
            worker.arms = Arms::default();
            worker.recruitable = true;
            worker.dest = None;
            worker.drop_path();
            if let Some(original) = original {
                worker.stats.unselectable = original.unselectable;
                worker.stats.fighter.can_passive_acquire = original.fighter.can_passive_acquire;
            }
            if let Some(at) = self.map.index(worker.cell().0, worker.cell().1) { self.spot_house[at] = owner; }
        }
    }

    fn slave_spawn_cell(&self, site: Site, stats: UnitStats) -> Option<(i32, i32)> {
        let probe = Unit::new(u32::MAX - 1, site.owner, site.centre(), 0x80, stats);
        let others: Vec<Other> = self.others();
        site.perimeter().into_iter().find(|&cell| self.map.in_playfield(cell.0, cell.1)
            && crate::entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, &probe, cell, None, probe.cell(), -1) == code::OK)
    }

    fn slave_ore(&self, index: usize, site: Site) -> Option<(i32, i32)> {
        let here = self.units[index].cell();
        let others = self.others();
        self.map.ore_cells().filter(|&(cell, _, _)| distance(cell, site.centre()) <= self.slave_rules.worker_scan)
            .filter(|&(cell, _, _)| self.can_harvest(index, &others, cell))
            .min_by_key(|&(cell, kind, amount)| (distance(here, cell), -(self.ore_values.get(kind as usize).copied().unwrap_or(0) * (amount as i32 + 1)), cell.1, cell.0))
            .map(|(cell, _, _)| cell)
    }

    fn slave_return(&mut self, index: usize, site: Site) -> bool {
        let unit = &self.units[index];
        if site.adjacent(unit.cell()) && !unit.is_moving() { return true; }
        // Keep a valid return path, rather than resetting it every tick.
        if unit.destination().is_some_and(|cell| site.adjacent(cell)) { return false; }
        let here = unit.cell();
        let mut cells = site.perimeter();
        cells.sort_by_key(|&cell| (distance(here, cell), cell.1, cell.0));
        if let Some(cell) = cells.into_iter().find(|&cell| self.can_move_to(unit.id, cell)) { self.send(index, cell); }
        false
    }

    fn slave_credit_ore(&mut self, owner: u8, cargo: [u32; 4]) {
        // The same ore values and real/virtual purifier bonus as normal miners.
        let amount: i32 = cargo.iter().enumerate().map(|(kind, &bales)| self.ore_values.get(kind).copied().unwrap_or(0) * bales as i32).sum();
        let real = if self.low_power(owner) { 0 } else {
            self.buildings.iter().filter(|building| building.owner == owner && building.buildup.is_none()
                && self.building_types.get(building.kind as usize).is_some_and(|kind| kind.ore_purifier)).count() as i32
        };
        let virtuals = self.houses.get(owner as usize).and_then(|house| house.ai.as_ref()).map_or(0, |brain| self.ai_rules.virtual_purifiers[brain.difficulty.min(2)]);
        let bonus = ((real + virtuals) as f64 * self.ai_rules.purifier_bonus * amount as f64) as i32;
        if let Some(house) = self.houses.get_mut(owner as usize) { house.credits += amount + bonus; }
    }

    fn run_slave_worker(&mut self, slot: &mut Slot, site: Site) {
        let Some(id) = slot.worker else { return; };
        let Some(index) = self.units.iter().position(|unit| unit.id == id && unit.health > 0) else {
            slot.worker = None;
            slot.due = self.tick + site.rules.regeneration as u64;
            slot.phase = Phase::Seeking;
            return;
        };
        if self.units[index].owner != site.owner {
            self.units[index].owner = site.owner;
            let cell = self.units[index].cell();
            if let Some(at) = self.map.index(cell.0, cell.1) { self.spot_house[at] = site.owner; }
        }
        if self.units[index].frozen.is_some() || self.units[index].paralysed_until > self.tick { return; }
        if !site.active {
            self.units[index].work.digging = false;
            slot.phase = Phase::Returning;
            self.slave_return(index, site);
            return;
        }
        if let Phase::Digging(cell) = slot.phase {
            if self.units[index].cell() != cell || self.units[index].is_moving() {
                self.units[index].work.digging = false;
                slot.phase = Phase::Seeking;
                slot.due = self.tick + 1;
            }
        }
        if self.tick < slot.due { return; }
        match slot.phase {
            Phase::Seeking => {
                if self.units[index].work.load() > 0 { slot.phase = Phase::Returning; return; }
                if let Some(cell) = self.slave_ore(index, site) {
                    self.send(index, cell);
                    slot.phase = Phase::Going(cell);
                    slot.due = self.tick + 1;
                } else { slot.due = self.tick + self.slave_rules.kick_delay.max(1) as u64; }
            }
            Phase::Going(cell) => {
                if self.map.ore_at(cell.0, cell.1).is_none() {
                    slot.phase = Phase::Seeking;
                } else if self.units[index].cell() == cell && !self.units[index].is_moving() {
                    self.units[index].work.digging = true;
                    slot.phase = Phase::Digging(cell);
                    let rate = self.kind_of(&self.units[index]).and_then(|kind| kind.slave_worker).map_or(150, |rules| rules.harvest_rate);
                    slot.due = self.tick + rate.max(1) as u64;
                } else if !self.units[index].is_moving() { slot.phase = Phase::Seeking; }
            }
            Phase::Digging(cell) => {
                let room = self.units[index].stats.storage.saturating_sub(self.units[index].work.load());
                if self.units[index].cell() == cell && room > 0 {
                    if let Some((kind, _)) = self.map.ore_at(cell.0, cell.1) {
                        let removed = self.map.reduce_ore(cell.0, cell.1, 1);
                        self.units[index].work.cargo[kind as usize & 3] += removed;
                        if self.map.ore_at(cell.0, cell.1).is_none() { self.ore_emptied(cell, kind as usize); }
                    }
                }
                if self.units[index].work.load() >= self.units[index].stats.storage || self.map.ore_at(cell.0, cell.1).is_none() {
                    self.units[index].work.digging = false;
                    slot.phase = Phase::Returning;
                    slot.due = self.tick;
                } else {
                    let rate = self.kind_of(&self.units[index]).and_then(|kind| kind.slave_worker).map_or(150, |rules| rules.harvest_rate);
                    slot.due = self.tick + rate.max(1) as u64;
                }
            }
            Phase::Returning => {
                if self.slave_return(index, site) {
                    slot.phase = Phase::Reloading;
                    slot.due = self.tick + site.rules.reload.max(1) as u64;
                }
            }
            Phase::Reloading => {
                if site.rules.resource_destination && site.adjacent(self.units[index].cell()) && !self.units[index].is_moving() {
                    let cargo = std::mem::take(&mut self.units[index].work.cargo);
                    self.slave_credit_ore(site.owner, cargo);
                    slot.phase = Phase::Seeking;
                    slot.due = self.tick + 1;
                } else { slot.phase = Phase::Returning; }
            }
        }
    }

    fn slave_best_site(&self, unit: &Unit, ore: (i32, i32)) -> Option<(i32, i32)> {
        let building = unit.stats.deploys_into?;
        let zones = self.map.zones();
        let from = zones.map(|zones| crate::unit_zones(&self.map, zones, unit));
        let worker_zone = self.kind_of(unit).and_then(|kind| kind.slave_spawner)
            .and_then(|rules| self.unit_types.get(rules.worker as usize)).map(|kind| kind.stats.movement_zone as usize);
        let mut sites: Vec<_> = (-4..=4).flat_map(|dy| (-4..=4).map(move |dx| (ore.0 + dx, ore.1 + dy))).collect();
        sites.sort_by_key(|&cell| (distance(cell, ore), distance(cell, unit.cell()), cell.1, cell.0));
        sites.into_iter().find(|&cell| {
            self.foundation_clear(Self::deploy_origin(cell), building, unit.id + 1)
                && from.as_ref().is_none_or(|from| from.contains(&crate::dest_zone(&self.map, cell, unit.stats.movement_zone as usize)))
                && worker_zone.is_none_or(|zone| zones.is_none() || {
                    let ore_zone = crate::dest_zone(&self.map, ore, zone);
                    ore_zone > 1 && ore_zone != u16::MAX && ore_zone == crate::dest_zone(&self.map, cell, zone)
                })
        })
    }

    fn slave_relocate(&mut self, pool: &mut Pool, site: Site) {
        if self.tick < pool.scan_at || !site.rules.resource_gatherer { return; }
        pool.scan_at = self.tick + self.slave_rules.kick_delay.max(1) as u64;
        let mut ores: Vec<_> = self.map.ore_cells().filter(|&(cell, _, _)| distance(cell, site.centre()) <= self.slave_rules.long_scan).map(|(cell, _, _)| cell).collect();
        ores.sort_by_key(|&cell| (distance(cell, site.centre()), cell.1, cell.0));
        match pool.host {
            Host::Unit(id) => {
                let Some(unit) = self.unit(id).filter(|unit| !unit.is_moving() && !unit.tethered && !unit.deploying && unit.arms.target.is_none()) else { return; };
                let Some((ore, planned)) = ores.iter().find_map(|&ore| self.slave_best_site(unit, ore).map(|cell| (ore, cell))) else { return; };
                if distance(ore, unit.cell()) <= self.slave_rules.short_scan && self.can_deploy(id) {
                    self.order(Order::Deploy { unit: id });
                } else if let Some(index) = self.units.iter().position(|unit| unit.id == id) {
                    self.send(index, planned);
                }
            }
            Host::Building(id) if site.active => {
                let Some(stats) = self.buildings.iter().find(|building| building.id == id).and_then(|building| self.building_types.get(building.kind as usize)).and_then(|kind| kind.undeploys_into) else { return; };
                let probe = Unit::new(u32::MAX - 1, site.owner, site.centre(), 0, stats);
                if let Some((ore, cell)) = ores.iter().find_map(|&ore| self.slave_best_site(&probe, ore).map(|cell| (ore, cell))) {
                    if distance(ore, site.centre()) > self.slave_rules.short_scan && distance(ore, site.centre()) - distance(ore, cell) >= self.slave_rules.scan_correction {
                        self.order(Order::Undeploy { building: id, x: cell.0, y: cell.1 });
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn slave_order(&mut self, order: Order) -> bool {
        if let Some(id) = order.unit().filter(|&id| self.is_slave_miner(id)) {
            if matches!(order, Order::Move { .. } | Order::Stop { .. } | Order::Attack { .. } | Order::Deploy { .. }) {
                if let Some(pool) = self.slave_pools.iter_mut().find(|pool| pool.host == Host::Unit(id)) {
                    pool.scan_at = self.tick + self.slave_rules.kick_delay.max(1) as u64;
                }
            }
        }
        let Order::Harvest { unit, x, y } = order else { return false; };
        if !self.is_slave_miner(unit) { return false; }
        let Some(index) = self.units.iter().position(|miner| miner.id == unit && !miner.tethered) else { return true; };
        if let Some(cell) = self.slave_best_site(&self.units[index], (x, y)) { self.send(index, cell); }
        true
    }

    pub(crate) fn run_slaves(&mut self) {
        let hosts: Vec<_> = self.buildings.iter().filter(|building| building.health > 0 && self.building_types.get(building.kind as usize).is_some_and(|kind| kind.slave_spawner.is_some())).map(|building| Host::Building(building.id))
            .chain(self.units.iter().filter(|unit| unit.health > 0 && self.kind_of(unit).is_some_and(|kind| kind.slave_spawner.is_some())).map(|unit| Host::Unit(unit.id))).collect();
        for host in hosts {
            if self.slave_pools.iter().any(|pool| pool.host == host) { continue; }
            let Some(site) = self.slave_site(host) else { continue; };
            let slots = (0..site.rules.count).map(|_| Slot { worker: None, due: self.tick, phase: Phase::Seeking }).collect();
            self.slave_pools.push(Pool { host, owner: site.owner, slots, spawn_at: self.tick, scan_at: self.tick + self.slave_rules.kick_delay.max(1) as u64, liberator: None });
        }
        let mut pools = std::mem::take(&mut self.slave_pools);
        let mut retained = Vec::with_capacity(pools.len());
        for mut pool in pools.drain(..) {
            let Some(site) = self.slave_site(pool.host) else {
                if self.stored_slave_host(pool.host) {
                    self.pause_slave_pool(&mut pool);
                    retained.push(pool);
                } else {
                    let owner = pool.liberator.unwrap_or(0);
                    self.liberate_workers(pool, owner);
                }
                continue;
            };
            pool.liberator = None;
            pool.owner = site.owner;
            for slot in &mut pool.slots { self.run_slave_worker(slot, site); }
            if site.active && self.tick >= pool.spawn_at {
                if let Some(slot) = pool.slots.iter_mut().find(|slot| slot.worker.is_none() && self.tick >= slot.due) {
                    if let Some(kind) = self.unit_types.get(site.rules.worker as usize).filter(|kind| kind.slave_worker.is_some()) {
                        let mut stats = UnitStats { kind: site.rules.worker + 1, ..kind.stats };
                        stats.unselectable = true;
                        stats.fighter.can_passive_acquire = false;
                        if let Some(cell) = self.slave_spawn_cell(site, stats) {
                            slot.worker = self.add_infantry(cell, 0, 0x80, stats, site.owner);
                            if let Some(worker) = slot.worker.and_then(|id| self.units.iter_mut().find(|unit| unit.id == id)) { worker.recruitable = false; }
                            slot.phase = Phase::Seeking;
                            slot.due = self.tick + 1;
                        }
                    }
                    pool.spawn_at = self.tick + site.rules.reload.max(1) as u64;
                }
            }
            self.slave_relocate(&mut pool, site);
            retained.push(pool);
        }
        // Relocation may convert a no-buildup test building immediately; apply
        // the conversion recorded by finish_packing to the temporarily held pool.
        for pool in &mut retained {
            if let Host::Building(id) = pool.host {
                if let Some(&(_, unit)) = self.unpacked.iter().rev().find(|&&(old, _)| old == id) { pool.host = Host::Unit(unit); }
            }
        }
        self.slave_pools.extend(retained);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildingType, House, Map, UnitType};

    fn fixture() -> (World, u32) {
        let mut map = Map::new(40, 40);
        for y in 0..40 { for x in 0..40 { map.set_cell(x, y, true, 0); } }
        let mut world = World::new(map, None);
        world.houses = vec![House::default(), House { human: true, ..House::default() }, House { human: true, ..House::default() }];
        world.end_checks = false;
        world.slave_rules.kick_delay = 10_000;
        world.ore_values = vec![25, 50];
        world.set_ore_types(vec![crate::ore::OreType { growth: 0, spread: 0, growth_percentage: 0.0, spread_percentage: 0.0 }]);
        let workforce = SpawnerRules { worker: 0, count: 2, regeneration: 10, reload: 3, resource_gatherer: true, resource_destination: true };
        world.unit_types.push(UnitType { slave_worker: Some(WorkerRules { harvest_rate: 2 }),
            stats: UnitStats { kind: 1, infantry: true, speed_type: crate::SpeedType::Foot, speed: 8, strength: 125, storage: 4, ..UnitStats::default() }, ..UnitType::default() });
        let miner = UnitStats { kind: 2, speed: 6, strength: 2000, deploys_into: Some(0), ..UnitStats::default() };
        world.unit_types.push(UnitType { slave_spawner: Some(workforce), deploy_facing: Some(0), stats: miner, ..UnitType::default() });
        world.building_types.push(BuildingType { slave_spawner: Some(workforce), deploy_facing: Some(0), size: (2, 2), strength: 2000,
            undeploys_into: Some(miner), base_normal: true, ..BuildingType::default() });
        for x in 14..18 { world.map.put_ore(x, 12, 0, 11); }
        let host = world.add_map_building(1, 0, (10, 10), 256);
        (world, host)
    }

    fn run(world: &mut World, ticks: u32) { for _ in 0..ticks { world.tick(); } }

    fn boarded_miner() -> (World, u32, u32, Vec<u32>) {
        let (mut world, host) = fixture();
        // Stock SMIN is Size=3; both landing transports accept SizeLimit=6.
        world.unit_types[1].size = 3;
        world.unit_types.push(UnitType { passengers: 12, size_limit: 6, size: 16,
            stats: UnitStats { kind: 3, strength: 300, ..UnitStats::default() }, ..UnitType::default() });
        run(&mut world, 6);
        let workers = world.slave_workers(Host::Building(host));
        assert_eq!(workers.len(), 2);
        world.order(Order::Undeploy { building: host, x: 20, y: 20 });
        let miner = world.unpacked_into(host).unwrap();
        run(&mut world, 300);
        let transport = world.add_unit((22, 20), 0, world.unit_types[2].stats, 1);
        assert!(world.can_board(miner, transport));
        world.order(Order::Board { unit: miner, transport });
        for _ in 0..500 {
            world.tick();
            if world.unit(miner).is_none() { break; }
        }
        assert!(world.unit(transport).unwrap().cargo.iter().any(|unit| unit.id == miner), "the real Board mission must take the miner off the map");
        world.tick();
        (world, miner, transport, workers)
    }

    #[test]
    fn boarding_pauses_the_workforce_and_unloading_reuses_it_without_duplicates() {
        let (mut world, miner, transport, workers) = boarded_miner();
        assert_eq!(world.slave_workers(Host::Unit(miner)), workers);
        let cargo: Vec<_> = workers.iter().map(|&id| world.unit(id).unwrap().work.cargo).collect();
        let credits = world.houses[1].credits;
        let remaining: Vec<_> = world.slave_pools[0].slots.iter().map(|slot| slot.due as i128 - world.tick as i128).collect();
        run(&mut world, 1000);
        assert_eq!(world.slave_workers(Host::Unit(miner)), workers);
        assert_eq!(world.slave_pools[0].slots.iter().map(|slot| slot.due as i128 - world.tick as i128).collect::<Vec<_>>(), remaining, "work and regeneration timers stay paused aboard");
        assert_eq!(world.houses[1].credits, credits);
        for (&id, &cargo) in workers.iter().zip(&cargo) {
            let worker = world.unit(id).unwrap();
            assert!(world.is_enslaved(id) && worker.owner == 1 && !worker.work.digging);
            assert_eq!(worker.work.cargo, cargo);
        }
        let (mut restored, _, _, _) = boarded_miner();
        restored.restore(&world.snapshot()).unwrap();
        world.order(Order::Deploy { unit: transport });
        restored.order(Order::Deploy { unit: transport });
        run(&mut world, 100);
        run(&mut restored, 100);
        assert_eq!(world.snapshot(), restored.snapshot(), "an off-map workforce resumes identically after restore");
        assert!(world.unit(miner).is_some());
        assert_eq!(world.slave_workers(Host::Unit(miner)), workers);
        world.order(Order::Deploy { unit: miner });
        run(&mut world, 100);
        let deployed = world.buildings().iter().find(|building| building.owner == 1).unwrap().id;
        assert_eq!(world.slave_workers(Host::Building(deployed)), workers);
        assert_eq!(world.units().iter().filter(|unit| world.is_enslaved(unit.id)).count(), 2);
        assert_eq!(world.units().iter().filter(|unit| unit.stats.kind == 1).count(), 2, "unloading and redeploying must not manufacture another workforce");
    }

    #[test]
    fn a_sinking_transport_releases_its_lost_miners_workforce_to_the_killer() {
        let (mut world, miner, transport, workers) = boarded_miner();
        let cell = world.unit(transport).unwrap().cell();
        world.map.set_land(cell.0, cell.1, crate::Land::Water);
        world.map.lands.0[crate::Land::Water as usize][crate::SpeedType::Track as usize] = 0;
        let killer = world.add_unit((30, 30), 0, UnitStats::default(), 2);
        world.warheads.push(crate::combat::Warhead::default());
        assert!(world.receive_damage(Aim::Unit(transport), 100_000, 0, 0, Some(Aim::Unit(killer))));
        assert_eq!(world.slave_workers(Host::Unit(miner)), workers, "a carrier's fatal hit must first allow ordinary passenger survival handling");
        run(&mut world, 2);
        assert!(world.unit(transport).is_none() && !world.stored_slave_host(Host::Unit(miner)));
        assert!(world.slave_pools.is_empty());
        for id in workers {
            assert_eq!(world.unit(id).unwrap().owner, 2);
            assert!(!world.is_enslaved(id));
        }
    }

    #[test]
    fn surviving_a_carriers_death_keeps_the_miners_original_workforce() {
        let (mut world, miner, transport, workers) = boarded_miner();
        let killer = world.add_unit((30, 30), 0, UnitStats::default(), 2);
        world.warheads.push(crate::combat::Warhead::default());
        assert!(world.receive_damage(Aim::Unit(transport), 100_000, 0, 0, Some(Aim::Unit(killer))));
        run(&mut world, 2);
        assert!(world.unit(transport).is_none() && world.unit(miner).is_some(), "a ground carrier ejects the surviving passenger");
        assert_eq!(world.slave_workers(Host::Unit(miner)), workers);
        assert_eq!(world.slave_pools[0].liberator, None);
        assert!(workers.iter().all(|&id| world.is_enslaved(id) && world.unit(id).unwrap().owner == 1));
    }

    #[test]
    fn workers_walk_harvest_return_and_pay_only_for_removed_ore() {
        let (mut world, host) = fixture();
        let start: u32 = world.map.ore_cells().map(|(_, _, amount)| amount as u32 + 1).sum();
        run(&mut world, 800);
        let workers = world.slave_workers(Host::Building(host));
        assert_eq!(workers.len(), 2);
        assert!(world.houses[1].credits > 0, "workers must actually return and unload");
        let remaining: u32 = world.map.ore_cells().map(|(_, _, amount)| amount as u32 + 1).sum();
        let carrying: u32 = workers.iter().filter_map(|&id| world.unit(id)).map(|worker| worker.work.load()).sum();
        assert_eq!(world.houses[1].credits, ((start - remaining - carrying) * 25) as i32);
        for id in workers {
            assert!(world.is_enslaved(id));
            let old = world.unit(id).unwrap().destination();
            world.order(Order::Move { unit: id, x: 30, y: 30 });
            assert_eq!(world.unit(id).unwrap().destination(), old, "workers obey their miner, not player orders");
        }
        assert!(world.units().iter().all(|unit| !unit.stats.harvester), "slave workers and their miner are not ordinary harvesters");
    }

    #[test]
    fn dead_workers_regenerate_after_the_configured_delay_without_duplicating_survivors() {
        let (mut world, host) = fixture();
        run(&mut world, 6);
        let before = world.slave_workers(Host::Building(host));
        assert_eq!(before.len(), 2);
        world.remove_unit(before[0]);
        run(&mut world, 2);
        let deadline = world.slave_pools[0].slots.iter().find(|slot| slot.worker.is_none()).unwrap().due;
        while world.current_tick() + 1 < deadline { world.tick(); }
        assert_eq!(world.slave_workers(Host::Building(host)), vec![before[1]]);
        world.tick();
        let after = world.slave_workers(Host::Building(host));
        assert_eq!(after.len(), 2);
        assert!(after.contains(&before[1]) && !after.contains(&before[0]));
        run(&mut world, 100);
        assert_eq!(world.slave_workers(Host::Building(host)), after);
    }

    #[test]
    fn packing_and_deploying_keeps_the_same_workers_and_their_ore() {
        let (mut world, host) = fixture();
        run(&mut world, 50);
        let workers = world.slave_workers(Host::Building(host));
        let cargo: u32 = workers.iter().filter_map(|&id| world.unit(id)).map(|worker| worker.work.load()).sum();
        let paid = world.houses[1].credits;
        world.order(Order::Undeploy { building: host, x: 21, y: 20 });
        let mobile = world.unpacked_into(host).expect("miner packed");
        assert_eq!(world.slave_workers(Host::Unit(mobile)), workers);
        assert_eq!(world.unit(mobile).unwrap().facing, 0, "SMIN uses its DeployFacing, not the MCV's default");
        run(&mut world, 300);
        assert_eq!(world.houses[1].credits, paid, "workers do not fabricate income while the mine is moving");
        assert_eq!(workers.iter().filter_map(|&id| world.unit(id)).map(|worker| worker.work.load()).sum::<u32>(), cargo);
        world.order(Order::Deploy { unit: mobile });
        run(&mut world, 50);
        let deployed = world.buildings().iter().find(|building| building.owner == 1).unwrap().id;
        assert_eq!(world.slave_workers(Host::Building(deployed)), workers);
        assert_eq!(world.units().iter().filter(|unit| world.is_enslaved(unit.id)).count(), 2);
    }

    #[test]
    fn a_blocked_miner_waits_for_room_instead_of_losing_or_duplicating_workers() {
        let (mut world, host) = fixture();
        let site = world.slave_site(Host::Building(host)).unwrap();
        for cell in site.perimeter() { world.map.set_cell(cell.0, cell.1, false, 0); }
        run(&mut world, 20);
        assert!(world.slave_workers(Host::Building(host)).is_empty());
        world.map.set_cell(10, 9, true, 0);
        run(&mut world, 10);
        assert_eq!(world.slave_workers(Host::Building(host)).len(), 2);
        assert_eq!(world.units().len(), 2);
    }

    #[test]
    fn liberating_workers_stops_their_economy_and_returns_player_control() {
        let (mut world, host) = fixture();
        run(&mut world, 50);
        let workers = world.slave_workers(Host::Building(host));
        world.slave_host_died(Aim::Building(host), Some(2));
        world.demolish(0);
        let credits = world.houses[1].credits;
        for &id in &workers {
            assert!(!world.is_enslaved(id));
            let worker = world.unit(id).unwrap();
            assert_eq!(worker.owner, 2);
            assert!(!worker.stats.unselectable);
            world.order(Order::Move { unit: id, x: 25, y: 25 });
            assert!(world.unit(id).unwrap().destination().is_some());
        }
        run(&mut world, 100);
        assert_eq!(world.houses[1].credits, credits);
        assert!(world.slave_pools.is_empty());
    }

    #[test]
    fn snapshot_restores_workforce_cargo_timers_and_future_income() {
        let (mut original, _) = fixture();
        run(&mut original, 75);
        let bytes = original.snapshot();
        let (mut restored, _) = fixture();
        restored.restore(&bytes).unwrap();
        assert_eq!(restored.snapshot(), bytes);
        run(&mut original, 200);
        run(&mut restored, 200);
        assert_eq!(original.snapshot(), restored.snapshot());
        assert_eq!(original.hash(), restored.hash());
    }

    #[test]
    fn a_miner_relocates_to_distant_ore_and_resumes_the_same_workforce() {
        let (mut world, original_host) = fixture();
        world.slave_rules.short_scan = 2;
        world.slave_rules.kick_delay = 5;
        run(&mut world, 5);
        let first = world.slave_workers(Host::Building(original_host));
        assert!(!first.is_empty());
        run(&mut world, 800);
        assert!(world.unpacked_into(original_host).is_some(), "the mine packs and moves when ore is too far away");
        assert!(world.houses[1].credits > 0, "it deploys near ore and resumes collecting");
        let pool = &world.slave_pools[0];
        let workers = world.slave_workers(pool.host);
        assert_eq!(workers.len(), 2);
        assert!(first.iter().all(|id| workers.contains(id)), "relocation retains living workers");
        assert!(matches!(pool.host, Host::Building(_)));
        assert_eq!(world.units().iter().filter(|unit| world.is_enslaved(unit.id)).count(), 2);
    }

    #[test]
    fn lethal_combat_damage_liberates_only_that_miners_workers_to_the_killer() {
        let (mut world, host) = fixture();
        let other = world.add_map_building(1, 0, (25, 20), 256);
        run(&mut world, 6);
        let freed = world.slave_workers(Host::Building(host));
        let kept = world.slave_workers(Host::Building(other));
        assert_eq!((freed.len(), kept.len()), (2, 2));
        let killer = world.add_unit((30, 30), 0, UnitStats::default(), 2);
        world.warheads.push(crate::combat::Warhead::default());
        assert!(world.receive_damage(Aim::Building(host), 100_000, 0, 0, Some(Aim::Unit(killer))));
        for &id in &freed {
            assert_eq!(world.unit(id).unwrap().owner, 2);
            assert!(!world.is_enslaved(id));
        }
        assert!(kept.iter().all(|&id| world.is_enslaved(id) && world.unit(id).is_some_and(|unit| unit.owner == 1)));
        world.tick();
        assert!(!world.buildings().iter().any(|building| building.id == host));
        assert_eq!(world.slave_workers(Host::Building(other)), kept);
    }

    #[test]
    fn relocation_skips_nearer_ore_on_an_unreachable_island() {
        let (mut world, host) = fixture();
        let old: Vec<_> = world.map.ore_cells().map(|(cell, _, _)| cell).collect();
        for cell in old { world.map.reduce_ore(cell.0, cell.1, u32::MAX); }
        for y in 0..40 { world.map.set_cell(20, y, false, 0); }
        world.map.put_ore(22, 10, 0, 11);
        world.map.put_ore(2, 25, 0, 11);
        world.map.build_zones();
        world.slave_rules.short_scan = 2;
        world.slave_rules.kick_delay = 5;
        run(&mut world, 1600);
        assert!(world.unpacked_into(host).is_some(), "the miner relocates toward the reachable field");
        let pool = &world.slave_pools[0];
        let site = world.slave_site(pool.host).unwrap();
        assert!(site.cell.0 < 20, "it stays in its reachable land area");
        assert!(world.houses[1].credits > 0);
        assert_eq!(world.map.ore_at(22, 10), Some((0, 11)), "ore across the impassable barrier was not mined");
    }
}
