//! Radiation on the ground.
//!
//! Follows the original game
//! (10 to 12).

use crate::combat::{Aim, Stance};
use crate::{World, LEPTONS_PER_CELL};

/// The `[Radiation]` rules.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    pub duration_multiple: u32,
    pub application_delay: u32,
    pub level_max: i32,
    pub level_delay: u32,
    /// `RadLightDelay`: frames between dimmings of a site's light.
    pub light_delay: u32,
    pub level_factor: f64,
    /// `RadSiteWarhead`.
    pub warhead: u16,
}

impl Default for Rules {
    fn default() -> Self {
        Self { duration_multiple: 1, application_delay: 16, level_max: 500, level_delay: 90, light_delay: 90, level_factor: 0.2, warhead: 0 }
    }
}

/// A dose laid on a cell and those round it.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Site {
    pub cell: (i32, i32),
    /// Cells out from the middle.
    pub spread: i32,
    pub level: i32,
    pub born: u64,
}

impl World {
    /// Lays radiation where a weapon with a `RadLevel` goes off.
    // A site on the cell of the blast,
    // reaching the whole cells of the warhead's `CellSpread`.
    pub(crate) fn irradiate(&mut self, at: (i32, i32, i32), level: i32, warhead: u16) {
        if level <= 0 {
            return;
        }
        let spread = self.warheads.get(warhead as usize).map_or(0, |w| w.cell_spread as i32);
        let cell = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
        // A cell has one site.
        // More on the same cell is added to what is left of it, level times
        // the share of its life remaining, and the site starts afresh at
        // that, with a life to match.
        let (tick, multiple) = (self.tick, self.rad_rules.duration_multiple as u64);
        if let Some(site) = self.rad_sites.iter_mut().find(|site| site.cell == cell) {
            let life = (site.level as u64 * multiple).max(1);
            let left = life.saturating_sub(tick - site.born);
            site.level = (site.level as u64 * left / life) as i32 + level;
            site.born = tick;
            site.spread = spread;
            return;
        }
        self.rad_sites.push(Site { cell, spread, level, born: self.tick });
    }

    /// What is left of the site on a cell: its level times the share of its
    /// life remaining.
    // `CurrentLevel`.
    pub fn site_level(&self, cell: (i32, i32)) -> i32 {
        self.rad_sites.iter().find(|site| site.cell == cell).map_or(0, |site| {
            let life = (site.level as u64 * self.rad_rules.duration_multiple as u64).max(1);
            (site.level as u64 * life.saturating_sub(self.tick - site.born) / life) as i32
        })
    }

    /// How strongly the sites' green light falls on a cell, 0 to 1.
    // The
    // only thing drawn for radiation is a light at each site's middle, of the
    // colour `RadColor` times `RadTintFactor` whatever the level, reaching the
    // site's spread and half a cell, full at the middle and falling evenly to
    // nothing at the edge; every `RadLightDelay` frames the colour is set to
    // the share of the site's life that is left. Lights add, and a cell's
    // tint stops at twice full, which is this value at 1.
    pub fn rad_glow(&self, cell: (i32, i32)) -> f32 {
        let rules = self.rad_rules;
        let step = rules.light_delay.max(1) as u64;
        self.rad_sites
            .iter()
            .map(|site| {
                let radius = (site.spread * 256 + 128) as f64;
                let d = ((cell.0 - site.cell.0) as f64 * 256.0).hypot((cell.1 - site.cell.1) as f64 * 256.0).trunc();
                if d > radius {
                    return 0.0;
                }
                let life = (site.level as u64 * rules.duration_multiple as u64).max(1);
                let age = (self.tick - site.born) / step * step;
                ((radius - d) / radius * life.saturating_sub(age) as f64 / life as f64) as f32
            })
            .sum::<f32>()
            .min(1.0)
    }

    /// The radiation on a cell now.
    // A cell `d` leptons from the middle starts with
    // (radius - d) / radius of the level, radius being the spread in leptons and
    // half a cell; every `RadLevelDelay` frames it loses that share of what it
    // started with which brings it to nothing when the site's life,
    // level times `RadDurationMultiple` frames, is over.
    pub fn radiation(&self, cell: (i32, i32)) -> i32 {
        let rules = self.rad_rules;
        self.rad_sites
            .iter()
            .map(|site| {
                let (dx, dy) = ((cell.0 - site.cell.0) as f64 * 256.0, (cell.1 - site.cell.1) as f64 * 256.0);
                if dx.abs().max(dy.abs()) > site.spread as f64 * 256.0 {
                    return 0;
                }
                let radius = (site.spread * 256 + 128) as f64;
                let start = ((radius - dx.hypot(dy)) / radius * site.level as f64).max(0.0);
                let life = (site.level as u64 * rules.duration_multiple as u64).max(1);
                let steps = (self.tick - site.born) / rules.level_delay.max(1) as u64;
                let lost = start * (steps * rules.level_delay as u64) as f64 / life as f64;
                (start - lost).max(0.0) as i32
            })
            .sum()
    }

    /// Sites run out; what stands in radiation is hurt; a deployed Desolator
    /// keeps its ground hot.
    // On every `RadApplicationDelay`th frame each
    // thing on foot or wheels that is not `ImmuneToRadiation` and not in the
    // air takes the cell's level, at most `RadLevelMax`, times `RadLevelFactor`,
    // with `RadSiteWarhead` and nobody to blame. Buildings are not hurt.
    pub(crate) fn run_radiation(&mut self) {
        let rules = self.rad_rules;
        let tick = self.tick;
        self.rad_sites.retain(|site| tick < site.born + site.level as u64 * rules.duration_multiple as u64);
        if !tick.is_multiple_of(rules.application_delay.max(1) as u64) {
            return;
        }
        // A deployed soldier whose second weapon has a `RadLevel` fires it at
        // his own cell again when the cell has less than a third of that level.
        let deployed: Vec<((i32, i32, i32), (i32, i32), u16)> = self
            .units
            .iter()
            .filter(|u| u.stance == Stance::Deployed && u.health > 0)
            .filter_map(|u| u.stats.fighter.secondary.map(|mount| ((u.x, u.y, 0), u.cell(), mount.weapon)))
            .collect();
        for (at, cell, weapon) in deployed {
            let Some(weapon) = self.weapons.get(weapon as usize).cloned() else {
                continue;
            };
            if weapon.rad_level > 0 && self.site_level(cell) < weapon.rad_level / 3 {
                let point = (at.0, at.1, self.ground_z(at.0, at.1));
                self.irradiate(point, weapon.rad_level, weapon.warhead);
                self.blast_event(weapon.warhead, weapon.damage, point, false);
            }
        }
        if self.rad_sites.is_empty() {
            return;
        }
        let victims: Vec<(u32, i32)> = self
            .units
            .iter()
            .filter(|u| u.health > 0 && !u.stats.fighter.immune_to_radiation && self.altitude(u) == 0 && u.falling.is_none())
            .filter_map(|u| {
                let level = self.radiation(u.cell()).min(rules.level_max);
                let damage = (level as f64 * rules.level_factor) as i32;
                (damage > 0).then_some((u.id, damage))
            })
            .collect();
        for (id, damage) in victims {
            self.receive_damage(Aim::Unit(id), damage, 0, rules.warhead, None);
        }
    }

    /// Soldiers in hospitals and armories come out when their time is up.
    // Whole from a hospital, elite from an armory, put out beside the building.
    pub(crate) fn run_patients(&mut self) {
        for index in 0..self.buildings.len() {
            if self.buildings[index].patients.is_empty() {
                continue;
            }
            let hospital = self.building_types.get(self.buildings[index].kind as usize).is_some_and(|kind| kind.hospital);
            let tick = self.tick;
            let cap = self.veterancy.cap;
            let building = &mut self.buildings[index];
            let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut building.patients).into_iter().partition(|(at, _)| *at <= tick);
            building.patients = waiting;
            if ready.is_empty() {
                continue;
            }
            for (_, mut unit) in ready {
                if hospital {
                    unit.health = unit.stats.strength;
                } else {
                    unit.veterancy = cap.max(2.0);
                }
                self.buildings[index].occupants.push(unit);
            }
            self.remove_occupants(index);
        }
    }

    /// Repair depots mend the vehicle standing on them.
    // The
    // first step is taken at once and then one every `URepairRate` (15 frames):
    // `RepairStep` points for the type's cost over the steps a full repair
    // takes, times `RepairPercent`, at least one credit; with too little money
    // nothing is mended and the depot looks again 72 frames later; a whole
    // vehicle is sent off to the depot's rally point or a cell beside it.
    // Not original behaviour: any open cell of the depot serves as its dock
    // (the engine has the vehicle drive to the dock coordinate and call in by
    // radio).
    pub(crate) fn run_cash(&mut self) {
        let tick = self.tick;
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            let Some((_, amount, delay)) = self.building_types.get(building.kind as usize).map(|kind| kind.cash).filter(|cash| cash.2 != 0) else {
                continue;
            };
            if building.buildup.is_some() {
                continue;
            }
            if building.cash_at == 0 {
                self.buildings[index].cash_at = tick + delay as u64;
                continue;
            }
            if tick < building.cash_at {
                continue;
            }
            let (owner, online) = (building.owner, self.online(building));
            self.buildings[index].cash_at = tick + delay as u64;
            if owner != 0 && online
                && let Some(house) = self.houses.get_mut(owner as usize) {
                    house.credits = (house.credits + amount).max(0);
                }
        }
    }

    pub(crate) fn run_depots(&mut self) {
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            let Some(kind) = self.building_types.get(building.kind as usize).filter(|kind| kind.unit_repair) else {
                continue;
            };
            if building.buildup.is_some() || self.tick < building.depot_next || !self.online(building) {
                continue;
            }
            let (owner, cell, size, rally, id) = (building.owner, building.cell, building.size, building.rally, building.id);
            let _ = (kind, id);
            let inside = |at: (i32, i32)| (cell.0..cell.0 + size.0).contains(&at.0) && (cell.1..cell.1 + size.1).contains(&at.1);
            let contact = self
                .units
                .iter()
                .find(|u| !u.stats.infantry && u.plane.is_none() && u.health > 0 && self.allied(u.owner, owner) && inside(u.cell()) && !u.is_moving() && self.altitude(u) == 0)
                .map(|u| (u.id, u.health, u.stats.strength, u.stats.kind));
            let Some((unit, health, strength, unit_kind)) = contact else {
                self.buildings[index].depot_busy = false;
                continue;
            };
            if health < strength {
                let cost = (unit_kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).map_or(0, |t| t.cost);
                let steps = (strength / self.repair_step.max(1)).max(1);
                let price = (((cost / steps) as f64 * self.repair_percent) as i32).max(1);
                if self.houses[owner as usize].credits < price {
                    self.buildings[index].depot_next = self.tick + 72;
                    continue;
                }
                self.houses[owner as usize].credits -= price;
                let step = self.repair_step;
                if let Some(u) = self.units.iter_mut().find(|u| u.id == unit) {
                    u.health = (u.health + step).min(strength);
                }
                if !self.buildings[index].depot_busy {
                    self.buildings[index].depot_busy = true;
                    self.events.push(crate::combat::Event::DepotRepairing { house: owner });
                }
                self.buildings[index].depot_next = self.tick + 15;
                continue;
            }
            if self.buildings[index].depot_busy {
                self.events.push(crate::combat::Event::UnitRepaired { house: owner });
            }
            self.buildings[index].depot_busy = false;
            self.buildings[index].depot_next = self.tick + 72;
            let to = rally.unwrap_or((cell.0 + size.0, cell.1 + size.1 / 2));
            self.order(crate::Order::Move { unit, x: to.0, y: to.1 });
        }
    }
}
