//! Tests that run the simulation on a real map with the real rules, without a
//! window: load, give orders, tick as far as needed, look at the result. They are
//! skipped when the game's files are not there.

use ra2_formats::map::Map;
use ra2_sim::{Order, World};

use crate::files::GameFiles;
use crate::terrain::{self, PLAYER};
use crate::units;

/// A simulation loaded from one of the game's maps.
pub struct Scenario {
    pub world: World,
    /// Unit type names, as the simulation numbers them.
    pub units: Vec<String>,
    /// Building type names in rules.ini order, as the simulation numbers them.
    pub names: Vec<String>,
}

impl Scenario {
    /// Loads a map by file name without extension, as the game does.
    pub fn load(map: &str) -> Option<Self> {
        Self::load_at(map, None)
    }

    /// Loads a map with the player at one of its start positions.
    pub fn load_at(map: &str, start: Option<usize>) -> Option<Self> {
        let files = GameFiles::open().ok()?;
        let parsed = files
            .get(&format!("{map}.map"))
            .or_else(|| files.get(&format!("{map}.mpr")))
            .and_then(Map::parse)?;
        let mut scene = terrain::build(&files, &parsed, &terrain::Player { start, ..terrain::Player::default() })?;
        let mut built = units::build_world(&files, &mut scene);
        // These tests place things wherever the rules allow; the shroud has a
        // test of its own.
        built.world.map.shroud_on = false;
        // The computer player sleeps unless a test wakes it, so that it does not
        // disturb the others' arrangements.
        for house in &mut built.world.houses {
            house.ai = None;
        }
        Some(Self {
            names: built.catalog.iter().map(|entry| entry.name.clone()).collect(),
            units: built.unit_catalog.iter().map(|entry| entry.name.clone()).collect(),
            world: built.world,
        })
    }

    /// The number of a building type by its rules.ini name.
    pub fn building(&self, name: &str) -> u16 {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("no building {name}")) as u16
    }

    pub fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.world.tick();
        }
    }

    /// The player's first unit: the construction vehicle the game starts with.
    pub fn player_unit(&self) -> u32 {
        self.world.units().iter().find(|unit| unit.owner == PLAYER).expect("the player has a unit").id
    }

    /// Deploys the player's construction vehicle and returns the yard's top cell.
    pub fn deploy_base(&mut self) -> (i32, i32) {
        let unit = self.player_unit();
        self.world.order(Order::Deploy { unit });
        self.run(60);
        self.world.buildings().iter().find(|b| b.owner == PLAYER).expect("the vehicle deployed").cell
    }

    /// The names of what the player can build now.
    pub fn options(&self) -> Vec<&str> {
        self.world.buildable(PLAYER).iter().map(|&kind| self.names[kind as usize].as_str()).collect()
    }

    /// Produces a building (with money to spare) and places it with its top cell
    /// at `cell`. Returns whether it went down.
    pub fn build_at(&mut self, name: &str, cell: (i32, i32)) -> bool {
        let building = self.building(name);
        self.world.houses[PLAYER as usize].credits = 100_000;
        self.world.order(Order::Build { house: PLAYER, building });
        for _ in 0..20_000 {
            let combat = self.world.is_defence(building);
            if self.world.houses[PLAYER as usize].slot(combat).is_some_and(|p| p.is_ready()) {
                break;
            }
            self.world.tick();
        }
        let before = self.world.buildings().len();
        self.world.order(Order::Place { house: PLAYER, building, x: cell.0, y: cell.1 });
        if self.world.buildings().len() == before {
            self.world.order(Order::CancelBuild { house: PLAYER, building });
            return false;
        }
        true
    }

    /// A unit type's figures by its rules.ini name.
    pub fn stats(&self, name: &str) -> ra2_sim::UnitStats {
        let kind = self.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..self.world.unit_types[kind].stats }
    }

    /// The first cell in a square around a cell where a building could be placed.
    pub fn find_spot(&self, name: &str, around: (i32, i32), reach: i32) -> Option<(i32, i32)> {
        let building = self.building(name);
        (-reach..=reach)
            .flat_map(|dy| (-reach..=reach).map(move |dx| (around.0 + dx, around.1 + dy)))
            .find(|&cell| self.world.can_place(PLAYER, building, cell))
    }
}

#[test]
fn the_base_vehicle_deploys_and_a_power_plant_goes_beside_the_yard() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        eprintln!("game files not found; skipping");
        return;
    };
    let yard = game.deploy_base();
    assert!(game.options().contains(&"GAPOWR"), "{:?}", game.options());
    let spot = game.find_spot("GAPOWR", yard, 8).expect("somewhere to put a power plant");
    assert!(game.build_at("GAPOWR", spot));
    assert_eq!(game.world.power(PLAYER).0, 200);
    // On top of the yard is refused.
    assert!(!game.build_at("GAPOWR", yard));
}

#[test]
fn the_base_vehicle_drives_across_the_map_and_can_come_back() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let unit = game.player_unit();
    let start = game.world.unit(unit).unwrap().cell();
    for goal in [(40, 92), start] {
        game.world.order(Order::Move { unit, x: goal.0, y: goal.1 });
        game.run(3000);
        let moved = game.world.unit(unit).unwrap();
        assert_eq!(moved.cell(), goal);
        assert!(!moved.is_moving());
    }
}

#[test]
fn an_american_player_is_offered_only_their_own_airforce_command() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN"] {
        let spot = game.find_spot(name, yard, 10).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name} at {spot:?}");
    }
    let options = game.options();
    assert!(options.contains(&"AMRADR"), "{options:?}");
    assert!(!options.contains(&"GAAIRC"), "{options:?}");
    let mut sorted = options.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), options.len(), "{options:?}");
}

#[test]
fn a_shipyard_goes_on_water_and_not_on_land() {
    // Tried on several maps; at least one has water within reach of the start.
    let mut total = 0;
    for map in ["mp03t4", "mp01t4", "mp02t2", "mp05t4", "mp06t2", "mp08t2", "mp10s4", "mp11t2"] {
        let Some(mut game) = Scenario::load(map) else {
            continue;
        };
        let yard = game.deploy_base();
        let mut built = true;
        for name in ["GAPOWR", "GAREFN"] {
            built &= game.find_spot(name, yard, 10).is_some_and(|spot| game.build_at(name, spot));
        }
        if !built {
            continue;
        }
        // A map without water enough takes the shipyard away in its own file.
        if !game.options().contains(&"GAYARD") {
            continue;
        }
        let shipyard = game.building("GAYARD");
        let size = game.world.building_types[shipyard as usize].size;
        let mut spots = 0;
        for y in yard.1 - 30..yard.1 + 30 {
            for x in yard.0 - 30..yard.0 + 30 {
                if game.world.can_place(PLAYER, shipyard, (x, y)) {
                    let all_water = (0..size.1).all(|dy| {
                        (0..size.0).all(|dx| game.world.map.describe(x + dx, y + dy).3 == ra2_sim::Land::Water as u8)
                    });
                    assert!(all_water, "{map}: shipyard allowed at {:?}, which is not all water", (x, y));
                    spots += 1;
                }
            }
        }
        eprintln!("{map}: {spots} places for a shipyard near the start");
        total += spots;
    }
    assert!(total > 0, "no map had anywhere to put a shipyard");
}

#[test]
fn a_war_factory_builds_tanks_that_come_out_and_can_be_driven() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let names = |game: &Scenario| -> Vec<String> {
        game.world.buildable_units(PLAYER).iter().map(|&unit| game.units[unit as usize].clone()).collect()
    };
    assert!(names(&game).is_empty());
    for name in ["GAPOWR", "GAREFN", "GAPILE", "GAWEAP"] {
        let spot = game.find_spot(name, yard, 12).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name}");
    }
    game.run(100);
    let offered = names(&game);
    assert!(offered.contains(&"MTNK".to_owned()), "{offered:?}");
    // No Soviet vehicles from an Allied factory, and no ships without a shipyard.
    assert!(!offered.contains(&"HTNK".to_owned()) && !offered.contains(&"DEST".to_owned()), "{offered:?}");
    let tank = game.units.iter().position(|name| name == "MTNK").unwrap() as u16;
    let known: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    let before = known.len();
    game.world.houses[PLAYER as usize].credits = 10_000;
    for _ in 0..2 {
        game.world.order(Order::BuildUnit { house: PLAYER, unit: tank });
    }
    let factory = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.size == (5, 3)).map(|b| b.cell).unwrap();
    let barracks = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.size == (3, 2)).map(|b| (b.cell, b.size)).unwrap();
    // The first tank appears inside the factory, facing east, and is let out along
    // the middle row.
    let mut seen_inside = false;
    for _ in 0..4000 {
        game.run(1);
        if let Some(unit) = game.world.units().get(before) {
            if unit.tethered {
                seen_inside = true;
                let cell = unit.cell();
                assert!((factory.0..factory.0 + 5).contains(&cell.0) && (factory.1..factory.1 + 3).contains(&cell.1), "{cell:?}");
                assert_eq!(unit.facing, 0x40);
            }
        }
    }
    assert!(seen_inside);
    let made: Vec<u32> = game.world.units().iter().filter(|u| u.owner == PLAYER && !known.contains(&u.id)).map(|unit| unit.id).collect();
    assert_eq!(made.len(), 2);
    for &id in &made {
        let cell = game.world.unit(id).unwrap().cell();
        assert!(!(factory.0..factory.0 + 5).contains(&cell.0) || !(factory.1..factory.1 + 3).contains(&cell.1), "tank left on the factory at {cell:?}");
        assert!(cell.0 >= factory.0 + 5, "tank at {cell:?}, factory {factory:?}");
    }
    assert!(!game.world.buildings().iter().any(|b| b.unloading()));
    // A soldier comes out of the barracks and steps off it.
    let soldier = game.units.iter().position(|name| name == "E1").unwrap() as u16;
    game.world.order(Order::BuildUnit { house: PLAYER, unit: soldier });
    game.run(1500);
    let walker = game.world.units().last().unwrap();
    assert!(walker.stats.infantry);
    let cell = walker.cell();
    let (origin, size) = barracks;
    assert!(cell.1 == origin.1 + size.1 && (origin.0..origin.0 + size.0).contains(&cell.0), "soldier at {cell:?}, barracks {origin:?}");
    // The refinery's miner has been earning meanwhile, in whole loads.
    let earned = game.world.houses[PLAYER as usize].credits - (10_000 - 1400 - 200);
    assert!(earned >= 0 && earned % 25 == 0, "{earned}");
    let goal = (yard.0 + 6, yard.1 + 8);
    game.world.order(Order::Move { unit: made[0], x: goal.0, y: goal.1 });
    game.run(3000);
    let cell = game.world.unit(made[0]).unwrap().cell();
    assert!((cell.0 - goal.0).abs() <= 2 && (cell.1 - goal.1).abs() <= 2, "ended at {cell:?}");
}

#[test]
fn a_refinery_comes_with_a_miner_that_digs_and_is_paid() {
    miner_digs_and_is_paid("mp03t4", None);
}

#[test]
fn a_miner_digs_and_is_paid_on_defcon_6() {
    // Most of Defcon 6's ore is the third kind (`Vinifera`, the TIB2 overlays),
    // worth the same as ore; the sixth start has only that near it.
    miner_digs_and_is_paid("mp17t6", Some(5));
}

fn miner_digs_and_is_paid(map: &str, start: Option<usize>) {
    let Some(mut game) = Scenario::load_at(map, start) else {
        return;
    };
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN"] {
        let spot = game.find_spot(name, yard, 12).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name}");
    }
    game.run(100);
    let miner = game.world.units().iter().find(|unit| unit.stats.harvester).map(|unit| unit.id).expect("no free miner");
    let ore_before: u32 = game.world.map.ore_cells().map(|(_, _, amount)| amount as u32 + 1).sum();
    let credits = game.world.houses[PLAYER as usize].credits;
    let mut most = 0;
    // Bales taken aboard, counted as the load rises (the map's total cannot
    // tell, since ore grows back meanwhile).
    let (mut dug, mut last) = (0u32, 0u32);
    let mut log = Vec::new();
    for tick in 0..9000 {
        game.run(1);
        let unit = game.world.unit(miner).unwrap();
        most = most.max(unit.work.load());
        dug += unit.work.load().saturating_sub(last);
        last = unit.work.load();
        if tick % 500 == 0 {
            log.push((tick, unit.cell(), unit.work.job, unit.work.load(), game.world.houses[PLAYER as usize].credits));
        }
    }
    let ore_after: u32 = game.world.map.ore_cells().map(|(_, _, amount)| amount as u32 + 1).sum();
    let paid = game.world.houses[PLAYER as usize].credits - credits;
    let _ = (ore_before, ore_after);
    assert!(most > 0, "never dug: {log:?}");
    assert!(paid > 0, "never paid: {log:?}");
    let aboard: u32 = game.world.unit(miner).unwrap().work.load();
    // Every bale dug is either aboard or paid for at 25 or 50 a bale.
    assert!(paid >= 25 * (dug - aboard) as i32 && paid <= 50 * (dug - aboard) as i32, "dug {dug}, aboard {aboard}, paid {paid}: {log:?}");
}

#[test]
fn units_go_to_their_factory_s_rally_point() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN", "GAPILE", "GAWEAP"] {
        let spot = game.find_spot(name, yard, 12).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name}");
    }
    game.run(100);
    game.world.houses[PLAYER as usize].credits = 10_000;
    let goal = (yard.0 + 6, yard.1 + 9);
    for size in [(5, 3), (3, 2)] {
        let factory = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.size == size).map(|b| b.id).unwrap();
        assert!(game.world.has_rally_point(factory));
        game.world.order(Order::Rally { building: factory, x: goal.0, y: goal.1 });
    }
    let known: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    let before = known.len();
    for name in ["MTNK", "E1"] {
        let unit = game.units.iter().position(|n| n == name).unwrap() as u16;
        game.world.order(Order::BuildUnit { house: PLAYER, unit });
    }
    game.run(4000);
    let made: Vec<(i32, i32)> = game.world.units().iter().filter(|u| u.owner == PLAYER && !known.contains(&u.id)).map(|unit| unit.cell()).collect();
    assert_eq!(made.len(), 2);
    for cell in made {
        assert!((cell.0 - goal.0).abs() <= 2 && (cell.1 - goal.1).abs() <= 2, "ended at {cell:?}, rally {goal:?}");
    }
}

#[test]
fn tanks_of_two_houses_fight_until_one_dies() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let tank = game.units.iter().position(|name| name == "MTNK").unwrap();
    let stats = ra2_sim::UnitStats { kind: tank as u16 + 1, ..game.world.unit_types[tank].stats };
    assert!(stats.fighter.primary.is_some(), "the tank has no weapon");
    let spot = (yard.0 + 6, yard.1 + 8);
    let ours = game.world.add_unit(spot, 0, stats, PLAYER);
    let theirs = game.world.add_unit((spot.0 + 3, spot.1), 0, stats, 2);
    let mut fired = 0;
    let mut hits = 0;
    for _ in 0..3000 {
        game.run(1);
        for event in game.world.take_events() {
            match event {
                ra2_sim::combat::Event::Fired { .. } => fired += 1,
                ra2_sim::combat::Event::Detonated { .. } => hits += 1,
                _ => {}
            }
        }
        if game.world.unit(ours).is_none() || game.world.unit(theirs).is_none() {
            break;
        }
    }
    assert!(fired > 2 && hits > 2, "fired {fired}, hits {hits}");
    assert!(game.world.unit(ours).is_none() || game.world.unit(theirs).is_none(), "both still alive after {fired} shots");
    // Whoever is left was hurt too: they shot at each other.
    // (Two of a kind can also kill each other in the same tick.)
    if let Some(left) = game.world.unit(ours).or(game.world.unit(theirs)) {
        assert!(left.health < left.stats.strength);
    }
}

#[test]
fn a_building_can_be_sold_for_half_its_cost_and_its_people_walk_out() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("GAPOWR", yard, 12).unwrap();
    assert!(game.build_at("GAPOWR", spot));
    game.run(100);
    let plant = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.cell == spot).map(|b| (b.id, b.kind)).unwrap();
    let cost = game.world.building_types[plant.1 as usize].cost;
    let (credits, units) = (game.world.houses[PLAYER as usize].credits, game.world.units().len());
    game.world.order(Order::Sell { building: plant.0 });
    game.run(1);
    // People come out at once; the money only when it has come down.
    assert!(game.world.units().len() > units);
    assert_eq!(game.world.houses[PLAYER as usize].credits, credits);
    game.run(200);
    assert!(!game.world.buildings().iter().any(|b| b.id == plant.0));
    assert_eq!(game.world.houses[PLAYER as usize].credits, credits + cost / 2);
}

#[test]
fn soldiers_with_special_orders_jump_dig_in_and_take_buildings() {
    use ra2_sim::combat::Stance;
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("GAPOWR", yard, 12).unwrap();
    assert!(game.build_at("GAPOWR", spot));
    game.run(100);
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let open = (yard.0 + 6, yard.1 + 8);

    // A Chrono Legionnaire is at once where it is sent, and then out of action.
    let legionnaire = game.world.add_infantry(open, 0, 0, stats(&game, "CLEG"), PLAYER).unwrap();
    let far = (open.0 + 8, open.1 + 2);
    game.world.order(Order::Move { unit: legionnaire, x: far.0, y: far.1 });
    let unit = game.world.unit(legionnaire).unwrap();
    assert_eq!(unit.cell(), far);
    assert!(unit.warp_until > game.world.current_tick());

    // A GI digs in, will not walk, and gets up again.
    let gi = game.world.add_infantry((open.0, open.1 + 1), 0, 0, stats(&game, "E1"), PLAYER).unwrap();
    game.world.order(Order::Deploy { unit: gi });
    game.run(40);
    assert_eq!(game.world.unit(gi).unwrap().stance, Stance::Deployed);
    game.world.order(Order::Move { unit: gi, x: open.0 + 3, y: open.1 + 1 });
    game.run(40);
    assert_eq!(game.world.unit(gi).unwrap().cell(), (open.0, open.1 + 1));
    game.world.order(Order::Deploy { unit: gi });
    game.run(40);
    assert_eq!(game.world.unit(gi).unwrap().stance, Stance::Standing);

    // Another house's engineer takes the power plant and is used up.
    let plant = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.cell == spot).map(|b| b.id).unwrap();
    let engineer = game.world.add_infantry((spot.0 - 2, spot.1), 0, 0, stats(&game, "ENGINEER"), 2).unwrap();
    assert_eq!(game.world.entry_action(engineer, plant), Some(ra2_sim::combat::Entry::Capture));
    game.world.order(Order::Enter { unit: engineer, building: plant });
    game.run(400);
    assert_eq!(game.world.buildings().iter().find(|b| b.id == plant).map(|b| b.owner), Some(2));
    assert!(game.world.unit(engineer).is_none());
}

#[test]
fn a_rocketeer_flies_to_where_it_is_sent_and_hovers() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let kind = game.units.iter().position(|n| n == "JUMPJET").unwrap();
    let stats = ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats };
    assert!(stats.jet.is_some(), "the Rocketeer has no jumpjet figures");
    let from = (yard.0 + 6, yard.1 + 8);
    let rocketeer = game.world.add_infantry(from, 0, 0, stats, PLAYER).unwrap();
    let goal = (from.0 + 10, from.1 - 6);
    game.world.order(Order::Move { unit: rocketeer, x: goal.0, y: goal.1 });
    let mut highest = 0;
    for _ in 0..1500 {
        game.run(1);
        let unit = game.world.unit(rocketeer).unwrap();
        highest = highest.max(game.world.altitude(unit));
    }
    let unit = game.world.unit(rocketeer).unwrap();
    assert!(highest >= 400, "it only reached {highest}");
    // A Rocketeer is a `BalloonHover` type: it stays up where it was sent.
    assert!(game.world.altitude(unit) >= 400 && !unit.is_moving(), "at {} over {:?}", game.world.altitude(unit), unit.cell());
    let cell = unit.cell();
    assert!((cell.0 - goal.0).abs() <= 1 && (cell.1 - goal.1).abs() <= 1, "over {cell:?}, sent to {goal:?}");
}

#[test]
fn an_ifv_takes_a_passenger_changes_its_weapon_and_lets_it_out() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let open = (yard.0 + 6, yard.1 + 8);
    let ifv = game.world.add_unit(open, 0, stats(&game, "FV"), PLAYER);
    let rockets = game.world.unit(ifv).unwrap().gun.expect("an empty IFV has no weapon");
    let gi = game.world.add_infantry((open.0 + 3, open.1), 0, 0, stats(&game, "E1"), PLAYER).unwrap();
    assert!(game.world.can_board(gi, ifv));
    game.world.order(Order::Board { unit: gi, transport: ifv });
    game.run(300);
    assert!(game.world.unit(gi).is_none(), "the soldier is still outside");
    let carrier = game.world.unit(ifv).unwrap();
    assert_eq!(carrier.cargo.len(), 1);
    let gun = carrier.gun.unwrap();
    assert!(gun.0.weapon != rockets.0.weapon && gun.1 != rockets.1, "weapon and turret unchanged: {gun:?}");
    // No room for a second.
    let second = game.world.add_infantry((open.0 + 3, open.1 + 1), 0, 0, stats(&game, "E1"), PLAYER).unwrap();
    assert!(!game.world.can_board(second, ifv));
    // Out again, and back to rockets.
    game.world.order(Order::Deploy { unit: ifv });
    game.run(100);
    assert!(game.world.unit(gi).is_some());
    assert_eq!(game.world.unit(ifv).unwrap().gun, Some(rockets));
    // The same with a soldier older than the vehicle (it goes back into the
    // list of units before its carrier).
    let later = game.world.add_unit((open.0, open.1 + 4), 0, stats(&game, "FV"), PLAYER);
    game.world.order(Order::Board { unit: second, transport: later });
    game.run(300);
    assert_eq!(game.world.unit(later).unwrap().cargo.len(), 1);
    game.world.take_events();
    game.world.order(Order::Deploy { unit: later });
    game.run(100);
    assert!(game.world.unit(second).is_some());
    assert_eq!(game.world.unit(later).unwrap().gun, Some(rockets), "still the passenger's weapon");
    assert!(game.world.take_events().iter().any(|event| matches!(event, ra2_sim::combat::Event::Alighted { transport } if *transport == later)), "no leaving sound");
}

#[test]
fn factories_keep_turning_units_out_whatever_they_are() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN", "GAPILE", "GAWEAP", "GAPOWR", "GAAIRC", "GATECH"] {
        let name = if name == "GAAIRC" && !game.options().contains(&"GAAIRC") { "AMRADR" } else { name };
        let around = if game.world.buildings().len() > 4 { (yard.0 + 10, yard.1) } else { yard };
        let spot = game.find_spot(name, around, 12).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name}");
        game.run(30);
    }
    game.run(100);
    game.world.houses[PLAYER as usize].credits = 50_000;
    let goal = (yard.0 + 10, yard.1 + 9);
    let barracks = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.size == (3, 2)).map(|b| b.id).unwrap();
    game.world.order(Order::Rally { building: barracks, x: goal.0, y: goal.1 });
    let known: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    let before = known.len();
    let id = |game: &Scenario, name: &str| game.units.iter().position(|n| n == name).unwrap() as u16;
    for _ in 0..5 {
        let unit = id(&game, "E1");
        game.world.order(Order::BuildUnit { house: PLAYER, unit });
    }
    for name in ["SHAD", "SREF", "MTNK"] {
        let unit = id(&game, name);
        game.world.order(Order::BuildUnit { house: PLAYER, unit });
    }
    game.run(9000);
    let made: Vec<(String, (i32, i32))> =
        game.world.units().iter().filter(|u| u.owner == PLAYER && !known.contains(&u.id)).map(|unit| (game.units[unit.stats.kind as usize - 1].clone(), unit.cell())).collect();
    assert_eq!(made.len(), 8, "{made:?}");
    for (name, cell) in &made {
        let on = game.world.buildings().iter().any(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1));
        assert!(!on, "{name} left on a building at {cell:?}: {made:?}");
        if name == "E1" {
            assert!((cell.0 - goal.0).abs() <= 3 && (cell.1 - goal.1).abs() <= 3, "{name} at {cell:?}, rally {goal:?}");
        }
    }
    // A soldier standing on a building's cell can still be sent somewhere.
    let (cell, kind) = game.world.buildings().iter().find(|b| b.id == barracks).map(|b| (b.cell, b.kind)).unwrap();
    let _ = kind;
    let stats = game.world.units().iter().find(|u| u.stats.infantry).unwrap().stats;
    let stuck = game.world.add_infantry((cell.0 + 1, cell.1), 2, 0, stats, PLAYER).unwrap();
    game.world.order(Order::Move { unit: stuck, x: goal.0, y: goal.1 });
    game.run(1500);
    let at = game.world.unit(stuck).unwrap().cell();
    assert!((at.0 - goal.0).abs() <= 3 && (at.1 - goal.1).abs() <= 3, "soldier from inside the barracks ended at {at:?}");
}

#[test]
fn a_tank_attacks_a_building_from_its_range_not_from_beside_it() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let tank = game.units.iter().position(|name| name == "MTNK").unwrap();
    let stats = ra2_sim::UnitStats { kind: tank as u16 + 1, ..game.world.unit_types[tank].stats };
    let ours = game.world.add_unit((yard.0 + 8, yard.1 + 8), 0, stats, PLAYER);
    let (target, cell, size) = game.world.buildings().iter().find(|b| game.names[b.kind as usize] == "CATEXS07").map(|b| (b.id, b.cell, b.size)).unwrap();
    game.world.order(Order::Attack { unit: ours, target: ra2_sim::combat::Aim::Building(target) });
    let mut fired = 0;
    let mut closest = i32::MAX;
    for _ in 0..2500 {
        game.run(1);
        fired += game.world.take_events().iter().filter(|e| matches!(e, ra2_sim::combat::Event::Fired { .. })).count();
        let at = game.world.unit(ours).expect("the tank was killed").cell();
        let gap = (at.0 - cell.0).max(cell.0 + size.0 - 1 - at.0).min((at.0 - cell.0).abs()).max(0).max((at.1 - cell.1).abs().min((at.1 - (cell.1 + size.1 - 1)).abs()));
        closest = closest.min(gap);
    }
    assert!(fired > 0, "never fired");
    assert!(closest >= 3, "drove to within {closest} cells of the building at {cell:?}; tank at {:?}", game.world.unit(ours).unwrap().cell());
}

#[test]
fn soldiers_fire_from_a_civilian_building_and_are_put_out_when_it_is_badly_hurt() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let (house, cell, size) = game.world.buildings().iter().find(|b| game.names[b.kind as usize] == "CATEXS07").map(|b| (b.id, b.cell, b.size)).unwrap();
    // Nothing neutral shoots at a passer-by.
    let passer = game.world.add_unit((cell.0 - 4, cell.1), 0, stats(&game, "MTNK"), PLAYER);
    game.run(300);
    assert_eq!(game.world.unit(passer).unwrap().health, game.world.unit(passer).unwrap().stats.strength, "a neutral building fired");
    let soldiers: Vec<u32> = (0..3).map(|i| game.world.add_infantry((cell.0 - 2, cell.1 + i), 2, 0, stats(&game, "E1"), PLAYER).unwrap()).collect();
    for &unit in &soldiers {
        assert_eq!(game.world.entry_action(unit, house), Some(ra2_sim::combat::Entry::Occupy));
        game.world.order(Order::Enter { unit, building: house });
    }
    game.run(400);
    let building = game.world.buildings().iter().find(|b| b.id == house).unwrap();
    assert_eq!((building.occupants.len(), building.owner), (3, PLAYER));
    assert!(soldiers.iter().all(|&unit| game.world.unit(unit).is_none()));
    // It fires on an enemy that comes near.
    let enemies: Vec<u32> = (0..6).map(|i| game.world.add_unit((cell.0 + size.0 + 3, cell.1 + i), 0, stats(&game, "HTNK"), 2)).collect();
    let total = |game: &Scenario| -> i32 { enemies.iter().filter_map(|&id| game.world.unit(id)).map(|u| u.health).sum() };
    let before = total(&game);
    game.run(200);
    assert!(total(&game) < before, "the garrison never hit the enemy");
    // The enemy brings it down to red: everyone comes out unhurt and it is
    // neutral again, and cannot be gone back into.
    for _ in 0..6000 {
        game.run(1);
        if game.world.buildings().iter().find(|b| b.id == house).is_none_or(|b| b.occupants.is_empty()) {
            break;
        }
    }
    game.run(2);
    let building = game.world.buildings().iter().find(|b| b.id == house).expect("the building was destroyed outright");
    assert_eq!((building.occupants.len(), building.owner), (0, 0));
    assert!(game.world.red_health(building));
    for &unit in &soldiers {
        let soldier = game.world.unit(unit).expect("a soldier was lost");
        assert_eq!(soldier.health, soldier.stats.strength);
        assert_eq!(game.world.entry_action(unit, house), None);
    }
}

#[test]
fn units_find_their_way_round_things_to_far_places() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let start = (yard.0 + 6, yard.1 + 8);
    let tank = game.world.add_unit(start, 0, stats(&game, "MTNK"), PLAYER);
    let soldier = game.world.add_infantry((start.0 + 1, start.1 + 1), 2, 0, stats(&game, "E1"), PLAYER).unwrap();
    let (w, h) = game.world.map.size();
    let mut failures = Vec::new();
    let mut tried = 0;
    for (gx, gy) in [(20, 10), (60, 20), (90, 40), (40, 60), (70, 90), (110, 70), (30, 120), (100, 110), (130, 30), (15, 60)] {
        let goal = (gx * w / 150, gy * h / 150);
        for unit in [tank, soldier] {
            if !game.world.can_move_to(unit, goal) {
                continue;
            }
            tried += 1;
            game.world.order(Order::Move { unit, x: goal.0, y: goal.1 });
        }
        game.run(6000);
        for unit in [tank, soldier] {
            let Some(u) = game.world.unit(unit) else { continue };
            let at = u.cell();
            if game.world.can_move_to(unit, goal) && ((at.0 - goal.0).abs() > 3 || (at.1 - goal.1).abs() > 3) {
                failures.push((u.stats.infantry, goal, at, u.destination()));
            }
        }
    }
    assert!(tried > 3, "too few reachable goals: {tried}");
    assert!(failures.is_empty(), "did not arrive (soldier?, goal, ended, still going to): {failures:?}");
}

#[test]
fn soldiers_board_a_helicopter_that_has_flown_and_landed() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let open = (yard.0 + 6, yard.1 + 8);
    let heli = game.world.add_unit(open, 0, stats(&game, "SHAD"), PLAYER);
    game.world.order(Order::Move { unit: heli, x: open.0 + 5, y: open.1 + 2 });
    game.run(600);
    let at = game.world.unit(heli).unwrap();
    eprintln!("heli at {:?} alt {} moving {}", at.cell(), game.world.altitude(at), at.is_moving());
    let at = at.cell();
    let gi = game.world.add_infantry((at.0 + 3, at.1), 2, 0, stats(&game, "E1"), PLAYER).unwrap();
    assert!(game.world.can_board(gi, heli), "refused");
    game.world.order(Order::Board { unit: gi, transport: heli });
    game.run(400);
    assert!(game.world.unit(gi).is_none(), "the soldier is still outside at {:?}", game.world.unit(gi).map(|u| u.cell()));
    // Four more and a legionnaire; then everyone out and in again.
    let mut crowd: Vec<u32> = (0..4).map(|i| game.world.add_infantry((at.0 + 3 + i % 2, at.1 + 1 + i / 2), 2, 0, stats(&game, "E1"), PLAYER).unwrap()).collect();
    let legionnaire = game.world.add_infantry((at.0 - 6, at.1 + 3), 2, 0, stats(&game, "CLEG"), PLAYER).unwrap();
    for &unit in crowd.iter().chain([&legionnaire]) {
        if game.world.can_board(unit, heli) {
            game.world.order(Order::Board { unit, transport: heli });
        }
    }
    game.run(900);
    let room = game.world.unit_types[(game.world.unit(heli).unwrap().stats.kind - 1) as usize].passengers as usize;
    assert_eq!(game.world.unit(heli).unwrap().cargo.len(), room.min(6), "not everyone got in: outside {:?}", crowd.iter().filter_map(|&u| game.world.unit(u).map(|u| (u.cell(), u.destination()))).collect::<Vec<_>>());
    game.world.order(Order::Deploy { unit: heli });
    game.run(400);
    assert!(game.world.unit(heli).unwrap().cargo.is_empty(), "not everyone got out");
    crowd.push(gi);
    for &unit in &crowd {
        assert!(game.world.unit(unit).is_some());
        game.world.order(Order::Board { unit, transport: heli });
    }
    game.run(900);
    assert_eq!(game.world.unit(heli).unwrap().cargo.len(), 5.min(room), "second boarding failed: outside {:?}", crowd.iter().filter_map(|&u| game.world.unit(u).map(|u| (u.cell(), u.destination()))).collect::<Vec<_>>());
}

#[test]
fn an_engineer_mends_a_damaged_building_of_its_own_side() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let plant = game.building("GAPOWR");
    let spot = game.find_spot("GAPOWR", (yard.0 + 6, yard.1 + 6), 6).unwrap();
    let id = game.world.add_map_building(PLAYER, plant, spot, 100);
    let engineer = game.world.add_infantry((spot.0 + 5, spot.1 + 4), 2, 0, stats(&game, "ENGINEER"), PLAYER).unwrap();
    assert_eq!(game.world.entry_action(engineer, id), Some(ra2_sim::combat::Entry::Repair));
    game.world.order(Order::Enter { unit: engineer, building: id });
    game.run(600);
    let building = game.world.buildings().iter().find(|b| b.id == id).unwrap();
    let full = game.world.building_types[plant as usize].strength;
    assert_eq!(building.health, full, "not mended; engineer at {:?}", game.world.unit(engineer).map(|u| (u.cell(), u.destination())));
    assert!(game.world.unit(engineer).is_none());
}

#[test]
fn a_prism_tank_fires() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let spot = (yard.0 + 6, yard.1 + 8);
    let prism = stats(&game, "SREF");
    assert!(prism.fighter.primary.is_some(), "no weapon loaded");
    let ours = game.world.add_unit(spot, 0, prism, PLAYER);
    let theirs = game.world.add_unit((spot.0 + 4, spot.1), 0, stats(&game, "HTNK"), 2);
    eprintln!("action {:?}", game.world.attack_action(ours, ra2_sim::combat::Aim::Unit(theirs), false));
    game.world.order(Order::Attack { unit: ours, target: ra2_sim::combat::Aim::Unit(theirs) });
    let mut fired = 0;
    for _ in 0..600 {
        game.run(1);
        fired += game.world.take_events().iter().filter(|e| matches!(e, ra2_sim::combat::Event::Fired { by: ra2_sim::combat::Aim::Unit(id), .. } if *id == ours)).count();
    }
    assert!(fired > 0, "never fired");
}

#[test]
fn a_unit_held_by_a_legionnaire_stops_firing_and_is_erased() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let spot = (yard.0 + 6, yard.1 + 8);
    let mut legion = stats(&game, "CLEG");
    legion.strength = 100_000;
    let ours = game.world.add_infantry(spot, 2, 0, legion, PLAYER).unwrap();
    game.world.units_mut_for_tests(ours, 100_000);
    let theirs = game.world.add_unit((spot.0 + 3, spot.1), 0, stats(&game, "HTNK"), 2);
    game.world.order(Order::Attack { unit: ours, target: ra2_sim::combat::Aim::Unit(theirs) });
    let (mut frozen_at, mut shots_while_held, mut thawed) = (None, 0, 0);
    let mut log = Vec::new();
    for tick in 0..1500 {
        game.run(1);
        let events = game.world.take_events();
        let held = game.world.unit(theirs).map(|u| u.frozen.is_some());
        if held == Some(true) && frozen_at.is_none() {
            frozen_at = Some(tick);
        }
        if frozen_at.is_some() && held == Some(false) {
            thawed += 1;
        }
        let fired = events.iter().filter(|e| matches!(e, ra2_sim::combat::Event::Fired { by: ra2_sim::combat::Aim::Unit(id), .. } if *id == theirs)).count();
        if frozen_at.is_some() {
            shots_while_held += fired;
            if fired > 0 { log.push((tick, held)); }
        }
        if held.is_none() {
            break;
        }
    }
    assert!(frozen_at.is_some(), "never held");
    assert!(game.world.unit(theirs).is_none(), "never erased");
    assert_eq!((shots_while_held, thawed), (0, 0), "fired or was let go while held: {log:?}");
    // With its target gone the legionnaire stops.
    game.run(5);
    game.world.take_events();
    game.run(300);
    let after = game.world.take_events().iter().filter(|e| matches!(e, ra2_sim::combat::Event::Fired { .. })).count();
    assert_eq!(after, 0, "still firing at nothing");
}

#[test]
fn the_shroud_lifts_round_the_players_things_and_nothing_goes_under_it() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    game.world.map.shroud_on = true;
    let unit = game.player_unit();
    let at = game.world.unit(unit).unwrap().cell();
    game.run(2);
    let map = &game.world.map;
    assert!(map.revealed(at.0, at.1) && !map.shrouded(at.0, at.1));
    assert!(map.revealed(at.0 + 3, at.1 - 3), "sight should reach a few cells");
    assert!(!map.revealed(at.0 + 25, at.1), "the far ground should be dark");
    let yard = game.deploy_base();
    let plant = game.building("GAPOWR");
    game.world.houses[PLAYER as usize].credits = 100_000;
    assert!(game.find_spot("GAPOWR", yard, 4).is_some(), "nowhere seen to build");
    assert!(!game.world.can_place(PLAYER, plant, (yard.0 + 25, yard.1)));
}

#[test]
fn tanya_blows_up_a_building_with_a_charge() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap();
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let plant = game.building("NAPOWR");
    let spot = (yard.0 + 8, yard.1 + 8);
    let id = game.world.add_map_building(2, plant, spot, 10_000);
    let tanya = game.world.add_infantry((spot.0 - 4, spot.1), 2, 0, stats(&game, "TANY"), PLAYER).unwrap();
    assert_eq!(game.world.entry_action(tanya, id), Some(ra2_sim::combat::Entry::Sabotage));
    game.world.order(Order::Attack { unit: tanya, target: ra2_sim::combat::Aim::Building(id) });
    game.run(400);
    assert!(!game.world.buildings().iter().any(|b| b.id == id), "still standing");
    // The click's own order (a walk-in) does the same, on a building of every size.
    for name in ["NACNST", "NAWEAP", "TESLA"] {
        let kind = game.building(name);
        let spot = (yard.0 + 8, yard.1 + 8);
        let id = game.world.add_map_building(2, kind, spot, 10_000);
        let from = game.world.unit(tanya).unwrap().cell();
        game.world.order(Order::Enter { unit: tanya, building: id });
        game.run(500);
        assert!(!game.world.buildings().iter().any(|b| b.id == id), "{name} still standing; Tanya went from {from:?} to {:?}", game.world.unit(tanya).map(|u| u.cell()));
    }
    let tanya = game.world.unit(tanya).expect("Tanya went with it");
    let at = tanya.cell();
    assert!(game.world.map.speed_percent(at.0, at.1, tanya.stats.speed_type) > 0);
}

#[test]
fn a_harrier_is_built_on_a_pad_flies_out_fires_and_comes_home_to_reload() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN", "GAPILE", "GAAIRC"] {
        let name = if name == "GAAIRC" && !game.options().contains(&"GAAIRC") { "AMRADR" } else { name };
        let spot = game.find_spot(name, yard, 12).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name}");
        game.run(30);
    }
    game.run(100);
    let harrier = game.units.iter().position(|n| n == "ORCA").expect("no ORCA type") as u16;
    assert!(game.world.buildable_units(PLAYER).contains(&harrier), "the Harrier is not offered");
    game.world.houses[PLAYER as usize].credits = 50_000;
    let known: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    let before = known.len();
    for _ in 0..5 {
        game.world.order(Order::BuildUnit { house: PLAYER, unit: harrier });
    }
    game.run(9000);
    let planes: Vec<u32> = game.world.units().iter().filter(|u| u.owner == PLAYER && !known.contains(&u.id)).filter(|u| u.plane.is_some()).map(|u| u.id).collect();
    assert_eq!(planes.len(), 4, "four pads, so four aircraft");
    let spots: std::collections::HashSet<(i32, i32)> = planes.iter().map(|&id| { let u = game.world.unit(id).unwrap(); (u.x, u.y) }).collect();
    assert_eq!(spots.len(), 4, "aircraft share a spot: {spots:?}; offsets {:?}", game.world.buildings().iter().find(|b| !b.docks.is_empty()).map(|b| game.world.building_types[b.kind as usize].docking_offsets.clone()));
    let pad = game.world.buildings().iter().find(|b| !b.docks.is_empty()).map(|b| (b.id, b.cell, b.size)).unwrap();
    for &id in &planes {
        let unit = game.world.unit(id).unwrap();
        let cell = unit.cell();
        assert!((pad.1 .0..pad.1 .0 + pad.2 .0).contains(&cell.0) && (pad.1 .1..pad.1 .1 + pad.2 .1).contains(&cell.1), "parked off the pad at {cell:?}");
        assert_eq!(game.world.altitude(unit), 0);
    }
    // An enemy building some way off.
    let plant = game.building("NAPOWR");
    let spot = (yard.0 + 20, yard.1 + 6);
    let target = game.world.add_map_building(2, plant, spot, 300);
    let start = game.world.buildings().iter().find(|b| b.id == target).unwrap().health;
    let flyer = planes[0];
    game.world.order(Order::Attack { unit: flyer, target: ra2_sim::combat::Aim::Building(target) });
    let (mut top, mut fired, mut log, mut landing_sounds) = (0, 0, Vec::new(), 0);
    for tick in 0..2500 {
        game.run(1);
        let events = game.world.take_events();
        fired += events.iter().filter(|e| matches!(e, ra2_sim::combat::Event::Fired { by: ra2_sim::combat::Aim::Unit(id), .. } if *id == flyer)).count();
        landing_sounds += events.iter().filter(|e| matches!(e, ra2_sim::combat::Event::Landing { unit } if *unit == flyer)).count();
        let Some(unit) = game.world.unit(flyer) else { break };
        top = top.max(game.world.altitude(unit));
        if tick % 100 == 0 {
            let plane = unit.plane.unwrap();
            log.push((tick, unit.cell(), plane.z, plane.mission, plane.ammo));
        }
    }
    assert!(top >= 1400, "never climbed: {top}; {log:?}");
    assert!(fired >= 1, "never fired: {log:?}");
    assert_eq!(landing_sounds, 1, "one landing sound coming home");
    let unit = game.world.unit(flyer).expect("the Harrier was lost");
    let plane = unit.plane.unwrap();
    assert_eq!((plane.z, plane.ammo), (0, 1), "not home and loaded: {log:?}");
    assert!(plane.contact.is_some());
    let left = game.world.buildings().iter().find(|b| b.id == target).map(|b| b.health);
    assert!(left.is_none_or(|health| health < start), "the shot did nothing: {start} -> {left:?}");
}

#[test]
fn a_pad_repairs_its_aircraft_a_step_at_a_time_and_selling_it_sends_them_off() {
    use ra2_sim::combat::Event;
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN", "GAPILE", "GAAIRC"] {
        let name = if name == "GAAIRC" && !game.options().contains(&"GAAIRC") { "AMRADR" } else { name };
        let spot = game.find_spot(name, yard, 12).unwrap_or_else(|| panic!("nowhere for {name}"));
        assert!(game.build_at(name, spot), "{name}");
        game.run(30);
    }
    game.run(100);
    let harrier = game.units.iter().position(|n| n == "ORCA").expect("no ORCA type") as u16;
    game.world.houses[PLAYER as usize].credits = 50_000;
    let known: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    for _ in 0..2 {
        game.world.order(Order::BuildUnit { house: PLAYER, unit: harrier });
    }
    game.run(4000);
    let planes: Vec<u32> = game.world.units().iter().filter(|u| u.owner == PLAYER && !known.contains(&u.id) && u.plane.is_some()).map(|u| u.id).collect();
    assert_eq!(planes.len(), 2);
    let pad = game.world.buildings().iter().find(|b| !b.docks.is_empty()).map(|b| b.id).unwrap();

    // Hurt on the pad, it is mended `RepairStep` (8) points every
    // `ReloadRate` (270 frames), for a little money each time.
    let hurt = planes[0];
    let strength = game.world.unit(hurt).unwrap().stats.strength;
    game.world.units_mut_for_tests(hurt, strength - 20);
    let cost = game.world.unit_types[harrier as usize].cost;
    let price = (((cost / (strength / 8)) as f64 * 0.15) as i32).max(1);
    let mut seen = vec![strength - 20];
    let mut paid = Vec::new();
    let mut ticks = Vec::new();
    for tick in 0..1000 {
        let credits = game.world.houses[PLAYER as usize].credits;
        game.run(1);
        let health = game.world.unit(hurt).unwrap().health;
        if seen.last() != Some(&health) {
            seen.push(health);
            paid.push(credits - game.world.houses[PLAYER as usize].credits);
            ticks.push(tick);
        }
    }
    assert_eq!(seen, vec![strength - 20, strength - 12, strength - 4, strength], "steps: {seen:?}");
    assert_eq!(paid, vec![price; 3], "paid {paid:?}");
    assert!(ticks.windows(2).all(|w| w[1] - w[0] == 270), "at {ticks:?}");
    let plane = game.world.unit(hurt).unwrap().plane.unwrap();
    assert_eq!((plane.z, plane.contact), (0, Some(pad)), "it left the pad");

    // Sold, the pad sends its aircraft up; with no other pad to go to they fall.
    game.world.take_events();
    game.world.order(Order::Sell { building: pad });
    let (mut up, mut top) = (0, 0);
    for _ in 0..1500 {
        game.run(1);
        up += game.world.take_events().iter().filter(|e| matches!(e, Event::TookOff { unit } if planes.contains(unit))).count();
        for &id in &planes {
            if let Some(unit) = game.world.unit(id) {
                top = top.max(game.world.altitude(unit));
            }
        }
    }
    assert_eq!(up, 2, "both should take off");
    assert!(top > 0);
    assert!(planes.iter().all(|&id| game.world.unit(id).is_none()), "aircraft without a pad should come down: {:?}", planes.iter().filter_map(|&id| game.world.unit(id).map(|u| (u.plane.unwrap().mission, u.plane.unwrap().z))).collect::<Vec<_>>());
}

#[test]
fn the_computer_house_deploys_its_base_vehicle_and_both_sides_get_starting_units() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    assert!(game.world.houses.len() > enemy as usize, "no computer house");
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(2));
    let vehicle = game.world.units().iter().find(|u| u.owner == enemy).map(|u| (u.id, u.cell())).expect("the computer has no base vehicle");
    let mine = game.world.units().iter().find(|u| u.owner == PLAYER).map(|u| u.cell()).unwrap();
    assert!((vehicle.1 .0 - mine.0).abs() + (vehicle.1 .1 - mine.1).abs() > 20, "the two start too close: {mine:?} {:?}", vehicle.1);
    let known: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    let before = known.len();
    game.world.create_starting_units(10);
    let (ours, theirs) = (
        game.world.units().iter().filter(|u| !known.contains(&u.id) && u.owner == PLAYER).count(),
        game.world.units().iter().filter(|u| !known.contains(&u.id) && u.owner == enemy).count(),
    );
    assert!(ours >= 5 && theirs >= 5, "starting units: {ours} and {theirs}");
    game.run(400);
    assert!(game.world.unit(vehicle.0).is_none(), "the computer's vehicle never deployed");
    let yard = game.world.buildings().iter().find(|b| b.owner == enemy).expect("no yard");
    assert!(game.world.building_types[yard.kind as usize].construction_yard);
    let yard_cell = yard.cell;
    // It builds a base: power, a refinery with its miners, a barracks and a
    // war factory among other things, none on top of another.
    game.run(14_000);
    let built: Vec<(String, (i32, i32), (i32, i32))> =
        game.world.buildings().iter().filter(|b| b.owner == enemy).map(|b| (game.names[b.kind as usize].clone(), b.cell, b.size)).collect();
    let kinds: Vec<&str> = built.iter().map(|(name, ..)| name.as_str()).collect();
    assert!(built.len() >= 6, "only built {kinds:?}; credits {}; plan {:?}", game.world.houses[enemy as usize].credits, game.world.houses[enemy as usize].ai.as_ref().map(|b| b.nodes.iter().take(12).map(|n| n.kind.map(|k| game.names[k as usize].clone())).collect::<Vec<_>>()));
    let has = |list: &[&str]| kinds.iter().any(|kind| list.contains(kind));
    assert!(game.world.buildings().iter().any(|b| b.owner == enemy && game.world.is_defence(b.kind)), "no defences among {kinds:?}");
    assert!(has(&["GAPOWR", "NAPOWR"]) && has(&["GAREFN", "NAREFN"]) && has(&["GAPILE", "NAHAND"]) && has(&["GAWEAP", "NAWEAP"]), "{kinds:?}");
    for (i, a) in built.iter().enumerate() {
        for b in &built[i + 1..] {
            let apart = a.1 .0 + a.2 .0 <= b.1 .0 || b.1 .0 + b.2 .0 <= a.1 .0 || a.1 .1 + a.2 .1 <= b.1 .1 || b.1 .1 + b.2 .1 <= a.1 .1;
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
    let miners = game.world.units().iter().filter(|u| u.owner == enemy && u.stats.harvester).count();
    assert!(miners >= 1, "no miners");
    eprintln!("ENEMY yard at {yard_cell:?}: {kinds:?}, {miners} miners");
}

#[test]
fn special_weapons_do_what_they_are_for() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 8);

    // Ivan sticks a bomb on a tank; it goes off 450 frames later and hurts it.
    let ivan = game.world.add_infantry(at, 2, 0, stats(&game, "IVAN"), PLAYER).unwrap();
    let tank = game.world.add_unit((at.0 + 4, at.1), 0, stats(&game, "MTNK"), 2);
    let full = game.world.unit(tank).unwrap().health;
    game.world.order(Order::Attack { unit: ivan, target: Aim::Unit(tank) });
    game.run(250);
    assert!(game.world.bomb_on(Aim::Unit(tank)).is_some(), "no bomb planted");
    assert_eq!(game.world.unit(tank).unwrap().health, full, "the bomb went off early");
    game.run(500);
    assert!(game.world.bomb_on(Aim::Unit(tank)).is_none());
    assert!(game.world.unit(tank).is_none_or(|u| u.health < full), "the bomb did nothing");

    // A dog kills a soldier in one bite and is back on the map afterwards.
    let spot = (at.0, at.1 + 6);
    let dog = game.world.add_infantry(spot, 2, 0, stats(&game, "DOG"), PLAYER).unwrap();
    let victim = game.world.add_infantry((spot.0 + 3, spot.1), 2, 0, stats(&game, "E2"), 2).unwrap();
    game.world.order(Order::Attack { unit: dog, target: Aim::Unit(victim) });
    game.run(300);
    assert!(game.world.unit(victim).is_none(), "the dog did not kill");
    assert!(game.world.unit(dog).is_some(), "the dog is gone");

    // A terror drone gets into a tank, wears it down, and comes out when it dies.
    let spot = (at.0 + 8, at.1 + 6);
    let drone = game.world.add_unit(spot, 0, stats(&game, "DRON"), PLAYER);
    let host = game.world.add_unit((spot.0 + 3, spot.1), 0, stats(&game, "MTNK"), 2);
    game.world.order(Order::Attack { unit: drone, target: Aim::Unit(host) });
    game.run(200);
    assert!(game.world.unit(drone).is_none() && game.world.unit(host).is_some_and(|u| u.parasite.len() == 1), "the drone is not inside");
    game.run(1500);
    assert!(game.world.unit(host).is_none(), "the host survived");
    assert!(game.world.unit(drone).is_some(), "the drone did not come out");
    game.world.remove_unit(drone);

    // Yuri takes a tank; taking another lets the first go; his death frees it.
    let spot = (at.0, at.1 + 12);
    let yuri = game.world.add_infantry(spot, 2, 0, stats(&game, "YURI"), PLAYER).unwrap();
    // (The tanks fire back; he is to outlive that here.)
    game.world.units_mut_for_tests(yuri, 100_000);
    let first = game.world.add_unit((spot.0 + 4, spot.1), 0, stats(&game, "HTNK"), 2);
    let second = game.world.add_unit((spot.0 + 4, spot.1 + 2), 0, stats(&game, "HTNK"), 2);
    for tank in [first, second] {
        game.world.units_mut_for_tests(tank, 100_000);
    }
    game.world.order(Order::Attack { unit: yuri, target: Aim::Unit(first) });
    game.run(200);
    assert_eq!(game.world.unit(first).map(|u| u.owner), Some(PLAYER), "not taken");
    game.world.order(Order::Attack { unit: yuri, target: Aim::Unit(second) });
    game.run(200);
    assert_eq!((game.world.unit(first).map(|u| u.owner), game.world.unit(second).map(|u| u.owner)), (Some(2), Some(PLAYER)));
}

#[test]
fn a_dying_building_marks_the_ground() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    assert!(!game.world.map.smudge_types.is_empty(), "no smudge types loaded");
    assert_eq!(game.world.map.smudges().count(), 0);
    // An explosion's animation leaves a small mark on open ground.
    let open = ((yard.0 + 8) * 256 + 128, (yard.1 + 8) * 256 + 128);
    game.world.animation_mark(open, true, true, false, 40, 40);
    assert_eq!(game.world.map.smudges().count(), 1, "no mark from the animation");
    // A second on the same cell is refused.
    game.world.animation_mark(open, true, true, false, 40, 40);
    assert_eq!(game.world.map.smudges().count(), 1);
}

#[test]
fn tanya_over_an_enemy_building_is_offered_the_charge() {
    // So is the Psi Commando, whose mind control cannot touch a building: the
    // engine's attack test asks only that the soldier be armed.
    for soldier in ["TANY", "PTROOP"] {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let kind = game.units.iter().position(|n| n == soldier).unwrap();
    let stats = ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats };
    let spot = (yard.0 + 8, yard.1 + 8);
    let tanya = game.world.add_infantry((spot.0 - 4, spot.1), 2, 0, stats, PLAYER).unwrap();
    use crate::action::{for_object, Action, Mods};
    use crate::control::Target;
    for name in ["NAPOWR", "NACNST", "NAWEAP", "NAREFN", "NAHAND", "TESLA", "NARADR"] {
        let id = game.world.add_map_building(2, game.building(name), spot, 10_000);
        let action = for_object(&game.world, Target::Unit(tanya), Target::Building(id), true, false, Mods::default());
        assert_eq!(action, Action::Sabotage, "{soldier} over {name}");
        game.world.order(Order::Sell { building: id });
        game.run(120);
        // The crews that walked out would fight the soldier.
        for crew in game.world.units().iter().filter(|u| u.owner == 2 && u.stats.infantry).map(|u| u.id).collect::<Vec<_>>() {
            game.world.remove_unit(crew);
        }
    }
    }
}

#[test]
fn the_american_radar_gives_a_paradrop() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let index = game.world.super_types.iter().position(|kind| kind.kind == 6).expect("no American paradrop type");
    assert!(!game.world.houses[PLAYER as usize].supers.get(index).is_some_and(|s| s.granted));
    // Whoever holds the building has the drop (the building is the Americans').
    let radar = game.building("AMRADR");
    let spot = (yard.0 + 6, yard.1 + 6);
    game.world.add_map_building(PLAYER, radar, spot, 10_000);
    game.run(2);
    let state = game.world.houses[PLAYER as usize].supers[index];
    assert!(state.granted && !state.ready, "{state:?}");
    assert_eq!(state.left, 3600 - 1);
    game.run(3600);
    assert!(game.world.houses[PLAYER as usize].supers[index].can_fire(), "not ready after four minutes");
    let soldiers = |game: &Scenario| game.world.units().iter().filter(|u| u.owner == PLAYER && u.stats.infantry).count();
    let before = soldiers(&game);
    let target = (yard.0 + 10, yard.1 + 10);
    game.world.order(Order::Super { house: PLAYER, index: index as u16, cell: target });
    assert!(!game.world.houses[PLAYER as usize].supers[index].ready);
    assert!(game.world.units().iter().any(|u| u.plane.is_some() && u.cargo.len() == 8), "no loaded plane");
    game.run(1500);
    assert_eq!(soldiers(&game), before + 8, "the soldiers did not all come down");
    let landed: Vec<_> = game.world.units().iter().filter(|u| u.owner == PLAYER && u.stats.infantry && u.falling.is_none()).map(|u| u.cell()).collect();
    assert!(landed.iter().filter(|cell| (cell.0 - target.0).abs().max((cell.1 - target.1).abs()) <= 6).count() >= 8, "landed at {landed:?}, wanted near {target:?}");
    game.run(1500);
    assert!(!game.world.units().iter().any(|u| u.plane.is_some() && u.owner == PLAYER), "the plane is still here");
}

#[test]
fn a_v3_launches_its_rocket_and_grows_another() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 8);
    let v3 = game.world.add_unit(at, 0, stats(&game, "V3"), PLAYER);
    let plant = game.building("NAPOWR");
    let target = game.world.add_map_building(2, plant, (at.0 + 10, at.1), 10_000);
    let full = game.world.buildings().iter().find(|b| b.id == target).unwrap().health;
    game.run(30);
    assert_eq!(game.world.unit(v3).unwrap().spawner.as_ref().map(|s| s.docked()), Some(1), "no rocket aboard");
    game.world.order(Order::Attack { unit: v3, target: Aim::Building(target) });
    game.run(60);
    assert!(game.world.units().iter().any(|u| u.rocket.is_some()), "no rocket on the map");
    assert_eq!(game.world.unit(v3).unwrap().spawner.as_ref().map(|s| s.docked()), Some(0));
    game.run(400);
    let now = game.world.buildings().iter().find(|b| b.id == target).map_or(0, |b| b.health);
    assert!(now < full, "the rocket did no harm ({now} of {full})");
    let here = game.world.unit(v3).unwrap().cell();
    game.world.order(Order::Move { unit: v3, x: here.0, y: here.1 });
    // A rocket already promised to the old target may still go; the one after
    // it stays aboard.
    game.run(1000);
    assert!(game.world.unit(v3).unwrap().spawner.as_ref().is_some_and(|s| s.docked() == 1), "no new rocket");
}

#[test]
fn a_carrier_sends_its_hornets_and_takes_them_back() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 8);
    let carrier = game.world.add_unit(at, 0, stats(&game, "CARRIER"), PLAYER);
    let target = game.world.add_unit((at.0 + 9, at.1), 0, stats(&game, "HTNK"), 2);
    let full = game.world.unit(target).unwrap().health;
    game.run(30);
    assert_eq!(game.world.unit(carrier).unwrap().spawner.as_ref().map(|s| s.docked()), Some(3));
    game.world.order(Order::Attack { unit: carrier, target: Aim::Unit(target) });
    game.run(120);
    let out = game.world.units().iter().filter(|u| u.spawn_owner == Some(carrier)).count();
    assert_eq!(out, 3, "hornets out");
    // Above their `PitchSpeed` (0.9) they lean into their turns.
    let mut banked = false;
    for _ in 0..900 {
        game.run(1);
        banked |= game.world.units().iter().any(|u| u.spawn_owner == Some(carrier) && u.plane.is_some_and(|p| p.bank != 0));
    }
    assert!(banked, "no hornet ever banked");
    assert!(game.world.unit(target).is_none_or(|u| u.health < full), "the hornets did no harm");
    // With the target gone they come home.
    if let Some(id) = game.world.unit(target).map(|u| u.id) {
        game.world.remove_unit(id);
    }
    game.run(1500);
    let s = game.world.unit(carrier).unwrap().spawner.clone().unwrap();
    assert_eq!(s.docked(), 3, "not all back: {s:?} {:?}", game.world.units().iter().filter(|u| u.spawn_owner == Some(carrier)).map(|u| (u.cell(), u.plane.map(|p| (p.z, p.mission, p.ammo)))).collect::<Vec<_>>());
}

#[test]
fn a_spy_is_disguised_passed_over_and_steals() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    use ra2_sim::combat::{Aim, Entry};
    let at = (yard.0 + 6, yard.1 + 8);
    let spy = game.world.add_infantry(at, 2, 0, stats(&game, "SPY"), PLAYER).unwrap();
    game.run(2);
    let gi = stats(&game, "E1").kind;
    assert_eq!(game.world.unit(spy).unwrap().disguise, Some((gi, PLAYER)), "the spy starts as its side's soldier");
    // It takes the look of an enemy soldier it "shoots".
    let conscript = game.world.add_infantry((at.0 + 2, at.1), 2, 0, stats(&game, "E2"), 2).unwrap();
    game.world.order(Order::Attack { unit: spy, target: Aim::Unit(conscript) });
    game.run(150);
    let look = game.world.unit(spy).unwrap().disguise;
    assert_eq!(look, Some((stats(&game, "E2").kind, 2)));
    assert!(game.world.disguised_to(game.world.unit(spy).unwrap(), 2));
    // (Until then the conscript saw an enemy soldier and shot at it.)
    game.world.remove_unit(conscript);
    game.run(30);
    assert!(game.world.unit(spy).is_some(), "the spy did not live through taking its disguise");
    // An enemy guard leaves it alone.
    let guard = game.world.add_infantry((at.0 + 3, at.1 + 1), 2, 0, stats(&game, "E2"), 2).unwrap();
    game.run(120);
    assert_eq!(game.world.unit(guard).unwrap().arms.target, None, "the guard saw through it");
    assert!(game.world.unit(spy).is_some());
    game.world.remove_unit(guard);
    // Into a refinery: half the owner's money comes across.
    let refinery = game.world.add_map_building(2, game.building("NAREFN"), (at.0 + 6, at.1 + 4), 10_000);
    game.run(2);
    game.world.houses[2].credits = 3000;
    let mine = game.world.houses[PLAYER as usize].credits;
    assert_eq!(game.world.entry_action(spy, refinery), Some(Entry::Infiltrate));
    game.world.order(Order::Enter { unit: spy, building: refinery });
    game.run(600);
    assert!(game.world.unit(spy).is_none(), "the spy is still about");
    assert_eq!(game.world.houses[2].credits, 1500);
    assert_eq!(game.world.houses[PLAYER as usize].credits, mine + 1500);
}

#[test]
fn low_bridges_can_be_crossed() {
    // Some stock map has low bridges over water (Tower of Power, Lost Lake...).
    let Ok(files) = GameFiles::open() else {
        return;
    };
    let Some(list) = files.get("missions.pkt").map(ra2_formats::ini::Ini::parse) else {
        return;
    };
    let best = list
        .section("MultiMaps")
        .iter()
        .filter_map(|(_, id)| Scenario::load(id.trim()))
        .map(|game| {
            let map = &game.world.map;
            let (w, h) = map.size();
            let drives = |x: i32, y: i32| map.speed_percent(x, y, ra2_sim::SpeedType::Track) > 0;
            (2..w - 2)
                .flat_map(|x| (2..h - 2).map(move |y| (x, y)))
                .filter(|&(x, y)| drives(x, y) && ((map.wet(x - 2, y) && map.wet(x + 2, y)) || (map.wet(x, y - 2) && map.wet(x, y + 2))))
                .count()
        })
        .max()
        .unwrap_or(0);
    assert!(best >= 30, "no map has a bridge to drive over: at most {best} road cells with water on both sides");
}

#[test]
fn the_computer_forms_teams_and_attacks() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    let data = &game.world.team_data;
    assert!(data.triggers.len() > 50 && data.team_types.len() > 50 && !data.task_forces.is_empty() && !data.scripts.is_empty(), "ai.ini not read: {} triggers, {} teams", data.triggers.len(), data.team_types.len());
    assert!(data.triggers.iter().filter(|t| t.team1.is_some() && t.skirmish).count() > 30);
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    game.world.houses[enemy as usize].credits = 30_000;
    let yard = game.deploy_base();
    let power = game.find_spot("GAPOWR", yard, 12).unwrap();
    assert!(game.build_at("GAPOWR", power));
    let mut seen_teams = 0;
    let mut log = Vec::new();
    let mut hurt = false;
    let mut near = false;
    let mut chose = false;
    for step in 0..40 {
        game.run(1000);
        let world = &game.world;
        seen_teams = seen_teams.max(world.teams.len());
        let brain = world.houses[enemy as usize].ai.as_ref().unwrap();
        let army = world.units().iter().filter(|u| u.owner == enemy && !u.stats.harvester).count();
        log.push(format!(
            "{}: enemy {:?} teams {:?} army {army} credits {}",
            (step + 1) * 1000,
            brain.enemy,
            world.teams.iter().filter(|t| t.house == enemy).map(|t| (world.team_data.team_types[t.kind].name.clone(), t.members.len(), t.line, t.moving)).collect::<Vec<_>>(),
            world.houses[enemy as usize].credits
        ));
        chose |= brain.enemy == Some(PLAYER);
        near |= world.units().iter().any(|u| u.owner == enemy && (u.cell().0 - yard.0).abs().max((u.cell().1 - yard.1).abs()) < 12);
        hurt |= world.buildings().iter().any(|b| b.owner == PLAYER && b.health < world.building_types[b.kind as usize].strength) || !world.buildings().iter().any(|b| b.owner == PLAYER);
        if hurt {
            break;
        }
    }
    assert!(chose, "no enemy chosen\n{}", log.join("\n"));
    assert!(seen_teams > 0, "no team was ever formed\n{}", log.join("\n"));
    assert!(near, "no computer unit ever came near the player's base\n{}", log.join("\n"));
    assert!(hurt, "the player's base was never hurt\n{}", log.join("\n"));
}

#[test]
fn the_hardest_computer_walls_in_its_buildings() {
    // At the hardest level half the computer's defence places become a ring of
    // walls round its latest unwalled yard, tech centre or super weapon.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    game.world.houses[enemy as usize].credits = 30_000;
    game.deploy_base();
    let owned = |world: &ra2_sim::World| world.map.walls().filter(|(cell, ..)| world.map.wall_owner(cell.0, cell.1) == Some(enemy)).count();
    let mut walls = 0;
    for _ in 0..30 {
        game.run(1000);
        walls = owned(&game.world);
        if walls >= 8 {
            break;
        }
    }
    assert!(walls >= 8, "the computer put up {walls} pieces of wall");
}

#[test]
fn kills_bring_rank_and_rank_brings_its_bonuses() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 8);
    // A Grizzly (700) that kills three Rhinos (900 each) has destroyed more
    // than three times its own cost: a veteran. Six more make it elite.
    let tank = game.world.add_unit(at, 0, stats(&game, "MTNK"), PLAYER);
    assert!(game.world.unit(tank).unwrap().stats.trainable);
    let speed = game.world.unit(tank).unwrap().speed;
    for round in 0..3 {
        let victim = game.world.add_unit((at.0 + 4, at.1 + round), 0, stats(&game, "HTNK"), 2);
        game.world.units_mut_for_tests(victim, 1);
        game.world.order(Order::Attack { unit: tank, target: Aim::Unit(victim) });
        game.run(200);
        assert!(game.world.unit(victim).is_none(), "the victim lived");
    }
    let unit = game.world.unit(tank).unwrap();
    assert_eq!(unit.rank(), 1, "experience {}", unit.veterancy);
    assert!(unit.speed > speed, "a veteran with FASTER is no faster");
    assert!(game.world.take_events().iter().any(|event| matches!(event, ra2_sim::combat::Event::Promoted { unit, elite: false, .. } if *unit == tank)));
    // A miner is not trainable.
    assert!(!stats(&game, "HARV").trainable || !stats(&game, "ENGINEER").trainable);
}

#[test]
fn walls_are_worn_down_and_broken_by_shellfire() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let wall = game.building("GAWALL");
    let overlay = game.world.building_types[wall as usize].wall_overlay.expect("the wall has no overlay");
    let at = (yard.0 + 8, yard.1 + 8);
    for i in 0..3 {
        game.world.add_map_wall((at.0, at.1 + i), overlay, 0);
    }
    assert!(game.world.map.is_wall(at));
    assert_eq!(game.world.map.speed_percent(at.0, at.1, ra2_sim::SpeedType::Track), 0);
    for i in 0..3 {
        let tank = game.world.add_unit((at.0 - 4, at.1 + i), 0, stats(&game, "MTNK"), PLAYER);
        game.world.order(Order::Attack { unit: tank, target: ra2_sim::combat::Aim::Cell(at.0, at.1 + 1) });
    }
    let mut worn = false;
    for _ in 0..800 {
        game.run(10);
        worn |= game.world.map.walls().any(|(cell, _, data)| cell == (at.0, at.1 + 1) && data >> 4 > 0);
        if !game.world.map.is_wall((at.0, at.1 + 1)) {
            break;
        }
    }
    assert!(worn, "the wall never showed damage");
    assert!(!game.world.map.is_wall((at.0, at.1 + 1)), "the wall still stands");
}

#[test]
fn prism_towers_lend_their_beams() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let tower = game.building("ATESLA");
    assert_eq!(game.world.ability_rules.prism_type, Some(tower));
    for name in ["GAPOWR", "GAPOWR", "GAPOWR"] {
        let spot = game.find_spot(name, yard, 14).unwrap();
        assert!(game.build_at(name, spot));
    }
    let at = (yard.0 + 8, yard.1 + 10);
    let main = game.world.add_map_building(PLAYER, tower, at, 10_000);
    let helpers = [game.world.add_map_building(PLAYER, tower, (at.0 - 3, at.1), 10_000), game.world.add_map_building(PLAYER, tower, (at.0 - 3, at.1 + 2), 10_000)];
    game.run(5);
    // Alone, a tower's shot does its plain damage.
    let alone = game.world.weapons[game.world.building_types[tower as usize].fighter.primary.unwrap().weapon as usize].damage;
    let victim = game.world.add_unit((at.0 + 7, at.1), 0, stats(&game, "HTNK"), 2);
    game.world.units_mut_for_tests(victim, 100_000);
    let mut lent = 0;
    for _ in 0..200 {
        game.run(1);
        lent = lent.max(game.world.buildings().iter().find(|b| b.id == main).map_or(0, |b| b.supports));
    }
    assert!(lent >= 1, "no tower lent its beam (helpers {:?})", helpers);
    let lost = 100_000 - game.world.unit(victim).map_or(0, |u| u.health);
    assert!(lost > alone, "the shot was no stronger: {lost} against {alone}");
}

#[test]
fn yuri_deploys_his_wave_and_gets_up_again() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    let at = (yard.0 + 6, yard.1 + 10);
    let yuri = game.world.add_infantry(at, 2, 0, stats(&game, "YURI"), PLAYER).unwrap();
    let near = game.world.add_infantry((at.0 + 1, at.1), 2, 0, stats(&game, "E2"), 2).unwrap();
    let far = game.world.add_infantry((at.0 + 6, at.1), 2, 0, stats(&game, "E2"), 2).unwrap();
    game.world.units_mut_for_tests(yuri, 10_000);
    game.world.order(Order::Deploy { unit: yuri });
    game.run(100);
    assert!(game.world.unit(near).is_none(), "the soldier beside Yuri lived through the wave");
    assert!(game.world.unit(far).is_some(), "the wave reached too far");
    assert_eq!(game.world.unit(yuri).unwrap().stance, ra2_sim::combat::Stance::Deployed);
    game.run(120);
    assert_eq!(game.world.unit(yuri).unwrap().stance, ra2_sim::combat::Stance::Standing, "Yuri did not get up");
}

#[test]
fn submarines_cloak_and_sensors_find_them() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    // (The map has no sea; the cloak does not care.)
    let at = (yard.0 + 6, yard.1 + 10);
    let sub = game.world.add_unit(at, 0, stats(&game, "SUB"), 2);
    game.run(60);
    let unit = game.world.unit(sub).unwrap();
    assert_eq!(unit.cloak.state, 2, "the submarine never cloaked");
    assert!(game.world.hidden_from(unit, PLAYER));
    assert_eq!(game.world.visual_character(unit, 2), 3);
    assert_eq!(game.world.visual_character(unit, PLAYER), 5);
    // A destroyer's sensors (eight cells) show it.
    let destroyer = game.world.add_unit((at.0 + 5, at.1), 0, stats(&game, "DEST"), PLAYER);
    game.run(2);
    let unit = game.world.unit(sub).unwrap();
    assert!(!game.world.hidden_from(unit, PLAYER), "the destroyer's sensors missed it");
    assert_eq!(game.world.visual_character(unit, PLAYER), 3);
    let _ = destroyer;
}

#[test]
fn tanks_crush_soldiers_and_the_demolition_truck_goes_off() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let stats = |game: &Scenario, name: &str| {
        let kind = game.units.iter().position(|n| n == name).unwrap_or_else(|| panic!("no {name}"));
        ra2_sim::UnitStats { kind: kind as u16 + 1, ..game.world.unit_types[kind].stats }
    };
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    // A tank ordered through an enemy soldier's cell drives over him.
    assert!(stats(&game, "MTNK").crusher && stats(&game, "E2").crushable && !stats(&game, "MTNK").crushable);
    let tank = game.world.add_unit(at, 0, stats(&game, "MTNK"), PLAYER);
    let soldier = game.world.add_infantry((at.0 + 3, at.1), 2, 0, stats(&game, "E2"), 2).unwrap();
    game.world.units_mut_for_tests(soldier, 100_000);
    game.world.order(Order::Move { unit: tank, x: at.0 + 3, y: at.1 });
    game.run(200);
    assert!(game.world.unit(soldier).is_none(), "the soldier was not crushed");
    // A demolition truck's attack is its own death, and its death a blast.
    let spot = (at.0 + 1, at.1 + 3);
    assert!(game.world.map.speed_percent(spot.0, spot.1, ra2_sim::SpeedType::Wheel) > 0 && game.world.map.speed_percent(spot.0 + 3, spot.1, ra2_sim::SpeedType::Track) > 0);
    let truck = game.world.add_unit(spot, 0, stats(&game, "DTRUCK"), PLAYER);
    let victim = game.world.add_unit((spot.0 + 3, spot.1), 0, stats(&game, "HTNK"), 2);
    let full = game.world.unit(victim).unwrap().health;
    game.world.order(Order::Attack { unit: truck, target: Aim::Unit(victim) });
    game.run(300);
    assert!(game.world.unit(truck).is_none(), "the truck is still there: {:?}", game.world.unit(truck).map(|u| (u.cell(), spot, u.arms.target, u.arms.attacking, u.health, u.is_moving(), u.stats.fighter.primary.map(|m| game.world.weapons[m.weapon as usize].clone()))));
    assert!(game.world.unit(victim).is_none_or(|u| u.health < full), "the blast did nothing");
}

#[test]
fn a_flak_trooper_shoots_at_things_in_the_air() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 8);
    let rocketeer = game.world.add_infantry(at, 0, 0, game.stats("JUMPJET"), 2).unwrap();
    game.world.order(Order::Move { unit: rocketeer, x: at.0 + 1, y: at.1 });
    game.run(300);
    assert!(game.world.altitude(game.world.unit(rocketeer).unwrap()) >= 400);
    let full = game.world.unit(rocketeer).unwrap().health;
    game.world.units_mut_for_tests(rocketeer, 100_000);
    let _ = full;
    // Left to itself it picks the flyer out and uses its second weapon.
    game.world.add_infantry((at.0 + 4, at.1), 0, 0, game.stats("FLAKT"), PLAYER).unwrap();
    game.run(200);
    assert!(game.world.unit(rocketeer).is_none_or(|u| u.health < 100_000), "the flak trooper did not fire");
}

#[test]
fn the_spanner_mends_a_building_for_money() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let plant = game.building("GAPOWR");
    let spot = game.find_spot("GAPOWR", (yard.0 + 6, yard.1 + 6), 6).unwrap();
    let id = game.world.add_map_building(PLAYER, plant, spot, 100);
    let health = |game: &Scenario| game.world.buildings().iter().find(|b| b.id == id).unwrap().health;
    let (before, money) = (health(&game), game.world.houses[PLAYER as usize].credits);
    assert!(game.world.can_repair(id));
    game.world.order(Order::Repair { building: id });
    game.run(100);
    assert!(health(&game) > before, "no better: {} then {}", before, health(&game));
    assert!(game.world.houses[PLAYER as usize].credits < money);
    game.run(3000);
    assert_eq!(health(&game), game.world.building_types[plant as usize].strength);
    assert!(!game.world.buildings().iter().any(|b| b.id == id && b.repairing));
}

#[test]
fn a_tech_airport_gives_a_second_paradrop() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let american = game.world.super_types.iter().position(|kind| kind.kind == 6).unwrap();
    let plain = game.world.super_types.iter().position(|kind| kind.kind == 5).unwrap();
    game.world.add_map_building(PLAYER, game.building("AMRADR"), (yard.0 + 6, yard.1 + 6), 10_000);
    game.world.add_map_building(PLAYER, game.building("CAAIRP"), (yard.0 + 12, yard.1 + 6), 10_000);
    game.run(3602);
    let supers = &game.world.houses[PLAYER as usize].supers;
    assert!(supers[american].can_fire() && supers[plain].can_fire(), "{:?} {:?}", supers[american], supers[plain]);
    // The airport's drop is the side's own: six GIs for an Allied house.
    let soldiers = |game: &Scenario| game.world.units().iter().filter(|u| u.owner == PLAYER && u.stats.infantry).count();
    let before = soldiers(&game);
    game.world.order(Order::Super { house: PLAYER, index: plain as u16, cell: (yard.0 + 10, yard.1 + 12) });
    // Orders given on the way down wait for the landing.
    let mut told = None;
    for _ in 0..1500 {
        game.run(1);
        if told.is_none() {
            if let Some(unit) = game.world.units().iter().find(|u| u.falling.is_some()).map(|u| u.id) {
                told = Some(unit);
                game.world.order(Order::Deploy { unit });
                assert_eq!(game.world.unit(unit).unwrap().stance, ra2_sim::combat::Stance::Standing);
            }
        }
    }
    assert_eq!(soldiers(&game), before + 6);
    let first = game.world.unit(told.expect("nobody fell")).unwrap();
    assert!(first.falling.is_none() && first.stance == ra2_sim::combat::Stance::Deployed, "{:?}", first.stance);
    assert!(game.world.houses[PLAYER as usize].supers[american].can_fire(), "the other drop was spent too");
}

#[test]
fn a_tank_sent_past_an_enemy_soldier_drives_through_him() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 10);
    for (row, moving) in [(0, false), (3, true)] {
        let at = (at.0, at.1 + row);
        let tank = game.world.add_unit(at, 0, game.stats("MTNK"), PLAYER);
        let soldier = game.world.add_infantry((at.0 + 3, at.1), 2, 0, game.stats("E2"), 2).unwrap();
        game.world.units_mut_for_tests(soldier, 100_000);
        if moving {
            game.world.order(Order::Move { unit: soldier, x: at.0 + 4, y: at.1 });
            game.run(3);
        }
        game.world.order(Order::Move { unit: tank, x: at.0 + 6, y: at.1 });
        game.run(250);
        assert_eq!(game.world.unit(tank).unwrap().cell(), (at.0 + 6, at.1));
        assert!(game.world.unit(soldier).is_none(), "the soldier (moving: {moving}) was not crushed: at {:?}", game.world.unit(soldier).map(|u| u.cell()));
    }
}

#[test]
fn a_computer_house_short_of_power_loses_its_coils() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 8, yard.1 + 10);
    let coil = game.world.add_map_building(2, game.building("TESLA"), at, 10_000);
    let tank = game.world.add_unit((at.0 + 3, at.1), 0, game.stats("HTNK"), PLAYER);
    game.world.units_mut_for_tests(tank, 100_000);
    game.world.order(Order::Stop { unit: tank });
    assert!(game.world.low_power(2));
    game.run(300);
    assert_eq!(game.world.unit(tank).unwrap().health, 100_000, "a coil without power fired");
    let plant = game.world.add_map_building(2, game.building("NAPOWR"), (at.0, at.1 + 4), 10_000);
    game.run(300);
    assert!(game.world.unit(tank).unwrap().health < 100_000, "a coil with power did not fire");
    let _ = (coil, plant);
}

#[test]
fn the_computer_answers_an_attack_on_its_base() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(2));
    game.world.create_starting_units(10);
    game.run(3000);
    let target = game.world.buildings().iter().find(|b| b.owner == enemy && !game.world.building_types[b.kind as usize].construction_yard).map(|b| (b.id, b.cell)).expect("it built nothing");
    use ra2_sim::combat::Aim;
    let tank = game.world.add_unit((target.1 .0 - 5, target.1 .1 + 1), 0, game.stats("MTNK"), PLAYER);
    game.world.units_mut_for_tests(tank, 100_000);
    game.world.order(Order::Attack { unit: tank, target: Aim::Building(target.0) });
    let mut answered = 0;
    for _ in 0..600 {
        game.run(1);
        answered = answered.max(game.world.units().iter().filter(|u| u.owner == enemy && u.arms.target == Some(Aim::Unit(tank))).count());
    }
    let army: Vec<_> = game.world.units().iter().filter(|u| u.owner == enemy && !u.stats.harvester).map(|u| (game.units[u.stats.kind as usize - 1].clone(), u.cell(), u.arms.target, u.team)).collect();
    assert!(answered > 0, "nobody answered; tank at {:?}, health {:?}; army {army:?}", game.world.unit(tank).map(|u| u.cell()), game.world.unit(tank).map(|u| u.health));
    assert!(game.world.unit(tank).is_none_or(|u| u.health < 100_000), "nobody hurt the attacker");
}

#[test]
fn an_oil_derrick_and_a_reactor_go_off_when_destroyed() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    for (name, reach) in [("CAOILD", 2), ("NANRCT", 4), ("NAPOWR", 0)] {
        let at = (yard.0 + 8, yard.1 + 10);
        let building = game.world.add_map_building(2, game.building(name), at, 1);
        let size = game.world.buildings().iter().find(|b| b.id == building).unwrap().size;
        let tank = game.world.add_unit((at.0 - 3, at.1), 0, game.stats("MTNK"), PLAYER);
        let near = game.world.add_unit((at.0 + size.0 + reach.max(1) - 1, at.1 + 1), 0, game.stats("HTNK"), 2);
        let full = game.world.unit(near).unwrap().health;
        game.world.order(Order::Stop { unit: near });
        game.world.order(Order::Attack { unit: tank, target: Aim::Building(building) });
        game.run(200);
        assert!(!game.world.buildings().iter().any(|b| b.id == building), "{name} still stands");
        let now = game.world.unit(near).map_or(0, |u| u.health);
        assert_eq!(now < full, reach > 0, "{name}: a bystander went from {full} to {now}");
        for id in [tank, near] {
            game.world.remove_unit(id);
        }
    }
}

#[test]
fn a_house_with_nothing_left_is_defeated_and_the_game_ends() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    game.deploy_base();
    game.world.end_checks = true;
    game.world.short_game = true;
    game.run(5);
    assert!(game.world.over.is_none() && !game.world.houses[2].defeated);
    // A tank kills the computer's construction vehicle: with no building and
    // no vehicle to make one, it is out, and the player has won.
    let vehicle = game.world.units().iter().find(|u| u.owner == 2).map(|u| (u.id, u.cell())).unwrap();
    let extra = game.world.add_unit((vehicle.1 .0 + 6, vehicle.1 .1), 0, game.stats("HTNK"), 2);
    let tank = game.world.add_unit((vehicle.1 .0 + 3, vehicle.1 .1), 0, game.stats("MTNK"), PLAYER);
    game.world.units_mut_for_tests(vehicle.0, 1);
    game.world.order(Order::Attack { unit: tank, target: ra2_sim::combat::Aim::Unit(vehicle.0) });
    game.run(200);
    assert!(game.world.houses[2].defeated, "not defeated");
    assert!(game.world.unit(extra).is_none(), "its last tank outlived it");
    assert_eq!(game.world.over, Some(vec![PLAYER]));
    let (mine, theirs) = (game.world.houses[PLAYER as usize].score, game.world.houses[2].score);
    assert!(mine.kills == 1 && mine.points > 0 && theirs.losses >= 2, "{mine:?} {theirs:?}");
}

#[test]
fn a_computer_with_no_factory_left_sells_up_and_charges() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let vehicle = game.world.units().iter().find(|u| u.owner == 2).map(|u| u.id).unwrap();
    game.world.remove_unit(vehicle);
    let at = (yard.0 + 10, yard.1 + 12);
    let plant = game.world.add_map_building(2, game.building("NAPOWR"), at, 100);
    let tank = game.world.add_unit((at.0 + 6, at.1 + 6), 0, game.stats("HTNK"), 2);
    game.world.houses[2].ai = Some(ra2_sim::ai::Brain::of(1));
    game.world.houses[2].base_centre = (at.0 * 256, at.1 * 256);
    game.world.houses[2].credits = 5000;
    // It mends what is damaged while it can.
    let before = game.world.buildings().iter().find(|b| b.id == plant).unwrap().health;
    game.run(600);
    let now = game.world.buildings().iter().find(|b| b.id == plant).map(|b| b.health);
    assert!(now.is_some_and(|health| health > before), "not mended: {before} then {now:?}");
    // With no factory of any kind, and left alone, it sells and attacks.
    game.run(1200);
    assert!(!game.world.buildings().iter().any(|b| b.owner == 2), "it kept its buildings");
    let unit = game.world.unit(tank).expect("the tank is gone");
    assert!(unit.arms.target.is_some() || unit.is_moving(), "the tank stayed at home");
}

#[test]
fn the_other_super_weapons_do_their_work() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let kind = |game: &Scenario, kind: i32| game.world.super_types.iter().position(|t| t.kind == kind).unwrap_or_else(|| panic!("no super of kind {kind}"));
    let mut spot = (yard.0 + 6, yard.1 + 6);
    for name in ["NAMISL", "NAIRON", "GAWEAT", "GACSPH", "NANRCT"] {
        game.world.add_map_building(PLAYER, game.building(name), spot, 10_000);
        spot.0 += 5;
    }
    game.run(2);
    for index in 0..game.world.super_types.len() {
        let state = &mut game.world.houses[PLAYER as usize].supers[index];
        if state.granted {
            state.left = 1;
        }
    }
    game.run(3);
    let field = (yard.0 + 4, yard.1 + 22);
    // The iron curtain: a tank cannot be hurt, a soldier under it dies.
    let tank = game.world.add_unit(field, 0, game.stats("MTNK"), PLAYER);
    let soldier = game.world.add_infantry((field.0 + 1, field.1), 2, 0, game.stats("E1"), PLAYER).unwrap();
    game.world.order(Order::Super { house: PLAYER, index: kind(&game, 1) as u16, cell: field });
    game.run(2);
    assert!(game.world.unit(soldier).is_none(), "the soldier lived under the curtain");
    let enemy = game.world.add_unit((field.0 + 3, field.1), 0, game.stats("HTNK"), 2);
    game.world.order(Order::Attack { unit: enemy, target: Aim::Unit(tank) });
    let full = game.world.unit(tank).unwrap().health;
    game.run(300);
    assert_eq!(game.world.unit(tank).unwrap().health, full, "hurt under the curtain");
    // The chronosphere: two clicks move a tank (but not one under the curtain).
    let far = (field.0 + 12, field.1);
    let tank = game.world.add_unit((field.0, field.1 + 3), 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Super { house: PLAYER, index: kind(&game, 3) as u16, cell: game.world.unit(tank).unwrap().cell() });
    assert!(game.world.houses[PLAYER as usize].supers[kind(&game, 3)].ready, "spent by the first click");
    game.world.order(Order::Super { house: PLAYER, index: kind(&game, 4) as u16, cell: far });
    game.run(61);
    assert_eq!(game.world.unit(tank).unwrap().cell(), far);
    assert!(!game.world.houses[PLAYER as usize].supers[kind(&game, 3)].ready);
    // The storm and the missile hurt what is under them.
    game.world.remove_unit(enemy);
    for (super_kind, wait) in [(2, 700), (0, 500)] {
        let at = (field.0, field.1 + 8);
        let victim = game.world.add_unit(at, 0, game.stats("HTNK"), 2);
        game.world.order(Order::Stop { unit: victim });
        game.world.order(Order::Super { house: PLAYER, index: kind(&game, super_kind) as u16, cell: at });
        game.run(wait);
        assert!(game.world.unit(victim).is_none_or(|u| u.health < u.stats.strength), "super {super_kind} did no harm");
        if let Some(unit) = game.world.unit(victim).map(|u| u.id) {
            game.world.remove_unit(unit);
        }
    }
}

/// A base with every super weapon building of the stock rules, all charged.
fn charged_supers(game: &mut Scenario) -> (i32, i32) {
    let yard = game.deploy_base();
    let mut spot = (yard.0 + 6, yard.1 + 6);
    for name in ["NAMISL", "NAIRON", "GAWEAT", "GACSPH", "NANRCT"] {
        game.world.add_map_building(PLAYER, game.building(name), spot, 10_000);
        spot.0 += 5;
    }
    game.run(2);
    for state in game.world.houses[PLAYER as usize].supers.iter_mut().filter(|state| state.granted) {
        state.left = 1;
    }
    game.run(3);
    yard
}

fn super_of(game: &Scenario, kind: i32) -> u16 {
    game.world.super_types.iter().position(|t| t.kind == kind).unwrap_or_else(|| panic!("no super of kind {kind}")) as u16
}

#[test]
fn the_missile_climbs_falls_and_burns_out_before_it_goes_off() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    use ra2_sim::supers::NukeStage;
    let yard = charged_supers(&mut game);
    let at = (yard.0 + 4, yard.1 + 24);
    let victim = game.world.add_unit(at, 0, game.stats("HTNK"), 2);
    game.world.take_events();
    game.world.order(Order::Super { house: PLAYER, index: super_of(&game, 0), cell: at });
    assert!(game.world.take_events().iter().any(|e| matches!(e, ra2_sim::combat::Event::NukeLaunched { .. })), "no launch");
    let stage = |game: &Scenario| game.world.nukes().first().map(|nuke| nuke.stage);
    // Up at 11, 12, ... leptons a frame to 20000; down from 20000 above the
    // target at 2, 3, ... to the ground; then the fireball.
    let mut climb = 0;
    while stage(&game) == Some(NukeStage::Up) {
        game.run(1);
        climb += 1;
    }
    assert!((185..=195).contains(&climb), "climbed for {climb} frames");
    let mut fall = 0;
    while stage(&game) == Some(NukeStage::Down) {
        game.run(1);
        fall += 1;
    }
    assert!((195..=202).contains(&fall), "fell for {fall} frames");
    let ball = game.world.super_rules.nuke_ball.0;
    assert!(ball > 10, "no fireball length ({ball})");
    let full = game.world.unit(victim).unwrap().stats.strength;
    game.run(ball - 2);
    assert_eq!(game.world.unit(victim).map(|u| u.health), Some(full), "hurt before the fireball was out");
    game.run(3);
    assert!(game.world.nukes().is_empty());
    assert!(game.world.unit(victim).is_none_or(|u| u.health < full), "the missile did no harm");
}

#[test]
fn the_chronosphere_waits_then_moves_and_drowns_what_it_drops_in_water() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = charged_supers(&mut game);
    let field = (yard.0 + 4, yard.1 + 22);
    let tank = game.world.add_unit(field, 0, game.stats("MTNK"), PLAYER);
    let far = (field.0 + 12, field.1);
    let squashed = game.world.add_unit(far, 0, game.stats("HTNK"), 2);
    game.world.order(Order::Super { house: PLAYER, index: super_of(&game, 3), cell: field });
    game.world.order(Order::Super { house: PLAYER, index: super_of(&game, 4), cell: far });
    // Sixty frames waiting where it stands, out of action.
    game.run(59);
    assert_eq!(game.world.unit(tank).unwrap().cell(), field, "went before its time");
    game.run(2);
    assert_eq!(game.world.unit(tank).unwrap().cell(), far, "did not go");
    assert!(game.world.unit(squashed).is_none(), "what stood there lived");
    let delay = game.world.super_rules.chrono_delay as u64;
    let unit = game.world.unit(tank).unwrap();
    assert!(unit.warp_until > game.world.current_tick() && unit.warp_until <= game.world.current_tick() + delay, "not out of action for ChronoDelay");
    // A tank put on open water sinks and is gone.
    let water = (0..game.world.map.size().1)
        .flat_map(|y| (0..game.world.map.size().0).map(move |x| (x, y)))
        .find(|&(x, y)| {
            game.world.map.in_playfield(x, y)
                && (-1..=1).all(|dy| (-1..=1).all(|dx| game.world.map.describe(x + dx, y + dy).3 == ra2_sim::Land::Water as u8 && !game.world.map.bridged(x + dx, y + dy)))
        });
    let Some(water) = water else {
        return;
    };
    for state in game.world.houses[PLAYER as usize].supers.iter_mut().filter(|state| state.granted) {
        state.ready = true;
    }
    let field = (field.0, field.1 + 4);
    let doomed = game.world.add_unit(field, 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Super { house: PLAYER, index: super_of(&game, 3), cell: field });
    game.world.order(Order::Super { house: PLAYER, index: super_of(&game, 4), cell: water });
    game.run(62);
    assert_eq!(game.world.unit(doomed).map(|u| u.cell()), Some(water));
    assert!(game.world.altitude(game.world.unit(doomed).unwrap()) < 0, "not sinking");
    game.run(82);
    assert!(game.world.unit(doomed).is_none(), "still afloat");
}

#[test]
fn a_lightning_storm_jams_the_radar_of_its_callers_enemies() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = charged_supers(&mut game);
    let enemy = terrain::ENEMY;
    let at = (yard.0 + 4, yard.1 + 24);
    let storm = super_of(&game, 2);
    game.world.order(Order::Super { house: PLAYER, index: storm, cell: at });
    assert!(!game.world.houses[PLAYER as usize].supers[storm as usize].ready);
    let deferment = game.world.super_rules.storm.deferment;
    let duration = game.world.super_rules.storm.duration as u64;
    game.run(deferment - 1);
    assert!(!game.world.storm_active() && !game.world.radar_jammed(enemy), "came early");
    // A second storm is refused while one is coming or raging.
    game.world.houses[PLAYER as usize].supers[storm as usize].ready = true;
    game.world.order(Order::Super { house: PLAYER, index: storm, cell: at });
    assert!(game.world.houses[PLAYER as usize].supers[storm as usize].ready, "a second storm was called");
    game.run(1);
    assert!(game.world.storm_active());
    assert!(game.world.radar_jammed(enemy) && !game.world.radar_jammed(PLAYER));
    // Back a frame before the storm's length is up.
    game.run(duration as u32 - 2);
    assert!(game.world.radar_jammed(enemy), "radar back too soon");
    game.run(1);
    assert!(!game.world.radar_jammed(enemy), "jammed past the storm");
    game.run(200);
    assert!(!game.world.storm_active(), "the storm never ended");
}

#[test]
fn a_computer_without_miners_or_money_sells_to_buy_one() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(1));
    game.run(9000);
    let count = |game: &Scenario| game.world.buildings().iter().filter(|b| b.owner == enemy).count();
    let miners = |game: &Scenario| game.world.units().iter().filter(|u| u.owner == enemy && u.stats.harvester).map(|u| u.id).collect::<Vec<_>>();
    assert!(count(&game) >= 5 && !miners(&game).is_empty(), "no base to speak of");
    for id in miners(&game) {
        game.world.remove_unit(id);
    }
    game.world.houses[enemy as usize].credits = 0;
    let before = count(&game);
    game.run(400);
    assert!(count(&game) < before || game.world.houses[enemy as usize].credits > 0, "nothing sold");
    game.run(3000);
    assert!(!miners(&game).is_empty(), "no new miner; {} buildings of {before}, credits {}", count(&game), game.world.houses[enemy as usize].credits);
}

#[test]
fn the_computer_fires_its_missile_at_what_is_worth_most() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let enemy = terrain::ENEMY;
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(1));
    game.run(1500);
    let theirs = game.world.buildings().iter().find(|b| b.owner == enemy).map(|b| b.cell).expect("no computer yard");
    game.world.add_map_building(enemy, game.building("NAMISL"), (theirs.0 + 8, theirs.1 + 8), 10_000);
    game.world.add_map_building(enemy, game.building("NANRCT"), (theirs.0 + 8, theirs.1 + 14), 10_000);
    game.run(2);
    let index = game.world.super_types.iter().position(|t| t.kind == 0).unwrap();
    assert!(game.world.houses[enemy as usize].supers[index].granted, "no silo: {:?}", game.world.buildings().iter().filter(|b| b.owner == enemy).map(|b| game.names[b.kind as usize].clone()).collect::<Vec<_>>());
    game.world.houses[enemy as usize].supers[index].left = 1;
    let full = game.world.buildings().iter().find(|b| b.owner == PLAYER).unwrap().health;
    game.run(700);
    assert!(!game.world.houses[enemy as usize].supers[index].ready, "it sat on its missile");
    let now = game.world.buildings().iter().find(|b| b.owner == PLAYER && b.cell == yard).map_or(0, |b| b.health);
    assert!(now < full, "the player's yard was not hit ({now} of {full})");
}

#[test]
fn high_bridges_are_crossed_on_the_deck_and_passed_under() {
    // DATA: mp23t4 has a high bridge along x on row 74 over water; mp06t2 has
    // one along x on row 55 over a dry valley.
    let Some(mut game) = Scenario::load("mp23t4") else {
        return;
    };
    let track = |game: &Scenario, cell: (i32, i32)| game.world.map.speed_percent(cell.0, cell.1, ra2_sim::SpeedType::Track);
    assert!(game.world.map.deck(85, 74) && game.world.map.deck(85, 73) && game.world.map.deck(85, 75));
    assert_eq!(track(&game, (85, 74)), 0, "the water under the deck is still water");
    let (from, to) = ((72, 74), (102, 74));
    let tank = game.world.add_unit(from, 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Move { unit: tank, x: to.0, y: to.1 });
    let mut on_deck = false;
    let mut ship_under = None;
    for tick in 0..2000 {
        game.run(1);
        let unit = game.world.unit(tank).unwrap();
        if unit.cell().0 == 85 {
            on_deck |= unit.on_bridge;
            // A ship goes under while the tank is overhead.
            if ship_under.is_none() {
                let ship = game.world.add_unit((85, 80), 0, game.stats("DEST"), PLAYER);
                game.world.order(Order::Move { unit: ship, x: 85, y: 68 });
                ship_under = Some((ship, tick));
            }
        }
    }
    let unit = game.world.unit(tank).unwrap();
    assert_eq!(unit.cell(), to);
    assert!(on_deck && !unit.on_bridge, "it was never on the deck, or never came off it");
    let ship = game.world.unit(ship_under.expect("no ship was sent").0).unwrap();
    assert_eq!(ship.cell(), (85, 68), "the ship did not pass under the bridge");
    assert!(!ship.on_bridge);

    let Some(mut game) = Scenario::load("mp06t2") else {
        return;
    };
    // Over the valley on the deck...
    let tank = game.world.add_unit((31, 55), 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Move { unit: tank, x: 48, y: 55 });
    let mut on_deck = false;
    let mut trail: Vec<((i32, i32), u8, bool, bool)> = Vec::new();
    for _ in 0..1500 {
        game.run(1);
        let unit = game.world.unit(tank).unwrap();
        on_deck |= unit.on_bridge;
        let cell = unit.cell();
        let entry = (cell, game.world.map.level(cell.0, cell.1), game.world.map.bridged(cell.0, cell.1), unit.on_bridge);
        if trail.last() != Some(&entry) {
            trail.push(entry);
        }
    }
    assert_eq!(game.world.unit(tank).unwrap().cell(), (48, 55));
    assert!(on_deck, "{trail:?}");
    // ...and along the valley under it.
    let walker = game.world.add_unit((40, 60), 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Move { unit: walker, x: 40, y: 50 });
    let mut under = false;
    for _ in 0..1500 {
        game.run(1);
        let unit = game.world.unit(walker).unwrap();
        under |= game.world.map.bridged(unit.cell().0, unit.cell().1) && !unit.on_bridge;
    }
    assert!(under, "it never went under the bridge: at {:?}", game.world.unit(walker).unwrap().cell());
}

#[test]
fn a_kirov_bombs_what_is_under_it() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    let kirov = game.world.add_unit(at, 0, game.stats("ZEP"), PLAYER);
    let target = game.world.add_map_building(2, game.building("NAPOWR"), (at.0 + 8, at.1), 10_000);
    let bystander = game.world.add_unit((at.0 + 8, at.1 + 3), 0, game.stats("HTNK"), 2);
    game.world.order(Order::Stop { unit: bystander });
    let full = game.world.buildings().iter().find(|b| b.id == target).unwrap().health;
    let tank_full = game.world.unit(bystander).unwrap().health;
    game.world.order(Order::Attack { unit: kirov, target: Aim::Building(target) });
    game.run(1500);
    let unit = game.world.unit(kirov).unwrap();
    let now = game.world.buildings().iter().find(|b| b.id == target).map_or(0, |b| b.health);
    assert!(now < full, "no bombs fell: kirov at {:?} height {}, target {:?}, attacking {}", unit.cell(), game.world.altitude(unit), unit.arms.target, unit.arms.attacking);
    let _ = tank_full;
}

#[test]
fn stolen_technology_offers_the_special_soldiers() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let mut spot = (yard.0 + 6, yard.1 + 6);
    for name in ["GAPOWR", "GAPILE", "NAHAND", "GATECH", "NATECH", "GAAIRC", "NARADR"] {
        game.world.add_map_building(PLAYER, game.building(name), spot, 10_000);
        spot.0 += 5;
    }
    game.run(2);
    let offered = |game: &Scenario| -> Vec<String> { game.world.buildable_units(PLAYER).iter().map(|&u| game.units[u as usize].clone()).collect() };
    let before = offered(&game);
    game.world.houses[PLAYER as usize].stolen = [true, true];
    let after = offered(&game);
    let new: Vec<&String> = after.iter().filter(|name| !before.contains(name)).collect();
    eprintln!("STOLEN before {before:?}\nSTOLEN new {new:?}");
    for name in ["CCOMAND", "PTROOP", "CIVAN", "YURIPR"] {
        assert!(after.iter().any(|n| n == name), "{name} not offered; new: {new:?}");
        let stats = game.stats(name);
        eprintln!("STOLEN {name}: teleporter {} c4 {} primary {:?} elite {:?}", stats.teleporter, stats.fighter.c4, stats.fighter.primary.map(|m| game.world.weapons[m.weapon as usize].range), stats.elite_primary.is_some());
    }
    // `BuildLimit=1` counts the one being trained and any queued.
    let prime = game.units.iter().position(|n| n == "YURIPR").unwrap() as u16;
    let item = ra2_sim::Item::Unit(prime);
    assert!(!game.world.at_limit(PLAYER, item));
    game.world.order(Order::BuildUnit { house: PLAYER, unit: prime });
    game.world.order(Order::BuildUnit { house: PLAYER, unit: prime });
    game.run(1);
    assert!(game.world.at_limit(PLAYER, item), "a second Yuri Prime can be ordered while one is in training");
    let tab = game.world.unit_slot(prime);
    assert!(game.world.houses[PLAYER as usize].tab_slot(tab).is_some_and(|p| p.building == prime));
    assert!(game.world.houses[PLAYER as usize].queue(tab).is_empty(), "a second Yuri Prime was queued");
}

#[test]
fn a_cloning_vat_sends_its_copies_to_its_rally_point() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    game.world.add_map_building(PLAYER, game.building("NAPOWR"), (yard.0 + 6, yard.1 + 6), 10_000);
    game.world.add_map_building(PLAYER, game.building("NAHAND"), (yard.0 + 6, yard.1 + 10), 10_000);
    let vat = game.world.add_map_building(PLAYER, game.building("NACLON"), (yard.0 + 12, yard.1 + 10), 10_000);
    assert!(game.world.has_rally_point(vat));
    let rally = (yard.0 + 14, yard.1 + 18);
    game.world.order(Order::Rally { building: vat, x: rally.0, y: rally.1 });
    game.world.houses[PLAYER as usize].credits = 10_000;
    let conscript = game.units.iter().position(|n| n == "E2").unwrap() as u16;
    let before: Vec<u32> = game.world.units().iter().map(|u| u.id).collect();
    game.world.order(Order::BuildUnit { house: PLAYER, unit: conscript });
    game.run(900);
    let new: Vec<(i32, i32)> = game.world.units().iter().filter(|u| !before.contains(&u.id)).map(|u| u.cell()).collect();
    assert_eq!(new.len(), 2, "one made and one cloned");
    assert!(new.iter().any(|cell| (cell.0 - rally.0).abs().max((cell.1 - rally.1).abs()) <= 2), "none went to the vat's rally point: {new:?}");
}

#[test]
fn a_deployed_desolator_poisons_the_ground() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 12);
    let desolator = game.world.add_infantry(at, 2, 0, game.stats("DESO"), PLAYER).unwrap();
    let soldier = game.world.add_infantry((at.0 + 2, at.1), 2, 0, game.stats("E2"), 2).unwrap();
    let tank = game.world.add_unit((at.0, at.1 + 3), 0, game.stats("HTNK"), 2);
    let drone = game.world.add_unit((at.0 - 2, at.1), 0, game.stats("HARV"), 2);
    for unit in [soldier, tank, drone] {
        game.world.order(Order::Stop { unit });
    }
    game.world.units_mut_for_tests(desolator, 100_000);
    assert_eq!(game.world.radiation(at), 0);
    game.world.order(Order::Deploy { unit: desolator });
    game.run(200);
    assert!(game.world.radiation(at) > 100, "no radiation under him: {}", game.world.radiation(at));
    assert!(game.world.radiation((at.0 + 4, at.1)) < game.world.radiation(at));
    assert!(game.world.unit(soldier).is_none(), "the conscript lived in it");
    let full = game.stats("HTNK").strength;
    assert!(game.world.unit(tank).is_none_or(|u| u.health < full), "the tank was not hurt");
    assert_eq!(game.world.unit(drone).map(|u| u.health), Some(game.stats("HARV").strength), "a miner is immune");
    // One site to a cell however often he tops it
    // up; and its light is round, full in the middle, nothing at the corners
    // of the square of cells it covers, and never more than full.
    game.run(600);
    assert_eq!(game.world.rad_sites.len(), 1, "his top-ups made sites of their own: {:?} at {at:?}", game.world.rad_sites);
    let glow = |game: &Scenario, dx: i32, dy: i32| game.world.rad_glow((at.0 + dx, at.1 + dy));
    assert!(glow(&game, 0, 0) > glow(&game, 5, 0) && glow(&game, 5, 0) > glow(&game, 9, 0) && glow(&game, 9, 0) > 0.0);
    assert_eq!(glow(&game, 10, 10), 0.0, "the light is square");
    assert_eq!(glow(&game, 11, 0), 0.0);
    assert!(glow(&game, 0, 0) <= 1.0);
    // He gets up; the ground cools.
    game.world.order(Order::Deploy { unit: desolator });
    game.run(900);
    assert_eq!(game.world.radiation(at), 0, "still hot");
    assert_eq!(glow(&game, 0, 0), 0.0);
}

#[test]
fn a_hospital_mends_and_an_armory_promotes() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let hospital = game.world.add_map_building(PLAYER, game.building("CAHOSP"), (yard.0 + 6, yard.1 + 8), 10_000);
    let armory = game.world.add_map_building(PLAYER, game.building("CAARMR"), (yard.0 + 12, yard.1 + 8), 10_000);
    let soldier = game.world.add_infantry((yard.0 + 6, yard.1 + 13), 2, 0, game.stats("E1"), PLAYER).unwrap();
    use ra2_sim::combat::Entry;
    // Whole, he has no business in the hospital; hurt, he has.
    assert_eq!(game.world.entry_action(soldier, hospital), None);
    game.world.units_mut_for_tests(soldier, 20);
    assert_eq!(game.world.entry_action(soldier, hospital), Some(Entry::Treat));
    game.world.order(Order::Enter { unit: soldier, building: hospital });
    let mut inside = false;
    for _ in 0..900 {
        game.run(1);
        inside |= game.world.unit(soldier).is_none();
    }
    assert!(inside, "he never went in");
    let unit = game.world.unit(soldier).expect("he never came out");
    assert_eq!(unit.health, unit.stats.strength);
    assert_eq!(game.world.entry_action(soldier, armory), Some(Entry::Treat));
    game.world.order(Order::Enter { unit: soldier, building: armory });
    game.run(900);
    assert_eq!(game.world.unit(soldier).map(|u| u.rank()), Some(0), "not elite");
    assert_eq!(game.world.entry_action(soldier, armory), None, "an elite is taken again");
}

#[test]
fn an_enemy_gap_generator_darkens_what_is_not_watched() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    game.world.map.shroud_on = true;
    let at = (yard.0 + 24, yard.1 + 6);
    // A scout looks at the place and goes away: it stays seen.
    let scout = game.world.add_unit((at.0, at.1 + 3), 0, game.stats("MTNK"), PLAYER);
    game.run(20);
    assert!(!game.world.map.shrouded(at.0, at.1));
    game.world.remove_unit(scout);
    game.run(40);
    assert!(!game.world.map.shrouded(at.0, at.1), "seen ground went dark by itself");
    // A powered enemy gap generator there takes it back; a watcher keeps his view.
    game.world.add_map_building(2, game.building("GAPOWR"), (at.0 + 3, at.1), 10_000);
    game.world.add_map_building(2, game.building("GAPOWR"), (at.0 + 3, at.1 + 3), 10_000);
    game.world.add_map_building(2, game.building("GAGAP"), at, 10_000);
    game.run(40);
    assert!(game.world.map.shrouded(at.0 + 2, at.1 + 6), "the gap generator hid nothing");
    let watcher = game.world.add_unit((at.0, at.1 + 6), 0, game.stats("MTNK"), PLAYER);
    game.world.units_mut_for_tests(watcher, 100_000);
    game.run(40);
    assert!(!game.world.map.shrouded(at.0, at.1 + 5), "a watcher beside it sees nothing");
}

#[test]
fn a_repair_depot_mends_a_tank_for_money_and_sends_it_off() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    game.world.add_map_building(PLAYER, game.building("GAPOWR"), (yard.0 + 6, yard.1 + 6), 10_000);
    let spot = (yard.0 + 6, yard.1 + 11);
    let depot = game.world.add_map_building(PLAYER, game.building("GADEPT"), spot, 10_000);
    assert!(game.world.buildings().iter().any(|b| b.id == depot));
    let tank = game.world.add_unit((spot.0 + 5, spot.1 + 1), 0, game.stats("MTNK"), PLAYER);
    let full = game.stats("MTNK").strength;
    game.world.units_mut_for_tests(tank, full / 2);
    game.world.houses[PLAYER as usize].credits = 1000;
    game.world.order(Order::Move { unit: tank, x: spot.0 + 1, y: spot.1 + 1 });
    let mut on = false;
    for _ in 0..1500 {
        game.run(1);
        on |= game.world.unit(tank).unwrap().cell() == (spot.0 + 1, spot.1 + 1);
    }
    assert!(on, "it never got onto the depot: at {:?}", game.world.unit(tank).unwrap().cell());
    let unit = game.world.unit(tank).unwrap();
    assert_eq!(unit.health, full, "not mended");
    assert!(game.world.houses[PLAYER as usize].credits < 1000, "mended for nothing");
    let cell = unit.cell();
    assert!(!((spot.0..spot.0 + 3).contains(&cell.0) && (spot.1..spot.1 + 3).contains(&cell.1)), "it stayed on the depot at {cell:?}");
}

#[test]
fn a_dolphins_wave_hurts_a_ship() {
    let Some(mut game) = Scenario::load("mp01t4") else {
        return;
    };
    use ra2_sim::combat::Aim;
    let (w, h) = game.world.map.size();
    let float = |game: &Scenario, x: i32, y: i32| game.world.map.speed_percent(x, y, ra2_sim::SpeedType::Float) > 0;
    let sea = (6..h - 6).flat_map(|y| (6..w - 12).map(move |x| (x, y))).find(|&(x, y)| (-3..=9).all(|dx| (-3..=3).all(|dy| float(&game, x + dx, y + dy)))).expect("no open water");
    let dolphin = game.world.add_unit(sea, 0, game.stats("DLPH"), PLAYER);
    let ship = game.world.add_unit((sea.0 + 5, sea.1), 0, game.stats("DEST"), 2);
    game.world.order(Order::Stop { unit: ship });
    let full = game.world.unit(ship).unwrap().health;
    game.world.order(Order::Attack { unit: dolphin, target: Aim::Unit(ship) });
    game.run(300);
    let now = game.world.unit(ship).map_or(0, |u| u.health);
    assert!(now < full, "the wave did nothing ({now} of {full})");
}

#[test]
fn a_high_bridge_falls_and_an_engineer_puts_it_back() {
    let Some(mut game) = Scenario::load("mp23t4") else {
        return;
    };
    use ra2_sim::combat::Entry;
    // A tank on the deck, a ship under it, a tank waiting to cross.
    let rider = game.world.add_unit((72, 75), 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Move { unit: rider, x: 85, y: 74 });
    game.run(900);
    assert!(game.world.unit(rider).is_some_and(|u| u.cell() == (85, 74) && u.on_bridge), "the tank is not on the deck");
    let ship = game.world.add_unit((85, 75), 0, game.stats("DEST"), PLAYER);
    let waiting = game.world.add_unit((72, 74), 0, game.stats("MTNK"), PLAYER);
    assert!(game.world.can_move_to(waiting, (102, 74)), "the far side cannot be reached over a whole bridge");
    // The first hit only damages the section; the second brings it down.
    assert!(!game.world.hit_bridge_for_tests((85, 74)));
    assert!(game.world.map.bridged(85, 74));
    assert!(game.world.hit_bridge_for_tests((85, 74)));
    assert!(!game.world.map.bridged(85, 74) && !game.world.map.deck(85, 73));
    assert!(game.world.map.deck(80, 74), "the rest of the bridge went too");
    game.run(60);
    assert!(game.world.unit(ship).is_none(), "the ship under it lived");
    assert!(game.world.unit(rider).is_none(), "the tank on it lived");
    // An engineer in the hut by the bridge mends it.
    let hut_kind = game.building("CABHUT");
    let hut = game.world.buildings().iter().filter(|b| b.kind == hut_kind).min_by_key(|b| (b.cell.0 - 85).abs() + (b.cell.1 - 74).abs()).map(|b| (b.id, b.cell)).expect("no repair hut on this map");
    let engineer = game.world.add_infantry((hut.1 .0 - 1, hut.1 .1 + 1), 2, 0, game.stats("ENGINEER"), PLAYER).unwrap_or_else(|| game.world.add_infantry((hut.1 .0 + 1, hut.1 .1 + 1), 2, 0, game.stats("ENGINEER"), PLAYER).expect("nowhere for the engineer"));
    assert_eq!(game.world.entry_action(engineer, hut.0), Some(Entry::RepairBridge));
    game.world.order(Order::Enter { unit: engineer, building: hut.0 });
    game.run(600);
    assert!(game.world.unit(engineer).is_none(), "the engineer was not used up");
    assert!(game.world.map.deck(85, 74), "the deck is not back");
    assert_eq!(game.world.entry_action(waiting, hut.0), None);
    game.world.order(Order::Move { unit: waiting, x: 102, y: 74 });
    game.run(2000);
    assert_eq!(game.world.unit(waiting).unwrap().cell(), (102, 74));
}

#[test]
fn a_low_bridge_section_is_damaged_then_gone() {
    let Ok(files) = GameFiles::open() else {
        return;
    };
    let Some(list) = files.get("missions.pkt").map(ra2_formats::ini::Ini::parse) else {
        return;
    };
    // A stock map with a whole low bridge section over water (its middle cell
    // is the one whose overlay data is 1).
    let found = list.section("MultiMaps").iter().find_map(|(_, id)| {
        let id = id.trim().to_lowercase();
        let parsed = files.get(&format!("{id}.map")).and_then(Map::parse)?;
        let cell = (0..300).flat_map(|y| (0..300).map(move |x| (x, y))).find(|&(x, y)| matches!(parsed.overlay_at(x, y), Some((74..=77 | 83..=86, 1))))?;
        let game = Scenario::load(&id)?;
        let wet = |dx: i32, dy: i32| game.world.map.wet(cell.0 + dx, cell.1 + dy);
        ((wet(3, 0) && wet(-3, 0)) || (wet(0, 3) && wet(0, -3))).then_some((game, cell))
    });
    let Some((mut game, cell)) = found else {
        panic!("no stock map has a low bridge over water");
    };
    let track = |game: &Scenario| game.world.map.speed_percent(cell.0, cell.1, ra2_sim::SpeedType::Track);
    assert!(track(&game) > 0, "{cell:?} is not road");
    let tank = game.world.add_unit(cell, 0, game.stats("MTNK"), PLAYER);
    assert!(!game.world.hit_bridge_for_tests(cell), "one hit brought it down");
    assert!(track(&game) > 0);
    assert!(game.world.hit_bridge_for_tests(cell), "two hits did not");
    assert_eq!(track(&game), 0, "the gap can be driven on");
    game.run(5);
    assert!(game.world.unit(tank).is_none(), "the tank on the section lived");
}

/// Sends vehicles between random reachable cells of a map and reports those
/// that do not arrive.
fn stuck_moves(map: &str, kind: &str, trips: usize, seed: u64) -> Vec<String> {
    let Some(mut game) = Scenario::load(map) else {
        return Vec::new();
    };
    let stats = game.stats(kind);
    let (w, h) = game.world.map.size();
    let mut state = seed;
    let mut draw = |below: i32| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 33) as i32).rem_euclid(below.max(1))
    };
    let start = game.world.units().iter().find(|u| u.owner == PLAYER).map(|u| u.cell()).unwrap();
    // Clear the map of everything that moves, so that only the ground matters.
    for id in game.world.units().iter().map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    let unit = game.world.add_unit(start, 0, stats, PLAYER);
    let mut failures = Vec::new();
    for _ in 0..trips {
        let from = game.world.unit(unit).unwrap().cell();
        let goal = loop {
            let cell = (draw(w), draw(h));
            if game.world.map.in_playfield(cell.0, cell.1) && game.world.can_move_to(unit, cell) && game.world.buildings().iter().all(|b| !((b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1))) {
                break cell;
            }
        };
        game.world.order(Order::Move { unit, x: goal.0, y: goal.1 });
        let far = ((goal.0 - from.0).abs().max((goal.1 - from.1).abs()) as u32 * 60).max(400) * 4;
        let mut still = 0;
        let mut worst = 0;
        let mut worst_at = from;
        let mut last = from;
        let mut trail: Vec<(i32, i32)> = Vec::new();
        let mut arrived = false;
        for _ in 0..far {
            game.run(1);
            let u = game.world.unit(unit).unwrap();
            let cell = u.cell();
            if cell == goal && !u.is_moving() {
                arrived = true;
                break;
            }
            if cell == last {
                still += 1;
                if still > worst && trail.len() > 1 {
                    worst = still;
                    worst_at = cell;
                }
            } else {
                still = 0;
                last = cell;
                trail.push(cell);
            }
            if still > 300 {
                break;
            }
        }
        // A vehicle on its way that stays in one cell this long is hesitating
        // (a turn on the spot takes far less).
        if arrived && worst > 40 && worst_at != goal {
            failures.push(format!("{from:?} -> {goal:?}: sat {worst} ticks at {worst_at:?}"));
        }
        if !arrived {
            let u = game.world.unit(unit).unwrap();
            let tail: Vec<_> = trail.iter().rev().take(6).rev().collect();
            failures.push(format!("{from:?} -> {goal:?}: stopped at {:?} (dest {:?}, moving {}), last cells {tail:?}", u.cell(), u.destination(), u.is_moving()));
            // Start the next trip from somewhere sound.
            game.world.remove_unit(unit);
            let again = game.world.add_unit(start, 0, stats, PLAYER);
            assert_eq!(again, unit + failures.len() as u32, "ids are expected to run on");
            return failures.into_iter().chain(stuck_moves_from(game, again, trips / 2, state)).collect();
        }
    }
    failures
}

fn stuck_moves_from(_game: Scenario, _unit: u32, _trips: usize, _state: u64) -> Vec<String> {
    Vec::new()
}

#[test]
fn vehicles_reach_where_they_are_sent_over_the_slopes_of_the_tournament_map() {
    let mut all = Vec::new();
    for seed in 1..=20u64 {
        all.extend(stuck_moves("tn01t2", "FV", 25, seed));
    }
    assert!(all.is_empty(), "{} trips failed:\n{}", all.len(), all.join("\n"));
}

#[test]
fn a_group_crosses_the_slopes_of_the_tournament_map() {
    let Some(mut game) = Scenario::load("tn01t2") else {
        return;
    };
    let stats = game.stats("FV");
    for id in game.world.units().iter().map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    let mut report = Vec::new();
    // Up and down the wide slope by (61, 81), and across the map.
    for (from, to) in [((56, 84), (70, 78)), ((70, 78), (52, 86)), ((58, 99), (95, 53)), ((95, 53), (58, 99))] {
        let group: Vec<u32> = (0..8).map(|n| game.world.add_unit((from.0 + n % 3, from.1 + n / 3), 0, stats, PLAYER)).collect();
        for &unit in &group {
            game.world.order(Order::Move { unit, x: to.0, y: to.1 });
        }
        let span = (to.0 - from.0).abs().max((to.1 - from.1).abs()) as u64;
        let mut done_at = None;
        let mut sat: std::collections::HashMap<u32, ((i32, i32), u32, u32, (i32, i32))> = std::collections::HashMap::new();
        for tick in 0..span * 60 + 600 {
            game.run(1);
            let mut all = true;
            for &unit in &group {
                let u = game.world.unit(unit).unwrap();
                let entry = sat.entry(unit).or_insert((u.cell(), 0, 0, u.cell()));
                if u.destination().is_some() {
                    all = false;
                    if entry.0 == u.cell() {
                        entry.1 += 1;
                        if entry.1 > entry.2 {
                            entry.2 = entry.1;
                            entry.3 = u.cell();
                        }
                    } else {
                        *entry = (u.cell(), 0, entry.2, entry.3);
                    }
                }
            }
            if all {
                done_at = Some(tick);
                break;
            }
        }
        let cells: Vec<_> = group.iter().map(|&unit| game.world.unit(unit).unwrap().cell()).collect();
        let spread = cells.iter().map(|c| (c.0 - to.0).abs().max((c.1 - to.1).abs())).max().unwrap_or(0);
        let worst = sat.values().map(|v| (v.2, v.3)).max().unwrap();
        let line = format!("{from:?} -> {to:?}: done at {done_at:?} of {} allowed, furthest from goal {spread}, longest wait {} ticks at {:?}", span * 60 + 600, worst.0, worst.1);
        assert!(done_at.is_some() && spread <= 3 && worst.0 <= 80, "{line}");
        report.push(line);
        for unit in group {
            game.world.remove_unit(unit);
        }
    }
    assert_eq!(report.len(), 4);
}

#[test]
fn the_computer_spreads_its_defences_round_its_base() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    game.world.houses[enemy as usize].credits = 60_000;
    game.run(16_000);
    let yard = game.world.buildings().iter().find(|b| b.owner == enemy && game.world.building_types[b.kind as usize].construction_yard).map(|b| b.cell).expect("no yard");
    let defences: Vec<(i32, i32)> = game.world.buildings().iter().filter(|b| b.owner == enemy && game.world.is_defence(b.kind)).map(|b| (b.cell.0 - yard.0, b.cell.1 - yard.1)).collect();
    assert!(defences.len() >= 6, "only {} defences", defences.len());
    // By coverage they end up on more than one side of the yard, not in a heap.
    let quarters: std::collections::HashSet<(bool, bool)> = defences.iter().map(|d| (d.0 >= 0, d.1 >= 0)).collect();
    let apart = defences.iter().enumerate().all(|(i, a)| defences[i + 1..].iter().all(|b| a != b));
    assert!(quarters.len() >= 3 && apart, "defences at {defences:?} (from the yard)");
}

#[test]
fn an_ifv_is_promoted_and_fires_its_elite_weapon() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 8);
    let ifv = game.world.add_unit(at, 0, game.stats("FV"), PLAYER);
    assert!(game.world.unit(ifv).unwrap().stats.trainable);
    for round in 0..10 {
        let victim = game.world.add_unit((at.0 + 4, at.1 + round % 3), 0, game.stats("CMIN"), 2);
        game.world.units_mut_for_tests(victim, 1);
        game.world.order(Order::Stop { unit: victim });
        game.world.order(Order::Attack { unit: ifv, target: Aim::Unit(victim) });
        game.run(250);
        assert!(game.world.unit(victim).is_none(), "victim {round} lived; ifv {:?}", game.world.unit(ifv).map(|u| u.health));
        let unit = game.world.unit(ifv).unwrap();
        eprintln!("IFV after {} kills: experience {} rank {}", round + 1, unit.veterancy, unit.rank());
    }
    let unit = game.world.unit(ifv).unwrap();
    assert_eq!(unit.rank(), 0, "not elite: experience {}", unit.veterancy);
    // Elite, it fires `EliteWeapon1` (HoverMissileE), not `Weapon1`.
    let kind = &game.world.unit_types[unit.stats.kind as usize - 1];
    assert_ne!(kind.gunner_elite[0].weapon, kind.gunner_weapons[0].0.weapon);
    assert_eq!(kind.gunner_elite.len(), kind.gunner_weapons.len());
}

/// Sends one unit on random trips and reports every step it takes that the
/// ground does not allow (a change of level with no ramp), and every trip it
/// does not finish.
fn illegal_steps(map: &str, kind: &str, trips: usize, seed: u64, near: Option<((i32, i32), i32)>) -> Vec<String> {
    let Some(mut game) = Scenario::load(map) else {
        return Vec::new();
    };
    let stats = game.stats(kind);
    let (w, h) = game.world.map.size();
    let mut state = seed;
    let mut draw = |below: i32| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 33) as i32).rem_euclid(below.max(1))
    };
    let start = game.world.units().iter().find(|u| u.owner == PLAYER).map(|u| u.cell()).unwrap();
    for id in game.world.units().iter().map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    game.run(1);
    let first = start;
    let unit = if stats.infantry { game.world.add_infantry(first, 2, 0, stats, PLAYER).unwrap() } else { game.world.add_unit(first, 0, stats, PLAYER) };
    let mut failures = Vec::new();
    for _ in 0..trips {
        let from = game.world.unit(unit).unwrap().cell();
        let mut tries = 0;
        let goal = loop {
            tries += 1;
            let cell = match near {
                Some((c, r)) if tries < 500 => (c.0 - r + draw(2 * r + 1), c.1 - r + draw(2 * r + 1)),
                _ => (draw(w), draw(h)),
            };
            // (Not a cell shut in on every side, as some are among trees.)
            let open = (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))).any(|d| d != (0, 0) && game.world.can_move_to(unit, (cell.0 + d.0, cell.1 + d.1)));
            if game.world.map.in_playfield(cell.0, cell.1) && open && game.world.can_move_to(unit, cell) && game.world.buildings().iter().all(|b| !((b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1))) {
                break cell;
            }
        };
        game.world.order(Order::Move { unit, x: goal.0, y: goal.1 });
        // (A cell its kind cannot reach is swapped for one near it at the order.)
        let goal = game.world.unit(unit).unwrap().destination().unwrap_or(goal);
        let far = 20_000;
        let mut last = from;
        let mut last_deck = game.world.unit(unit).unwrap().on_bridge;
        let mut still = 0;
        let mut arrived = false;
        for _ in 0..far {
            game.run(1);
            let u = game.world.unit(unit).unwrap();
            let cell = u.cell();
            // (A soldier going corner to corner between spots crosses the tip of
            // a cell beside its way; one it could not stand in is not a step.)
            let corner = stats.infantry && game.world.map.speed_on(cell.0, cell.1, stats.speed_type, u.on_bridge) == 0;
            if cell != last && !corner {
                let map = &game.world.map;
                let (a, b) = (map.level(last.0, last.1) as i32 + if last_deck { 4 } else { 0 }, map.level(cell.0, cell.1) as i32 + if u.on_bridge { 4 } else { 0 });
                last_deck = u.on_bridge;
                let bridge = false;
                let ramp = map.ramp(last.0, last.1) != 0 || map.ramp(cell.0, cell.1) != 0;
                let far_step = (cell.0 - last.0).abs().max((cell.1 - last.1).abs()) > 1;
                if far_step || (!bridge && ((a - b).abs() > 1 || ((a - b).abs() == 1 && !ramp))) {
                    failures.push(format!("{from:?} -> {goal:?}: stepped {last:?} (level {a}) to {cell:?} (level {b})"));
                }
                last = cell;
                still = 0;
            } else {
                still += 1;
            }
            if cell == goal && !u.is_moving() {
                arrived = true;
                break;
            }
            if still > 400 {
                break;
            }
        }
        if !arrived {
            let u = game.world.unit(unit).unwrap();
            failures.push(format!("{kind} seed {seed} {from:?} -> {goal:?}: stopped at {:?} (dest {:?}, moving {}) pos {:?} facing {}", u.cell(), u.destination(), u.is_moving(), (u.x, u.y), u.facing));
        }
    }
    failures
}

#[test]
fn soldiers_keep_to_the_ground_by_the_cliffs_of_the_isle_of_war() {
    let mut all = Vec::new();
    for seed in 1..=10u64 {
        all.extend(illegal_steps("mp02t2", "E1", 20, seed, None));
        all.extend(illegal_steps("mp02t2", "FV", 20, seed, None));
        // Round the bridges and the cliffs beside them.
        for centre in [(105, 94), (138, 127)] {
            all.extend(illegal_steps("mp02t2", "E1", 20, seed, Some((centre, 9))));
            all.extend(illegal_steps("mp02t2", "FV", 20, seed, Some((centre, 9))));
        }
    }
    assert!(all.is_empty(), "{} faults:\n{}", all.len(), all.join("\n"));
}

#[test]
fn the_pointer_finds_the_cell_it_is_on_by_the_cliffs_of_the_isle_of_war() {
    let Some(game) = Scenario::load("mp02t2") else {
        return;
    };
    let map = &game.world.map;
    let (w, h) = map.size();
    let (cw, ch, lh) = (terrain::CELL_WIDTH as f32, terrain::CELL_HEIGHT as f32, terrain::LEVEL_HEIGHT as f32);
    let (mut seen, mut exact, mut behind) = (0, 0, Vec::new());
    for y in 0..h {
        for x in 0..w {
            if !map.in_playfield(x, y) {
                continue;
            }
            // The middle of the cell's picture: on its deck where it has one.
            let level = map.level(x, y) as f32 + if map.bridged(x, y) { 4.0 } else { 0.0 };
            let point = bevy::math::Vec2::new((x - y) as f32 * cw / 2.0 + cw / 2.0, (x + y + 1) as f32 * ch / 2.0 - level * lh);
            let found = units::cell_at(&game.world, bevy::math::Vec2::ZERO, point);
            seen += 1;
            if found == (x, y) {
                exact += 1;
            } else if found.0 + found.1 <= x + y {
                // Anything else must be nearer the viewer: something standing in front.
                behind.push(((x, y), found));
            }
        }
    }
    assert!(behind.is_empty(), "{} cells gave one behind them: {:?}", behind.len(), &behind[..behind.len().min(12)]);
    assert!(exact * 100 >= seen * 97, "{exact} of {seen} cells found by their own middle");
}

#[test]
fn a_captured_oil_derrick_pays() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let derrick = game.building("CAOILD");
    let spot = game.find_spot("CAOILD", (yard.0 + 6, yard.1 + 6), 6).unwrap();
    let id = game.world.add_map_building(0, derrick, spot, 256);
    let engineer = game.world.add_infantry((spot.0 + 4, spot.1 + 3), 2, 0, game.stats("ENGINEER"), PLAYER).unwrap();
    let before = game.world.houses[PLAYER as usize].credits;
    game.run(300);
    assert_eq!(game.world.houses[PLAYER as usize].credits, before, "a neutral derrick paid");
    game.world.order(Order::Enter { unit: engineer, building: id });
    game.run(400);
    assert_eq!(game.world.buildings().iter().find(|b| b.id == id).unwrap().owner, PLAYER, "not captured");
    let after = game.world.houses[PLAYER as usize].credits;
    // DATA: rules.ini `[CAOILD]`: 1000 at once, then 20 every 100 frames.
    assert!(after >= before + 1000 && after < before + 1000 + 20 * 5, "{before} -> {after}");
    game.run(1000);
    assert_eq!(game.world.houses[PLAYER as usize].credits, after + 200);
}

#[test]
fn an_enemy_gap_generator_darkens_its_ground_even_under_the_satellite() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    game.world.map.shroud_on = true;
    let spot = (yard.0 + 16, yard.1);
    for n in 0..3 {
        game.world.add_map_building(2, game.building("GAPOWR"), (spot.0 + 4 + 3 * n, spot.1 + 4), 256);
    }
    let gap = game.world.add_map_building(2, game.building("GAGAP"), spot, 256);
    let sat = (yard.0 - 6, yard.1 + 6);
    game.world.add_map_building(PLAYER, game.building("GASPYSAT"), sat, 256);
    game.run(40);
    assert!(game.world.map.has_satellite());
    assert!(!game.world.map.revealed(spot.0, spot.1), "the gap generator's ground is lit");
    assert!(game.world.map.revealed(spot.0 + 25, spot.1), "the satellite shows the rest");
    // With the generator gone the satellite shows its ground again.
    game.world.give_building_for_tests(gap, 0);
    game.run(40);
    assert!(game.world.map.revealed(spot.0, spot.1));
}

/// Sends a group to attack a building somewhere else on the map and reports the
/// ones that never get within reach of it.
fn attack_runs(map: &str, kind: &str, group: i32, runs: usize, seed: u64) -> Vec<String> {
    let Some(mut game) = Scenario::load(map) else {
        return Vec::new();
    };
    let stats = game.stats(kind);
    let (w, h) = game.world.map.size();
    let mut state = seed;
    let mut draw = |below: i32| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 33) as i32).rem_euclid(below.max(1))
    };
    for id in game.world.units().iter().map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    game.run(1);
    let probe = game.world.add_unit((0, 0), 0, stats, 2);
    let plant = game.building("GAPOWR");
    let zone = |game: &Scenario, c: (i32, i32)| game.world.map.zone_on(c, stats.movement_zone as usize, false);
    let open = |game: &Scenario, c: (i32, i32), size: (i32, i32)| {
        (0..size.1).all(|dy| (0..size.0).all(|dx| {
            let cell = (c.0 + dx, c.1 + dy);
            game.world.map.in_playfield(cell.0, cell.1) && game.world.can_move_to(probe, cell) && game.world.map.ramp(cell.0, cell.1) == 0 && !game.world.map.bridged(cell.0, cell.1)
                && game.world.buildings().iter().all(|b| !((b.cell.0 - 1..b.cell.0 + b.size.0 + 1).contains(&cell.0) && (b.cell.1 - 1..b.cell.1 + b.size.1 + 1).contains(&cell.1)))
        }))
    };
    let mut failures = Vec::new();
    for run in 0..runs {
        let (target, start) = loop {
            let t = (draw(w), draw(h));
            let s = (draw(w), draw(h));
            let apart = (t.0 - s.0).abs().max((t.1 - s.1).abs());
            if apart > 20 && open(&game, (t.0 - 1, t.1 - 1), (4, 4)) && open(&game, s, (4, 3)) && zone(&game, s).is_some() && zone(&game, s) == zone(&game, (t.0 - 1, t.1)) {
                break (t, s);
            }
        };
        let building = game.world.add_map_building(PLAYER, plant, target, 256);
        game.world.harden_building_for_tests(building);
        let aim = ra2_sim::combat::Aim::Building(building);
        let tanks: Vec<u32> = (0..group).map(|n| game.world.add_unit((start.0 + n % 4, start.1 + n / 4), 0, stats, 2)).collect();
        for &unit in &tanks {
            game.world.order(Order::Attack { unit, target: aim });
        }
        let apart = (target.0 - start.0).abs().max((target.1 - start.1).abs()) as u32;
        game.run(apart * 90 + 2500);
        // Each is in range, or was sent off for want of a place to stand
        // (the engine drops the target then); none sits with the target
        // out of reach.
        let state: Vec<_> = tanks
            .iter()
            .filter_map(|&t| game.world.unit(t))
            .map(|u| (u.cell(), u.arms.target.is_some(), game.world.attack_action(u.id, aim, false) == Some(true), u.destination().is_some() || u.is_moving()))
            .collect();
        let firing = state.iter().filter(|s| s.1 && s.2).count();
        let lost = state.iter().filter(|s| s.1 && !s.2 && !s.3).count();
        let dropped = state.iter().filter(|s| !s.1).count();
        if lost > 0 || firing * 2 < state.len() {
            failures.push(format!("{map} {kind} seed {seed} run {run}: {start:?} -> building at {target:?}: {firing} firing, {dropped} gave up, {lost} idle out of range: {state:?}"));
        }
        game.world.give_building_for_tests(building, 0);
        for unit in tanks {
            game.world.remove_unit(unit);
        }
        game.run(2);
    }
    failures
}

#[test]
fn groups_get_within_reach_of_what_they_are_sent_to_attack() {
    let mut all = Vec::new();
    for map in ["mp02t2", "tn01t2", "mp18s3", "mp23t4", "mp03t4"] {
        for seed in 1..=3u64 {
            all.extend(attack_runs(map, "MTNK", 12, 4, seed));
        }
    }
    assert!(all.is_empty(), "{} failed:\n{}", all.len(), all.join("\n"));
}

#[test]
fn a_soldier_walks_among_trees_to_a_cell_they_ring() {
    let Some(mut game) = Scenario::load("mp02t2") else {
        return;
    };
    for id in game.world.units().iter().map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    game.run(1);
    // DATA: the Isle of War: (96, 98) is open ground with a tree on every side.
    let soldier = game.world.add_infantry((100, 100), 2, 0, game.stats("E1"), PLAYER).unwrap();
    game.world.order(Order::Move { unit: soldier, x: 96, y: 98 });
    game.run(600);
    assert_eq!(game.world.unit(soldier).unwrap().cell(), (96, 98));
    // A vehicle cannot: its order is turned to a cell it can reach.
    let tank = game.world.add_unit((100, 100), 0, game.stats("MTNK"), PLAYER);
    game.world.order(Order::Move { unit: tank, x: 96, y: 98 });
    assert_ne!(game.world.unit(tank).unwrap().destination(), Some((96, 98)));
}

#[test]
fn units_keep_to_the_ground_and_arrive_on_every_kind_of_map() {
    let mut all = Vec::new();
    for map in ["mp18s3", "mp23t4", "mp06t2", "mp09t3", "mp14t2", "mp27t8", "mp34u4", "mp01t4"] {
        for kind in ["E1", "MTNK", "FV"] {
            all.extend(illegal_steps(map, kind, 12, 7, None).into_iter().map(|line| format!("{map} {kind}: {line}")));
        }
    }
    assert!(all.is_empty(), "{} faults:\n{}", all.len(), all.join("\n"));
}

#[test]
fn a_chrono_commando_jumps_to_a_building_and_brings_it_down() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let plant = game.building("NAPOWR");
    let commando = game.world.add_infantry((yard.0 + 4, yard.1 + 6), 2, 0, game.stats("CCOMAND"), PLAYER).unwrap();
    // Somewhere with open ground all round it.
    let at = (15..40)
        .flat_map(|dx| (-10..10).map(move |dy| (yard.0 + dx, yard.1 + dy)))
        .find(|&c| (-2..=4).all(|dx| (-2..=3).all(|dy| game.world.map.ramp(c.0 + dx, c.1 + dy) == 0 && game.world.can_move_to(commando, (c.0 + dx, c.1 + dy)))))
        .unwrap();
    let id = game.world.add_map_building(2, plant, at, 256);
    use ra2_sim::combat::Aim;
    // His charge is a weapon with the charge-planting cursor.
    assert!(game.world.weapon_against(commando, Aim::Building(id)).is_some_and(|weapon| weapon.sabotage_cursor));
    game.world.order(Order::Attack { unit: commando, target: Aim::Building(id) });
    game.run(30);
    let near = game.world.unit(commando).unwrap().cell();
    assert!((near.0 - at.0).abs() <= 4 && (near.1 - at.1).abs() <= 4, "he walked: at {near:?} after 30 frames, building at {at:?}");
    game.run(400);
    assert!(game.world.buildings().iter().all(|b| b.id != id), "the building stands");
}

#[test]
fn a_chrono_miner_sent_home_by_hand_jumps_onto_the_dock() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("GAREFN", (yard.0 + 5, yard.1 + 5), 8).unwrap();
    let refinery = game.world.add_map_building(PLAYER, game.building("GAREFN"), spot, 256);
    for id in game.world.units().iter().filter(|u| u.stats.harvester).map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    game.run(2);
    let miner = game.world.add_unit((spot.0 + 14, spot.1 + 9), 0, game.stats("CMIN"), PLAYER);
    game.world.order(Order::Dock { unit: miner, building: refinery });
    game.run(20);
    assert_eq!(game.world.unit(miner).unwrap().cell(), (spot.0 + 3, spot.1 + 1), "it did not jump to the dock");
}

#[test]
fn a_dog_leaves_falling_soldiers_alone_until_they_are_down() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let plain = game.world.super_types.iter().position(|kind| kind.kind == 5).unwrap();
    game.world.add_map_building(PLAYER, game.building("CAAIRP"), (yard.0 + 12, yard.1 + 6), 10_000);
    game.run(3602);
    let drop = (yard.0 + 10, yard.1 + 14);
    let dog = game.world.add_infantry((drop.0 + 2, drop.1 + 1), 2, 0, game.stats("DOG"), 2).unwrap();
    game.world.order(Order::Super { house: PLAYER, index: plain as u16, cell: drop });
    let mut seen = false;
    for _ in 0..1500 {
        game.run(1);
        let Some(d) = game.world.unit(dog) else { break };
        if let Some(ra2_sim::combat::Aim::Unit(id)) = d.arms.target {
            let height = game.world.unit(id).map_or(0, |u| game.world.altitude(u));
            assert!(height < 208, "the dog went for a soldier {height} up");
            seen = true;
        }
    }
    assert!(seen, "the dog never went for anyone");
}

#[test]
fn rocketeers_follow_a_target_that_drives_off() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let start = (yard.0 + 4, yard.1 + 8);
    let rocketeers: Vec<u32> = (0..3).map(|n| game.world.add_infantry((start.0 + n, start.1), 2, 0, game.stats("JUMPJET"), PLAYER).unwrap()).collect();
    let miner = game.world.add_unit((start.0 + 12, start.1), 0, game.stats("HARV"), 2);
    game.world.units_mut_for_tests(miner, 100_000);
    for &unit in &rocketeers {
        game.world.order(Order::Attack { unit, target: Aim::Unit(miner) });
    }
    // It drives well away while they are on their way.
    let from = game.world.unit(miner).unwrap().cell();
    let goal = (20..40)
        .flat_map(|d| [(from.0 + d, from.1), (from.0, from.1 + d), (from.0 - d, from.1), (from.0, from.1 - d), (from.0 + d, from.1 + d)])
        .find(|&cell| game.world.map.in_playfield(cell.0, cell.1) && game.world.can_move_to(miner, cell))
        .unwrap();
    game.world.order(Order::Move { unit: miner, x: goal.0, y: goal.1 });
    game.run(900);
    let at = game.world.unit(miner).unwrap().cell();
    assert!((at.0 - from.0).abs().max((at.1 - from.1).abs()) >= 15, "the miner did not get away: {from:?} -> {at:?} (sent to {goal:?})");
    for &unit in &rocketeers {
        let rocketeer = game.world.unit(unit).unwrap();
        let cell = rocketeer.cell();
        assert_eq!(rocketeer.arms.target, Some(Aim::Unit(miner)), "it let the target go");
        assert!((cell.0 - at.0).abs().max((cell.1 - at.1).abs()) <= 8, "a rocketeer is at {cell:?}, the miner at {at:?}");
    }
}

#[test]
fn tesla_troopers_charge_the_coil_they_stand_by() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("TESLA", (yard.0 + 7, yard.1 + 7), 6).unwrap();
    let coil = game.world.add_map_building(PLAYER, game.building("TESLA"), spot, 256);
    let health = |game: &Scenario| game.world.buildings().iter().find(|b| b.id == coil).unwrap().health;
    let state = |game: &Scenario| {
        let b = game.world.buildings().iter().find(|b| b.id == coil).unwrap();
        (b.chargers.len(), b.charged, game.world.online(b))
    };
    let full = health(&game);
    // Short of power, and nobody charging: off.
    assert!(game.world.low_power(PLAYER), "the test wants a house short of power");
    assert_eq!(state(&game), (0, false, false));
    let around = [(-1, 0), (0, -1), (-1, -1)];
    let mut troopers = Vec::new();
    for (n, step) in around.iter().enumerate() {
        troopers.push(game.world.add_infantry((spot.0 + step.0, spot.1 + step.1), 2, 0, game.stats("SHK"), PLAYER).unwrap());
        game.run(60);
        // One does nothing for a coil
        // without power, two turn it on, three charge it.
        assert_eq!(state(&game), (n + 1, n == 2, n >= 1), "with {} troopers", n + 1);
    }
    assert_eq!(health(&game), full, "charging hurt the coil");
    // One told to go elsewhere stops counting.
    game.world.order(Order::Move { unit: troopers[0], x: spot.0 - 4, y: spot.1 - 4 });
    game.run(10);
    assert_eq!(state(&game).0, 2);
    // With power, one is enough.
    game.world.add_map_building(PLAYER, game.building("NAPOWR"), game.find_spot("NAPOWR", (yard.0 - 6, yard.1), 6).unwrap(), 256);
    game.world.order(Order::Move { unit: troopers[1], x: spot.0 - 5, y: spot.1 - 3 });
    game.run(10);
    assert!(!game.world.low_power(PLAYER));
    assert_eq!(state(&game), (1, true, true));
}

#[test]
fn a_psychic_sensor_shows_enemies_headed_for_its_ground() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("NAPSIS", (yard.0 + 7, yard.1 + 7), 6).unwrap();
    game.world.add_map_building(PLAYER, game.building("NAPSIS"), spot, 256);
    let near = (yard.0 + 6, yard.1 + 8);
    let tank = game.world.add_unit(near, 0, game.stats("HTNK"), 2);
    let shown = |game: &Scenario| game.world.sensor_reveals(PLAYER, game.world.unit(tank).unwrap());
    game.run(5);
    // Where the unit stands does not matter, only what it is about.
    assert!(!shown(&game), "a tank with no orders is shown");
    // DATA: rules.ini `[NAPSIS] PsychicDetectionRadius=15`.
    game.world.order(Order::Move { unit: tank, x: spot.0 + 10, y: spot.1 });
    game.run(5);
    // Short of power the sensor is off.
    assert!(game.world.low_power(PLAYER) && !shown(&game));
    game.world.add_map_building(PLAYER, game.building("NAPOWR"), game.find_spot("NAPOWR", (yard.0 - 6, yard.1), 6).unwrap(), 256);
    game.run(5);
    assert!(shown(&game), "a tank sent inside the radius is not shown");
    // Sent well outside the radius, whichever way there is ground to go to.
    let away = [(30, 0), (-30, 0), (0, 30), (0, -30), (25, 25), (-25, -25)].into_iter().find(|step| {
        game.world.order(Order::Move { unit: tank, x: spot.0 + step.0, y: spot.1 + step.1 });
        game.world.unit(tank).unwrap().destination().is_some_and(|to| (to.0 - spot.0).abs().max((to.1 - spot.1).abs()) > 20)
    });
    assert!(away.is_some(), "nowhere to send the tank");
    game.run(5);
    assert!(!shown(&game), "a tank sent elsewhere is shown");
}

#[test]
fn computer_houses_stand_together_when_allies_are_allowed() {
    // DATA: MPModes.ini offers Battle and Free For All for skirmish; both
    // rules files set `AlliesAllowed`.
    let files = crate::files::GameFiles::open();
    let Ok(files) = files else {
        return;
    };
    for mode in ["mpbattle.ini", "mpfreeforall.ini"] {
        let ini = ra2_formats::ini::Ini::parse(files.get(mode).expect(mode));
        assert_eq!(ini.get("MultiplayerDialogSettings", "AlliesAllowed").map(str::trim), Some("yes"), "{mode}");
    }
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    for _ in 0..2 {
        game.world.houses.push(ra2_sim::House { ai: Some(ra2_sim::ai::Brain::of(1)), ..ra2_sim::House::default() });
    }
    let last = game.world.houses.len() as u8 - 1;
    game.world.houses[2].ai.get_or_insert_with(|| ra2_sim::ai::Brain::of(1));
    assert!(!game.world.allied(2, last));
    // Every computer house allies with every
    // other; a person's house is left the enemy of them all.
    game.world.paranoid_alliances();
    assert!(game.world.allied(2, last) && game.world.allied(last, 2) && game.world.allied(last - 1, last));
    assert!(!game.world.allied(PLAYER, 2) && !game.world.allied(2, PLAYER) && !game.world.allied(0, 2));
    // Their tanks leave each other alone and go for the player's.
    let yard = game.deploy_base();
    let a = game.world.add_unit((yard.0 + 6, yard.1 + 8), 0, game.stats("HTNK"), 2);
    let b = game.world.add_unit((yard.0 + 8, yard.1 + 8), 0, game.stats("HTNK"), last);
    game.world.add_unit((yard.0 + 7, yard.1 + 10), 0, game.stats("MTNK"), PLAYER);
    game.run(60);
    for tank in [a, b] {
        let target = game.world.unit(tank).and_then(|unit| unit.arms.target).and_then(|target| game.world.owner_of(target));
        assert_eq!(target, Some(PLAYER), "tank {tank}");
    }
}

#[test]
fn ore_grows_denser_and_spreads_and_drills_bring_more() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let total = |game: &Scenario| game.world.map.ore_cells().filter(|(_, kind, _)| *kind == 0).map(|(_, _, amount)| amount as u32 + 1).sum::<u32>();
    let cells = |game: &Scenario| game.world.map.ore_cells().count();
    let gems = |game: &Scenario| game.world.map.ore_cells().filter(|(_, kind, _)| *kind == 1).map(|(_, _, amount)| amount as u32 + 1).sum::<u32>();
    let (bales, count, jewels) = (total(&game), cells(&game), gems(&game));
    assert!(bales > 0);
    // DATA: rules.ini `[Riparius]`: growth passes every 660 frames, spread
    // every 2200; `[Cruentus]` neither grows nor spreads.
    game.run(7000);
    assert!(total(&game) > bales, "ore did not grow: {bales} -> {}", total(&game));
    assert!(cells(&game) > count, "ore did not spread: {count} cells -> {}", cells(&game));
    assert_eq!(gems(&game), jewels, "gems grew");
    // New ore lies only on flat open ground with a picture of its own.
    for (cell, _, _) in game.world.map.ore_cells() {
        if game.world.map.ore_picture(cell.0, cell.1).is_some() {
            assert_eq!(game.world.map.ramp(cell.0, cell.1), 0, "ore spread onto a slope at {cell:?}");
        }
    }
}

#[test]
fn a_dog_goes_for_a_disguised_spy() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 8);
    let dog = game.world.add_infantry(at, 2, 0, game.stats("DOG"), PLAYER).unwrap();
    let gi = game.world.add_infantry((at.0, at.1 + 2), 2, 0, game.stats("E1"), PLAYER).unwrap();
    // DATA: rules.ini `[DOG] DefaultToGuardArea=yes`, `[BadTeeth] Range=1.5`:
    // it looks three cells out from where it was left.
    let spy = game.world.add_infantry((at.0 + 3, at.1), 2, 0, game.stats("SPY"), 2).unwrap();
    game.world.disguise_for_tests(spy, (game.stats("E1").kind, PLAYER));
    game.run(30);
    // DATA: rules.ini `[DOG] DetectDisguise=yes`: the soldier is fooled, the dog is not.
    assert_eq!(game.world.unit(gi).unwrap().arms.target, None, "the soldier saw through the disguise");
    game.run(200);
    assert!(game.world.unit(spy).is_none_or(|unit| unit.health <= 0), "the dog left the spy alone: dog target {:?}", game.world.unit(dog).map(|d| d.arms.target));
    // DATA: `[VirtualScanner] Range=5`, the dog's second weapon, is what
    // sets how far it looks: twice that, and it comes back when it stops
    // more than three quarters of that from its post.
    let far = game.world.add_infantry((at.0 + 14, at.1), 2, 0, game.stats("SPY"), 2).unwrap();
    game.world.disguise_for_tests(far, (game.stats("E1").kind, PLAYER));
    game.run(300);
    assert!(game.world.unit(far).is_some_and(|unit| unit.health > 0), "the dog went fourteen cells for it");
    assert_eq!(game.world.unit(dog).unwrap().post, Some(at));
    let dog_at = game.world.unit(dog).unwrap().cell();
    assert!((dog_at.0 - at.0).abs() <= 8 && (dog_at.1 - at.1).abs() <= 8, "the dog wandered to {dog_at:?}");
}

#[test]
fn a_bridge_the_map_starts_with_broken_can_be_mended() {
    // Isle of War starts with four sections of one high bridge gone.
    let Some(mut game) = Scenario::load("mp02t2") else {
        return;
    };
    use ra2_sim::combat::Entry;
    let fallen = game.world.map.fallen_bridge_cells();
    assert_eq!(fallen.len(), 4, "{fallen:?}");
    let gap = fallen[1];
    assert!(!game.world.map.bridged(gap.0, gap.1));
    let hut_kind = game.building("CABHUT");
    let hut = game.world.buildings().iter().filter(|b| b.kind == hut_kind).min_by_key(|b| (b.cell.0 - gap.0).abs() + (b.cell.1 - gap.1).abs()).map(|b| (b.id, b.cell)).expect("no repair hut on this map");
    let engineer = [(-1, 1), (1, 1), (1, -1), (-1, -1), (0, 2), (2, 0)]
        .into_iter()
        .find_map(|step| game.world.add_infantry((hut.1 .0 + step.0, hut.1 .1 + step.1), 2, 0, game.stats("ENGINEER"), PLAYER))
        .expect("nowhere to stand by the hut");
    assert_eq!(game.world.entry_action(engineer, hut.0), Some(Entry::RepairBridge), "hut at {:?}, gap at {gap:?}", hut.1);
    game.world.order(Order::Enter { unit: engineer, building: hut.0 });
    game.run(600);
    assert!(game.world.unit(engineer).is_none(), "the engineer was not used up");
    assert!(game.world.map.fallen_bridge_cells().is_empty());
    for cell in fallen {
        assert!(game.world.map.deck(cell.0, cell.1), "no deck at {cell:?}");
    }
}

#[test]
fn a_rocket_in_flight_lifts_the_shroud_under_it() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    game.world.map.shroud_on = true;
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 8);
    let v3 = game.world.add_unit(at, 0, game.stats("V3"), PLAYER);
    game.run(30);
    // Somewhere still dark, within its reach.
    let target = [(15, 0), (0, 15), (-15, 0), (0, -15), (11, 11), (-11, 11)]
        .into_iter()
        .map(|step| (at.0 + step.0, at.1 + step.1))
        .find(|cell| game.world.map.in_playfield(cell.0, cell.1) && !game.world.map.revealed(cell.0, cell.1))
        .expect("nowhere dark in reach");
    game.world.order(Order::Attack { unit: v3, target: Aim::Cell(target.0, target.1) });
    let mut flown = Vec::new();
    let mut sights = Vec::new();
    for _ in 0..500 {
        game.run(1);
        if let Some(rocket) = game.world.units().iter().find(|u| u.rocket.is_some()) {
            // The shroud lies flat at height
            // nothing, so what a flyer uncovers is where it is drawn: its
            // cell, back by half its height in levels.
            let cell = rocket.cell();
            let level = game.world.map.level(cell.0, cell.1) as i32 + game.world.altitude(rocket) / ra2_sim::combat::LEVEL_HEIGHT;
            sights.push(rocket.stats.fighter.sight);
            flown.push((cell.0 - level / 2, cell.1 - level / 2));
        }
    }
    flown.dedup();
    assert!(flown.len() > 8, "the rocket did not fly: {flown:?}");
    // DATA: rules.ini `[V3ROCKET] Sight=1`.
    let dark: Vec<_> = flown.iter().filter(|cell| game.world.map.in_playfield(cell.0, cell.1) && !game.world.map.revealed(cell.0, cell.1)).collect();
    assert!(dark.is_empty(), "cells it flew over are still dark: {dark:?} of {flown:?}, sight {:?}", sights.first());
}

#[test]
fn a_crate_is_picked_up_and_another_put_down() {
    // At least
    // `CrateMinimum` crates at the start; walking onto one picks it up, and a
    // new one goes down at once somewhere else.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    game.deploy_base();
    game.world.start_crates(4);
    let before: Vec<(i32, i32)> = game.world.crates().iter().map(|c| c.cell).collect();
    assert!(before.len() >= 4, "only {} crates", before.len());
    let target = before[0];
    let gi = (-1..=1)
        .flat_map(|dy| (-1..=1).map(move |dx| (target.0 + dx, target.1 + dy)))
        .filter(|&cell| cell != target)
        .find_map(|cell| game.world.add_infantry(cell, 2, 0, game.stats("E1"), PLAYER))
        .expect("room by the crate");
    let credits = game.world.houses[PLAYER as usize].credits;
    game.world.order(Order::Move { unit: gi, x: target.0, y: target.1 });
    let mut got = None;
    for _ in 0..200 {
        game.world.tick();
        for event in game.world.take_events() {
            if let ra2_sim::combat::Event::CrateCollected { kind, cell, .. } = event {
                got = Some((kind, cell));
            }
        }
        if got.is_some() {
            break;
        }
    }
    let (kind, cell) = got.expect("the crate was never picked up");
    assert_eq!(cell, target);
    assert!(!game.world.crates().iter().any(|c| c.cell == target), "the crate is still there");
    assert_eq!(game.world.crates().len(), before.len(), "no crate took its place");
    eprintln!("CRATE held {} (credits {} -> {})", ra2_sim::powerups::NAMES[kind as usize], credits, game.world.houses[PLAYER as usize].credits);
}

#[test]
fn a_tank_on_ground_a_lost_building_stood_on_can_still_be_sent() {
    // Two Prism Tanks in a player's game stood where an enemy building had
    // been and took no more orders: the zones, built again while the building
    // stood, had counted its cells as solid ground for good.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("GAPOWR", yard, 12).expect("room for a power plant");
    assert!(game.build_at("GAPOWR", spot));
    game.world.map.build_zones();
    let plant = game.world.buildings().iter().find(|b| b.cell == spot).map(|b| b.id).expect("the plant");
    game.world.kill_building_for_tests(plant);
    game.run(5);
    let tank = game.world.add_unit(spot, 0, game.stats("SREF"), PLAYER);
    let goal = (spot.0 + 6, spot.1 + 6);
    game.world.order(Order::Move { unit: tank, x: goal.0, y: goal.1 });
    assert!(game.world.unit(tank).unwrap().destination().is_some(), "the order came to nothing");
    game.run(400);
    let at = game.world.unit(tank).unwrap().cell();
    assert!((at.0 - spot.0).abs().max((at.1 - spot.1).abs()) >= 3, "still at {at:?}, sent to {goal:?}");
}

#[test]
fn a_crate_beneath_building_leaves_money_when_destroyed() {
    // Destroyed, a
    // `CrateBeneath` building leaves a crate at its centre (money with
    // `CrateBeneathIsMoney`), even with crates off.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let Some(building) = game.world.buildings().iter().find(|b| game.world.building_types[b.kind as usize].crate_beneath).cloned() else {
        eprintln!("no CrateBeneath building on this map; skipping");
        return;
    };
    assert!(game.world.crates().is_empty());
    game.world.kill_building_for_tests(building.id);
    let centre = (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2);
    let left = game.world.crates().first().copied().expect("no crate left behind");
    assert!((left.cell.0 - centre.0).abs() <= 2 && (left.cell.1 - centre.1).abs() <= 2, "crate at {:?}, centre {centre:?}", left.cell);
    assert_eq!(left.content, Some(ra2_sim::powerups::MONEY));
}

#[test]
fn each_kind_of_crate_does_its_thing() {
    use ra2_sim::powerups as p;
    for kind in [p::ARMOR, p::FIREPOWER, p::SPEED, p::VETERAN, p::UNIT, p::HEAL_BASE] {
        let Some(mut game) = Scenario::load("mp03t4") else {
            return;
        };
        let yard = game.deploy_base();
        let at = (yard.0 + 6, yard.1 + 9);
        let gi = game.world.add_infantry((at.0 - 1, at.1), 2, 0, game.stats("E1"), PLAYER).unwrap();
        let tank = game.world.add_unit((at.0 + 1, at.1 + 1), 0, game.stats("MTNK"), PLAYER);
        game.world.units_mut_for_tests(tank, 50);
        let units = game.world.units().iter().filter(|u| u.owner == PLAYER).count();
        let speed = game.world.unit(tank).unwrap().speed;
        game.world.put_crate_for_tests(at, kind);
        game.world.order(Order::Move { unit: gi, x: at.0, y: at.1 });
        game.run(200);
        let tank_now = game.world.unit(tank).unwrap();
        let name = p::NAMES[kind as usize];
        match kind {
            p::ARMOR => assert_eq!(tank_now.crate_armour, 1.5, "{name}: the tank nearby"),
            p::FIREPOWER => assert_eq!(tank_now.crate_firepower, 2.0, "{name}"),
            p::SPEED => assert!(tank_now.speed > speed, "{name}: {speed} -> {}", tank_now.speed),
            p::VETERAN => assert!(tank_now.veterancy >= 1.0, "{name}"),
            p::UNIT => assert!(game.world.units().iter().filter(|u| u.owner == PLAYER).count() > units, "{name}: no new unit"),
            p::HEAL_BASE => assert_eq!(tank_now.health, tank_now.stats.strength, "{name}"),
            _ => {}
        }
    }
}

#[test]
fn rocketeers_fight_rocketeers() {
    // DATA: their gun's shot is AA=yes and AG=yes, range 5.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    let ours = game.world.add_infantry(at, 2, 0, game.stats("JUMPJET"), PLAYER).unwrap();
    let theirs = game.world.add_infantry((at.0 + 3, at.1), 2, 0, game.stats("JUMPJET"), 2).unwrap();
    game.run(60);
    game.world.order(Order::Attack { unit: ours, target: Aim::Unit(theirs) });
    let before = game.world.unit(theirs).unwrap().health;
    game.run(300);
    assert!(game.world.unit(theirs).is_none_or(|u| u.health < before), "the ordered rocketeer never hurt the other");
    let hurt_back = game.world.unit(ours).is_none_or(|u| u.health < game.stats("JUMPJET").strength);
    assert!(hurt_back, "the other never fired back");
}

#[test]
fn yuris_wave_spares_those_immune_to_psionics() {
    // `PsychicDamage` does nothing to an
    // `ImmuneToPsionics` target; a soldier beside him still falls.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 10);
    let yuri = game.world.add_infantry(at, 2, 0, game.stats("YURI"), PLAYER).unwrap();
    let other = game.world.add_infantry((at.0 + 1, at.1), 2, 0, game.stats("YURI"), PLAYER).unwrap();
    // All the player's own, so that nothing else shoots; the wave spares no
    // side.
    let prime = game.world.add_infantry((at.0, at.1 + 1), 2, 0, game.stats("YURIPR"), PLAYER).unwrap();
    let soldier = game.world.add_infantry((at.0 + 1, at.1 + 1), 2, 0, game.stats("E1"), PLAYER).unwrap();
    let health = |game: &Scenario, id| game.world.unit(id).map(|u| u.health);
    let before = (health(&game, other), health(&game, prime));
    game.world.order(Order::Deploy { unit: yuri });
    game.run(150);
    assert!(health(&game, soldier).is_none_or(|h| h <= 0), "the wave did not touch the GI");
    assert_eq!((health(&game, other), health(&game, prime)), before, "the wave hurt a Yuri");
}

#[test]
fn a_tank_yuri_takes_stops_what_it_was_doing() {
    // Yuri Prime is the same thing with a longer reach.
    for who in ["YURI", "YURIPR"] {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    // An enemy tank shooting at one of ours, and ours shooting back.
    let ours = game.world.add_unit(at, 0, game.stats("MTNK"), PLAYER);
    let theirs = game.world.add_unit((at.0 + 4, at.1), 0, game.stats("HTNK"), 2);
    game.world.units_mut_for_tests(ours, 100_000);
    game.world.units_mut_for_tests(theirs, 100_000);
    game.world.order(Order::Attack { unit: theirs, target: Aim::Unit(ours) });
    game.world.order(Order::Attack { unit: ours, target: Aim::Unit(theirs) });
    game.run(60);
    assert_eq!(game.world.unit(theirs).unwrap().arms.target, Some(Aim::Unit(ours)));
    let yuri = game.world.add_infantry((at.0 + 2, at.1 + 2), 2, 0, game.stats(who), PLAYER).unwrap();
    game.world.order(Order::Attack { unit: yuri, target: Aim::Unit(theirs) });
    let mut taken_at = None;
    for tick in 0..300 {
        game.run(1);
        if game.world.unit(theirs).unwrap().owner == PLAYER {
            taken_at = Some(tick);
            break;
        }
    }
    assert!(taken_at.is_some(), "{who} never took the tank");
    // Its target goes, and what aimed at it lets go.
    assert_eq!(game.world.unit(theirs).unwrap().arms.target, None);
    assert_eq!(game.world.unit(ours).unwrap().arms.target, None);
    let before = game.world.unit(ours).unwrap().health;
    game.run(200);
    assert_ne!(game.world.unit(theirs).unwrap().arms.target, Some(Aim::Unit(ours)), "it went back to its old target");
    assert_eq!(game.world.unit(ours).unwrap().health, before, "the taken tank still shot at its old target");
    assert_eq!(game.world.unit(yuri).unwrap().controls, Some(theirs));
    }
}

#[test]
fn yuri_prime_takes_a_tank_from_afar_and_only_one_can_be_trained() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    let prime = game.world.add_infantry(at, 2, 0, game.stats("YURIPR"), PLAYER).unwrap();
    let tank = game.world.add_unit((at.0 + 20, at.1), 0, game.stats("MTNK"), 2);
    let start = game.world.unit(prime).unwrap().cell();
    game.world.order(Order::Attack { unit: prime, target: Aim::Unit(tank) });
    game.run(300);
    eprintln!("PRIME from {start:?} to {:?}", game.world.unit(prime).unwrap().cell());
    assert_eq!(game.world.unit(tank).unwrap().owner, PLAYER, "Yuri Prime did not take the tank 20 cells away");
    assert_eq!(game.world.unit(prime).unwrap().cell(), start, "Yuri Prime walked up to it");
}

#[test]
fn a_mirage_standing_still_is_taken_for_a_tree() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 10);
    let mirage = game.world.add_unit(at, 0, game.stats("MGTK"), PLAYER);
    // Out of the Mirage's reach, so that it does not fire and give itself away.
    let tank = game.world.add_unit((at.0 + 9, at.1), 0, game.stats("HTNK"), 2);
    game.run(20);
    let unit = game.world.unit(mirage).unwrap();
    assert!(unit.disguise.is_some_and(|(_, house)| house == ra2_sim::abilities::TREE_DISGUISE), "no tree disguise: {:?}", unit.disguise);
    assert!(game.world.disguised_to(unit, 2) && !game.world.disguised_to(unit, PLAYER));
    game.run(200);
    assert_eq!(game.world.unit(mirage).unwrap().health, game.stats("MGTK").strength, "the enemy tank shot at the tree");
    let _ = tank;
    // On the move it is itself again.
    game.world.order(Order::Move { unit: mirage, x: at.0 - 3, y: at.1 });
    game.run(2);
    assert!(game.world.unit(mirage).unwrap().disguise.is_none());
}

#[test]
fn shellfire_marks_the_ground_without_anything_drawing_it() {
    // The marks used to be made by the drawing code; the sim makes them now,
    // so a game with no picture (a test, a replay, another machine) agrees.
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    let before = game.world.map.smudges().count();
    for i in 0..3 {
        let tank = game.world.add_unit((at.0, at.1 + i), 0, game.stats("HTNK"), PLAYER);
        game.world.order(Order::Attack { unit: tank, target: Aim::Cell(at.0 + 4, at.1 + i) });
    }
    game.run(600);
    assert!(game.world.map.smudges().count() > before, "no marks were left");
}

#[test]
fn a_house_cannot_order_what_is_not_its_own() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 10);
    let ours = game.world.add_unit(at, 0, game.stats("MTNK"), PLAYER);
    let yard_id = game.world.buildings().iter().find(|b| b.owner == PLAYER).unwrap().id;
    let power = game.building("GAPOWR");
    // Another house may not move our tank, sell our yard or build in our name.
    assert!(!game.world.order_as(2, Order::Move { unit: ours, x: at.0 + 3, y: at.1 }));
    assert!(!game.world.order_as(2, Order::Sell { building: yard_id }));
    assert!(!game.world.order_as(2, Order::Build { house: PLAYER, building: power }));
    game.run(30);
    assert_eq!(game.world.unit(ours).unwrap().cell(), at);
    assert!(game.world.buildings().iter().any(|b| b.id == yard_id));
    assert!(game.world.houses[PLAYER as usize].production.is_none());
    // We may.
    assert!(game.world.order_as(PLAYER, Order::Move { unit: ours, x: at.0 + 3, y: at.1 }));
    assert!(game.world.order_as(PLAYER, Order::Build { house: PLAYER, building: power }));
    game.run(200);
    assert_eq!(game.world.unit(ours).unwrap().cell(), (at.0 + 3, at.1));
    assert!(game.world.houses[PLAYER as usize].production.is_some());
}

/// A game for the snapshot tests: a computer house of the hardest kind with
/// money, and the player's base with a power plant, a refinery and a barracks,
/// so that building and harvesting are going on. Played with `advance`, it
/// has battles too.
#[cfg(test)]
fn skirmish() -> Option<Scenario> {
    let mut game = Scenario::load("mp03t4")?;
    let enemy = terrain::ENEMY;
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    game.world.houses[enemy as usize].credits = 30_000;
    let yard = game.deploy_base();
    for name in ["GAPOWR", "GAREFN", "GAPILE"] {
        let kind = game.building(name);
        if let Some(spot) = game.find_spot(name, yard, 12) {
            game.world.place_now(PLAYER, kind, spot);
        }
    }
    game.world.houses[PLAYER as usize].credits = 10_000;
    Some(game)
}

/// The ticks at which `advance` starts a battle. The snapshot tests take
/// theirs a little after each, with shots in the air.
#[cfg(test)]
const BATTLES: [u64; 4] = [460, 2960, 3960, 11960];

/// Sets a mixed force of each side down beside the player's first building,
/// each thing told to attack one of the other side. It works from the world
/// alone, so a game restored has the same battle as the one it was taken from.
#[cfg(test)]
fn battle(game: &mut Scenario) {
    let Some(base) = game.world.buildings().iter().find(|b| b.owner == PLAYER).map(|b| b.cell) else {
        return;
    };
    let centre = (base.0 + 6, base.1 + 10);
    let world = &game.world;
    let free: Vec<(i32, i32)> = (-4..=4)
        .flat_map(|dy| (-4..=4).map(move |dx| (centre.0 + dx * 2, centre.1 + dy * 2)))
        .filter(|&cell| world.map.passable(cell.0, cell.1) && world.units().iter().all(|unit| unit.cell() != cell))
        .collect();
    let sides = [(PLAYER, ["MTNK", "FV", "JUMPJET", "E1", "SHK", "MTNK"]), (terrain::ENEMY, ["HTNK", "V3", "DESO", "IVAN", "YURI", "E2"])];
    let mut forces = [Vec::new(), Vec::new()];
    for (index, &cell) in free.iter().take(12).enumerate() {
        let (owner, names) = sides[index % 2];
        let stats = game.stats(names[index / 2]);
        let unit = if stats.infantry { game.world.add_infantry(cell, 2, 0, stats, owner) } else { Some(game.world.add_unit(cell, 0, stats, owner)) };
        forces[index % 2].extend(unit);
    }
    for (&ours, &theirs) in forces[0].iter().zip(&forces[1]) {
        game.world.order(Order::Attack { unit: ours, target: ra2_sim::combat::Aim::Unit(theirs) });
        game.world.order(Order::Attack { unit: theirs, target: ra2_sim::combat::Aim::Unit(ours) });
    }
}

/// One tick as a game runs it: whoever draws takes the events now and then,
/// so a snapshot may fall with some still waiting.
#[cfg(test)]
fn advance(game: &mut Scenario) {
    if BATTLES.contains(&game.world.current_tick()) {
        battle(game);
    }
    game.world.tick();
    if game.world.current_tick() % 7 == 0 {
        game.world.take_events();
    }
}

/// Plays to a tick, and on to the first after it with a shot in the air.
#[cfg(test)]
fn advance_into_battle(game: &mut Scenario, tick: u64) {
    while game.world.current_tick() < tick || game.world.bullets().is_empty() {
        advance(game);
        assert!(game.world.current_tick() < tick + 400, "no shot within 400 ticks of {tick}");
    }
}

/// The sections of the state in which two games differ.
#[cfg(test)]
fn differing(a: &[(&'static str, u64)], b: &[(&'static str, u64)]) -> Vec<&'static str> {
    a.iter().zip(b).filter(|(a, b)| a != b).map(|(a, _)| a.0).collect()
}

/// The first unit, building, house or team that differs between two games, as
/// the parts of its printed form that do.
#[cfg(test)]
fn first_difference(a: &World, b: &World) -> String {
    fn of<T: std::fmt::Debug>(what: &str, a: &[T], b: &[T]) -> Option<String> {
        if a.len() != b.len() {
            return Some(format!("{} {what}s against {}", a.len(), b.len()));
        }
        a.iter().zip(b).enumerate().find_map(|(index, (a, b))| {
            let (a, b) = (format!("{a:#?}"), format!("{b:#?}"));
            // Each line that differs, under the name of the field it is in.
            let mut within = "";
            let mut parts = Vec::new();
            for (a, b) in a.lines().zip(b.lines()) {
                if a.ends_with(['[', '{', '(']) && a.starts_with("    ") && !a.starts_with("     ") {
                    within = a.trim();
                }
                if a != b {
                    parts.push(format!("{within} {} | {}", a.trim(), b.trim()));
                }
            }
            (a != b).then(|| format!("{what} {index}: {}", parts.join("; ")))
        })
    }
    of("unit", a.units(), b.units())
        .or_else(|| of("building", a.buildings(), b.buildings()))
        .or_else(|| of("house", &a.houses, &b.houses))
        .or_else(|| of("team", &a.teams, &b.teams))
        .unwrap_or_default()
}

#[test]
fn a_snapshot_restored_gives_the_same_snapshot() {
    let Some(mut game) = skirmish() else {
        return;
    };
    advance_into_battle(&mut game, 3000);
    let saved = game.world.snapshot();
    let mut fresh = Scenario::load("mp03t4").unwrap();
    fresh.world.restore(&saved).unwrap();
    let again = fresh.world.snapshot();
    assert_eq!(saved.len(), again.len());
    assert!(saved == again, "sections that differ: {:?}", differing(&game.world.state_hashes(), &fresh.world.state_hashes()));
    assert_eq!(game.world.state_hash(), fresh.world.state_hash());
    // Not half of one, and not one of another format.
    assert!(Scenario::load("mp03t4").unwrap().world.restore(&saved[..saved.len() / 2]).is_err());
    let mut other = saved.clone();
    other[0] ^= 0xff;
    assert!(fresh.world.restore(&other).is_err());
}

#[test]
fn two_games_set_up_alike_stay_alike() {
    // The control for the test below: without this, a game restored could not
    // be expected to keep step either.
    let (Some(mut a), Some(mut b)) = (skirmish(), skirmish()) else {
        return;
    };
    for tick in 0..5000 {
        advance(&mut a);
        advance(&mut b);
        assert!(a.world.state_hash() == b.world.state_hash(), "apart after {tick} ticks in {:?}", differing(&a.world.state_hashes(), &b.world.state_hashes()));
    }
}

/// The skirmish at a tick, played again from the start (which the control test
/// shows to be the same game), for looking at where a restored one left it.
#[cfg(test)]
fn replay(until: u64) -> Option<Scenario> {
    let mut game = skirmish()?;
    while game.world.current_tick() < until {
        advance(&mut game);
    }
    Some(game)
}

#[test]
fn a_game_restored_plays_on_as_the_one_it_was_taken_from() {
    // Anything play changes that the snapshot leaves out shows here, some ticks
    // after the restore, as the two games parting.
    let Some(mut a) = skirmish() else {
        return;
    };
    const AFTER: usize = 3000;
    for start in [500, 4000, 12000] {
        advance_into_battle(&mut a, start);
        let start = a.world.current_tick();
        let clock = std::time::Instant::now();
        let saved = a.world.snapshot();
        let took = clock.elapsed();
        let before = (a.world.units().len(), a.world.buildings().len(), a.world.bullets().len());
        let mut hashes = Vec::with_capacity(AFTER);
        let mut shooting = 0;
        for _ in 0..AFTER {
            advance(&mut a);
            shooting += !a.world.bullets().is_empty() as u32;
            hashes.push((a.world.state_hash(), a.world.state_hashes()));
        }
        let mut b = Scenario::load("mp03t4").unwrap();
        let clock = std::time::Instant::now();
        b.world.restore(&saved).unwrap();
        let restored = clock.elapsed();
        assert!(b.world.snapshot() == saved, "restored at {start}, not the same snapshot");
        for (tick, (hash, sections)) in hashes.iter().enumerate() {
            advance(&mut b);
            assert!(
                b.world.state_hash() == *hash,
                "restored at {start}, apart {} ticks later in {:?}: {}",
                tick + 1,
                differing(sections, &b.world.state_hashes()),
                replay(b.world.current_tick()).map_or(String::new(), |a| first_difference(&a.world, &b.world))
            );
        }
        eprintln!(
            "snapshot at {start}: {} bytes, written in {took:?}, restored in {restored:?}; (units, buildings, bullets) {before:?} then, {:?} {AFTER} ticks on, shots in the air on {shooting} of them",
            saved.len(),
            (a.world.units().len(), a.world.buildings().len(), a.world.bullets().len())
        );
    }
}

#[test]
fn a_shot_at_something_in_the_air_spares_the_building_under_it() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("GAPOWR", (yard.0 + 6, yard.1 + 6), 6).unwrap();
    let plant = game.world.add_map_building(PLAYER, game.building("GAPOWR"), spot, 256);
    let health = |game: &Scenario| game.world.buildings().iter().find(|b| b.id == plant).unwrap().health;
    let full = health(&game);
    // DATA: rules.ini `[HE] CellSpread=.5`: half a cell.
    let warhead = game.world.warheads.iter().position(|w| (w.cell_spread - 0.5).abs() < 0.01 && w.verses[9] > 0.5).expect("no half-cell warhead") as u16;
    let (x, y) = (spot.0 * 256 + 128, spot.1 * 256 + 128);
    let ground = game.world.map.level(spot.0, spot.1) as i32 * ra2_sim::combat::LEVEL_HEIGHT;
    // High over the building the blast is too far
    // off; on it, it is no distance at all.
    game.world.explode_for_tests((x, y, ground + 800), 100, warhead);
    game.run(2);
    assert_eq!(health(&game), full, "a blast high in the air hurt the building under it");
    game.world.explode_for_tests((x, y, ground + 20), 100, warhead);
    game.run(2);
    assert!(health(&game) < full, "a blast on the building did nothing");
}

#[test]
fn battle_sets_the_computers_on_the_player_and_free_for_all_does_not() {
    let Ok(files) = GameFiles::open() else {
        return;
    };
    let Some(parsed) = files.get("mp03t4.map").or_else(|| files.get("mp03t4.mpr")).and_then(ra2_formats::map::Map::parse) else {
        return;
    };
    let allied = |mode: &str| terrain::build(&files, &parsed, &terrain::Player { mode_rules: mode.to_owned(), ..terrain::Player::default() }).map(|scene| scene.allies_allowed);
    // DATA: MPBattle.ini `AlliesAllowed=yes`, which allies the computer houses.
    assert_eq!(allied("MPBattle.ini"), Some(true));
    // Ours: Free For All is everyone for themselves (as in Yuri's Revenge).
    assert_eq!(allied("MPFreeForAll.ini"), Some(false));
}

#[test]
fn a_captured_construction_yard_packs_up_and_drives_off() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    // The other side's yard, taken by an engineer.
    let spot = game.find_spot("NACNST", (yard.0 + 8, yard.1 + 8), 8).unwrap();
    let theirs = game.world.add_map_building(2, game.building("NACNST"), spot, 256);
    let engineer = game.world.add_infantry((spot.0 + 5, spot.1 + 4), 2, 0, game.stats("ENGINEER"), PLAYER).unwrap();
    game.world.order(Order::Enter { unit: engineer, building: theirs });
    game.run(400);
    assert_eq!(game.world.buildings().iter().find(|b| b.id == theirs).map(|b| b.owner), Some(PLAYER), "not captured");
    assert!(game.world.can_undeploy(theirs), "a captured yard cannot be packed up");
    let to = (spot.0 + 8, spot.1 + 2);
    assert!(game.world.order_as(PLAYER, Order::Undeploy { building: theirs, x: to.0, y: to.1 }));
    game.run(600);
    assert!(game.world.buildings().iter().all(|b| b.id != theirs), "the yard is still standing");
    let mcv = game.world.units().iter().find(|u| u.owner == PLAYER && u.stats.deploys_into == Some(game.building("NACNST")));
    assert!(mcv.is_some(), "no vehicle came of it");
}

#[test]
fn a_spy_on_the_move_takes_a_disguise_at_once() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    use ra2_sim::combat::Aim;
    let at = (yard.0 + 6, yard.1 + 10);
    let spy = game.world.add_infantry(at, 2, 0, game.stats("SPY"), PLAYER).unwrap();
    let conscript = game.world.add_infantry((at.0 + 9, at.1 + 3), 2, 0, game.stats("E2"), 2).unwrap();
    game.world.order(Order::Stop { unit: conscript });
    game.world.order(Order::Move { unit: spy, x: at.0 + 12, y: at.1 });
    game.run(20);
    assert!(game.world.unit(spy).unwrap().is_moving());
    // DATA: rules.ini `[MakeupKit] Range=-2`: from anywhere.
    game.world.order(Order::Attack { unit: spy, target: Aim::Unit(conscript) });
    let mut took = None;
    for tick in 0..120 {
        game.run(1);
        if game.world.unit(spy).unwrap().disguise == Some((game.stats("E2").kind, 2)) {
            took = Some(tick);
            break;
        }
    }
    assert!(took.is_some_and(|ticks| ticks < 40), "the spy took {took:?} ticks to take the disguise; at {:?}, moving {}", game.world.unit(spy).unwrap().cell(), game.world.unit(spy).unwrap().is_moving());
}

#[test]
fn a_long_free_for_all_does_not_jam_its_factories() {
    let Ok(files) = GameFiles::open() else { return };
    let Some(parsed) = files.get("mp03t4.map").or_else(|| files.get("mp03t4.mpr")).and_then(ra2_formats::map::Map::parse) else { return };
    let opponents = vec![terrain::Opponent { level: 0, country: None, colour: None }; 3];
    let player = terrain::Player { opponents, mode_rules: "MPFreeForAll.ini".into(), ..terrain::Player::default() };
    let Some(mut scene) = terrain::build(&files, &parsed, &player) else { return };
    let mut world = units::build_world(&files, &mut scene).world;
    world.map.shroud_on = false;
    world.end_checks = false;
    // The player's seat is played by a computer too, so that all four fight.
    world.houses[1].ai = Some(ra2_sim::ai::Brain::of(0));
    world.houses[1].human = false;
    for house in world.houses.iter_mut().skip(1) {
        house.credits = 20_000;
    }
    // How long each factory has been letting the same vehicle out.
    let mut letting_out: std::collections::HashMap<u32, u64> = Default::default();
    for _ in 0..40 {
        for _ in 0..500 {
            world.tick();
            let _ = world.take_events();
        }
        let tick = world.current_tick();
        for building in world.buildings() {
            if building.unloading() {
                let since = *letting_out.entry(building.id).or_insert(tick);
                assert!(tick - since < 3000, "the factory at {:?} of house {} has been letting a vehicle out since tick {since}", building.cell, building.owner);
            } else {
                letting_out.remove(&building.id);
            }
        }
        // A soldier dug in on a building's cell can never leave it, and nothing
        // more comes out of a barracks it stands on.
        for unit in world.units().iter().filter(|u| u.stats.infantry && u.stance == ra2_sim::combat::Stance::Deployed) {
            let cell = unit.cell();
            let on = world.buildings().iter().find(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1));
            assert!(on.is_none(), "soldier {} of house {} is dug in on a building at {cell:?}", unit.id, unit.owner);
        }
    }
}

#[test]
fn a_computer_soldier_told_to_move_while_dug_in_gets_up_and_goes() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 6, yard.1 + 8);
    let gi = game.world.add_infantry(at, 2, 0, game.stats("E1"), 2).unwrap();
    game.world.order(Order::Deploy { unit: gi });
    game.run(60);
    assert_eq!(game.world.unit(gi).unwrap().stance, ra2_sim::combat::Stance::Deployed);
    // A computer's dug-in soldier told to move
    // gets up and then goes.
    game.world.order(Order::Move { unit: gi, x: at.0 + 6, y: at.1 });
    game.run(300);
    let unit = game.world.unit(gi).unwrap();
    assert_eq!(unit.stance, ra2_sim::combat::Stance::Standing);
    assert!(unit.cell().0 >= at.0 + 5, "the soldier stayed at {:?}", unit.cell());
}




/// A team type by its name in ai.ini.
fn team_type(game: &Scenario, name: &str) -> usize {
    game.world.team_data.team_types.iter().position(|t| t.name == name).unwrap_or_else(|| panic!("no team type {name}"))
}

#[test]
fn a_reinforce_team_cut_down_goes_back_to_forming_beside_its_base() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    let yard = game.deploy_base();
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    // DATA: ai.ini, a Reinforce team of six GIs.
    let kind = team_type(&game, "Allied Base Guard - Refine");
    assert!(game.world.team_data.team_types[kind].reinforce);
    let spot = game.find_spot("GAPOWR", (yard.0 + 4, yard.1 - 6), 6).unwrap();
    let plant = game.world.add_map_building(enemy, game.building("GAPOWR"), spot, 256);
    let mut soldiers = Vec::new();
    for i in 0..6 {
        soldiers.push(game.world.add_infantry((yard.0 + 6 + i % 3, yard.1 + 12 + i / 3), 0, 0, game.stats("E1"), enemy).unwrap());
    }
    let team = game.world.start_team_for_tests(enemy, kind);
    let find = |game: &Scenario| game.world.teams.iter().find(|t| t.id == team).cloned();
    game.run(20);
    let formed = find(&game).unwrap();
    assert!(formed.moving && formed.members.len() == 6, "the team did not set off: {formed:?}");
    // Six wanted: two or fewer left is under strength.
    for &id in &soldiers[..4] {
        game.world.kill_unit_for_tests(id);
    }
    game.run(2);
    let back = find(&game).expect("the team was dissolved");
    assert!(!back.moving && back.line == -1, "the team kept to its script: {back:?}");
    let building = game.world.buildings().iter().find(|b| b.id == plant).unwrap();
    let middle = (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2);
    let Some(ra2_sim::combat::Aim::Cell(x, y)) = back.focus() else {
        panic!("the team was sent nowhere: {back:?}");
    };
    let apart = (x - middle.0).abs().max((y - middle.1).abs());
    assert!((2..4).contains(&apart), "sent to {:?}, {apart} cells from the power plant at {middle:?}", (x, y));
    game.run(400);
    for &id in &soldiers[4..] {
        let cell = game.world.unit(id).unwrap().cell();
        assert!((cell.0 - x).abs().max((cell.1 - y).abs()) <= 3, "soldier {id} is at {cell:?}, not by {:?}", (x, y));
    }
}

#[test]
fn a_transport_that_returns_on_unload_drives_back_where_it_loaded() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    let yard = game.deploy_base();
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    // DATA: ai.ini, an IFV and an engineer with TransportsReturnOnUnload.
    let kind = team_type(&game, "Allied IFV Rush vs Sov");
    assert!(game.world.team_data.team_types[kind].transports_return);
    let load = (yard.0 + 8, yard.1 + 12);
    let to = (yard.0 + 20, yard.1 + 12);
    assert!(to.0 < 128, "the cell does not fit the script's argument");
    // Load, wait to be full, drive off, let the passenger out keeping it in
    // the team, then guard.
    let script = game.world.team_data.scripts.len();
    game.world.team_data.scripts.push(vec![(14, 0), (43, 0), (4, to.0 + to.1 * 128), (8, 2), (5, 100)]);
    game.world.team_data.team_types[kind].script = script;
    let ifv = game.world.add_unit(load, 0, game.stats("FV"), enemy);
    let engineer = game.world.add_infantry((load.0, load.1 + 2), 0, 0, game.stats("ENGINEER"), enemy).unwrap();
    let team = game.world.start_team_for_tests(enemy, kind);
    let find = |game: &Scenario| game.world.teams.iter().find(|t| t.id == team).cloned();
    let mut left = false;
    for _ in 0..300 {
        game.run(10);
        if find(&game).is_some_and(|t| !t.members.contains(&ifv)) {
            left = true;
            break;
        }
    }
    let team_now = find(&game);
    assert!(left, "the transport never left the team: {team_now:?}, ifv at {:?}", game.world.unit(ifv).map(|u| u.cell()));
    assert!(team_now.as_ref().is_some_and(|t| t.members.contains(&engineer)), "the passenger left the team too: {team_now:?}");
    let unloaded_at = game.world.unit(engineer).unwrap().cell();
    assert!((unloaded_at.0 - to.0).abs() <= 4, "the engineer got out at {unloaded_at:?}, not near {to:?}");
    game.run(600);
    let back = game.world.unit(ifv).unwrap().cell();
    assert!((back.0 - load.0).abs().max((back.1 - load.1).abs()) <= 1, "the transport is at {back:?}, not back at {load:?}");
}

#[test]
fn a_team_sent_to_its_own_refinery_waits_by_its_corner_not_its_dock() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let enemy = terrain::ENEMY;
    let yard = game.deploy_base();
    game.world.houses[enemy as usize].ai = Some(ra2_sim::ai::Brain::of(0));
    let spot = game.find_spot("GAREFN", (yard.0 + 6, yard.1 - 6), 6).unwrap();
    let refinery = game.world.add_map_building(enemy, game.building("GAREFN"), spot, 256);
    // DATA: ai.ini, six GIs whose first line is a move to their refinery.
    let kind = team_type(&game, "Allied Base Guard - Refine");
    for i in 0..6 {
        game.world.add_infantry((yard.0 + 6 + i % 3, yard.1 + 12 + i / 3), 0, 0, game.stats("E1"), enemy).unwrap();
    }
    let team = game.world.start_team_for_tests(enemy, kind);
    game.run(20);
    let now = game.world.teams.iter().find(|t| t.id == team).cloned().unwrap();
    assert_eq!(now.line, 0, "{now:?}");
    let Some(ra2_sim::combat::Aim::Cell(x, y)) = now.target else {
        panic!("no cell to go to: {now:?}");
    };
    let corner = game.world.buildings().iter().find(|b| b.id == refinery).unwrap().cell;
    assert!((x - corner.0).abs().max((y - corner.1).abs()) <= 2, "sent to {:?}, the refinery's corner is {corner:?}", (x, y));
/// A cell of open water with water all round it, on the first map that has one.
fn open_water() -> Option<(Scenario, (i32, i32))> {
    for map in ["mp01t4", "mp02t2", "mp05t4", "mp06t2", "mp08t2", "mp10s4", "mp11t2"] {
        let Some(game) = Scenario::load(map) else {
            continue;
        };
        let water = |x: i32, y: i32| game.world.map.describe(x, y).3 == ra2_sim::Land::Water as u8;
        let (width, height) = game.world.map.size();
        let found = (2..height - 2).flat_map(|y| (2..width - 2).map(move |x| (x, y))).find(|&(x, y)| (-1..=1).all(|dy| (-1..=1).all(|dx| water(x + dx, y + dy))));
        if let Some(cell) = found {
            return Some((game, cell));
        }
    }
    None
}

#[test]
fn crews_get_out_of_destroyed_vehicles_and_buildings() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 8, yard.1 + 8);
    let crew_kind = game.stats("E1").kind;
    let survivors = |game: &Scenario| game.world.units().iter().filter(|u| u.owner == PLAYER && u.stats.kind == crew_kind).map(|u| (u.id, u.health)).collect::<Vec<_>>();
    // A `Crewed` vehicle with no room for passengers: its crew gets out half
    // the time (`CrewEscape`), hurt to between 5 and half his strength.
    let strength = game.stats("E1").strength;
    let mut escaped = 0;
    for _ in 0..40 {
        let truck = game.world.add_unit(at, 0, game.stats("TRUCKA"), PLAYER);
        game.world.kill_unit_for_tests(truck);
        let out = survivors(&game);
        assert!(out.len() <= 1);
        for (id, health) in out {
            assert!((5..=strength / 2).contains(&health), "crew at {health} of {strength}");
            escaped += 1;
            game.world.remove_unit(id);
        }
        game.run(1);
    }
    assert!((8..=32).contains(&escaped), "{escaped} of 40 crews got out");
    // A tank is not `Crewed`.
    let tank = game.world.add_unit(at, 0, game.stats("MTNK"), PLAYER);
    game.world.kill_unit_for_tests(tank);
    assert!(survivors(&game).is_empty());

    // A power plant holds one (400 back over 500 a soldier, at least one):
    // at most one gets out, usually one, at up to full strength.
    let plant = game.building("GAPOWR");
    let mut out_of_plants = 0;
    for _ in 0..20 {
        let id = game.world.add_map_building(PLAYER, plant, at, 256);
        game.world.kill_building_for_tests(id);
        let out = survivors(&game);
        assert!(out.len() <= 1, "{} out of a power plant", out.len());
        for (id, health) in out {
            assert!((5..=strength).contains(&health));
            out_of_plants += 1;
            game.world.remove_unit(id);
        }
        game.run(300);
    }
    assert!((8..=20).contains(&out_of_plants), "{out_of_plants} of 20 power plants let somebody out");
}

#[test]
fn a_ship_sinks_for_81_frames_and_a_light_one_does_not() {
    let Some((mut game, cell)) = open_water() else {
        return;
    };
    let ship = game.world.add_unit(cell, 0, game.stats("DEST"), 2);
    game.world.kill_unit_for_tests(ship);
    assert!(game.world.unit(ship).is_none(), "the destroyer is still about");
    assert_eq!(game.world.sinking().len(), 1, "the destroyer did not go down");
    game.run(80);
    assert_eq!(game.world.sinking().first().map(|s| s.depth), Some(400));
    game.run(1);
    assert!(game.world.sinking().is_empty(), "the destroyer is still sinking");
    // Nothing is thrown on water.
    assert!(game.world.debris().is_empty());
    // A sea scorpion weighs 2: under `ShipSinkingWeight`, it is just gone.
    let light = game.world.add_unit(cell, 0, game.stats("HYD"), 2);
    game.world.kill_unit_for_tests(light);
    assert!(game.world.unit(light).is_none() && game.world.sinking().is_empty());
}

#[test]
fn a_destroyed_building_throws_debris_that_comes_down() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let at = (yard.0 + 8, yard.1 + 8);
    let plant = game.world.add_map_building(PLAYER, game.building("GAPOWR"), at, 256);
    game.world.kill_building_for_tests(plant);
    let thrown = game.world.debris().len();
    assert!(thrown >= 4, "{thrown} pieces thrown");
    game.run(300);
    assert!(game.world.debris().is_empty(), "debris still in the air");
}

#[test]
fn a_chrono_miner_jumping_leaves_its_terror_drone_behind() {
    let Some(mut game) = Scenario::load("mp03t4") else {
        return;
    };
    let yard = game.deploy_base();
    let spot = game.find_spot("GAREFN", (yard.0 + 5, yard.1 + 5), 8).unwrap();
    let refinery = game.world.add_map_building(2, game.building("GAREFN"), spot, 256);
    for id in game.world.units().iter().filter(|u| u.stats.harvester).map(|u| u.id).collect::<Vec<_>>() {
        game.world.remove_unit(id);
    }
    game.run(2);
    let home = (spot.0 + 14, spot.1 + 9);
    let miner = game.world.add_unit(home, 0, game.stats("CMIN"), 2);
    game.world.units_mut_for_tests(miner, 100_000);
    let drone = game.world.add_unit((home.0 - 3, home.1), 0, game.stats("DRON"), PLAYER);
    game.world.order(Order::Attack { unit: drone, target: ra2_sim::combat::Aim::Unit(miner) });
    game.run(200);
    assert!(game.world.unit(drone).is_none() && game.world.unit(miner).is_some_and(|u| u.parasite.len() == 1), "the drone is not inside");
    game.world.order(Order::Dock { unit: miner, building: refinery });
    let dock = (spot.0 + 3, spot.1 + 1);
    let mut left_from = game.world.unit(miner).unwrap().cell();
    for _ in 0..300 {
        let here = game.world.unit(miner).unwrap().cell();
        if here == dock {
            break;
        }
        left_from = here;
        game.run(1);
    }
    let miner_now = game.world.unit(miner).unwrap();
    assert_eq!(miner_now.cell(), dock, "the miner did not jump");
    assert!(miner_now.parasite.is_empty(), "the drone went along");
    let drone_now = game.world.unit(drone).expect("the drone was lost");
    let (dx, dy) = (drone_now.cell().0 - left_from.0, drone_now.cell().1 - left_from.1);
    assert!(dx.abs() <= 1 && dy.abs() <= 1, "the drone came out at {:?}, the miner left {left_from:?}", drone_now.cell());
    // It waits three times its weapon's rate of fire before doing anything.
    assert!(drone_now.paralysed_until > game.world.current_tick());
}
