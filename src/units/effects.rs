//! What the player sees and hears of the simulation's events: one system for
//! each family of them.

use super::*;
use ra2_sim::combat::Event;

#[derive(PartialEq, Eq)]
enum Family {
    Combat,
    Deaths,
    Supers,
    Announce,
    /// `endgame.rs` watches for these.
    Elsewhere,
}

// Every event has a family, so a new one does not compile until it is given one.
fn family(event: &Event) -> Family {
    match event {
        Event::Fired { .. } | Event::Detonated { .. } | Event::Bitten { .. } | Event::Teleported { .. } | Event::Erased { .. } | Event::Forked { .. } | Event::Wake { .. } | Event::Sparkle { .. } | Event::Rocked { .. } => Family::Combat,
        Event::UnitDied { .. } | Event::BuildingDied { .. } | Event::Crushed { .. } => Family::Deaths,
        Event::SuperFired { .. } | Event::StormStarted { .. } | Event::StormApproaching | Event::StormRefused { .. } | Event::StormEnded | Event::Cloud { .. } | Event::Bolt { .. } | Event::NukeLaunched { .. } | Event::NukeLanded { .. } | Event::SuperDetected { .. } | Event::SuperReady { .. } => Family::Supers,
        Event::Captured { .. } | Event::BombPlanted { .. } | Event::Cloaked { .. } | Event::Promoted { .. } | Event::CrateCollected { .. } | Event::Infiltrated { .. } | Event::BridgeBroken { .. } | Event::BridgeRepaired { .. } | Event::DepotRepairing { .. } | Event::UnitRepaired { .. } | Event::Parachuted { .. } | Event::MindControlled { .. } | Event::TookOff { .. } | Event::Landed { .. } | Event::Landing { .. } | Event::Boarded { .. } | Event::Alighted { .. } | Event::Garrisoned { .. } | Event::Abandoned { .. } | Event::Mended { .. } | Event::SaleBegan { .. } | Event::Sold { .. } | Event::Attacked { .. } => Family::Announce,
        Event::SuperGranted { .. } | Event::SuperLost { .. } | Event::HouseDefeated { .. } | Event::GameOver => Family::Elsewhere,
    }
}

/// Picks for what is only seen or heard (which sound, which body), apart from
/// the simulation's own.
#[derive(Resource, Default)]
pub(super) struct EffectRng(u32);

impl EffectRng {
    fn pick(&mut self, count: usize) -> usize {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 16) as usize % count.max(1)
    }
}

/// Starts an animation. It is upright, tested 2 nearer than the ground at its
/// height (in pixels, where it is known).
fn start_anim(commands: &mut Commands, cues: &mut MessageWriter<Cue>, view: &UnitView, tick: u64, name: &str, at: Vec2, height: Option<i32>, depth: f32) {
    let name = name.to_uppercase();
    let Some(anim) = view.anim_types.get(&name) else {
        return;
    };
    cues.write_batch(anim.report.clone().map(|report| Cue::SoundOn(report, at)));
    let mut spawned = commands.spawn((DespawnOnExit(Screen::InGame), Effect { anim: name, began: tick, at, depth }, Sprite::default()));
    if let Some(height) = height {
        spawned.insert(Depth::Upright { adjust: -2 - height, write: false });
    }
}

pub(super) fn effect_systems() -> bevy::ecs::schedule::ScheduleConfigs<bevy::ecs::system::ScheduleSystem> {
    (combat_effects, death_effects, super_effects, announce_events).into_configs()
}

/// Shots, explosions, rocking, warps and wakes.
fn combat_effects(
    mut commands: Commands,
    sim: Res<Sim>,
    mut events: MessageReader<SimEvent>,
    mut view: ResMut<UnitView>,
    armoury: Option<Res<crate::armoury::Armoury>>,
    mut cues: MessageWriter<Cue>,
    mut rng: ResMut<EffectRng>,
    rules: Res<crate::files::Rules>,
) {
    let Some(armoury) = armoury else {
        return;
    };
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let mut pick = |count: usize| rng.pick(count);
    for SimEvent(event) in events.read().cloned() {
        if family(&event) != Family::Combat {
            continue;
        }
        match event {
            Event::Fired { weapon, at, to, facing, by } => {
                let Some(look) = armoury.weapon_looks.get(weapon as usize) else {
                    continue;
                };
                // A laser's line is 2
                // layers thick in house colours, 3 from a prism tower.
                // 5 and brighter when other
                // towers have lent their beams to the shot.
                if let Some(colour) = look.rad_beam {
                    crate::beams::rad_beam(&mut commands, at, to, view.origin, colour);
                }
                if look.is_laser || look.electric_bolt {
                    let (from, end) = (scene_point_3d(view.origin, at), scene_point_3d(view.origin, to));
                    if look.electric_bolt {
                        crate::beams::bolt(&mut commands, at, to, view.origin, (tick as u32).wrapping_mul(31).wrapping_add(weapon as u32), look.alternate_colour);
                    } else {
                        let (owner, layers) = match by {
                            ra2_sim::combat::Aim::Unit(id) => (sim.0.unit(id).map(|u| u.owner), 2),
                            ra2_sim::combat::Aim::Building(id) => {
                                let tower = sim.0.building(id);
                                (tower.map(|b| b.owner), if tower.is_some_and(|b| b.last_supports > 0) { 5 } else { 3 })
                            }
                            ra2_sim::combat::Aim::Cell(..) => (None, 2),
                        };
                        let colour = view.art.owner_colour(owner.and_then(|owner| view.palette_of(owner)));
                        let house = look.house_colour.then_some((colour, layers));
                        crate::beams::laser(&mut commands, from, end, look.laser_inner, look.laser_outer, house, tick, look.laser_duration.max(1));
                    }
                }
                if !look.report.is_empty() {
                    cues.write(Cue::SoundOn(look.report[pick(look.report.len())].clone(), scene_point_3d(view.origin, at)));
                }
                let point = scene_point_3d(view.origin, at);
                let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                let flash = if look.anims.len() == 8 {
                    let direction = ((((facing as usize) >> 4) + 1) >> 1) & 7;
                    look.anims.get((direction + 1) % 8)
                } else {
                    look.anims.first()
                };
                if let Some(flash) = flash {
                    start_anim(&mut commands, &mut cues, view, tick, flash, point, Some(height_pixels(at.2)), depth);
                }
            }
            // Only a vehicle drawn from a model rocks.
            Event::Rocked { unit, from, strength } => {
                let Some(unit) = sim.0.unit(unit).filter(|u| !u.stats.infantry) else {
                    continue;
                };
                let Some(kind) = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).filter(|kind| kind.sheet.is_none()) else {
                    continue;
                };
                let at = (unit.x, unit.y, sim.0.ground_z(unit.x, unit.y) + sim.0.altitude(unit));
                view.rocks.entry(unit.id).or_default().push(at, from, strength, (unit.facing as u16) << 8, kind.weight);
            }
            // The sim has chosen the explosion (by the damage carried) and
            // marked the ground under it; here it is only shown.
            Event::Detonated { at, anim, .. } => {
                if let Some(name) = view.anim_names.get(anim as usize).cloned() {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, &name, scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                }
            }
            // The warp animation at both ends and the
            // type's jump sound.
            Event::Teleported { kind, from, to } => {
                for at in [from, to] {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, "WARPOUT", scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                }
                // The out sound where it
                // left and the in sound where it arrived.
                let sounds = (kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).map(|k| k.chrono_sounds.clone()).unwrap_or_default();
                cues.write_batch(sounds.0.map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, from))));
                cues.write_batch(sounds.1.map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, to))));
            }
            // `DefaultSparkSystem` at the vehicle, the
            // weapon's animation for its direction, and the rock.
            Event::Bitten { host, weapon, side } => {
                let Some(unit) = sim.0.unit(host) else {
                    continue;
                };
                let at = sim.0.aim_point(ra2_sim::combat::Aim::Unit(host)).unwrap_or((unit.x, unit.y, 0));
                spawn_sparks(&mut commands, at, tick, &mut pick);
                if let Some(look) = armoury.weapon_looks.get(weapon as usize).filter(|look| !look.anims.is_empty()) {
                    let direction = ((((unit.facing as usize) >> 4) + 1) >> 1) & 7;
                    let name = look.anims[direction.min(look.anims.len() - 1)].clone();
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, &name, scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                }
                let weight = (unit.stats.kind as usize).checked_sub(1).and_then(|k| sim.0.unit_types.get(k)).map_or(1.0, |k| k.weight) as f32;
                view.drone_rocks.entry(host).or_default().push(unit.facing, side as f32, 1.5, weight);
            }
            Event::Erased { at } => {
                let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                start_anim(&mut commands, &mut cues, view, tick, "WARPAWAY", scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
            }
            Event::Forked { weapon, from, to, house } => {
                if let Some(look) = armoury.weapon_looks.get(weapon as usize).filter(|look| look.is_laser) {
                    let colour = view.art.owner_colour(view.palette_of(house));
                    let style = look.house_colour.then_some((colour, 2));
                    crate::beams::laser(&mut commands, scene_point_3d(view.origin, from), scene_point_3d(view.origin, to), look.laser_inner, look.laser_outer, style, tick, look.laser_duration.max(1));
                }
            }
            // `Wake` over a sinking vehicle; `ChronoSparkle1` over one held by a
            // time weapon or jumping.
            Event::Wake { at } | Event::Sparkle { at } => {
                let key = if matches!(event, Event::Wake { .. }) { "Wake" } else { "ChronoSparkle1" };
                if let Some(name) = rules.sound(key) {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 2.5) * DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, &name, scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                }
            }
            _ => {}
        }
    }
}

/// Units and buildings dying, and vehicles crushing soldiers.
fn death_effects(
    mut commands: Commands,
    sim: Res<Sim>,
    mut events: MessageReader<SimEvent>,
    mut view: ResMut<UnitView>,
    armoury: Option<Res<crate::armoury::Armoury>>,
    units: Res<UnitCatalog>,
    catalog: Res<BuildCatalog>,
    mut cues: MessageWriter<Cue>,
    mut rng: ResMut<EffectRng>,
) {
    let Some(armoury) = armoury else {
        return;
    };
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let mut pick = |count: usize| rng.pick(count);
    for SimEvent(event) in events.read().cloned() {
        if family(&event) != Family::Deaths {
            continue;
        }
        match event {
            Event::UnitDied { kind, at, owner, facing, infantry, death, explosion, .. } => {
                let Some(name) = (kind as usize).checked_sub(1).and_then(|k| units.0.get(k)).map(|entry| entry.name.to_lowercase()) else {
                    continue;
                };
                // A
                // soldier killed by a warhead of `InfDeath` 1 or 2 plays its
                // Die1 or Die2 sequence, a stage a frame, and leaves one of the
                // dead bodies; 3 to 7 show an animation of their own.
                if infantry {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    match death {
                        1 | 2 => {
                            let look = (kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k));
                            let sequence = look.and_then(|k| k.sequences.as_ref()).and_then(|s| s.get(if death == 1 { "die1" } else { "die2" })).copied();
                            if let (Some(look), Some((first, count, _))) = (look, sequence) {
                                // The type's own `DeadBodies`, else
                                // the rules' unless it is `NotHuman`.
                                let own = armoury.own_bodies.get(&name).filter(|list| !list.is_empty());
                                let not_human = (kind as usize).checked_sub(1).and_then(|k| sim.0.unit_types.get(k)).is_some_and(|k| k.not_human);
                                let bodies = match own {
                                    Some(list) => list.as_slice(),
                                    None if not_human => &[],
                                    None => armoury.dead_bodies.as_slice(),
                                };
                                let body = (!bodies.is_empty()).then(|| bodies[pick(bodies.len())].clone());
                                commands.spawn((
                                    DespawnOnExit(Screen::InGame),
                                    Corpse { image: look.image.clone(), palette: view.palette_of(owner), first, count: count.max(1), began: tick, at, depth, body },
                                    Sprite::default(),
                                ));
                            }
                        }
                        3..=7 => {
                            if let Some(anim) = armoury.infantry_deaths[(death - 3) as usize].clone() {
                                start_anim(&mut commands, &mut cues, view, tick, &anim, scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                            }
                        }
                        _ => {}
                    }
                    let _ = facing;
                }
                if let Some(sounds) = armoury.die_sounds.get(&name).filter(|list| !list.is_empty()) {
                    cues.write(Cue::SoundOn(sounds[pick(sounds.len())].clone(), scene_point_3d(view.origin, at)));
                }
                if let Some(name) = view.anim_names.get(explosion as usize).cloned() {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 1.5) * DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, &name, scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                }
            }
            Event::BuildingDied { kind, cell, size, explosions, .. } => {
                let Some(name) = catalog.0.get(kind as usize).map(|entry| entry.name.to_lowercase()) else {
                    continue;
                };
                if let Some(sounds) = armoury.die_sounds.get(&name).filter(|list| !list.is_empty()) {
                    let at = scene_point(view.origin, cell.0 * LEPTONS_PER_CELL + size.0 * 128, cell.1 * LEPTONS_PER_CELL + size.1 * 128, 0);
                    cues.write(Cue::SoundOn(sounds[pick(sounds.len())].clone(), at));
                }
                // One explosion on each cell of the foundation, as the sim chose.
                let mut chosen = explosions.iter();
                for dy in 0..size.1 {
                    for dx in 0..size.0 {
                        let Some(name) = chosen.next().and_then(|&anim| view.anim_names.get(anim as usize)).cloned() else {
                            continue;
                        };
                        let (x, y) = ((cell.0 + dx) * LEPTONS_PER_CELL + 128, (cell.1 + dy) * LEPTONS_PER_CELL + 128);
                        let level = sim.0.map.level(cell.0 + dx, cell.1 + dy) as i32;
                        let depth = 2.0 + ((cell.0 + dx + cell.1 + dy) as f32 + 1.5) * DEPTH_PER_ROW;
                        start_anim(&mut commands, &mut cues, view, tick, &name, scene_point(view.origin, x, y, level), Some(15 * level), depth);
                    }
                }
            }
            // The victim type's `CrushSound`.
            Event::Crushed { kind, at } => {
                let sound = (kind as usize).checked_sub(1).and_then(|k| units.0.get(k)).and_then(|entry| armoury.crush_sounds.get(&entry.name.to_lowercase()));
                cues.write_batch(sound.cloned().map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
            }
            _ => {}
        }
    }
}

/// Super weapons and the lightning storm.
fn super_effects(
    mut commands: Commands,
    sim: Res<Sim>,
    mut events: MessageReader<SimEvent>,
    mut view: ResMut<UnitView>,
    mut cues: MessageWriter<Cue>,
    mut rng: ResMut<EffectRng>,
    mut radar_events: ResMut<crate::radar::RadarEvents>,
    mut status: MessageWriter<crate::bar::Status>,
    rules: Res<crate::files::Rules>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    let mut pick = |count: usize| rng.pick(count);
    for SimEvent(event) in events.read().cloned() {
        if family(&event) != Family::Supers {
            continue;
        }
        match event {
            // The announcer when the player's is
            // charged, by its type.
            Event::SuperReady { house, index } => {
                if house == local() {
                    let line = match sim.0.super_types.get(index as usize).map(|kind| kind.kind) {
                        Some(0) => Some("EVA_NuclearMissileReady"),
                        Some(1) => Some("EVA_IronCurtainReady"),
                        Some(2) => Some("EVA_LightningStormReady"),
                        Some(3) => Some("EVA_ChronosphereReady"),
                        Some(5 | 6) => Some("EVA_ReinforcementsReady"),
                        _ => None,
                    };
                    cues.write_batch(line.map(Cue::Eva));
                }
            }
            // Each super weapon's animation, voice and
            // sound as it is fired.
            // DATA: the animation and sound names are rules.ini keys.
            Event::SuperFired { kind, cell, from, .. } => {
                let point = |cell: (i32, i32), up: i32| {
                    let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                    scene_point_3d(view.origin, (x, y, sim.0.ground_z(x, y) + up))
                };
                let depth = |cell: (i32, i32)| 2.0 + ((cell.0 + cell.1) as f32 + 2.5) * DEPTH_PER_ROW;
                let key = |key: &str| rules.sound(key);
                match kind {
                    1 => {
                        if let Some(name) = key("IronCurtainInvokeAnim") {
                            start_anim(&mut commands, &mut cues, view, tick, &name, point(cell, 5), None, depth(cell));
                        }
                        radar_events.raise(13, cell, tick);
                        cues.write(Cue::Eva("EVA_IronCurtainActivated"));
                    }
                    2 => {
                        cues.write(Cue::Eva("EVA_LightningStormCreated"));
                    }
                    4 => {
                        if let Some(name) = key("ChronoBlastDest") {
                            start_anim(&mut commands, &mut cues, view, tick, &name, point(cell, 5), None, depth(cell));
                        }
                        if let (Some(name), Some(from)) = (key("ChronoBlast"), from) {
                            start_anim(&mut commands, &mut cues, view, tick, &name, point(from, 5), None, depth(from));
                            radar_events.raise(13, from, tick);
                        }
                        radar_events.raise(13, cell, tick);
                        cues.write(Cue::Eva("EVA_ChronosphereActivated"));
                    }
                    _ => {}
                }
            }
            // The missile leaves its silo over
            // `NukeTakeOff`, the siren sounds at the target and everyone is told.
            Event::NukeLaunched { at, cell, .. } => {
                if let Some(name) = rules.sound("NukeTakeOff") {
                    let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 2.5) * DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, &name, scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                }
                let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                let target = scene_point_3d(view.origin, (x, y, sim.0.ground_z(x, y)));
                cues.write_batch(rules.sound("DigSound").map(|name| Cue::SoundOn(name, target)));
                cues.write(Cue::Eva("EVA_NuclearMissileLaunched"));
            }
            // The fireball where it came down.
            Event::NukeLanded { at } => {
                let depth = 2.0 + ((at.0 + at.1) as f32 / LEPTONS_PER_CELL as f32 + 2.5) * DEPTH_PER_ROW;
                start_anim(&mut commands, &mut cues, view, tick, "NUKEBALL", scene_point_3d(view.origin, at), Some(height_pixels(at.2)), depth);
                radar_events.raise(13, (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL)), tick);
            }
            // `StormSound` and the message as the storm breaks; the warning
            // and the announcer while it is coming; a message to a player who
            // calls a storm while one is on.
            Event::StormStarted { cell, .. } => {
                cues.write_batch(rules.sound("StormSound").map(Cue::Sound));
                status.write(crate::bar::Status::Key("TXT_LIGHTNING_STORM"));
                radar_events.raise(13, cell, tick);
            }
            Event::StormApproaching => {
                cues.write(Cue::Eva("EVA_LightningStormCreated"));
                status.write(crate::bar::Status::Key("TXT_LIGHTNING_STORM_APPROACHING"));
            }
            Event::StormRefused { house } => {
                if house == local() {
                    status.write(crate::bar::Status::Key("Msg:LightningStormActive"));
                }
            }
            Event::StormEnded => {}
            // The cloud the sim chose over the cell; then the bolt, and one of
            // the strike sounds.
            Event::Cloud { cell, kind } | Event::Bolt { cell, kind } => {
                let bolt = matches!(event, Event::Bolt { .. });
                let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                let ground = sim.0.ground_z(x, y);
                let names = rules.list(if bolt { "WeatherConBolts" } else { "WeatherConClouds" });
                if let Some(name) = names.get(kind as usize).cloned() {
                    let at = scene_point_3d(view.origin, (x, y, ground + if bolt { 0 } else { 700 }));
                    start_anim(&mut commands, &mut cues, view, tick, &name, at, None, 2.0 + ((cell.0 + cell.1) as f32 + 3.0) * DEPTH_PER_ROW);
                }
                if bolt {
                    let sounds = rules.list("LightningSounds");
                    if !sounds.is_empty() {
                        cues.write(Cue::SoundOn(sounds[pick(sounds.len())].clone(), scene_point_3d(view.origin, (x, y, ground))));
                    }
                }
            }
            // Another house's super weapon
            // building finished: a radar event there and the announcer, by
            // which of the first four super weapons it holds.
            Event::SuperDetected { owner, index, cell }
                if owner != local() => {
                    radar_events.raise(12, cell, tick);
                    let line = ["EVA_NuclearSiloDetected", "EVA_IronCurtainDetected", "EVA_WeatherDeviceReady", "EVA_ChronosphereDetected"].get(index as usize);
                    cues.write_batch(line.map(|line| Cue::Eva(line)));
                }
            _ => {}
        }
    }
}

/// What is only heard: the announcer and sounds for the player's own doings.
fn announce_events(
    mut commands: Commands,
    sim: Res<Sim>,
    mut events: MessageReader<SimEvent>,
    mut view: ResMut<UnitView>,
    mut cues: MessageWriter<Cue>,
    mut radar_events: ResMut<crate::radar::RadarEvents>,
    rules: Res<crate::files::Rules>,
) {
    let view = &mut *view;
    let tick = sim.0.current_tick();
    for SimEvent(event) in events.read().cloned() {
        if family(&event) != Family::Announce {
            continue;
        }
        match event {
            // `SellSound` as a building starts to
            // come down; the announcer when it has gone.
            Event::SaleBegan { owner } => {
                if owner == local() {
                    cues.write_batch(view.sell_sound.clone().map(Cue::Sound));
                }
            }
            Event::Sold { owner } => {
                if owner == local() {
                    cues.write(Cue::Eva("EVA_StructureSold"));
                }
            }
            // The announcer on a capture.
            Event::Captured { by, from, .. } => {
                if by == local() || from == local() {
                    cues.write(Cue::Eva("EVA_BuildingCaptured"));
                }
            }
            // Heard by the player whose soldiers
            // they are.
            Event::Garrisoned { by, .. } => {
                if by == local() {
                    cues.write(Cue::Eva("EVA_StructureGarrisoned"));
                    cues.write_batch(rules.sound("BuildingGarrisonedSound").map(Cue::Sound));
                }
            }
            Event::Abandoned { by, .. } => {
                if by == local() {
                    cues.write(Cue::Eva("EVA_StructureAbandoned"));
                    cues.write_batch(rules.sound("BuildingAbandonedSound").map(Cue::Sound));
                }
            }
            // The transport type's
            // `EnterTransportSound` and `LeaveTransportSound` (the IFV's is its
            // turret-change sound).
            Event::Boarded { transport } | Event::Alighted { transport } => {
                let entering = matches!(event, Event::Boarded { .. });
                let name = sim
                    .0
                    .unit(transport)
                    .and_then(|unit| (unit.stats.kind as usize).checked_sub(1))
                    .and_then(|kind| view.kinds.get(kind))
                    .and_then(|kind| if entering { kind.transport_sounds.0.clone() } else { kind.transport_sounds.1.clone() });
                if let Some(name) = name {
                    cues.write(Cue::Sound(name));
                }
            }
            // The type's `AuxSound1` where it
            // leaves the ground, and `AuxSound2` where it nears it coming down.
            Event::TookOff { unit } | Event::Landing { unit } => {
                let up = matches!(event, Event::TookOff { .. });
                if let Some(unit) = sim.0.unit(unit) {
                    let sound = (unit.stats.kind as usize).checked_sub(1).and_then(|k| view.kinds.get(k)).and_then(|k| if up { k.aux_sounds.0.clone() } else { k.aux_sounds.1.clone() });
                    let at = (unit.x, unit.y, sim.0.ground_z(unit.x, unit.y));
                    cues.write_batch(sound.map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            Event::Landed { .. } => {}
            // `BombAttachSound` for the planter's
            // side. `YuriMindControlSound` at the
            // victim.
            Event::BombPlanted { target, house } => {
                if house == local()
                    && let Some(at) = sim.0.aim_point(target) {
                        cues.write_batch(rules.sound("BombAttachSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                    }
            }
            Event::MindControlled { victim, .. } => {
                if let Some(at) = sim.0.aim_point(ra2_sim::combat::Aim::Unit(victim)) {
                    cues.write_batch(rules.sound("YuriMindControlSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            // The player's engineer
            // mending a bridge is announced.
            Event::BridgeRepaired { house, .. } => {
                if house == local() {
                    cues.write(Cue::Eva("EVA_BridgeRepaired"));
                }
            }
            Event::BridgeBroken { .. } => {}
            // The announcer, for the depot's owner.
            Event::DepotRepairing { house } => {
                if house == local() {
                    cues.write(Cue::Eva("EVA_Repairing"));
                }
            }
            Event::UnitRepaired { house } => {
                if house == local() {
                    cues.write(Cue::Eva("EVA_UnitRepaired"));
                }
            }
            // `ChuteSound` at the plane.
            Event::Parachuted { from, .. } => {
                if let Some(at) = sim.0.aim_point(ra2_sim::combat::Aim::Unit(from)) {
                    cues.write_batch(rules.sound("ChuteSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            // What the
            // announcer says to the building's owner and to the spy's.
            Event::Infiltrated { victim, by, effect, .. } => {
                let lines: &[&'static str] = match (effect, victim == local(), by == local()) {
                    (1, true, _) => &["EVA_RadarSabotaged"],
                    (1, _, true) => &["EVA_BuildingInfRadarSabotaged"],
                    (2, true, _) => &["EVA_PowerSabotaged"],
                    (2, _, true) => &["EVA_BuildingInfiltrated", "EVA_EnemyBasePoweredDown"],
                    (3, true, _) | (3, _, true) => &["EVA_BuildingInfiltrated"],
                    (4 | 6 | 7, true, _) => &["EVA_TechnologyStolen"],
                    (4 | 6 | 7, _, true) => &["EVA_BuildingInfiltrated", "EVA_NewTechnologyAcquired"],
                    (5, true, _) => &["EVA_CashStolen"],
                    (5, _, true) => &["EVA_BuildingInfCashStolen"],
                    _ => &[],
                };
                cues.write_batch(lines.iter().copied().map(Cue::Eva));
            }
            // The player hears `UpgradeVeteranSound`
            // or `UpgradeEliteSound` where the unit is, and the announcer.
            Event::Promoted { unit, owner, elite } => {
                if owner == local() {
                    if let Some(at) = sim.0.aim_point(ra2_sim::combat::Aim::Unit(unit)) {
                        let key = if elite { "UpgradeEliteSound" } else { "UpgradeVeteranSound" };
                        cues.write_batch(rules.sound(key).map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                    }
                    cues.write(Cue::Eva("EVA_UnitPromoted"));
                }
            }
            // The
            // powerup's animation at the cell, 200 leptons up, for everyone; the
            // sound (and for armour, firepower and speed the announcer) only
            // for the player who picked it up.
            Event::CrateCollected { house, cell, kind, .. } => {
                let ground = sim.0.ground_z(cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                let at = scene_point_3d(view.origin, (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128, ground + 200));
                if let Some(anim) = sim.0.crate_rules.powerups.get(kind as usize).and_then(|p| p.anim.clone()) {
                    let depth = 2.0 + ((cell.0 + cell.1) as f32 + 1.0) * crate::game::DEPTH_PER_ROW;
                    start_anim(&mut commands, &mut cues, view, tick, &anim, at, None, depth);
                }
                if house == local() {
                    use ra2_sim::powerups as p;
                    let (sound, eva) = match kind {
                        p::MONEY => ("CrateMoneySound", None),
                        p::UNIT => ("CrateUnitSound", None),
                        p::HEAL_BASE => ("HealCrateSound", None),
                        p::REVEAL => ("CrateRevealSound", None),
                        p::ARMOR => ("CrateArmourSound", Some("EVA_UnitArmorUpgraded")),
                        p::SPEED => ("CrateSpeedSound", Some("EVA_UnitSpeedUpgraded")),
                        p::FIREPOWER => ("CrateFireSound", Some("EVA_UnitFirePowerUpgraded")),
                        p::VETERAN => ("CratePromoteSound", None),
                        _ => ("CrateMoneySound", None),
                    };
                    cues.write_batch(rules.sound(sound).map(|name| Cue::SoundOn(name, at)));
                    if let Some(eva) = eva {
                        cues.write(Cue::Eva(eva));
                    }
                }
            }
            // `CloakSound` where it is.
            Event::Cloaked { unit } => {
                if let Some(at) = sim.0.unit(unit).filter(|u| !sim.0.hidden_from(u, local())).and_then(|u| sim.0.aim_point(ra2_sim::combat::Aim::Unit(u.id))) {
                    cues.write_batch(rules.sound("CloakSound").map(|name| Cue::SoundOn(name, scene_point_3d(view.origin, at))));
                }
            }
            Event::Mended { .. } => {}
            // One of the player's
            // buildings hit starts a radar event at it, and only if that is
            // accepted (none like it alive within eight cells) the announcer
            // says so and the siren sounds.
            Event::Attacked { owner, at, building, miner } => {
                if building && owner == local() && radar_events.raise(3, at, tick) {
                    cues.write(Cue::Eva("EVA_OurBaseIsUnderAttack"));
                    cues.write_batch(rules.sound("BaseUnderAttackSound").map(Cue::Sound));
                }
                // The player's miner hurt and still alive: radar event 4 and
                // the announcer.
                if miner && owner == local() && radar_events.raise(4, at, tick) {
                    cues.write(Cue::Eva("EVA_OreMinerUnderAttack"));
                }
            }
            _ => {}
        }
    }
}
