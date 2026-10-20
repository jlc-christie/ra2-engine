//! Super weapons: gaining and losing them, charging, and what firing one does.
//!
//! Follows the original game
//!

use crate::aircraft::{Mission, Plane};
use crate::combat::Event;
use crate::{Land, Unit, UnitStats, World, LEPTONS_PER_CELL};

/// A `[SuperWeaponTypes]` entry.
#[derive(Clone, Copy, Debug)]
pub struct SuperType {
    /// `RechargeTime` in frames.
    pub recharge: u32,
    /// `Type`: 0 nuclear missile, 1 iron curtain, 2 lightning storm, 3
    /// chronosphere, 4 its second click, 5 paradrop, 6 American paradrop.
    pub kind: i32,
    pub powered: bool,
    pub show_timer: bool,
    /// `DisableableFromShell`: its building cannot be built with the
    /// skirmish screen's super weapons option off.
    pub disableable: bool,
    /// `WeaponType`: what the nuclear silo launches.
    pub weapon: Option<u16>,
}

/// One house's hold on one type.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Super {
    pub granted: bool,
    pub ready: bool,
    pub hold: bool,
    /// Frames still to charge.
    pub left: u32,
}

/// The rules the paradrops read: the plane, and each list of soldier types (by
/// place among the unit types) with how many of each.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    pub plane: Option<u16>,
    pub american: Vec<(u16, u32)>,
    pub allied: Vec<(u16, u32)>,
    pub soviet: Vec<(u16, u32)>,
    /// Yuri's Revenge `YuriParaDropInf` and `YuriParaDropNum`.
    pub third: Vec<(u16, u32)>,
    /// `ParadropRadius` in leptons.
    pub radius: i32,
    /// `ParachuteMaxFallRate`, as a speed downward.
    pub fall_rate: i32,
    /// `IronCurtainDuration` in frames.
    pub curtain_duration: u32,
    /// `LightningDeferment`, `LightningStormDuration`, `LightningDamage`,
    /// `LightningWarhead`, `LightningHitDelay`, `LightningScatterDelay`,
    /// `LightningCellSpread`, `LightningSeparation`.
    pub storm: StormRules,
    /// The `NukePayload` weapon.
    pub nuke_weapon: Option<u16>,
    /// The frames the `NUKEBALL` animation lasts, and its number among the
    /// animation types (for the crater it leaves).
    pub nuke_ball: (u32, Option<u16>),
    /// `ChronoDelay`: frames a unit the chronosphere moved is out of action.
    pub chrono_delay: u32,
}

#[derive(Clone, Debug, Default)]
pub struct StormRules {
    pub deferment: u32,
    pub duration: u32,
    pub damage: i32,
    pub warhead: u16,
    pub hit_delay: u32,
    pub scatter_delay: u32,
    pub spread: i32,
    pub separation: i32,
    /// For each of `WeatherConClouds`: the frames until it is past half its
    /// pictures (when its bolt comes down), and the frames it lasts.
    pub clouds: Vec<(u32, u32)>,
    /// How many `WeatherConBolts` there are.
    pub bolts: u32,
    /// `WeatherConBoltExplosion`, by its number among the animation types.
    pub bolt_explosion: Option<u16>,
}

/// A lightning storm on its way or raging.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Storm {
    pub cell: (i32, i32),
    pub house: u8,
    /// Frames until it starts; then the frame it started.
    pub deferment: u32,
    pub started: Option<u64>,
    /// Clouds that have yet to strike: where, and when.
    clouds: Vec<((i32, i32), u64)>,
    /// The frame the last cloud formed so far is gone.
    clouds_until: u64,
}

/// A nuclear missile: climbing from the silo, coming down on its target, or
/// down and waiting for its fireball to burn out.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Nuke {
    pub id: u32,
    pub house: u8,
    pub cell: (i32, i32),
    pub stage: NukeStage,
    /// Where it is, in leptons.
    pub at: (i32, i32, i32),
    speed: i32,
    /// When the fireball is out.
    until: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NukeStage {
    Up,
    Down,
    Ball,
}

/// A unit the chronosphere is moving: where to, and from when; then, for a
/// vehicle put down in water, how deep it has sunk.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Warp {
    pub to: (i32, i32),
    pub at: u64,
    pub sink: Option<i32>,
}

/// The cells an iron curtain or a chronosphere covers, from its middle.
const AROUND: [(i32, i32); 9] = [(0, 0), (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1), (0, 1), (1, 1)];

/// Frames a unit waits under the chronosphere before it goes.
const WARP_WAIT: u64 = 60;

/// How bright something under the iron curtain is drawn, as a share of its
/// usual brightness, `elapsed` frames after the curtain came down on it.
// It flares to double brightness
// in 6 frames, holds 4, dims to a fifth over 20, then pulses between a fifth
// and a half (8 frames up, 16 down) until fewer than 54 frames are left; it
// stays dim until 30 are left, then flares again (6 up, 4 held) and fades back
// to normal over the last 20.
pub fn curtain_brightness(elapsed: u64, duration: u64) -> f32 {
    let e = elapsed as i64;
    let d = duration as i64;
    if e < 1 || e >= d {
        return 1.0;
    }
    let left = |begin: i64, length: i64| (length - (e - begin)).max(0);
    let value = |stage: u8, left: i64| -> i64 {
        match stage {
            1 => (12 - left) * 256 / 6,
            2 | 8 => 512,
            3 => (left * 461 + 1020) / 20,
            4 => (1024 - 77 * left) / 8,
            5 => (77 * left + 816) / 16,
            6 => 51,
            7 => (3072 - 461 * left) / 6,
            9 => (left + 20) * 256 / 20,
            _ => 256,
        }
    };
    // The pulse ends at the first turn of it that finishes with fewer than 54
    // frames left; the last flare starts once fewer than 31 are left.
    let mut dim = 55;
    while d - dim >= 54 {
        dim += 24;
    }
    let flare = (d - 30).max(dim + 1);
    let shade = if e < 7 {
        value(1, left(1, 6))
    } else if e < 11 {
        value(2, 0)
    } else if e < 31 {
        value(3, left(11, 20))
    } else if e < dim {
        let into = (e - 31) % 24;
        let turn = e - into;
        if into < 8 { value(4, left(turn, 8)) } else { value(5, left(turn + 8, 16)) }
    } else if e < flare {
        value(6, 0)
    } else if e < flare + 6 {
        value(7, left(flare, 6))
    } else if e < flare + 10 {
        value(8, 0)
    } else if e < flare + 30 {
        value(9, left(flare + 10, 20))
    } else {
        256
    };
    shade.min(2000) as f32 / 256.0
}

impl Super {
    pub fn can_fire(&self) -> bool {
        self.granted && self.ready && !self.hold
    }
}

impl World {
    /// The stage of a super weapon's clock, of 54.
    pub fn super_stage(&self, house: u8, index: usize) -> u32 {
        let Some((state, kind)) = self.houses.get(house as usize).and_then(|h| h.supers.get(index)).zip(self.super_types.get(index)) else {
            return 0;
        };
        if !state.granted {
            0
        } else if state.ready {
            54
        } else {
            let total = kind.recharge.max(1);
            (((total.saturating_sub(state.left)) as f64 / total as f64 * 54.0) as u32).min(53)
        }
    }

    /// Grants what a house's buildings newly provide, takes away what none
    /// provides any more, and charges the rest.
    // A building provides from
    // the moment it is placed; a powered type is held while the house is short
    // of power; the charge runs down a frame at a time and then it is ready.
    pub(crate) fn run_supers(&mut self) {
        let types = self.super_types.clone();
        for house in 0..self.houses.len() {
            if self.houses[house].supers.len() != types.len() {
                self.houses[house].supers = vec![Super::default(); types.len()];
            }
            let low = self.low_power(house as u8);
            for (index, kind) in types.iter().enumerate() {
                let provided = self.buildings.iter().any(|building| {
                    building.owner as usize == house
                        && self.building_types.get(building.kind as usize).is_some_and(|t| t.super_weapon == Some(index as u16) || t.super_weapon2 == Some(index as u16))
                });
                let state = &mut self.houses[house].supers[index];
                if !state.granted {
                    if provided {
                        *state = Super { granted: true, ready: false, hold: false, left: kind.recharge };
                        self.events.push(Event::SuperGranted { house: house as u8, index: index as u16 });
                    }
                    continue;
                }
                if !provided {
                    *state = Super::default();
                    self.events.push(Event::SuperLost { house: house as u8, index: index as u16 });
                    continue;
                }
                state.hold = kind.powered && low;
                if state.ready || state.hold {
                    continue;
                }
                state.left = state.left.saturating_sub(1);
                if state.left == 0 {
                    state.ready = true;
                    self.events.push(Event::SuperReady { house: house as u8, index: index as u16 });
                }
            }
        }

        // Soldiers under parachutes come down.
        // The height changes by the fall rate, which
        // then grows by one a frame up to `ParachuteMaxFallRate`; on the ground
        // the fall is over.
        let (most, tick) = (self.super_rules.fall_rate.max(1), self.tick);
        let mut landed = Vec::new();
        for unit in &mut self.units {
            if let Some(height) = unit.falling {
                // With no
                // parachute the fall gathers 1.4 a frame, up to 100.
                let rate = if unit.doomed { ((tick.saturating_sub(unit.dropped_at) as f64 * 1.4) as i32).clamp(1, 100) } else { (tick.saturating_sub(unit.dropped_at) as i32).min(most) };
                unit.falling = (height > rate).then(|| height - rate);
                if unit.falling.is_none() {
                    // On the
                    // ground he looks round himself afresh.
                    unit.looked = None;
                    unit.landed_at = tick;
                    landed.push(unit.id);
                }
            }
        }
        // One that fell with a bridge dies where
        // it lands, on land or water.
        let doomed: Vec<u32> = landed.iter().copied().filter(|&id| self.unit(id).is_some_and(|unit| unit.doomed)).collect();
        for id in doomed {
            self.kill(crate::combat::Aim::Unit(id));
        }
        // What they were told on the way down starts now.
        for id in landed {
            if let Some(at) = self.landing_orders.iter().position(|order| order.unit() == Some(id)) {
                let order = self.landing_orders.remove(at);
                self.order(order);
            }
        }
    }

    /// Fires a house's super weapon at a cell.
    // It must be granted, ready and not
    // held; once launched it is no longer ready and its charge starts again.
    pub(crate) fn fire_super(&mut self, house: u8, index: usize, cell: (i32, i32)) {
        let Some(kind) = self.super_types.get(index).copied() else {
            return;
        };
        // The chronosphere's second click needs
        // neither a grant nor a charge, only the first click before it.
        if kind.kind == 4 {
            let Some(from) = self.houses.get_mut(house as usize).and_then(|h| h.chrono_source.take()) else {
                return;
            };
            self.chrono_shift(from, cell);
            if let Some(first) = self.super_types.iter().position(|t| t.kind == 3) {
                let recharge = self.super_types[first].recharge;
                let state = &mut self.houses[house as usize].supers[first];
                state.ready = false;
                state.left = recharge;
            }
            self.events.push(Event::SuperFired { house, kind: 4, cell, from: Some(from) });
            return;
        }
        if !self.houses.get(house as usize).and_then(|h| h.supers.get(index)).is_some_and(Super::can_fire) {
            return;
        }
        // While a storm is on its way or raging
        // another cannot be called: the weapon stays ready.
        if kind.kind == 2 && self.storm.is_some() {
            self.events.push(Event::StormRefused { house });
            return;
        }
        match kind.kind {
            5 | 6 => self.launch_paradrop(house, kind.kind == 6, cell),
            // With no silo nothing is launched but
            // the charge starts again all the same.
            0 => {
                let silo = self.buildings.iter().find(|b| {
                    b.owner == house && b.buildup.is_none() && self.building_types.get(b.kind as usize).is_some_and(|t| t.super_weapon == Some(index as u16) || t.super_weapon2 == Some(index as u16))
                });
                if let Some(silo) = silo.map(|b| (b.cell, b.size)) {
                    self.launch_nuke(house, silo, cell);
                }
            }
            1 => self.iron_curtain(cell),
            // The storm is put off by
            // `LightningDeferment`.
            2 => {
                let deferment = self.super_rules.storm.deferment;
                self.storm = Some(Storm { cell, house, deferment, started: None, clouds: Vec::new(), clouds_until: 0 });
            }
            // The first click only remembers the
            // cell; the charge is not spent until the second.
            3 => {
                self.houses[house as usize].chrono_source = Some(cell);
                self.events.push(Event::SuperFired { house, kind: 3, cell, from: None });
                return;
            }
            _ => return,
        }
        self.events.push(Event::SuperFired { house, kind: kind.kind, cell, from: None });
        let state = &mut self.houses[house as usize].supers[index];
        state.ready = false;
        state.left = kind.recharge;
    }

    /// The missile leaves the silo.
    // It starts at the middle of
    // the silo, going straight up at 10 leptons a frame, the `NukeTakeOff`
    // animation under it.
    fn launch_nuke(&mut self, house: u8, (cell, size): ((i32, i32), (i32, i32)), target: (i32, i32)) {
        let (x, y) = (cell.0 * LEPTONS_PER_CELL + size.0 * LEPTONS_PER_CELL / 2, cell.1 * LEPTONS_PER_CELL + size.1 * LEPTONS_PER_CELL / 2);
        let at = (x, y, self.ground_z(x, y));
        let id = self.next_id;
        self.next_id += 1;
        self.nukes.push(Nuke { id, house, cell: target, stage: NukeStage::Up, at, speed: 10, until: 0 });
        self.events.push(Event::NukeLaunched { house, at, cell: target });
    }

    /// The missiles in the air.
    pub fn nukes(&self) -> &[Nuke] {
        &self.nukes
    }

    /// The weapon whose projectile a missile is drawn as.
    pub fn nuke_weapon(&self, nuke: &Nuke) -> Option<u16> {
        match nuke.stage {
            NukeStage::Up => self.super_types.iter().find(|t| t.kind == 0).and_then(|t| t.weapon),
            NukeStage::Down => self.super_rules.nuke_weapon,
            NukeStage::Ball => None,
        }
    }

    // Both ways it is a `Vertical`
    // projectile: each frame its speed, while below the shot's, grows by the
    // projectile's `Acceleration`, then it moves. Going up (at most 255 a frame)
    // it is done once above its `DetonationAltitude`; then the `NukePayload`
    // weapon's projectile is made that high above the target, falling at 1, and
    // comes down at most at the speed that weapon's `Range` gives a shot. On the
    // ground the `NUKEBALL` fireball plays, and only once it is out does the
    // warhead go off.
    fn fly_nukes(&mut self) {
        let tick = self.tick;
        let up = self.super_types.iter().find(|t| t.kind == 0).and_then(|t| t.weapon).and_then(|w| self.weapons.get(w as usize)).map(|w| self.projectiles.get(w.projectile as usize).cloned().unwrap_or_default());
        let payload = self.super_rules.nuke_weapon.and_then(|w| self.weapons.get(w as usize).cloned());
        let down = payload.as_ref().map(|w| self.projectiles.get(w.projectile as usize).cloned().unwrap_or_default());
        let (climb, ceiling) = up.as_ref().map_or((1, 20_000), |p| (p.acceleration, p.detonation_altitude));
        let fall = down.as_ref().map_or(1, |p| p.acceleration);
        let top_down = payload.as_ref().map_or(235, |w| {
            let g = if down.as_ref().is_some_and(|p| p.floater) { self.gravity as f64 * 0.5 } else { self.gravity as f64 };
            (w.range as f64 * g * 1.2).sqrt() as i32
        });
        let mut landed = Vec::new();
        for nuke in &mut self.nukes {
            match nuke.stage {
                NukeStage::Up => {
                    if nuke.speed < 255 {
                        nuke.speed += climb;
                    }
                    nuke.at.2 += nuke.speed;
                    if nuke.at.2 > ceiling {
                        let (x, y) = (nuke.cell.0 * LEPTONS_PER_CELL + 128, nuke.cell.1 * LEPTONS_PER_CELL + 128);
                        nuke.at = (x, y, 0);
                        nuke.stage = NukeStage::Down;
                        nuke.speed = 1;
                        landed.push((nuke.id, ceiling));
                    }
                }
                NukeStage::Down => {
                    if nuke.speed < top_down {
                        nuke.speed += fall;
                    }
                    nuke.at.2 -= nuke.speed;
                    landed.push((nuke.id, 0));
                }
                NukeStage::Ball => {}
            }
        }
        // Placed over the target, or come to the ground.
        for (id, above) in landed {
            let Some(index) = self.nukes.iter().position(|n| n.id == id) else {
                continue;
            };
            let (x, y, z) = self.nukes[index].at;
            let ground = self.ground_z(x, y);
            if above > 0 {
                self.nukes[index].at.2 = ground + above;
            } else if z < ground {
                let ball = self.super_rules.nuke_ball;
                let nuke = &mut self.nukes[index];
                nuke.at.2 = ground;
                nuke.stage = NukeStage::Ball;
                nuke.until = tick + ball.0 as u64;
                let at = nuke.at;
                self.events.push(Event::NukeLanded { at });
                if let Some(anim) = ball.1 {
                    self.mark_of(anim, (at.0, at.1));
                }
            }
        }
        let gone: Vec<Nuke> = self.nukes.iter().copied().filter(|n| n.stage == NukeStage::Ball && n.until <= tick).collect();
        self.nukes.retain(|n| !(n.stage == NukeStage::Ball && n.until <= tick));
        for nuke in gone {
            let Some(weapon) = payload.clone() else {
                continue;
            };
            self.explode(nuke.at, weapon.damage, weapon.warhead, None);
            self.irradiate(nuke.at, weapon.rad_level, weapon.warhead);
            self.blast_event(weapon.warhead, weapon.damage, nuke.at, false);
        }
    }

    /// Whether a house's psychic sensors cover a cell: a working sensor within
    /// its `PsychicDetectionRadius`.
    pub fn psychic_covers(&self, viewer: u8, cell: (i32, i32)) -> bool {
        let half = LEPTONS_PER_CELL / 2;
        let centre = |cell: (i32, i32)| (cell.0 * LEPTONS_PER_CELL + half, cell.1 * LEPTONS_PER_CELL + half, self.map.level(cell.0, cell.1) as i32 * crate::combat::LEVEL_HEIGHT);
        let p = centre(cell);
        self.buildings.iter().any(|sensor| {
            let radius = self.building_types.get(sensor.kind as usize).map_or(0, |kind| kind.psychic_radius) as i64 * LEPTONS_PER_CELL as i64;
            if radius == 0 || sensor.owner != viewer || sensor.buildup.is_some() || !self.online(sensor) {
                return false;
            }
            let at = centre(sensor.cell);
            let (dx, dy, dz) = ((p.0 - at.0) as i64, (p.1 - at.1) as i64, (p.2 - at.2) as i64);
            dx * dx + dy * dy + dz * dz <= radius * radius
        })
    }

    /// Where a house sees the warning marker of a missile on its way: the
    /// target of one launched by a house it is not allied with, if its psychic
    /// sensors cover that cell.
    pub fn nuke_warnings(&self, viewer: u8) -> Vec<(i32, i32)> {
        self.nukes.iter().filter(|n| n.stage != NukeStage::Ball && !self.allied(viewer, n.house) && self.psychic_covers(viewer, n.cell)).map(|n| n.cell).collect()
    }

    /// The iron curtain comes down on the nine cells round a cell.
    // Soldiers and
    // `Organic` things are killed; vehicles and buildings can not be hurt for
    // `IronCurtainDuration` frames; a building loses a charge stuck on it.
    fn iron_curtain(&mut self, cell: (i32, i32)) {
        let until = self.tick + self.super_rules.curtain_duration as u64;
        for (dx, dy) in AROUND {
            let at = (cell.0 + dx, cell.1 + dy);
            let here: Vec<(u32, bool)> = self.units.iter().filter(|u| u.cell() == at && self.altitude(u) == 0).map(|u| (u.id, u.stats.infantry || u.stats.fighter.organic)).collect();
            for (id, dies) in here {
                if dies {
                    self.kill(crate::combat::Aim::Unit(id));
                } else if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                    unit.curtain_until = until;
                }
            }
            if let Some(index) = self.building_on(at) {
                let building = &mut self.buildings[index];
                building.curtain_until = until;
                building.c4 = None;
            }
        }
    }

    /// How bright a unit or building under the iron curtain is drawn (1 when
    /// it is not).
    pub fn curtain_shade(&self, until: u64) -> f32 {
        let duration = self.super_rules.curtain_duration as u64;
        if until <= self.tick || duration == 0 {
            return 1.0;
        }
        curtain_brightness((self.tick + duration).saturating_sub(until), duration)
    }

    /// The chronosphere takes hold of what stands on the nine cells round one
    /// cell, to put it on those round another.
    // Things in the air are left; `Organic` things
    // that do not teleport of themselves are killed; those under the iron
    // curtain, held by a time weapon or already jumping are left. A vehicle
    // goes to the middle of its cell, anything else keeps its place within the
    // cell. Each waits 60 frames, out of action, before it goes.
    fn chrono_shift(&mut self, from: (i32, i32), to: (i32, i32)) {
        let tick = self.tick;
        for (dx, dy) in AROUND {
            let (source, dest) = ((from.0 + dx, from.1 + dy), (to.0 + dx, to.1 + dy));
            let here: Vec<u32> = self.units.iter().filter(|u| u.cell() == source && self.altitude(u) == 0).map(|u| u.id).collect();
            for id in here {
                let Some(index) = self.units.iter().position(|u| u.id == id) else {
                    continue;
                };
                let unit = &self.units[index];
                if (unit.stats.infantry || unit.stats.fighter.organic) && !unit.stats.teleporter {
                    self.kill(crate::combat::Aim::Unit(id));
                    continue;
                }
                if unit.curtain_until > tick || unit.frozen.is_some() || unit.chrono.is_some() || tick < unit.warp_until {
                    continue;
                }
                // A parasite is put out where it was.
                self.parasite_exit(index);
                let unit = &self.units[index];
                let point = if unit.stats.infantry {
                    (dest.0 * LEPTONS_PER_CELL + unit.x.rem_euclid(LEPTONS_PER_CELL), dest.1 * LEPTONS_PER_CELL + unit.y.rem_euclid(LEPTONS_PER_CELL))
                } else {
                    (dest.0 * LEPTONS_PER_CELL + 128, dest.1 * LEPTONS_PER_CELL + 128)
                };
                let unit = &mut self.units[index];
                unit.dest = None;
                unit.head_to = None;
                unit.drop_path();
                unit.arms.target = None;
                unit.warp_until = tick + WARP_WAIT;
                unit.chrono = Some(Warp { to: point, at: tick + WARP_WAIT, sink: None });
            }
        }
    }

    // Once its wait is over a unit goes: whatever
    // else is on the cell is killed (another soldier only where the soldier
    // will stand); a building or a tree there sends it to a free cell nearby
    // instead, tried on the next frame. Where it comes down it is out of
    // action for `ChronoDelay` frames. A vehicle that is not a ship put on open
    // water sinks; anything put where it cannot be (a ship on land, rock) is
    // killed, or sinks if the cell is water.
    fn run_warps(&mut self) {
        let tick = self.tick;
        let warping: Vec<u32> = self.units.iter().filter(|u| u.chrono.is_some()).map(|u| u.id).collect();
        for id in warping {
            let Some(index) = self.units.iter().position(|u| u.id == id) else {
                continue;
            };
            let Some(mut warp) = self.units[index].chrono else {
                continue;
            };
            if let Some(depth) = warp.sink {
                // Down 5 leptons a frame, with a wake every fourth frame,
                // until 400 under.
                let depth = depth + 5;
                if depth > 400 {
                    self.vanish(id);
                    continue;
                }
                self.units[index].chrono = Some(Warp { sink: Some(depth), ..warp });
                if tick % 4 == 0 {
                    let (x, y) = (self.units[index].x, self.units[index].y);
                    let (rx, ry) = (self.random_below(341) as i32 - 170, self.random_below(341) as i32 - 170);
                    let at = (x + rx, y + ry, self.ground_z(x, y));
                    self.events.push(Event::Wake { at });
                }
                continue;
            }
            if tick < warp.at {
                continue;
            }
            let cell = (warp.to.0.div_euclid(LEPTONS_PER_CELL), warp.to.1.div_euclid(LEPTONS_PER_CELL));
            let Some(at) = self.map.index(cell.0, cell.1) else {
                self.units[index].chrono = None;
                continue;
            };
            let tree = matches!(self.map.obstacle_kind(cell), k if k == crate::Obstacle::TreeFull as u8 || k == crate::Obstacle::Tree as u8);
            if self.building_on(cell).is_some() || tree {
                let unit = &self.units[index];
                if let Some(free) = crate::nearby(&self.map, &self.claims, &self.spots, unit, cell, crate::Near::new(true, false, Some(cell)), tick) {
                    warp.to = (free.0 * LEPTONS_PER_CELL + warp.to.0.rem_euclid(LEPTONS_PER_CELL), free.1 * LEPTONS_PER_CELL + warp.to.1.rem_euclid(LEPTONS_PER_CELL));
                    self.units[index].chrono = Some(warp);
                }
                continue;
            }
            let infantry = self.units[index].stats.infantry;
            let victims: Vec<u32> = self
                .units
                .iter()
                .filter(|u| u.id != id && u.cell() == cell && self.altitude(u) == 0)
                .filter(|u| !(infantry && u.stats.infantry) || (u.x, u.y) == warp.to)
                .map(|u| u.id)
                .collect();
            for victim in victims {
                self.kill(crate::combat::Aim::Unit(victim));
            }
            let Some(index) = self.units.iter().position(|u| u.id == id) else {
                continue;
            };
            let from = (self.units[index].x, self.units[index].y);
            let from = (from.0, from.1, self.ground_z(from.0, from.1));
            let mut unit = self.lift(index);
            unit.x = warp.to.0;
            unit.y = warp.to.1;
            unit.chrono = None;
            unit.warp_until = tick + self.super_rules.chrono_delay as u64;
            if infantry {
                self.spots[at] |= 1 << crate::spot_index(unit.x, unit.y);
                self.spot_house[at] = unit.owner;
            } else {
                self.claims[at] = unit.id + 1;
            }
            let (kind, stats) = (unit.stats.kind, unit.stats);
            let naval = (kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).is_some_and(|k| k.naval);
            self.put_back(unit);
            let to = (warp.to.0, warp.to.1, self.ground_z(warp.to.0, warp.to.1));
            self.events.push(Event::Teleported { kind, from, to });
            let land = self.map.land[at];
            let water = land == Land::Water as u8;
            let bridge = self.map.deck(cell.0, cell.1);
            let ship_ashore = naval && (bridge || land == Land::Road as u8);
            let sinks = water && !naval && !infantry && !bridge;
            let stuck = sinks || ship_ashore || self.map.speed_on(cell.0, cell.1, stats.speed_type, true) <= 0;
            if !stuck {
                continue;
            }
            if water && !infantry {
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                    unit.chrono = Some(Warp { to: warp.to, at: tick, sink: Some(0) });
                    unit.warp_until = u64::MAX;
                }
            } else {
                self.kill(crate::combat::Aim::Unit(id));
            }
        }
        // Held by a time weapon or jumping, a unit
        // gives off a sparkle every 24th frame.
        if tick % 24 == 0 {
            let sparkling: Vec<(i32, i32, i32)> = self
                .units
                .iter()
                .filter(|u| u.frozen.is_some() || (u.chrono.is_some_and(|w| w.sink.is_none())) || (tick < u.warp_until && u.chrono.is_none()))
                .map(|u| (u.x + 0x78, u.y + 0x78, self.ground_z(u.x, u.y) + self.altitude(u).max(0)))
                .collect();
            for at in sparkling {
                self.events.push(Event::Sparkle { at });
            }
        }
    }

    /// Whether the lightning storm is raging (the map is lit by the
    /// scenario's ion lighting while it is).
    pub fn storm_active(&self) -> bool {
        self.storm.as_ref().is_some_and(|storm| storm.started.is_some())
    }

    /// Whether a house's radar is jammed by a storm.
    pub fn radar_jammed(&self, house: u8) -> bool {
        self.houses.get(house as usize).is_some_and(|h| self.tick < h.jammed_until)
    }

    /// Storms and missiles in the air, and units on the move by chronosphere.
    pub(crate) fn run_weather(&mut self) {
        self.fly_nukes();
        self.run_warps();
        // While the storm rages a cloud forms over
        // the target every `LightningHitDelay` frames, and every
        // `LightningScatterDelay` one at a random cell within half
        // `LightningCellSpread` of it that no cloud is within
        // `LightningSeparation` cells of (three tries); a cloud strikes once its
        // animation is past half way, with `LightningDamage` and nobody to
        // blame. The storm is over when its time is up and the last cloud has
        // gone.
        let tick = self.tick;
        let Some(mut storm) = self.storm.take() else {
            return;
        };
        let rules = self.super_rules.storm.clone();
        match storm.started {
            None => {
                storm.deferment = storm.deferment.saturating_sub(1);
                if storm.deferment == 0 {
                    storm.started = Some(tick);
                    // Every house not allied with the
                    // caller and still in the game loses its radar for the
                    // storm's length.
                    let jam = tick + (rules.duration as u64).saturating_sub(1);
                    for house in 0..self.houses.len() {
                        if !self.allied(house as u8, storm.house) && !self.houses[house].defeated {
                            self.houses[house].jammed_until = jam;
                        }
                    }
                    self.events.push(Event::StormStarted { cell: storm.cell, house: storm.house });
                } else if storm.deferment % 225 == 0 {
                    self.events.push(Event::StormApproaching);
                }
            }
            Some(started) => {
                let ending = started + (rules.duration as u64) < tick;
                if !ending {
                    let cloud = |world: &mut World, storm: &mut Storm, cell: (i32, i32)| {
                        let kind = if rules.clouds.is_empty() { 0 } else { world.random_below(rules.clouds.len() as u32) as usize };
                        let (strike, life) = rules.clouds.get(kind).copied().unwrap_or((20, 40));
                        storm.clouds.push((cell, tick + strike as u64));
                        storm.clouds_until = storm.clouds_until.max(tick + life as u64);
                        world.events.push(Event::Cloud { cell, kind: kind as u8 });
                    };
                    if rules.hit_delay > 0 && tick % rules.hit_delay as u64 == 0 {
                        let centre = storm.cell;
                        cloud(self, &mut storm, centre);
                    }
                    if rules.scatter_delay > 0 && tick % rules.scatter_delay as u64 == 0 {
                        for _ in 0..3 {
                            let span = rules.spread.max(1) as u32;
                            let cell = (storm.cell.0 + self.random_below(span) as i32 - rules.spread / 2, storm.cell.1 + self.random_below(span) as i32 - rules.spread / 2);
                            let apart = storm.clouds.iter().all(|(other, _)| (other.0 - cell.0).abs() + (other.1 - cell.1).abs() >= rules.separation);
                            if self.map.in_playfield(cell.0, cell.1) && apart {
                                cloud(self, &mut storm, cell);
                                break;
                            }
                        }
                    }
                }
                let due: Vec<(i32, i32)> = storm.clouds.iter().filter(|(_, at)| *at <= tick).map(|(cell, _)| *cell).collect();
                storm.clouds.retain(|(_, at)| *at > tick);
                for cell in due {
                    let (x, y) = (cell.0 * LEPTONS_PER_CELL + 128, cell.1 * LEPTONS_PER_CELL + 128);
                    let point = (x, y, self.ground_z(x, y));
                    let kind = self.random_below(rules.bolts.max(1)) as u8;
                    self.events.push(Event::Bolt { cell, kind });
                    self.explode(point, rules.damage, rules.warhead, None);
                    self.blast_event(rules.warhead, rules.damage, point, false);
                }
                if ending && storm.clouds.is_empty() && tick >= storm.clouds_until {
                    self.events.push(Event::StormEnded);
                    return;
                }
            }
        }
        self.storm = Some(storm);
    }

    // A target on water is moved to dry
    // ground nearby if there is any; the American drop has its own list, the
    // other takes the Allied or the Soviet list by the house's side; one plane
    // for each entry of the list.
    fn launch_paradrop(&mut self, house: u8, american: bool, cell: (i32, i32)) {
        let dry = |world: &World, cell: (i32, i32)| world.map.index(cell.0, cell.1).is_some_and(|index| world.map.land[index] != Land::Water as u8);
        let mut target = cell;
        if !dry(self, target) {
            'search: for ring in 1..12i32 {
                for dy in -ring..=ring {
                    for dx in -ring..=ring {
                        let at = (cell.0 + dx, cell.1 + dy);
                        if dx.abs().max(dy.abs()) == ring && self.map.in_playfield(at.0, at.1) && dry(self, at) {
                            target = at;
                            break 'search;
                        }
                    }
                }
            }
        }
        let rules = self.super_rules.clone();
        let list = if american {
            rules.american
        } else {
            match self.houses[house as usize].side { 0 => rules.allied, 1 => rules.soviet, 2 => rules.third, _ => Vec::new() }
        };
        for (soldier, count) in list {
            self.create_paradrop(house, target, soldier, count);
        }
    }

    // The plane is made at a cell on the house's edge of the map on the ground,
    // with the soldiers inside and the drop cell as its target, on the approach
    // mission.
    fn create_paradrop(&mut self, house: u8, target: (i32, i32), soldier: u16, count: u32) {
        let Some(stats) = self.super_rules.plane.and_then(|kind| self.unit_types.get(kind as usize).map(|t| UnitStats { kind: kind + 1, ..t.stats })) else {
            return;
        };
        let Some(plane_stats) = stats.plane else {
            return;
        };
        let Some(soldier_stats) = self.unit_types.get(soldier as usize).map(|t| UnitStats { kind: soldier + 1, infantry: true, ..t.stats }) else {
            return;
        };
        let entry = self.edge_cell();
        let id = self.next_id;
        self.next_id += 1;
        let mut unit = Unit::new(id, house, entry, 0, stats);
        let mut plane = Plane::parked(&plane_stats);
        // It is born at flight level, facing north.
        plane.z = plane_stats.flight_level;
        plane.mission = Mission::ParadropApproach;
        plane.drop = Some(target);
        plane.passes = 5;
        unit.plane = Some(plane);
        for _ in 0..count {
            let id = self.next_id;
            self.next_id += 1;
            unit.cargo.push(Unit::new(id, house, entry, 0x80, soldier_stats));
        }
        self.units.push(unit);
    }

    /// A cell on the top edge of the playing area, anywhere along it.
    // The edge and a random place along it.
    pub(crate) fn edge_cell(&mut self) -> (i32, i32) {
        let (width, height) = self.map.size();
        let Some((w, [lx, ly, lw, _])) = self.map.playfield else {
            return (self.random_below(width.max(1) as u32) as i32, 0);
        };
        let sum = w + 2 * ly + 2;
        let (low, high) = (2 * lx - w + 2, 2 * (lx + lw) - w - 2);
        let mut across = low + self.random_below((high - low).max(1) as u32) as i32;
        if (sum + across) % 2 != 0 {
            across += 1;
        }
        (((sum + across) / 2).clamp(0, width - 1), ((sum - across) / 2).clamp(0, height - 1))
    }

    /// The paradrop plane's missions. Returns the frames until the next.
    pub(crate) fn paradrop_mission(&mut self, index: usize, plane: &mut Plane) -> u64 {
        let radius = self.super_rules.radius as i64;
        let (id, here, at, loaded) = {
            let unit = &self.units[index];
            (unit.id, unit.cell(), (unit.x, unit.y), !unit.cargo.is_empty())
        };
        let far = |cell: (i32, i32)| {
            let (dx, dy) = ((cell.0 * LEPTONS_PER_CELL + 128 - at.0) as i64, (cell.1 * LEPTONS_PER_CELL + 128 - at.1) as i64);
            dx * dx + dy * dy > radius * radius
        };
        match plane.mission {
            Mission::ParadropApproach => {
                if plane.drop.is_none() {
                    self.plane_retreat(index, plane);
                    return 3;
                }
                if plane.drop.is_some_and(|target| !far(target)) {
                    plane.mission = Mission::ParadropOverfly;
                    plane.passes = plane.passes.saturating_sub(1);
                }
                3
            }
            Mission::ParadropOverfly => {
                match plane.drop.filter(|_| loaded) {
                    Some(target) if far(target) => {
                        if plane.passes > 0 {
                            plane.mission = Mission::ParadropApproach;
                        } else {
                            self.plane_retreat(index, plane);
                        }
                    }
                    Some(_) => {
                        if self.map.in_playfield(here.0, here.1) {
                            self.drop_one(index, plane);
                        }
                    }
                    None => self.plane_retreat(index, plane),
                }
                5
            }
            // It flies for a cell on the
            // edge and is gone on the first frame it is off the playing area
            // (`paradrop_fly`).
            _ => {
                let _ = id;
                5
            }
        }
    }

    /// One frame of the cargo plane's flight. Returns whether it has left the map.
    // It keeps
    // `FlightLevel` above the ground under it and never comes down; 38 leptons
    // a frame; it turns towards where it is going by its `ROT` (two of 256 a
    // frame), but while dropping it flies dead straight; on Retreat it is
    // removed when it is off the playing area.
    pub(crate) fn paradrop_fly(&mut self, index: usize, plane: &mut Plane) -> bool {
        let level = self.units[index].stats.plane.map_or(1500, |stats| stats.flight_level);
        plane.z = level;
        let goal = match plane.mission {
            Mission::ParadropApproach => plane.drop,
            Mission::Retreat => plane.retreat,
            _ => None,
        };
        let unit = &mut self.units[index];
        if let (Mission::Retreat, Some(goal)) = (plane.mission, goal) {
            let (dx, dy) = ((goal.0 * LEPTONS_PER_CELL + 128 - unit.x) as f64, (goal.1 * LEPTONS_PER_CELL + 128 - unit.y) as f64);
            if dx.hypot(dy) < LEPTONS_PER_CELL as f64 {
                plane.retreat = None;
            }
        }
        let goal = if plane.mission == Mission::Retreat { plane.retreat } else { goal };
        if let Some(goal) = goal {
            let (dx, dy) = ((goal.0 * LEPTONS_PER_CELL + 128 - unit.x) as f64, (goal.1 * LEPTONS_PER_CELL + 128 - unit.y) as f64);
            let wanted = (dx.atan2(-dy) / std::f64::consts::TAU * 256.0).rem_euclid(256.0) as i32;
            let turn = ((wanted - unit.facing as i32 + 128).rem_euclid(256)) - 128;
            unit.facing = ((unit.facing as i32 + turn.clamp(-2, 2)).rem_euclid(256)) as u8;
            // At full speed, so above any `PitchSpeed` under 1, it leans into
            // the turn.
            let fast = unit.stats.plane.is_some_and(|stats| stats.pitch_speed < 1000);
            plane.bank = if fast { turn.signum() as i8 } else { 0 };
        } else {
            plane.bank = 0;
        }
        let angle = unit.facing as f64 / 256.0 * std::f64::consts::TAU;
        unit.x += (angle.sin() * 38.0).round() as i32;
        unit.y -= (angle.cos() * 38.0).round() as i32;
        let cell = unit.cell();
        plane.mission == Mission::Retreat && !self.map.in_playfield(cell.0, cell.1)
    }

    fn plane_retreat(&mut self, _index: usize, plane: &mut Plane) {
        let edge = self.edge_cell();
        plane.drop = None;
        plane.retreat = Some(edge);
        plane.mission = Mission::Retreat;
    }

    // The first passenger goes out 128 leptons to
    // one side of the plane, sides taking turns, at the plane's height; if the
    // cell there will not have it, it stays aboard. Each one out sets the
    // passes left to five. It faces
    // south, falling under a parachute, and stands guard.
    fn drop_one(&mut self, index: usize, plane: &mut Plane) {
        if self.units[index].cargo.is_empty() {
            return;
        }
        plane.drops = plane.drops.wrapping_sub(1);
        let unit = &self.units[index];
        let turn = if plane.drops & 1 != 0 { -64i32 } else { 64 };
        let angle = ((unit.facing as i32 + turn) & 0xff) as f64 / 256.0 * std::f64::consts::TAU;
        let (x, y) = (unit.x + (angle.sin() * 128.0) as i32, unit.y - (angle.cos() * 128.0) as i32);
        let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        let (height, carrier) = (plane.z, unit.id);
        let open = self.map.index(cell.0, cell.1).filter(|&at| {
            self.map.in_playfield(cell.0, cell.1)
                && self.map.speed_percent(cell.0, cell.1, self.units[index].cargo[0].stats.speed_type) > 0
                && self.claims[at] == 0
                && self.building_on(cell).is_none()
        });
        let spot = open.and_then(|at| (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0).map(|spot| (at, spot)));
        let Some((at, spot)) = spot else {
            plane.drops = plane.drops.wrapping_add(1);
            return;
        };
        let mut soldier = self.units[index].cargo.remove(0);
        soldier.x = cell.0 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].0;
        soldier.y = cell.1 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].1;
        soldier.facing = 0x80;
        soldier.falling = Some(height.max(1));
        soldier.dropped_at = self.tick;
        self.spots[at] |= 1 << spot;
        self.spot_house[at] = soldier.owner;
        self.events.push(Event::Parachuted { unit: soldier.id, from: carrier });
        self.put_back(soldier);
        plane.passes = 5;
    }
}

#[cfg(test)]
mod tests {
    use super::curtain_brightness;

    #[test]
    fn the_iron_curtain_flares_pulses_and_fades() {
        let at = |e| curtain_brightness(e, 750);
        assert_eq!(at(0), 1.0);
        assert_eq!(at(7), 2.0);
        assert_eq!(at(10), 2.0);
        assert!((at(31) - 0.2).abs() < 0.01);
        assert!((at(39) - 0.5).abs() < 0.01);
        assert!((at(55) - 0.2).abs() < 0.01);
        assert!((at(710) - 0.2).abs() < 0.01);
        assert_eq!(at(726), 2.0);
        assert!(at(749) < 1.1 && at(749) > 0.9);
        assert_eq!(at(750), 1.0);
    }
}
