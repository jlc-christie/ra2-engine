//! Bridges that can be damaged, destroyed and repaired.
//!
//! Follows the original game (1a, 1b, 4, 5)
//! and (4).

use crate::combat::{Aim, Event};
use crate::{Land, World};

/// No bridge overlay on the cell.
pub const NONE: u8 = 0xff;

/// A low bridge's overlay type in the concrete, x-running form its rules are
/// written for, with what to add to get back.
fn low_form(kind: u8) -> Option<(u8, u8, bool)> {
    let (plain, wood) = match kind {
        0x4a..=0x65 => (kind, 0),
        0xcd..=0xe8 => (kind - 0x83, 0x83),
        _ => return None,
    };
    // Y-running: bodies 0x53..0x5b, ends 0x60..0x63, gone 0x65.
    Some(match plain {
        0x53..=0x5b => (plain - 9, wood, true),
        0x60..=0x63 => (plain - 4, wood, true),
        0x65 => (0x64, wood, true),
        0x5c..=0x5f | 0x4a..=0x52 | 0x64 => (plain, wood, false),
        _ => return None,
    })
}

fn low_back(form: u8, wood: u8, along_y: bool) -> u8 {
    let plain = if !along_y {
        form
    } else {
        match form {
            0x4a..=0x52 => form + 9,
            0x5c..=0x5f => form + 4,
            _ => 0x65,
        }
    };
    plain + wood
}

impl World {
    fn bridge_kind(&self, cell: (i32, i32)) -> u8 {
        self.map.index(cell.0, cell.1).map_or(NONE, |index| self.map.bridge_kind[index])
    }

    /// What a blast does to the bridge under its middle.
    // With bridges destroyable
    // and a `Wall` warhead, a draw from 1 to `BridgeStrength` below the damage
    // hurts the bridge on the cell under the middle of the blast: a high one
    // only if the blast is at its deck's height.
    pub(crate) fn blast_bridge(&mut self, at: (i32, i32, i32), damage: i32, wall: bool) {
        if !self.bridges_destroyable || !wall {
            return;
        }
        let cell = (at.0.div_euclid(crate::LEPTONS_PER_CELL), at.1.div_euclid(crate::LEPTONS_PER_CELL));
        let low = low_form(self.bridge_kind(cell)).is_some();
        let high = self.map.bridged(cell.0, cell.1);
        if !low && !high {
            return;
        }
        if high {
            let h = self.map.level(cell.0, cell.1) as i32;
            if !((h - 2) * 104 + 416 < at.2 && at.2 <= (h + 1) * 104 + 416) {
                return;
            }
        }
        let roll = 1 + self.random_below(self.bridge_strength.max(1) as u32) as i32;
        if roll >= damage {
            return;
        }
        if high {
            if let Some(owner) = self.high_owner(cell) {
                self.damage_high_bridge(owner);
            }
        } else {
            self.damage_low_bridge(cell);
        }
    }

    /// One sure hit on the bridge over or on a cell, for tests. Returns whether
    /// a section went.
    #[doc(hidden)]
    pub fn hit_bridge_for_tests(&mut self, cell: (i32, i32)) -> bool {
        match self.high_owner(cell) {
            Some(owner) => self.damage_high_bridge(owner),
            None => self.damage_low_bridge(cell),
        }
    }

    // ------------------------------------------------------------- low bridges

    /// The three cells across a low bridge section, from any one of them.
    // The middle is found from how many
    // bridge cells lie on one side.
    fn low_section(&self, cell: (i32, i32), along_y: bool) -> Vec<(i32, i32)> {
        let step = if along_y { (1, 0) } else { (0, 1) };
        let is_low = |c: (i32, i32)| low_form(self.bridge_kind(c)).is_some();
        let back = |n: i32| (cell.0 - step.0 * n, cell.1 - step.1 * n);
        let middle = if !is_low(back(1)) {
            (cell.0 + step.0, cell.1 + step.1)
        } else if is_low(back(2)) {
            back(1)
        } else {
            cell
        };
        [-1, 0, 1].iter().map(|n| (middle.0 + step.0 * n, middle.1 + step.1 * n)).filter(|c| is_low(*c)).collect()
    }

    fn set_low_section(&mut self, cells: &[(i32, i32)], kind: u8) {
        let gone = matches!(low_form(kind), Some((0x64, ..)));
        for &cell in cells {
            if let Some(index) = self.map.index(cell.0, cell.1) {
                self.map.bridge_kind[index] = kind;
                // A section is road; a gap is what the
                // ground was.
                self.map.land[index] = if gone { self.map.base_land[index] } else { Land::Road as u8 };
            }
        }
        self.map.bridge_version += 1;
        if gone {
            // What can no longer stand there dies.
            for &cell in cells {
                let doomed: Vec<u32> = self
                    .units
                    .iter()
                    .filter(|u| u.cell() == cell && !u.on_bridge && self.altitude(u) == 0 && self.map.speed_percent(cell.0, cell.1, u.stats.speed_type) == 0)
                    .map(|u| u.id)
                    .collect();
                for id in doomed {
                    self.kill(Aim::Unit(id));
                }
            }
        }
    }

    /// One successful hit on a low bridge section.
    // Whole, it becomes
    // damaged; damaged or a broken edge, it is gone; an end piece is damaged
    // and never goes; the sections either side are made to match.
    pub(crate) fn damage_low_bridge(&mut self, cell: (i32, i32)) -> bool {
        let Some((form, wood, along_y)) = low_form(self.bridge_kind(cell)) else {
            return false;
        };
        let cells = self.low_section(cell, along_y);
        let Some(&middle) = cells.get(cells.len() / 2) else {
            return false;
        };
        let along = if along_y { (0, 1) } else { (1, 0) };
        let (next, prev) = ((middle.0 + along.0, middle.1 + along.1), (middle.0 - along.0, middle.1 - along.1));
        let mut destroyed = false;
        match form {
            0x4a..=0x4f => {
                self.set_low_section(&cells, low_back(0x50, wood, along_y));
                self.fix_low_bridge(prev);
                self.fix_low_bridge(next);
            }
            0x50..=0x52 => {
                self.set_low_section(&cells, low_back(0x64, wood, along_y));
                self.fix_low_bridge(prev);
                self.fix_low_bridge(next);
                destroyed = true;
            }
            0x5c => {
                self.set_low_section(&cells, low_back(0x5d, wood, along_y));
                self.fix_low_bridge(prev);
            }
            0x5e => {
                self.set_low_section(&cells, low_back(0x5f, wood, along_y));
                self.fix_low_bridge(next);
            }
            _ => return false,
        }
        if destroyed {
            self.map.build_zones();
            self.events.push(Event::BridgeBroken { cell: middle });
        }
        destroyed
    }

    /// Makes a low section agree with the sections either side of it.
    // By which neighbour is damaged or
    // gone; one between two gaps goes too.
    fn fix_low_bridge(&mut self, cell: (i32, i32)) {
        let Some((form, wood, along_y)) = low_form(self.bridge_kind(cell)) else {
            return;
        };
        let along = if along_y { (0, 1) } else { (1, 0) };
        let form_at = |world: &World, c: (i32, i32)| low_form(world.bridge_kind(c)).map(|(form, ..)| form);
        let mut code = 0;
        match form_at(self, (cell.0 + along.0, cell.1 + along.1)) {
            Some(0x4e | 0x50 | 0x52 | 0x5d) => code |= 1,
            Some(0x51 | 0x64) => code |= 2,
            _ => {}
        }
        match form_at(self, (cell.0 - along.0, cell.1 - along.1)) {
            Some(0x4f | 0x50 | 0x51 | 0x5f) => code |= 4,
            Some(0x52 | 0x64) => code |= 8,
            _ => {}
        }
        if code == 0 {
            return;
        }
        let new = match form {
            0x5c => 0x5d,
            0x5e => 0x5f,
            0x4a..=0x52 => match code {
                1 => 0x4f,
                2 | 6 => 0x52,
                4 => 0x4e,
                5 => 0x50,
                8 | 9 => 0x51,
                10 => 0x64,
                _ => return,
            },
            _ => return,
        };
        if new == form {
            return;
        }
        let cells = self.low_section(cell, along_y);
        self.set_low_section(&cells, low_back(new, wood, along_y));
    }

    // ------------------------------------------------------------ high bridges

    /// The owner cell (the one with the overlay) of the bridge section over a cell.
    fn high_owner(&self, cell: (i32, i32)) -> Option<(i32, i32)> {
        for (dx, dy) in [(0, 0), (0, 1), (0, -1), (0, 2), (1, 0), (-1, 0), (2, 0)] {
            let at = (cell.0 + dx, cell.1 + dy);
            let across = match self.bridge_kind(at) {
                24 | 237 => (0, -1),
                25 | 238 => (-1, 0),
                _ => continue,
            };
            // The section's cells are the owner's and those one before and two
            // after it across the bridge.
            if (-1..=2).any(|step| (at.0 + across.0 * step, at.1 + across.1 * step) == cell) {
                return Some(at);
            }
        }
        None
    }

    fn high_data(&self, owner: (i32, i32)) -> Option<u8> {
        let index = self.map.index(owner.0, owner.1)?;
        matches!(self.map.bridge_kind[index], 24 | 25 | 237 | 238).then(|| self.map.bridge_data[index])
    }

    fn set_high_data(&mut self, owner: (i32, i32), data: u8) {
        if let Some(index) = self.map.index(owner.0, owner.1) {
            self.map.bridge_data[index] = data;
            self.map.bridge_version += 1;
        }
    }

    /// One successful hit on a high bridge section.
    // Whole, it
    // becomes damaged and its neighbours show it; damaged or a broken edge, it
    // falls.
    pub(crate) fn damage_high_bridge(&mut self, owner: (i32, i32)) -> bool {
        let Some(data) = self.high_data(owner) else {
            return false;
        };
        let y = data >= 9;
        let along = if y { (0, 1) } else { (1, 0) };
        let (next, prev) = ((owner.0 + along.0, owner.1 + along.1), (owner.0 - along.0, owner.1 - along.1));
        let base = if y { 9 } else { 0 };
        match data - base {
            0..=5 => {
                self.set_high_data(owner, base + 6);
                // The neighbour after: 0..3 -> 4, 5 -> 6; before: 0..3 -> 5, 4 -> 6.
                for (cell, crack, other) in [(next, 4, 5), (prev, 5, 4)] {
                    if let Some(theirs) = self.high_data(cell).filter(|d| (*d >= 9) == y) {
                        let new = match theirs - base {
                            0..=3 => Some(crack),
                            d if d == other => Some(6),
                            _ => None,
                        };
                        if let Some(new) = new {
                            self.set_high_data(cell, base + new);
                        }
                    }
                }
                false
            }
            // 6: gaps open on both sides; 7 has one on its near side already, 8
            // on its far side.
            6 => {
                self.fall_high_bridge(owner, true, true);
                true
            }
            7 => {
                self.fall_high_bridge(owner, true, false);
                true
            }
            _ => {
                self.fall_high_bridge(owner, false, true);
                true
            }
        }
    }

    /// A high bridge section falls, and the sections beside it become edges (or
    /// fall in their turn if they were edges already).
    fn fall_high_bridge(&mut self, owner: (i32, i32), fix_next: bool, fix_prev: bool) {
        let Some(data) = self.high_data(owner) else {
            return;
        };
        let y = data >= 9;
        let base = if y { 9 } else { 0 };
        let along = if y { (0, 1) } else { (1, 0) };
        let across = if y { (-1, 0) } else { (0, -1) };
        if let Some(index) = self.map.index(owner.0, owner.1) {
            self.map.bridge_kind[index] = NONE;
            self.map.bridge_data[index] = 0;
        }
        // What is under
        // dies at once; what is on the deck drops and dies where it lands.
        for step in -1..=2 {
            let cell = (owner.0 + across.0 * step, owner.1 + across.1 * step);
            self.map.set_bridge(cell.0, cell.1, 0);
            let here: Vec<(u32, bool)> = self.units.iter().filter(|u| u.cell() == cell && self.altitude(u) == 0).map(|u| (u.id, u.on_bridge)).collect();
            for (id, on_deck) in here {
                if !on_deck {
                    self.kill(Aim::Unit(id));
                } else if let Some(unit) = self.unit_at(id).map(|at| &mut self.units[at]) {
                    unit.on_bridge = false;
                    unit.falling = Some(4 * crate::combat::LEVEL_HEIGHT);
                    unit.doomed = true;
                    unit.dropped_at = self.tick;
                    unit.dest = None;
                    unit.drop_path();
                }
            }
        }
        self.map.bridge_version += 1;
        let (next, prev) = ((owner.0 + along.0, owner.1 + along.1), (owner.0 - along.0, owner.1 - along.1));
        // After: <= 6 -> 7, an 8 falls too. Before: <= 6 -> 8, a 7 falls too.
        for (cell, wanted, edge, falls, forward) in [(next, fix_next, 7, 8, true), (prev, fix_prev, 8, 7, false)] {
            if !wanted {
                continue;
            }
            if let Some(theirs) = self.high_data(cell).filter(|d| (*d >= 9) == y) {
                let state = theirs - base;
                if state == falls {
                    self.fall_high_bridge(cell, forward, !forward);
                } else if state <= 6 {
                    self.set_high_data(cell, base + edge);
                }
            }
        }
        self.map.build_zones();
        self.events.push(Event::BridgeBroken { cell: owner });
    }

    // ------------------------------------------------------------------ repair

    /// The cells of the bridge by a repair hut, as the map first had it.
    fn bridge_by_hut(&self, hut: (i32, i32)) -> Vec<(i32, i32)> {
        let home = |c: (i32, i32)| self.map.index(c.0, c.1).is_some_and(|index| self.map.bridge_home[index] != NONE);
        let mut start = None;
        'find: for r in 0..=4i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) == r && home((hut.0 + dx, hut.1 + dy)) {
                        start = Some((hut.0 + dx, hut.1 + dy));
                        break 'find;
                    }
                }
            }
        }
        let Some(start) = start else {
            return Vec::new();
        };
        let mut cells = vec![start];
        let mut at = 0;
        while at < cells.len() {
            let (x, y) = cells[at];
            at += 1;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let next = (x + dx, y + dy);
                    if home(next) && !cells.contains(&next) {
                        cells.push(next);
                    }
                }
            }
        }
        cells
    }

    /// Whether the bridge by a repair hut has a gap.
    // Only a destroyed section calls
    // for repair, not a damaged one.
    pub fn bridge_needs_repair(&self, hut: (i32, i32)) -> bool {
        self.bridge_by_hut(hut).into_iter().any(|cell| {
            let kind = self.bridge_kind(cell);
            kind == NONE || matches!(low_form(kind), Some((0x64, ..)))
        })
    }

    /// An engineer in the hut puts the whole bridge back.
    // Every section of a low bridge, damaged or gone, becomes whole (one of
    // the four intact kinds at random); a high bridge's spans are put back.
    pub(crate) fn repair_bridge(&mut self, hut: (i32, i32)) {
        for cell in self.bridge_by_hut(hut) {
            let Some(index) = self.map.index(cell.0, cell.1) else {
                continue;
            };
            let home = self.map.bridge_home[index];
            match low_form(home) {
                Some((form, wood, along_y)) => {
                    let new = match form {
                        0x4a..=0x52 | 0x64 => 0x4a + (self.random_below(4) as u8),
                        0x5c | 0x5d => 0x5c,
                        _ => 0x5e,
                    };
                    self.map.bridge_kind[index] = low_back(new, wood, along_y);
                    self.map.land[index] = Land::Road as u8;
                }
                None => {
                    let y = matches!(home, 25 | 238);
                    self.map.bridge_kind[index] = home;
                    self.map.bridge_data[index] = if y { 9 } else { 0 };
                    let across = if y { (-1, 0) } else { (0, -1) };
                    for step in -1..=2 {
                        let at = (cell.0 + across.0 * step, cell.1 + across.1 * step);
                        let flags = if step == 2 { 1 } else { 3 };
                        let old = self.map.index(at.0, at.1).map_or(0, |i| self.map.bridge[i]);
                        self.map.set_bridge(at.0, at.1, old | flags);
                    }
                }
            }
        }
        // A low bridge's three cells share their section's kind.
        self.map.bridge_version += 1;
        self.map.build_zones();
    }
}
