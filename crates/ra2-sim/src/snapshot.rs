//! The whole state of a game as bytes, and back: for saving, and for telling
//! whether two games are in step.
//!
//! Not original behaviour: the format is ours, not the engine's. It is
//! `bincode` over the state types' `serde` derives, in named sections, so that
//! the first section to differ says where two games parted.
//!
//! Every field of [`World`] and [`Map`] is one of three things:
//!
//! - State: play changes it. In the snapshot.
//! - Rules: set once at setup from the game's files and the match's options,
//!   and the same for every game set up alike. Not in the snapshot; `restore`
//!   is called on a world that has just been set up, where they are right.
//! - Derived: worked out from state alone. Not in the snapshot; rebuilt by
//!   `restore`. There are none yet: see below.
//!
//! | `World`   | fields |
//! |-----------|--------|
//! | State     | `random` `tick` `next_id` `over`; `houses`; `units` `landing_orders` `unpacked`; `buildings`; `claims` `spots` `spot_house`; `bullets` `events` `dying` `bombs` `pending_bites` `leaps` `waves` `rad_sites` `storm` `nukes` `animated`; `teams`; `ore_queues` `ore_ready`; `trigger_scores`; `map` (below) |
//! | Rules     | `end_checks` `short_game` `supers_allowed` `bridges_destroyable` `bridge_strength` `tracks` `blockage_delay` `path_delay` `close_enough` `building_types` `unit_types` `queue_limit` `build_rules` `mcv_repacks` `buildup_ticks` `chrono` `refund_percent` `repair_step` `repair_percent` `repair_interval` `survivor_divisors` `crew_types` `weapons` `warheads` `projectiles` `gravity` `max_damage` `missile_rot_var` `targeting_delay` `condition_red` `c4_delay` `ai_rules` `team_data` `rockets` `super_types` `super_rules` `veterancy` `rad_rules` `ability_rules` `reload_delay` `ore_values` `ore_near_scan` `ore_far_scan` `harvester_load_rate` `harvester_dump_rate` `harvester_too_far` `chrono_too_far` `ore_types` |
//! | Derived   | none |
//!
//! | `Map`     | fields |
//! |-----------|--------|
//! | State     | `passable` `land` `overlay` `obstacle` `tree_bits` `exempt`; `bridge` `bridge_kind` `bridge_data` `bridge_version`; `ore_kind` `ore_picture` `ore_amount` `ore_under` `ore_version`; `smudge` `smudge_part` `smudge_version`; `wall_kind` `wall_data` `wall_owner` `wall_cells` `walls_version`; `revealed` `satellite` `gaps` `shroud_on` `shroud_version`; `zones` `bridge_end` |
//! | Rules     | `width` `height` `level` `ramp` `bridge_home` `base_land` `ore_ground` `smudge_types` `playfield` `lands` |
//! | Derived   | none |
//!
//! Some of these are not what they look:
//!
//! - The ore queues are state, not a cache: each entry's due time took a number
//!   from the random stream when it went in, and the two pass timers run on.
//!   Building them again would draw the numbers afresh.
//! - The zones (and with them `bridge_end` and `exempt`) are worked out from the
//!   map, but not every time it changes: a building the player places or loses
//!   leaves them as they were until something else has them built again, and
//!   `exempt` only ever gains cells. A game goes on with the zones it has, so
//!   they are saved as they stand; building them afresh on restore sent units
//!   another way within fifty ticks.
//! The lists below are the classification the code uses: each names every
//! field of its struct, so a new field does not compile until it is put in one.
//!

use crate::{Map, World};

/// Goes up whenever a state type or the layout changes; a snapshot of another
/// number is refused.
pub const FORMAT: u32 = 4;

type Sections = Vec<(&'static str, Vec<u8>)>;

/// Writes a struct's state fields out in sections, and reads them back. The
/// patterns name every field, so one in neither list is an error.
macro_rules! classify {
    ($kind:ident, $write:ident, $read:ident,
     state { $( $section:literal: $( $state:ident )* ; )* }
     other { $( $other:ident )* }) => {
        fn $write(from: &$kind, out: &mut Sections) {
            let $kind { $( $( $state, )* )* $( $other: _, )* } = from;
            $(
                let mut bytes = Vec::new();
                $( bincode::serialize_into(&mut bytes, $state).expect(concat!("writing ", stringify!($state))); )*
                out.push(($section, bytes));
            )*
        }

        fn $read<'a>(to: &mut $kind, sections: &mut impl Iterator<Item = &'a [u8]>) -> Result<(), String> {
            let $kind { $( $( $state, )* )* $( $other: _, )* } = to;
            $(
                let mut bytes = sections.next().ok_or(concat!("no section ", $section))?;
                $( *$state = bincode::deserialize_from(&mut bytes).map_err(|error| format!(concat!($section, ": ", stringify!($state), ": {}"), error))?; )*
                if !bytes.is_empty() {
                    return Err(format!(concat!($section, ": {} bytes left over"), bytes.len()));
                }
            )*
            Ok(())
        }
    };
}

classify! { World, write_world, read_world,
    state {
        "clock": random tick next_id over;
        "houses": houses;
        "units": units landing_orders unpacked;
        "buildings": buildings;
        "cells": claims spots spot_house;
        "transient": bullets events dying bombs pending_bites leaps waves rad_sites storm nukes animated;
        "teams": teams trigger_scores;
        "crates": crates crates_on;
        "ore queues": ore_queues ore_ready;
    }
    other {
        // State, by the table below.
        map
        // Rules.
        end_checks short_game supers_allowed bridges_destroyable bridge_strength tracks
        blockage_delay path_delay close_enough sight_height_step building_types unit_types queue_limit
        build_rules mcv_repacks buildup_ticks chrono refund_percent repair_step
        repair_percent repair_interval survivor_divisors crew_types weapons warheads
        projectiles gravity ballistic_scatter elevation max_damage missile_rot_var targeting_delay condition_red
        c4_delay ai_rules team_data rockets super_types super_rules crate_rules veterancy rad_rules
        ability_rules reload_delay ore_values ore_near_scan ore_far_scan
        harvester_load_rate harvester_dump_rate harvester_too_far chrono_too_far ore_types
        anim_marks splash_anims
    }
}

classify! { Map, write_map, read_map,
    state {
        "map ground": passable land overlay obstacle tree_bits exempt;
        "map bridges": bridge bridge_kind bridge_data bridge_version;
        "map ore": ore_kind ore_picture ore_amount ore_under ore_version;
        "map smudges": smudge smudge_part smudge_version;
        "map walls": wall_kind wall_data wall_owner wall_cells walls_version;
        "map shroud": revealed satellite gaps shroud_on shroud_version;
        "map zones": zones bridge_end;
    }
    other {
        // Rules.
        width height level ramp bridge_home base_land ore_ground smudge_types playfield size_height lands
        slope_speed condition_yellow
        // Local: whose shroud this machine shows.
        viewer
    }
}

// FNV-1a.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl World {
    fn sections(&self) -> Sections {
        let mut sections = Vec::new();
        write_world(self, &mut sections);
        write_map(&self.map, &mut sections);
        sections
    }

    /// Everything play has changed, as bytes: the format number and the map's
    /// size, then each section with its length before it.
    pub fn snapshot(&self) -> Vec<u8> {
        let sections = self.sections();
        let mut bytes = Vec::with_capacity(12 + sections.iter().map(|(_, section)| section.len() + 8).sum::<usize>());
        bytes.extend(FORMAT.to_le_bytes());
        bytes.extend(self.map.width.to_le_bytes());
        bytes.extend(self.map.height.to_le_bytes());
        for (_, section) in &sections {
            bytes.extend((section.len() as u64).to_le_bytes());
            bytes.extend(section);
        }
        bytes
    }

    /// Puts a snapshot's state over this world's, which must have just been set
    /// up for the same map with the same rules and options. After an error the
    /// world is part one game and part another, and fit only to be set up again.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        let number = |at: usize| bytes.get(at..at + 4).map(|four| i32::from_le_bytes([four[0], four[1], four[2], four[3]])).ok_or("too short");
        let format = number(0)? as u32;
        if format != FORMAT {
            return Err(format!("format {format}, not {FORMAT}"));
        }
        let size = (number(4)?, number(8)?);
        if size != (self.map.width, self.map.height) {
            return Err(format!("for a map of {size:?}, not {:?}", (self.map.width, self.map.height)));
        }
        let mut rest = &bytes[12..];
        let mut sections = Vec::new();
        while !rest.is_empty() {
            let length = rest.get(..8).map(|eight| u64::from_le_bytes(eight.try_into().unwrap_or_default())).ok_or("cut short")? as usize;
            sections.push(rest.get(8..8 + length).ok_or("cut short")?);
            rest = &rest[8 + length..];
        }
        let mut sections = sections.into_iter();
        read_world(self, &mut sections)?;
        read_map(&mut self.map, &mut sections)?;
        if sections.next().is_some() {
            return Err("sections left over".into());
        }
        // Older saves: mark standing buildings' cells as the game now does,
        // and mend zones a lost building left solid.
        for index in 0..self.buildings.len() {
            let (cell, size) = (self.buildings[index].cell, self.buildings[index].size);
            for dy in 0..size.1 {
                for dx in 0..size.0 {
                    let at = (cell.0 + dx, cell.1 + dy);
                    if self.map.obstacle_kind(at) == crate::Obstacle::None as u8 {
                        self.map.set_obstacle(at.0, at.1, crate::Obstacle::Building);
                    }
                }
            }
        }
        self.map.mend_zones();
        let cells = self.map.level.len();
        let Map { passable, land, overlay, obstacle, tree_bits, exempt, bridge, bridge_end, bridge_kind, bridge_data, ore_kind, ore_picture, ore_amount, ore_under, smudge, smudge_part, wall_kind, wall_data, wall_owner, revealed, .. } = &self.map;
        let layers = [passable.len(), land.len(), overlay.len(), obstacle.len(), tree_bits.len(), exempt.len(), bridge.len(), bridge_end.len(), bridge_kind.len(), bridge_data.len(), ore_kind.len(), ore_picture.len(), ore_amount.len(), ore_under.len(), smudge.len(), smudge_part.len(), wall_kind.len(), wall_data.len(), wall_owner.len(), revealed.len(), self.spots.len(), self.spot_house.len(), self.claims.len() / 2];
        if layers.iter().any(|&length| length != cells) {
            return Err("a layer of the wrong size".into());
        }
        Ok(())
    }

    /// A digest of the whole state: two games with the same one are the same game.
    pub fn state_hash(&self) -> u64 {
        fnv(&self.snapshot())
    }

    /// A digest of each section of the state, for finding where two games differ.
    pub fn state_hashes(&self) -> Vec<(&'static str, u64)> {
        self.sections().into_iter().map(|(name, bytes)| (name, fnv(&bytes))).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Map, Order, UnitStats, World};

    fn field(size: i32) -> World {
        let mut map = Map::new(size, size);
        for y in 0..size {
            for x in 0..size {
                map.set_cell(x, y, true, 0);
            }
        }
        World::new(map, None)
    }

    #[test]
    fn a_world_restored_is_the_world_saved() {
        let mut world = field(12);
        let unit = world.add_unit((1, 5), 64, UnitStats { speed: 6, ..UnitStats::default() }, 0);
        world.order(Order::Move { unit, x: 10, y: 5 });
        for _ in 0..40 {
            world.tick();
        }
        let saved = world.snapshot();
        let mut other = field(12);
        assert_ne!(other.state_hash(), world.state_hash());
        other.restore(&saved).unwrap();
        assert!(other.snapshot() == saved);
        // And goes on as it would have.
        for _ in 0..400 {
            world.tick();
            other.tick();
            assert_eq!(other.state_hashes(), world.state_hashes());
        }
        assert_eq!(other.unit(unit).unwrap().cell(), (10, 5));
        // Not onto a map of another size, nor from another format, nor cut short.
        assert!(field(13).restore(&saved).is_err());
        let mut newer = saved.clone();
        newer[0] += 1;
        assert!(field(12).restore(&newer).is_err());
        assert!(field(12).restore(&saved[..saved.len() - 1]).is_err());
    }

    #[test]
    fn the_hash_tells_one_change_and_where() {
        let changed = |a: &[(&'static str, u64)], b: &[(&'static str, u64)]| -> Vec<&'static str> { a.iter().zip(b).filter(|(a, b)| a != b).map(|(a, _)| a.0).collect() };
        let mut world = field(8);
        world.add_unit((2, 2), 0, UnitStats::default(), 0);
        let before = (world.state_hash(), world.state_hashes());
        world.units[0].health -= 1;
        let after = world.state_hashes();
        assert_ne!(world.state_hash(), before.0);
        assert_eq!(changed(&before.1, &after), ["units"]);
        world.map.put_ore(3, 3, 0, 4);
        assert_eq!(changed(&after, &world.state_hashes()), ["map ground", "map ore"]);
    }
}
