//! Defeat, the end of the game, and the figures of the score screen.
//!
//! Follows the original game.

use crate::combat::{Aim, Event};
use crate::World;

/// What the score screen shows for a house.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Score {
    pub kills: u32,
    pub losses: u32,
    pub built: u32,
    pub points: i32,
}

impl World {
    /// Who killed something goes into both houses' figures.
    // The victim's cost in points
    // to the killer's house, doubled for a veteran and tripled for an elite,
    // nothing for an ally's; a loss for the victim's house and a kill for the
    // killer's; a building that is `Insignificant` counts for neither.
    pub(crate) fn score_kill(&mut self, victim: Aim, killer: Option<Aim>) {
        let (owner, cost, counts) = match victim {
            Aim::Unit(id) => match self.unit(id) {
                Some(unit) => {
                    let cost = (unit.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).map_or(0, |t| t.cost);
                    (unit.owner, cost * [3, 2, 1][unit.rank() as usize], true)
                }
                None => return,
            },
            Aim::Building(id) => match self.building_at(id).map(|at| &self.buildings[at]) {
                Some(building) => {
                    let kind = self.building_types.get(building.kind as usize);
                    (building.owner, kind.map_or(0, |t| t.cost), !kind.is_some_and(|t| t.fighter.insignificant))
                }
                None => return,
            },
            Aim::Cell(..) => return,
        };
        let by = killer.and_then(|killer| self.owner_of(killer));
        if let Some(by) = by {
            let points = if self.allied(by, owner) { 0 } else { cost };
            if let Some(house) = self.houses.get_mut(by as usize) {
                house.score.points += points;
                if counts {
                    house.score.kills += 1;
                }
            }
        }
        if counts
            && let Some(house) = self.houses.get_mut(owner as usize) {
                house.score.losses += 1;
            }
    }

    /// The defeat of houses and the end of the game.
    // Every
    // frame, a house not yet defeated is so when, in a short game, it has no
    // building and no construction vehicle, or otherwise nothing at all (things
    // that are `Insignificant` never count); what it has left is destroyed; the
    // game is over when one house is left, all those left are allied, or no
    // person is left.
    pub(crate) fn run_defeat(&mut self) {
        if !self.end_checks || self.over.is_some() || self.tick == 0 {
            return;
        }
        let mut fell = false;
        for house in 1..self.houses.len() as u8 {
            if self.houses[house as usize].defeated {
                continue;
            }
            let buildings = self.buildings.iter().filter(|b| b.owner == house && !self.building_types.get(b.kind as usize).is_some_and(|k| k.fighter.insignificant)).count();
            let base_unit = |unit: &crate::Unit| {
                unit.stats.deploys_into.and_then(|kind| self.building_types.get(kind as usize)).is_some_and(|kind| kind.construction_yard)
            };
            let mine = |unit: &&crate::Unit| unit.owner == house && unit.health > 0;
            let lost = if self.short_game {
                buildings == 0 && !self.units.iter().filter(mine).any(|unit| base_unit(unit) || unit.cargo.iter().any(base_unit))
            } else {
                buildings == 0 && !self.units.iter().filter(mine).any(|unit| !unit.stats.fighter.insignificant)
            };
            if !lost {
                continue;
            }
            let units: Vec<u32> = self.units.iter().filter(|unit| unit.owner == house).map(|unit| unit.id).collect();
            let buildings: Vec<u32> = self.buildings.iter().filter(|b| b.owner == house).map(|b| b.id).collect();
            // Each takes its whole health from
            // `C4Warhead`, so it dies as anything else would, by nobody's hand.
            for aim in units.into_iter().map(Aim::Unit).chain(buildings.into_iter().map(Aim::Building)) {
                let health = match aim {
                    Aim::Unit(id) => self.unit(id).map(|unit| unit.health),
                    Aim::Building(id) => self.building_at(id).map(|at| &self.buildings[at]).map(|b| b.health),
                    Aim::Cell(..) => None,
                };
                let Some(health) = health else {
                    continue;
                };
                let died = self.ability_rules.c4_warhead.is_some_and(|warhead| self.receive_damage(aim, health.max(1), 0, warhead, None));
                if !died && self.alive(aim) {
                    self.score_kill(aim, None);
                    self.kill(aim);
                }
            }
            self.houses[house as usize].defeated = true;
            self.houses[house as usize].ai = None;
            self.events.push(Event::HouseDefeated { house });
            fell = true;
        }
        if !fell {
            return;
        }
        let alive: Vec<u8> = (1..self.houses.len() as u8).filter(|&house| !self.houses[house as usize].defeated).collect();
        let allied = alive.iter().all(|&a| alive.iter().all(|&b| self.allied(a, b)));
        let people = alive.iter().filter(|&&house| self.human(house)).count();
        if allied || alive.len() == 1 || people == 0 {
            self.over = Some(alive.clone());
            self.events.push(Event::GameOver);
        }
    }
}
