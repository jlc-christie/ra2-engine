//! Foot soldiers' fear, lying down under fire, and fidgeting at rest.

use crate::combat::Stance;
use crate::{Scatter, World};

/// Fear at or above which a soldier lies down.
const PRONE_FEAR: u16 = 50;
/// The most fear a soldier can hold.
const MAX_FEAR: u16 = 300;

impl World {
    /// A soldier that was hurt and lived takes fright.
    // Hit by someone with fear under
    // 100: a `Fraidycat` goes to 300, any other that is not fearless to 100.
    // Otherwise (no attacker, or already that afraid) 50 more, 25 above
    // `ConditionRed`, halved above `ConditionYellow`, up to 300.
    pub(crate) fn frighten(&mut self, id: u32, attacked: bool) {
        let (red, yellow) = (self.condition_red, self.map.condition_yellow);
        let Some(unit) = self.units.iter_mut().find(|u| u.id == id && u.stats.infantry && u.health > 0) else {
            return;
        };
        let fighter = unit.stats.fighter;
        let fearless = fighter.fearless || unit.has_ability(crate::abilities::ability::FEARLESS);
        if attacked && unit.fear < 100 {
            if fighter.fraidycat {
                unit.fear = MAX_FEAR;
                return;
            }
            if !fearless {
                unit.fear = 100;
                return;
            }
        }
        if fighter.fearless {
            return;
        }
        let share = unit.health as f64 / unit.stats.strength.max(1) as f64;
        let mut add = 50;
        if share > red {
            add = 25;
        }
        if share > yellow {
            add /= 2;
        }
        unit.fear = (unit.fear + add).min(MAX_FEAR);
    }

    /// Each tick: fear wears off; a frightened soldier lies down and gets up
    /// again once it has calmed; a frightened `Fraidycat` runs; one at rest
    /// fidgets now and then.
    // A person's soldier lies down only
    // when it has nowhere to go and is not moving; a `Fraidycat` never does, nor a
    // soldier dug in, nor one without a `Down` sequence (nor gets up without an
    // `Up`). A fidget comes when the idle timer has run out: the timer
    // is set again to between 450 and 1800 times `IdleActionFrequency` frames,
    // then a number from 0 to 10 says what to do: 1, 2 or 7 `Idle2`; 3, 4 or 5
    // `Idle1`; 6, 8, 9 or 10 a turn to one of the eight directions; 0 nothing.
    pub(crate) fn run_soldiers(&mut self) {
        let tick = self.tick;
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            if !unit.stats.infantry || unit.health <= 0 {
                continue;
            }
            let human = self.human(unit.owner);
            let fighter = unit.stats.fighter;
            let unit = &mut self.units[index];
            if unit.fear > 0 && !fighter.fearless {
                unit.fear -= 1;
            }
            let dug_in = unit.stance != Stance::Standing;
            if dug_in && unit.prone {
                unit.prone = false;
                unit.prone_at = tick;
            }
            let moving = unit.is_moving();
            if !unit.prone && unit.fear >= PRONE_FEAR && !dug_in && !fighter.fraidycat && fighter.down_frames > 0 && (!human || (unit.dest.is_none() && !moving)) {
                unit.prone = true;
                unit.prone_at = tick;
                unit.fidget = None;
            } else if unit.prone && unit.fear < PRONE_FEAR && !dug_in && fighter.up_frames > 0 {
                unit.prone = false;
                unit.prone_at = tick;
            }
            let idle = unit.arms.target.is_none() && unit.dest.is_none() && !moving && unit.falling.is_none() && unit.jet.is_none() && !unit.tethered;
            if moving || unit.arms.firing > 0 || unit.prone || dug_in {
                unit.fidget = None;
            }
            if fighter.fraidycat && unit.fear > PRONE_FEAR && idle {
                let id = unit.id;
                self.scatter_unit_from(id, Scatter::Nearest);
                continue;
            }
            let unit = &self.units[index];
            if !idle || unit.prone || dug_in || unit.arms.firing > 0 || tick < unit.fidget_at || self.altitude(unit) > 0 {
                continue;
            }
            let armed = unit.fidget_at != 0;
            let f = self.idle_frequency;
            let share = (crate::random_step(&mut self.idle_random) % 0x7fff_ffff) as f64 / 2_147_483_648.0;
            let wait = (f * 450.0 + (f * 1800.0 - f * 450.0) * share) as u64;
            // The first time it is only set.
            if !armed {
                self.units[index].fidget_at = tick + wait;
                continue;
            }
            let roll = crate::random_step(&mut self.idle_random) % 11;
            let turn = matches!(roll, 6 | 8 | 9 | 10).then(|| crate::random_step(&mut self.idle_random) % 8);
            let unit = &mut self.units[index];
            unit.fidget_at = tick + wait;
            match roll {
                1 | 2 | 7 => unit.fidget = Some((2, tick)),
                3..=5 => unit.fidget = Some((1, tick)),
                _ => {}
            }
            if let Some(direction) = turn {
                unit.facing = (direction as u8) << 5;
                unit.turn = crate::Turn::facing(unit.facing);
            }
        }
    }
}
