//! A fuzz tester: computer-only games with random set-ups, run for a random
//! time, checked for broken rules and stuck units. It is not part of the normal
//! test run; start it with
//!
//! ```text
//! cargo test --release --bin ra2-rust fuzz -- --ignored --nocapture
//! ```
//!
//! `FUZZ_GAMES=n` (default 6) sets how many games, `FUZZ_SEED=n` the first seed
//! (default: from the clock), `FUZZ_TICKS=n` caps the length of each game,
//! `FUZZ_MAP=id` keeps to one map, `FUZZ_LENGTH=n` plays each game for n ticks
//! and `FUZZ_AI=1` prints every computer house's money, plan, miners and teams
//! every 5000 ticks (`watch_one_computer` does that for one Brutal computer). A
//! failure names the seed; `FUZZ_SEED=<it> FUZZ_GAMES=1` plays the same game
//! again.
//!
//! Hard failures (the test fails): a panic in the simulation; credits below
//! zero; a unit off the map or with more health than its type allows; a factory
//! letting the same vehicle out for over 3000 ticks; a soldier dug in on a
//! building's cell; a snapshot that does not restore to the same game. Soft
//! findings (printed): units that have not moved for a long time while meaning
//! to, and teams that have stood still a long time.

use std::collections::HashMap;

use ra2_formats::ini::Ini;
use ra2_sim::World;

use crate::files::GameFiles;
use crate::{terrain, units};

/// A small random number generator of our own, so that a seed always gives the
/// same game set-up.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// One game's set-up, all drawn from the seed.
#[derive(Debug, Clone)]
struct Setup {
    #[allow(dead_code)]
    seed: u64,
    map: String,
    opponents: Vec<usize>,
    player_level: usize,
    mode_rules: String,
    start: usize,
    short_game: bool,
    mcv_repacks: bool,
    crates: bool,
    credits: i32,
    ticks: u64,
    snapshot_at: Option<u64>,
}

fn maps(files: &GameFiles) -> Vec<(String, usize)> {
    let Some(missions) = files.get("missions.pkt").map(Ini::parse) else {
        return Vec::new();
    };
    missions
        .section("MultiMaps")
        .iter()
        .filter(|(_, id)| missions.get(id, "GameMode").is_some_and(|modes| modes.split(',').any(|m| m.trim() == "standard")))
        .map(|(_, id)| (id.to_lowercase(), missions.get(id, "MaxPlayers").and_then(|v| v.trim().parse().ok()).unwrap_or(8)))
        .collect()
}

fn mode_files(files: &GameFiles) -> Vec<String> {
    let Some(modes) = files.get("mpmodes.ini").map(Ini::parse) else {
        return Vec::new();
    };
    modes
        .sections()
        .flat_map(|section| modes.section(section))
        .filter_map(|(_, value)| {
            let fields: Vec<&str> = value.split(',').map(str::trim).collect();
            (fields.last() == Some(&"true") && fields.len() >= 3).then(|| fields[2].to_owned())
        })
        .collect()
}

fn draw(seed: u64, maps: &[(String, usize)], modes: &[String]) -> Setup {
    let mut rng = Rng(seed.max(1));
    let wanted = std::env::var("FUZZ_MAP").ok();
    let pool: Vec<&(String, usize)> = maps.iter().filter(|(id, _)| wanted.as_ref().is_none_or(|w| w.eq_ignore_ascii_case(id))).collect();
    let (map, max) = pool[rng.below(pool.len() as u64) as usize].clone();
    let others = 1 + rng.below(max.clamp(2, 8) as u64 - 1) as usize;
    let cap = std::env::var("FUZZ_TICKS").ok().and_then(|n| n.parse().ok()).unwrap_or(40_000u64);
    let ticks = (4_000 + rng.below(36_000)).min(cap);
    // FUZZ_LENGTH=n plays every game for n ticks.
    let ticks = std::env::var("FUZZ_LENGTH").ok().and_then(|n| n.parse().ok()).unwrap_or(ticks);
    Setup {
        seed,
        map,
        opponents: (0..others).map(|_| rng.below(3) as usize).collect(),
        player_level: rng.below(3) as usize,
        mode_rules: if modes.is_empty() { String::new() } else { modes[rng.below(modes.len() as u64) as usize].clone() },
        start: rng.below(8) as usize,
        short_game: rng.chance(50),
        mcv_repacks: rng.chance(80),
        crates: rng.chance(70),
        credits: [5_000, 10_000, 20_000, 50_000][rng.below(4) as usize],
        ticks,
        snapshot_at: rng.chance(40).then(|| 500 + rng.below(ticks.saturating_sub(1_000).max(1))),
    }
}

fn build(files: &GameFiles, setup: &Setup) -> Option<World> {
    build_named(files, setup).map(|(world, _)| world)
}

fn build_named(files: &GameFiles, setup: &Setup) -> Option<(World, Vec<String>)> {
    let parsed = files.get(&format!("{}.map", setup.map)).or_else(|| files.get(&format!("{}.mpr", setup.map))).and_then(ra2_formats::map::Map::parse)?;
    let opponents = setup.opponents.iter().map(|&level| terrain::Opponent { level, country: None, colour: None }).collect();
    let player = terrain::Player { opponents, mode_rules: setup.mode_rules.clone(), start: Some(setup.start), ..terrain::Player::default() };
    let mut scene = terrain::build(files, &parsed, &player)?;
    let built = units::build_world(files, &mut scene);
    let names = built.catalog.iter().map(|entry| entry.name.clone()).collect();
    let mut world = built.world;
    world.map.shroud_on = false;
    world.short_game = setup.short_game;
    world.mcv_repacks = setup.mcv_repacks;
    // The player's seat is played by a computer too.
    world.houses[1].ai = Some(ra2_sim::ai::Brain::of(setup.player_level));
    world.houses[1].human = false;
    for house in world.houses.iter_mut().skip(1) {
        house.credits = setup.credits;
    }
    if setup.crates {
        world.start_crates(0);
    }
    Some((world, names))
}

/// What one game turned up.
#[derive(Default)]
struct Findings {
    hard: Vec<String>,
    soft: Vec<String>,
}

fn check(world: &World, tick: u64, letting_out: &mut HashMap<u32, u64>, findings: &mut Findings) {
    let (width, height) = world.map.size();
    for (index, house) in world.houses.iter().enumerate() {
        if house.credits < 0 {
            findings.hard.push(format!("tick {tick}: house {index} has {} credits", house.credits));
        }
    }
    for unit in world.units() {
        let cell = unit.cell();
        if unit.plane.is_none() && (cell.0 < 0 || cell.1 < 0 || cell.0 >= width || cell.1 >= height) {
            findings.hard.push(format!("tick {tick}: unit {} (kind {}) off the map at {cell:?}", unit.id, unit.stats.kind));
        }
        if unit.health > unit.stats.strength {
            findings.hard.push(format!("tick {tick}: unit {} (kind {}) has {} health of {}", unit.id, unit.stats.kind, unit.health, unit.stats.strength));
        }
        if unit.stats.infantry && unit.stance == ra2_sim::combat::Stance::Deployed {
            let on = world.buildings().iter().any(|b| (b.cell.0..b.cell.0 + b.size.0).contains(&cell.0) && (b.cell.1..b.cell.1 + b.size.1).contains(&cell.1));
            if on {
                findings.hard.push(format!("tick {tick}: soldier {} of house {} dug in on a building at {cell:?}", unit.id, unit.owner));
            }
        }
    }
    for building in world.buildings() {
        if building.unloading() {
            let since = *letting_out.entry(building.id).or_insert(tick);
            if tick - since >= 3_000 {
                let text = format!("{building:?}");
                let unload = text.find("unload: ").map_or("", |at| &text[at..(at + 80).min(text.len())]);
                let near: Vec<_> = world
                    .units()
                    .iter()
                    .filter(|u| (u.cell().0 - building.cell.0 - 2).abs() <= 3 && (u.cell().1 - building.cell.1 - 1).abs() <= 3)
                    .map(|u| (u.id, u.stats.kind, u.owner, u.cell(), u.destination(), u.is_moving(), u.tethered, u.team.is_some()))
                    .collect();
                let teams: Vec<String> = world
                    .teams
                    .iter()
                    .filter(|t| t.members.iter().any(|id| world.unit(*id).is_some_and(|u| (u.cell().0 - building.cell.0 - 2).abs() <= 3 && (u.cell().1 - building.cell.1 - 1).abs() <= 3)))
                    .map(|t| {
                        let text = format!("{t:?}");
                        let zone = text.find("zone: ").map_or("", |at| &text[at..(at + 24).min(text.len())]);
                        format!("team {} line {} moving {} {zone}", t.id, t.line, t.moving)
                    })
                    .collect();
                findings.hard.push(format!(
                    "tick {tick}: the factory at {:?} of house {} has been letting a vehicle out since tick {since}; {unload}; near the door {near:?}; teams {teams:?}",
                    building.cell, building.owner
                ));
                letting_out.insert(building.id, tick);
            }
        } else {
            letting_out.remove(&building.id);
        }
    }
}

fn play(files: &GameFiles, setup: &Setup) -> Findings {
    let mut findings = Findings::default();
    let Some((mut world, names)) = build_named(files, setup) else {
        findings.soft.push(format!("map {} could not be set up", setup.map));
        return findings;
    };
    let mut letting_out = HashMap::new();
    // Where each unit was and since when, and when each team last moved.
    let mut still: HashMap<u32, ((i32, i32), u64)> = HashMap::new();
    let mut reported: std::collections::HashSet<u32> = Default::default();
    let mut planes: HashMap<u32, u64> = HashMap::new();
    let mut disguises: HashMap<u32, Vec<(u64, Option<u32>, i32)>> = HashMap::new();
    for _ in 0..setup.ticks {
        world.tick();
        // Each make-up kit shot, by whom, with its team and the team's line.
        for event in world.take_events() {
            if let ra2_sim::combat::Event::Fired { weapon, by: ra2_sim::combat::Aim::Unit(id), .. } = event {
                let makeup = world.weapons.get(weapon as usize).and_then(|w| world.warheads.get(w.warhead as usize)).is_some_and(|w| w.makes_disguise);
                if makeup {
                    let team = world.unit(id).and_then(|u| u.team);
                    let line = team.and_then(|team| world.teams.iter().find(|t| t.id == team)).map_or(-9, |t| t.line);
                    disguises.entry(id).or_default().push((world.current_tick(), team, line));
                }
            }
        }
        let tick = world.current_tick();
        if Some(tick) == setup.snapshot_at
            && let Err(why) = round_trip(files, setup, &mut world) {
                findings.hard.push(format!("tick {tick}: {why}"));
            }
        if tick % 5000 == 0 && std::env::var_os("FUZZ_AI").is_some() {
            watch_ai(&world, &names, tick);
        }
        if tick % 250 == 0 {
            check(&world, tick, &mut letting_out, &mut findings);
            // A cargo plane is over the map for a minute or so; one still on
            // its drop or on its way out after 3000 ticks is going round.
            for unit in world.units() {
                let Some(plane) = &unit.plane else {
                    continue;
                };
                let text = format!("{plane:?}");
                if !(text.contains("Paradrop") || text.contains("Retreat")) {
                    continue;
                }
                let born = *planes.entry(unit.id).or_insert(tick);
                if tick - born >= 3_000 && reported.insert(unit.id) {
                    findings.hard.push(format!("tick {tick}: plane {} of house {} has flown since tick {born}, at {:?} facing {}: {text}", unit.id, unit.owner, (unit.x, unit.y), unit.facing));
                }
            }
            for unit in world.units() {
                let entry = still.entry(unit.id).or_insert((unit.cell(), tick));
                if entry.0 != unit.cell() {
                    *entry = (unit.cell(), tick);
                } else if tick - entry.1 >= 6_000 && unit.destination().is_some() && reported.insert(unit.id) {
                    findings.soft.push(format!(
                        "tick {tick}: unit {} (kind {}, house {}) has meant to go to {:?} from {:?} since tick {} ({:?}, team {:?}, target {:?})",
                        unit.id, unit.stats.kind, unit.owner, unit.destination(), unit.cell(), entry.1, unit.stance, unit.team, unit.arms.target
                    ));
                    // FUZZ_WATCH=1 prints the whole unit too.
                    if std::env::var_os("FUZZ_WATCH").is_some() {
                        let text = format!("{unit:?}");
                        let at = 0;
                        findings.soft.push(format!("    {}", &text[at..]));
                        findings.soft.push(format!("    can move there: {}", world.can_move_to(unit.id, unit.destination().unwrap_or((0, 0)))));
                    }
                }
            }
            still.retain(|id, _| world.unit(*id).is_some());
            for team in &world.teams {
                let members: Vec<&ra2_sim::Unit> = team.members.iter().filter_map(|id| world.unit(*id)).collect();
                let quiet = !members.is_empty() && members.iter().all(|u| still.get(&u.id).is_some_and(|(_, since)| tick - since >= 8_000));
                let kind = &world.team_data.team_types[team.kind];
                let line = world.team_data.scripts[kind.script].get(team.line.max(0) as usize).copied();
                // Standing still is the point of guarding (5) and of a mission
                // such as area guard (11), and a team still forming (line -1)
                // waits where its members are.
                let resting = team.line < 0 || matches!(line, Some((5 | 11, _)));
                if quiet && !resting && reported.insert(u32::MAX - team.id) {
                    findings.soft.push(format!("tick {tick}: team {} '{}' of house {} has not moved for 8000 ticks, on line {} {:?}", team.id, kind.name, team.house, team.line, line));
                    if std::env::var_os("FUZZ_WATCH").is_some() {
                        findings.soft.push(format!("    team: {team:?}"));
                        for unit in &members {
                            let verdict = unit.arms.target.map(|target| (world.attack_action(unit.id, target, false), world.attack_action(unit.id, target, true), match target { ra2_sim::combat::Aim::Unit(id) => world.unit(id).map(|t| (t.owner, t.cell(), t.health)), _ => None }));
                            findings.soft.push(format!("    member {} kind {} at {:?} dest {:?} target {:?} disguise {:?} job {:?} moving {} verdict {verdict:?} arms {:?}", unit.id, unit.stats.kind, unit.cell(), unit.destination(), unit.arms.target, unit.disguise, unit.work.job, unit.is_moving(), unit.arms));
                        }
                    }
                }
            }
        }
        if world.over.is_some() {
            break;
        }
    }
    for (id, shots) in &disguises {
        if shots.len() > 2 {
            findings.soft.push(format!("spy {id} took a disguise {} times: {:?}", shots.len(), shots.iter().take(12).collect::<Vec<_>>()));
        }
    }
    // For checking that a change leaves the simulation as it was.
    println!("FUZZ   end: tick {} hash {:016x}", world.current_tick(), world.state_hash());
    findings
}

/// Snapshots the game, plays on a little, and checks that a fresh game restored
/// from the snapshot plays the same.
fn round_trip(files: &GameFiles, setup: &Setup, world: &mut World) -> Result<(), String> {
    let saved = world.snapshot();
    let mut fresh = build(files, setup).ok_or("the map could not be set up again")?;
    fresh.restore(&saved)?;
    for step in 0..300 {
        world.tick();
        fresh.tick();
        let _ = (world.take_events(), fresh.take_events());
        if world.state_hash() != fresh.state_hash() {
            let (a, b) = (world.state_hashes(), fresh.state_hashes());
            let differing: Vec<&str> = a.iter().zip(&b).filter(|(x, y)| x.1 != y.1).map(|(x, _)| x.0).collect();
            return Err(format!("a restored snapshot went its own way {step} ticks later, in {differing:?}"));
        }
    }
    Ok(())
}

#[test]
#[ignore]
fn fuzz_games() {
    let Ok(files) = GameFiles::open() else {
        println!("no game files; nothing to fuzz");
        return;
    };
    let maps = maps(&files);
    let modes = mode_files(&files);
    assert!(!maps.is_empty(), "no multiplayer maps found");
    let games: u64 = std::env::var("FUZZ_GAMES").ok().and_then(|n| n.parse().ok()).unwrap_or(6);
    let first: u64 = std::env::var("FUZZ_SEED").ok().and_then(|n| n.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs())
    });
    let mut failed = Vec::new();
    for seed in first..first + games {
        let setup = draw(seed, &maps, &modes);
        println!("FUZZ seed {seed}: {setup:?}");
        let started = std::time::Instant::now();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| play(&files, &setup)));
        match outcome {
            Ok(findings) => {
                for line in &findings.soft {
                    println!("FUZZ   soft: {line}");
                }
                for line in &findings.hard {
                    println!("FUZZ   HARD: {line}");
                }
                if !findings.hard.is_empty() {
                    failed.push(seed);
                }
            }
            Err(_) => {
                println!("FUZZ   HARD: the game panicked");
                failed.push(seed);
            }
        }
        println!("FUZZ   ({:.1} s)", started.elapsed().as_secs_f64());
    }
    assert!(failed.is_empty(), "seeds with hard failures: {failed:?} (FUZZ_SEED=<seed> FUZZ_GAMES=1 to replay)");
}

/// FUZZ_AI=1: each computer house's money, base plan, miners and teams.
fn watch_ai(world: &World, names: &[String], tick: u64) {
    for (h, house) in world.houses.iter().enumerate() {
        let Some(brain) = &house.ai else { continue };
        let mut have: Vec<&str> = world.buildings().iter().filter(|b| b.owner == h as u8).map(|b| names[b.kind as usize].as_str()).collect();
        have.sort();
        let plan: Vec<String> = brain.nodes.iter().filter(|n| n.cell.is_none()).map(|n| n.kind.map_or("?", |k| names[k as usize].as_str()).to_owned()).collect();
        let miners: Vec<String> = world.units().iter().filter(|u| u.owner == h as u8 && u.stats.harvester).map(|u| format!("{:?}{:?}", u.work.job, u.cell())).collect();
        let units = world.units().iter().filter(|u| u.owner == h as u8).count();
        let teams: Vec<String> = world.teams.iter().filter(|t| t.house == h as u8).map(|t| format!("{}:{}/{}{}", world.team_data.team_types[t.kind].name, t.line, t.members.len(), if t.moving { "" } else { "f" })).collect();
        println!("t{tick} house {h} enemy {:?} allies {:b} credits {} units {units} next {:?} mode {}\n  have {have:?}\n  plan left {plan:?}\n  miners {miners:?}\n  teams {teams:?}\n  slots {:?} {:?} {:?} {:?}", brain.enemy, house.allies, house.credits, brain.build_structure.map(|k| names[k as usize].clone()), brain.mode, house.production.map(|p| p.building), house.defence.map(|p| p.building), house.infantry.map(|p| p.building), house.vehicles.map(|p| p.building));
    }
}

/// One Brutal computer of a country (`WATCH_COUNTRY`, default Russia) against
/// an idle player on `FUZZ_MAP` (default mp03t4), printed as `FUZZ_AI` does,
/// for `FUZZ_TICKS` ticks (default 40000).
#[test]
#[ignore]
fn watch_one_computer() {
    let Ok(files) = GameFiles::open() else { return };
    let map = std::env::var("FUZZ_MAP").unwrap_or("mp03t4".into());
    let Some(parsed) = files.get(&format!("{map}.map")).or_else(|| files.get(&format!("{map}.mpr"))).and_then(ra2_formats::map::Map::parse) else { return };
    let country = std::env::var("WATCH_COUNTRY").ok().and_then(|n| n.parse().ok()).unwrap_or(8);
    let player = terrain::Player { opponents: vec![terrain::Opponent { level: 0, country: Some(country), colour: None }], start: Some(0), ..terrain::Player::default() };
    let mut scene = terrain::build(&files, &parsed, &player).unwrap();
    let built = units::build_world(&files, &mut scene);
    let names: Vec<String> = built.catalog.iter().map(|e| e.name.clone()).collect();
    let mut world = built.world;
    world.map.shroud_on = false;
    world.end_checks = false;
    let ticks: u64 = std::env::var("FUZZ_TICKS").ok().and_then(|n| n.parse().ok()).unwrap_or(40_000);
    for _ in 0..ticks {
        world.tick();
        let _ = world.take_events();
        if world.current_tick().is_multiple_of(5000) {
            watch_ai(&world, &names, world.current_tick());
        }
    }
}
