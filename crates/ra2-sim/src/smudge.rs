//! Scorch marks and craters: two values in a cell and nothing more.
//!
//! Follows the original game
//!
//! Not the original's structure: the engine makes a mark at the largest frame of
//! an animation, which is part of its simulation; animations here belong to the
//! view, which asks for the mark as it starts one.

use crate::{Land, Map, World, LEPTONS_PER_CELL};

/// A `[SmudgeTypes]` entry.
#[derive(Clone, Copy, Debug, Default)]
pub struct SmudgeType {
    pub crater: bool,
    pub burn: bool,
    pub width: i32,
    pub height: i32,
}

impl Map {
    /// The marks on the map, each by the cell it starts at.
    pub fn smudges(&self) -> impl Iterator<Item = ((i32, i32), u16)> + '_ {
        let width = self.width;
        self.smudge
            .iter()
            .zip(&self.smudge_part)
            .enumerate()
            .filter(|(_, (kind, part))| **kind != 0 && **part == 0)
            .map(move |(index, (kind, _))| ((index as i32 % width, index as i32 / width), kind - 1))
    }

    // Every cell of the block must be on the map, flat, without a mark or
    // an overlay, and of a tile that takes marks.
    fn takes_smudge(&self, cell: (i32, i32), kind: &SmudgeType) -> bool {
        (0..kind.height.max(1)).all(|row| {
            (0..kind.width.max(1)).all(|col| {
                self.index(cell.0 + col, cell.1 + row).is_some_and(|index| {
                    self.ramp[index] == 0
                        && self.smudge[index] == 0
                        && !self.overlay[index]
                        && self.ore_kind[index] == 0
                        && [Land::Clear as u8, Land::Road as u8, Land::Rough as u8].contains(&self.land[index])
                })
            })
        })
    }
}

/// No animation.
pub const NO_ANIM: u16 = u16::MAX;

/// What an animation type leaves on the ground: its `Scorch`, `Crater` and
/// `ForceBigCraters`, and the size of its picture.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnimMark {
    pub scorch: bool,
    pub crater: bool,
    pub force_big: bool,
    pub size: (i32, i32),
}

impl World {
    /// Leaves a scorch mark (`burn`) or a crater by a point, as an animation `w`
    /// by `h` pixels at its largest does; `big` asks for a two by two mark.
    // Of the types of the kind that fit,
    // those one cell in size (or any, for a large animation), or with `big` those
    // more than a cell each way; one at random of those, or failing them of all
    // that fit. One random number, and none when nothing fits.
    pub fn smudge(&mut self, burn: bool, at: (i32, i32), w: i32, h: i32, big: bool) {
        let cell = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
        if cell == (0, 0) {
            return;
        }
        let fits: Vec<usize> = (0..self.map.smudge_types.len())
            .filter(|&index| {
                let kind = &self.map.smudge_types[index];
                (if burn { kind.burn } else { kind.crater }) && self.map.takes_smudge(cell, kind) && !self.smudge_blocked(cell, kind, big)
            })
            .collect();
        let wanted: Vec<usize> = fits
            .iter()
            .copied()
            .filter(|&index| {
                let kind = &self.map.smudge_types[index];
                if big {
                    kind.width > 1 && kind.height > 1
                } else {
                    (kind.width == 1 && kind.height == 1) || (w > 60 && h > 50)
                }
            })
            .collect();
        let from = if wanted.is_empty() { &fits } else { &wanted };
        if from.is_empty() {
            return;
        }
        let index = from[self.random_below(from.len() as u32) as usize];
        // Each cell is given the type and which
        // part of the mark it is.
        let kind = self.map.smudge_types[index];
        for row in 0..kind.height.max(1) {
            for col in 0..kind.width.max(1) {
                if let Some(at) = self.map.index(cell.0 + col, cell.1 + row) {
                    self.map.smudge[at] = index as u16 + 1;
                    self.map.smudge_part[at] = (row * kind.width + col) as u8;
                }
            }
        }
        self.map.smudge_version += 1;
    }

    /// A building in the way, which only a mark asked for by a dying building
    /// (`big`) overlooks.
    fn smudge_blocked(&self, cell: (i32, i32), kind: &SmudgeType, big: bool) -> bool {
        !big && (0..kind.height.max(1)).any(|row| (0..kind.width.max(1)).any(|col| self.building_on((cell.0 + col, cell.1 + row)).is_some()))
    }

    /// The explosion a warhead's blast shows, by the damage it carried.
    // The `AnimList` entry for
    // the damage, 25 points an entry; on water a `Conventional` warhead takes
    // from the splash list instead, 35 an entry.
    fn blast_anim(&self, warhead: u16, damage: i32, water: bool) -> u16 {
        let rules = self.warheads.get(warhead as usize);
        let storm = &self.super_rules.storm;
        let (list, step) = match rules {
            Some(rules) if water && rules.conventional => (&self.splash_anims, 35),
            // The storm's warhead shows `WeatherConBoltExplosion`.
            Some(_) if warhead == storm.warhead && damage != 0 && storm.bolt_explosion.is_some() => return storm.bolt_explosion.unwrap_or(NO_ANIM),
            Some(rules) => (&rules.anim_list, 25),
            None => return NO_ANIM,
        };
        let count = list.len() as i32;
        if count == 0 || damage == 0 {
            return NO_ANIM;
        }
        list[(damage.min(count * step - 1) / step).max(0) as usize]
    }

    /// Whether a point is low enough for an animation there to mark the ground.
    // Lower than 30 above the cell's level.
    fn marks_ground(&self, at: (i32, i32, i32)) -> bool {
        at.2 - self.map.level(at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL)) as i32 * crate::combat::LEVEL_HEIGHT < 30
    }

    /// The mark of an animation type, by its number.
    pub(crate) fn mark_of(&mut self, anim: u16, at: (i32, i32)) {
        if let Some(mark) = self.anim_marks.get(anim as usize).copied() {
            self.animation_mark(at, mark.scorch, mark.crater, mark.force_big, mark.size.0, mark.size.1);
        }
    }

    /// A blast is seen and heard: its explosion is chosen and marks the ground.
    pub(crate) fn blast_event(&mut self, warhead: u16, damage: i32, at: (i32, i32, i32), water: bool) {
        let anim = self.blast_anim(warhead, damage, water);
        if anim != NO_ANIM && !water && self.marks_ground(at) {
            self.mark_of(anim, (at.0, at.1));
        }
        self.events.push(crate::combat::Event::Detonated { warhead, damage, at, water, anim });
    }

    /// One of a dying thing's `Explosion` animations, which marks the ground.
    pub(crate) fn death_anim(&mut self, list: &[u16], at: (i32, i32, i32)) -> u16 {
        if list.is_empty() {
            return NO_ANIM;
        }
        let anim = list[self.random_below(list.len() as u32) as usize];
        if anim != NO_ANIM && self.marks_ground(at) {
            self.mark_of(anim, (at.0, at.1));
        }
        anim
    }

    /// What an animation with `Scorch` or `Crater` leaves where it plays.
    // With both, an even chance of either; a crater
    // also takes six bales of ore off the cell; `ForceBigCraters` makes a large
    // crater whatever the animation's size.
    // Public for the tests; in play the sim calls it itself.
    pub fn animation_mark(&mut self, at: (i32, i32), scorch: bool, crater: bool, force_big: bool, w: i32, h: i32) {
        let burn = match (scorch, crater) {
            (true, true) => self.random_below(100) < 50,
            (true, false) => true,
            (false, true) => false,
            (false, false) => return,
        };
        if force_big {
            self.smudge(false, at, 300, 300, true);
        } else {
            self.smudge(burn, at, w, h, false);
        }
        if !burn || force_big {
            self.map.reduce_ore(at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL), 6);
        }
    }

    /// The one mark a building of two cells or more each way leaves as it dies.
    // Two random numbers for wide and deep
    // foundations whose results are unused, then an even chance of a scorch or a
    // crater at the building's own cell, asked for large.
    pub(crate) fn building_mark(&mut self, cell: (i32, i32), size: (i32, i32)) {
        if size.0 < 2 || size.1 < 2 {
            return;
        }
        if size.0 > 2 {
            self.random_below(size.0 as u32 - 1);
        }
        if size.1 > 2 {
            self.random_below(size.1 as u32 - 1);
        }
        let burn = self.random_below(100) < 50;
        self.smudge(burn, (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128), 100, 100, true);
    }
}
