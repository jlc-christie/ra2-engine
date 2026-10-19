//! Playing a saved game's recording through again, without the app, to find
//! out how it came to be as it was saved.
//!
//! `REPLAY_SAVE=saves/SAVE1234.SAV cargo test --release --bin ra2-rust replay -- --ignored --nocapture`
//! plays it to the tick it was saved at and checks that the replay arrives at
//! exactly the saved state. `REPLAY_TO=tick` stops earlier and prints the
//! units and buildings there; `REPLAY_UNIT=id` follows one unit tick by tick
//! over the last `REPLAY_TRACE` ticks (default 200); `REPLAY_ORDERS=1` lists
//! the orders recorded.
//!
//! Not original behaviour: a debugging aid.

#[test]
#[ignore]
fn replay() {
    let Ok(path) = std::env::var("REPLAY_SAVE") else {
        eprintln!("set REPLAY_SAVE to a save file");
        return;
    };
    let files = crate::files::GameFiles::open().expect("the game's files");
    let save = crate::saves::read(std::path::Path::new(&path)).expect("the save");
    let to = std::env::var("REPLAY_TO").ok().and_then(|n| n.trim().parse::<u64>().ok()).unwrap_or(save.tick).min(save.tick);
    let recording = save.recording.as_ref().expect("the save has no recording (saved before recordings were kept)");
    eprintln!("REPLAY {path}: {} on {}, recorded from tick {} with {} orders, saved at tick {}", save.description, save.setup.map, recording.start_tick, recording.orders.len(), save.tick);
    // REPLAY_ORDERS=1 lists every order recorded.
    if std::env::var_os("REPLAY_ORDERS").is_some() {
        for (at, order) in &recording.orders {
            eprintln!("REPLAY order at {at}: {order:?}");
        }
    }
    let watch = std::env::var("REPLAY_UNIT").ok().and_then(|n| n.trim().parse::<u32>().ok());
    let trace = std::env::var("REPLAY_TRACE").ok().and_then(|n| n.trim().parse::<u64>().ok()).unwrap_or(200);
    // REPLAY_DEFEATS=1 plays the game through and says when each house is
    // defeated, and what it had left a little before.
    if std::env::var_os("REPLAY_DEFEATS").is_some() {
        let mut world = crate::saves::replay(&files, &save, 0).expect("the replay");
        let mut orders: Vec<_> = recording.orders.iter().collect();
        orders.reverse();
        let mut down = vec![false; world.houses.len()];
        while world.current_tick() < to {
            let tick = world.current_tick();
            while orders.last().is_some_and(|(at, _)| *at <= tick) {
                world.order(orders.pop().unwrap().1);
            }
            world.tick();
            for event in world.take_events() {
                if let ra2_sim::combat::Event::HouseDefeated { house } = event {
                    eprintln!("REPLAY {tick}: house {house} defeated (event)");
                }
            }
            for (house, (state, down)) in world.houses.iter().zip(down.iter_mut()).enumerate() {
                if state.defeated && !*down {
                    *down = true;
                    eprintln!("REPLAY {tick}: house {house} defeated (flag)");
                }
            }
        }
        return;
    }
    // Played to just before the stretch to follow, then tick by tick.
    let quiet_to = if watch.is_some() { to.saturating_sub(trace) } else { to };
    let mut world = crate::saves::replay(&files, &save, quiet_to).expect("the replay");
    if let Some(id) = watch {
        let mut orders: Vec<_> = recording.orders.iter().filter(|(at, _)| *at >= world.current_tick()).collect();
        orders.reverse();
        // Not yet built is not gone.
        let mut seen = false;
        while world.current_tick() < to {
            let tick = world.current_tick();
            while orders.last().is_some_and(|(at, _)| *at <= tick) {
                let (_, order) = orders.pop().unwrap();
                eprintln!("REPLAY {tick}: order {order:?}");
                world.order(*order);
            }
            world.tick();
            if let Some(unit) = world.unit(id) {
                seen = true;
                eprintln!(
                    "REPLAY {tick}: unit {id} at {:?} ({}, {}) dest {:?} moving {} target {:?} health {} team {:?}",
                    unit.cell(),
                    unit.x,
                    unit.y,
                    unit.destination(),
                    unit.is_moving(),
                    unit.arms.target,
                    unit.health,
                    unit.team
                );
            } else if seen {
                eprintln!("REPLAY {tick}: unit {id} is gone");
                break;
            }
        }
    }
    // REPLAY_DUMP=1 prints the followed unit in full at the end.
    if let (Some(id), Some(_)) = (watch, std::env::var_os("REPLAY_DUMP"))
        && let Some(unit) = world.unit(id) {
            eprintln!("REPLAY dump {unit:#?}");
        }
    // REPLAY_CELL=x,y prints the ground round a cell: passable, track speed,
    // overlay, movement zone 2 (tracked vehicles), level.
    if let Some((cx, cy)) = std::env::var("REPLAY_CELL").ok().and_then(|text| {
        let (x, y) = text.split_once(',')?;
        Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
    }) {
        for y in cy - 4..=cy + 4 {
            let row: Vec<String> = (cx - 4..=cx + 4)
                .map(|x| {
                    let zone = world.map.zone_at((x, y), 2);
                    format!(
                        "{}{:>3}{}{:>4}L{}",
                        if world.map.passable(x, y) { "P" } else { "-" },
                        world.map.speed_on(x, y, ra2_sim::SpeedType::Track, false),
                        if world.map.has_overlay(x, y) { "o" } else { " " },
                        zone,
                        world.map.level(x, y)
                    )
                })
                .collect();
            eprintln!("REPLAY cell row {y}: {}", row.join(" | "));
        }
    }
    if to == save.tick {
        // The saved state, laid over a fresh world, must be what the replay
        // reached.
        let mut saved = crate::game::match_world(&files, &save.setup).expect("the map");
        saved.restore(&save.state).expect("the saved state");
        // REPLAY_MOVE=unit,x,y loads the save, sends a unit and follows it.
        if let Some([unit, x, y]) = std::env::var("REPLAY_MOVE").ok().and_then(|text| {
            let parts: Vec<i32> = text.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            <[i32; 3]>::try_from(parts).ok()
        }) {
            saved.order(ra2_sim::Order::Move { unit: unit as u32, x, y });
            for tick in 0..400 {
                saved.tick();
                if tick % 50 == 0
                    && let Some(u) = saved.unit(unit as u32) {
                        eprintln!("REPLAY loaded +{tick}: unit {unit} at {:?} dest {:?}", u.cell(), u.destination());
                    }
            }
            return;
        }
        let (a, b) = (world.state_hashes(), saved.state_hashes());
        let differing: Vec<&str> = a.iter().zip(&b).filter(|(x, y)| x.1 != y.1).map(|(x, _)| x.0).collect();
        assert!(differing.is_empty(), "the replay parted from the save in {differing:?}");
        eprintln!("REPLAY reached the saved state exactly at tick {to}");
    } else {
        for unit in world.units() {
            eprintln!("REPLAY unit {} kind {} house {} at {:?} dest {:?} target {:?} health {}", unit.id, unit.stats.kind, unit.owner, unit.cell(), unit.destination(), unit.arms.target, unit.health);
        }
        for building in world.buildings() {
            eprintln!("REPLAY building {} kind {} house {} at {:?} health {}", building.id, building.kind, building.owner, building.cell, building.health);
        }
    }
}
