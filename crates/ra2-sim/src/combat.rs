//! Combat: weapons, the fire test, shots in flight, explosions, damage and death.
//!
//! The rules match the original game's. Where the engine draws from its synchronised random numbers this uses
//! the world's own generator instead (the engine's is not reproduced yet).
//!

use super::*;

/// Kinds of death that leave nothing behind: worn out of existence, and used up.
const ERASED: i32 = -1;
const VANISHED: i32 = -2;

/// The engine's height of one level of ground, in leptons.
pub const LEVEL_HEIGHT: i32 = 104;

/// Armour names in the order the engine numbers them.
pub const ARMOURS: [&str; 11] =
    ["none", "flak", "plate", "light", "medium", "heavy", "wood", "steel", "concrete", "special_1", "special_2"];

/// A warhead type, from rules.ini.
// The keys and their defaults.
#[derive(Clone, Debug)]
pub struct Warhead {
    /// `Verses`: the share of damage each armour takes.
    pub verses: [f64; 11],
    pub cell_spread: f32,
    pub percent_at_max: f32,
    pub prone_damage: f64,
    pub inf_death: i32,
    pub wall: bool,
    pub wood: bool,
    pub conventional: bool,
    /// `Rocker`: it rocks the vehicles round where it goes off.
    pub rocker: bool,
    /// `Temporal`: it freezes its target and wears it out of existence.
    pub temporal: bool,
    /// `IvanBomb`, `BombDisarm`, `MindControl`, `Parasite`, `MakesDisguise`.
    pub ivan_bomb: bool,
    pub bomb_disarm: bool,
    pub mind_control: bool,
    pub parasite: bool,
    pub makes_disguise: bool,
    /// `AnimList`, as numbers in the world's list of animation types.
    pub anim_list: Vec<u16>,
    /// `ElectricAssault`: a foot soldier's shot with it charges a building.
    pub electric_assault: bool,
    /// `PsychicDamage`: does nothing to `ImmuneToPsionics` targets.
    pub psychic_damage: bool,
    /// How many animations `AnimList` names.
    pub anims: usize,
}

impl Default for Warhead {
    fn default() -> Self {
        Self {
            verses: [1.0; 11],
            cell_spread: 0.0,
            percent_at_max: 1.0,
            prone_damage: 1.0,
            inf_death: 0,
            wall: false,
            wood: false,
            conventional: false,
            rocker: false,
            temporal: false,
            ivan_bomb: false,
            bomb_disarm: false,
            mind_control: false,
            parasite: false,
            makes_disguise: false,
            anim_list: Vec::new(),
            electric_assault: false,
            psychic_damage: false,
            anims: 0,
        }
    }
}

/// A projectile type, from rules.ini.
// The keys and their defaults.
#[derive(Clone, Debug)]
pub struct Projectile {
    pub rot: i32,
    pub arcing: bool,
    pub inviso: bool,
    pub aa: bool,
    pub ag: bool,
    pub arm: i32,
    pub acceleration: i32,
    pub floater: bool,
    pub inaccurate: bool,
    pub ranged: bool,
    pub degenerates: bool,
    pub level: bool,
    pub very_high: bool,
    pub course_lock: i32,
    pub subject_to_elevation: bool,
    /// `FlakScatter`: an inaccurate shot strays by a share of its distance.
    pub flak_scatter: bool,
    /// `ShrapnelWeapon` and `ShrapnelCount`: the weapon thrown out where the shot
    /// lands, and how many times.
    pub shrapnel: Option<(u16, i32)>,
    /// `Vertical` and `DetonationAltitude`: the nuclear missile's way of flying.
    pub vertical: bool,
    pub detonation_altitude: i32,
}

impl Default for Projectile {
    fn default() -> Self {
        Self {
            rot: 0,
            arcing: false,
            inviso: false,
            aa: false,
            shrapnel: None,
            ag: true,
            arm: 0,
            acceleration: 3,
            floater: false,
            inaccurate: false,
            ranged: false,
            degenerates: false,
            level: false,
            very_high: false,
            course_lock: 0,
            subject_to_elevation: false,
            flak_scatter: false,
            vertical: false,
            detonation_altitude: 0,
        }
    }
}

/// A weapon type, from rules.ini.
// The keys and their defaults.
#[derive(Clone, Debug)]
pub struct Weapon {
    pub damage: i32,
    /// Frames between shots.
    pub rof: i32,
    /// `Range` and `MinimumRange` in leptons; a range of -512 is "anywhere".
    pub range: i32,
    pub minimum_range: i32,
    /// `Speed` as the engine stores it (0 to 255); used only by homing shots.
    pub speed: i32,
    pub burst: i32,
    pub projectile: u16,
    pub warhead: u16,
    pub omni_fire: bool,
    pub cell_rangefinding: bool,
    pub lobber: bool,
    pub bright: bool,
    /// `FireOnce`: the target is let go after one shot. `LimboLaunch`: the firer
    /// itself goes into the target.
    pub fire_once: bool,
    pub limbo_launch: bool,
    /// `Suicide`: firing it is the firer's own death.
    pub suicide: bool,
    /// `AreaFire`: fired at the ground under the firer's feet.
    pub area_fire: bool,
    /// `RadLevel`: the radiation its blast leaves.
    pub rad_level: i32,
    /// `IsSonic` and `AmbientDamage`: a wave that hurts what it passes over.
    pub is_sonic: bool,
    /// `SabotageCursor`: its attack shows the charge-planting cursor.
    pub sabotage_cursor: bool,
    /// `NeverUse`: never chosen to fire (it only widens the guard scan).
    pub never_use: bool,
    pub ambient_damage: i32,
    /// `Spawner`: firing it sends the unit's spawns at the target.
    pub spawner: bool,
    /// `DisguiseFakeBlinkTime`: frames a Mirage firing it shows its true self.
    pub fake_blink: u32,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            fake_blink: 0,
            damage: 0,
            rof: 0,
            range: 0,
            minimum_range: 0,
            speed: 0,
            burst: 1,
            projectile: 0,
            warhead: 0,
            omni_fire: false,
            cell_rangefinding: false,
            lobber: false,
            bright: false,
            fire_once: false,
            limbo_launch: false,
            area_fire: false,
            rad_level: 0,
            is_sonic: false,
            sabotage_cursor: false,
            never_use: false,
            ambient_damage: 0,
            suicide: false,
            spawner: false,
        }
    }
}

/// A weapon as a unit or building type carries it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mount {
    /// Index into the world's weapons.
    pub weapon: u16,
    /// art.ini `PrimaryFireFLH` and the like: forward, lateral, height.
    pub flh: (i32, i32, i32),
}

/// What makes a type a fighter or a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fighter {
    pub armour: u8,
    pub primary: Option<Mount>,
    pub secondary: Option<Mount>,
    /// `Turret`: the weapon turns apart from the body.
    pub turret: bool,
    /// `TurretOffset`, in leptons along the body.
    pub turret_offset: i32,
    pub mobile_fire: bool,
    pub opportunity_fire: bool,
    pub can_passive_acquire: bool,
    /// `DefaultToGuardArea`: left alone it guards the ground round where it
    /// stands, going after what comes near.
    pub default_guard_area: bool,
    pub can_retaliate: bool,
    pub can_approach_target: bool,
    /// `GuardRange`, in leptons.
    pub guard_range: i32,
    /// `Sight`, in cells.
    pub sight: i32,
    pub legal_target: bool,
    pub insignificant: bool,
    pub immune: bool,
    /// For a foot soldier: the frame of its firing sequence at which the shot
    /// leaves, and how many frames the sequence has.
    pub fire_up: u16,
    pub fire_frames: u16,
    /// The same lying down: `FireProne` and the `FireProne` sequence.
    pub fire_prone: u16,
    pub fire_prone_frames: u16,
    /// Frames of its `Down` and `Up` sequences: a type without one cannot lie
    /// down or get up.
    pub down_frames: u16,
    pub up_frames: u16,
    /// `Fearless`, `Fraidycat` and `Crawls`: how a soldier takes fire.
    pub fearless: bool,
    pub fraidycat: bool,
    pub crawls: bool,
    /// `Engineer`: it takes or mends buildings by walking into them.
    pub engineer: bool,
    /// `Occupier`: a soldier that can go into civilian buildings and fire from
    /// them.
    pub occupier: bool,
    /// `C4`: a soldier that blows buildings up from inside.
    pub c4: bool,
    /// `Ivan` (bombs), `Agent` (a spy), `DetectDisguise`, `ImmuneToPsionics`,
    /// `Parasiteable`, `Organic`.
    /// An aircraft type (its range is measured over the ground).
    pub aircraft: bool,
    /// `CanDisguise` and `PermaDisguise`.
    pub can_disguise: bool,
    pub perma_disguise: bool,
    /// `DisguiseWhenStill`: a Mirage, which takes the look of a tree when it
    /// stands still.
    pub disguise_when_still: bool,
    pub ivan: bool,
    pub agent: bool,
    pub detect_disguise: bool,
    pub immune_to_psionics: bool,
    pub immune_to_radiation: bool,
    pub parasiteable: bool,
    pub organic: bool,
    /// The type's own `ThreatPosed`.
    pub threat_posed: i32,
    /// `Capturable`, for a building.
    pub capturable: bool,
    /// `Warpable`: a `Temporal` weapon works on it.
    pub warpable: bool,
    /// `Deployer` and `DeployFire`: a foot soldier that digs in, and fires its
    /// second weapon when it has; with the frames its `Deploy` and `Undeploy`
    /// sequences take.
    pub deployer: bool,
    pub deploy_fire: bool,
    /// `UndeployDelay`: frames after deploying at which it gets up by itself;
    /// -1 for a type that stays down.
    pub undeploy_delay: i32,
    pub deploy_frames: u16,
    pub undeploy_frames: u16,
}

impl Default for Fighter {
    // The type constructor's
    // defaults for firing on the move, acquiring and retaliating.
    fn default() -> Self {
        Self {
            armour: 0,
            primary: None,
            secondary: None,
            turret: false,
            turret_offset: 0,
            mobile_fire: true,
            opportunity_fire: false,
            can_passive_acquire: true,
            default_guard_area: false,
            can_retaliate: true,
            can_approach_target: true,
            guard_range: 0,
            sight: 0,
            legal_target: true,
            insignificant: false,
            immune: false,
            fire_up: 0,
            fire_frames: 0,
            fire_prone: 0,
            fire_prone_frames: 0,
            down_frames: 0,
            up_frames: 0,
            fearless: false,
            fraidycat: false,
            crawls: true,
            engineer: false,
            occupier: false,
            c4: false,
            aircraft: false,
            can_disguise: false,
            perma_disguise: false,
            disguise_when_still: false,
            ivan: false,
            agent: false,
            detect_disguise: false,
            immune_to_psionics: false,
            immune_to_radiation: false,
            parasiteable: true,
            organic: false,
            threat_posed: 0,
            capturable: false,
            warpable: true,
            deployer: false,
            deploy_fire: false,
            undeploy_delay: -1,
            deploy_frames: 0,
            undeploy_frames: 0,
        }
    }
}

/// What can be aimed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Aim {
    Unit(u32),
    Building(u32),
    Cell(i32, i32),
}

/// The fighting state of one unit or building.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Arms {
    pub target: Option<Aim>,
    /// The target was picked by the unit itself, not ordered.
    passive: bool,
    /// Ordered to attack: it closes with the target and keeps at it.
    pub attacking: bool,
    /// The tick the weapon is ready again, and the wait it was given.
    rearm_at: u64,
    rearm_for: u64,
    burst: u32,
    /// The tick of the next look for targets.
    scan_at: u64,
    /// The turret's facing (for a building, its one facing).
    turret: Turn,
    /// A foot soldier's firing sequence: the stage it is at, plus one; zero when
    /// not firing.
    pub firing: u16,
    /// The tick of the last shot, for whoever draws it.
    pub fired_at: u64,
    /// Where it was sent to get in range, to tell that from an ordered move.
    approach: Option<(i32, i32)>,
    /// What its `Temporal` weapon has hold of.
    erasing: Option<Aim>,
}

/// How a foot soldier that can dig in stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Stance {
    #[default]
    Standing,
    /// Digging in or getting up, until a tick.
    Deploying(u64),
    Deployed,
    Undeploying(u64),
}

/// What walking into a building would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Wire it to blow up.
    Sabotage,
    /// Go inside and fire from it.
    Occupy,
    Capture,
    Repair,
    NothingToRepair,
    /// A spy going in.
    Infiltrate,
    /// A hurt soldier into a hospital, or one not yet elite into an armory.
    Treat,
    /// An engineer into a bridge repair hut.
    RepairBridge,
}

impl Arms {
    /// Which of `count` turrets an `IsChargeTurret` type shows: the last just
    /// after a shot, stepping back to the first as the weapon recharges.
    pub fn charge_turret(&self, now: u64, count: u8) -> u8 {
        let left = self.rearm_at.saturating_sub(now);
        if self.rearm_for == 0 || count == 0 {
            return 0;
        }
        (count as u64 * left / self.rearm_for).min(count as u64 - 1) as u8
    }

    /// The turret's facing, 0 to 255.
    pub fn turret_facing(&self) -> u8 {
        (self.turret.current() >> 8) as u8
    }

    pub(crate) fn facing(facing: u8) -> Self {
        Self { turret: Turn::facing(facing), ..Self::default() }
    }
}

/// A shot in flight.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Bullet {
    pub id: u32,
    pub weapon: u16,
    owner: Option<Aim>,
    pub house: u8,
    target: Aim,
    /// Where the target was at launch.
    target_at: (i32, i32, i32),
    pub position: (f64, f64, f64),
    pub velocity: (f64, f64, f64),
    speed: i32,
    damage: i32,
    arm_at: u64,
    fuse_last: i32,
    locked: bool,
    lock_count: i32,
    samples: i32,
    closing: f64,
    born: u64,
}

impl Bullet {
    /// Its facing, 0 to 255, from the way it is moving.
    pub fn facing(&self) -> u8 {
        (facing_of((-self.velocity.1).atan2(self.velocity.0)) as u16 >> 8) as u8
    }
}

/// A sonic wave on its way from its firer to its target.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Wave {
    pub owner: Aim,
    pub target: Aim,
    pub weapon: u16,
    pub born: u64,
}

/// Events the app has not taken yet. They are written out as none, so a
/// snapshot or state hash does not depend on when this machine last took them.
#[derive(Default)]
pub(crate) struct Pending(pub(crate) Vec<Event>);

impl std::ops::Deref for Pending {
    type Target = Vec<Event>;
    fn deref(&self) -> &Vec<Event> {
        &self.0
    }
}

impl std::ops::DerefMut for Pending {
    fn deref_mut(&mut self) -> &mut Vec<Event> {
        &mut self.0
    }
}

impl serde::Serialize for Pending {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Vec::<Event>::new().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Pending {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Older snapshots wrote them; they are dropped.
        Vec::<Event>::deserialize(deserializer).map(|_| Pending::default())
    }
}

/// Something that happened this tick that is seen or heard.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Event {
    /// A weapon went off at a point, pointing a way (0 to 255).
    Fired { weapon: u16, at: (i32, i32, i32), to: (i32, i32, i32), facing: u8, by: Aim },
    /// A warhead went off: the damage it carried, where, and whether on water.
    /// `anim` is the explosion chosen for it, by its number in the world's
    /// list of animation types.
    Detonated { warhead: u16, damage: i32, at: (i32, i32, i32), water: bool, anim: u16 },
    /// A unit died: its type tag, where, and for a soldier the kind of death.
    UnitDied { id: u32, kind: u16, owner: u8, at: (i32, i32, i32), facing: u8, infantry: bool, death: i32, explosion: u16 },
    /// `explosions`: one for each cell of the foundation, row by row.
    BuildingDied { id: u32, kind: u16, owner: u8, cell: (i32, i32), size: (i32, i32), explosions: Vec<u16> },
    /// A parasite bit the vehicle it is in, rocking it from a point `side`
    /// leptons to its left or right.
    Bitten { host: u32, weapon: u16, side: i32 },
    /// A unit jumped from one place to another.
    Teleported { kind: u16, from: (i32, i32, i32), to: (i32, i32, i32) },
    /// Something was worn out of existence.
    Erased { at: (i32, i32, i32) },
    /// A building changed hands, or was mended by an engineer.
    Captured { building: u32, by: u8, from: u8 },
    /// A fragment thrown from where a shot landed to another point.
    Forked { weapon: u16, from: (i32, i32, i32), to: (i32, i32, i32), house: u8 },
    /// A bomb has been stuck on something; a mind has been taken.
    BombPlanted { target: Aim, house: u8 },
    /// A soldier has gone under a vehicle's tracks.
    Crushed { kind: u16, at: (i32, i32, i32) },
    /// A unit has begun to cloak or to uncloak.
    Cloaked { unit: u32 },
    /// A unit has gone up a rank.
    Promoted { unit: u32, owner: u8, elite: bool },
    /// A crate was picked up: by whom, of which house, where, and what it
    /// turned out to hold (`powerups::NAMES`).
    CrateCollected { unit: u32, house: u8, cell: (i32, i32), kind: u8 },
    /// A spy has gone into a building: what it did there (1 radar, 2 power, 3
    /// a super weapon, 4 a laboratory, 5 money, 6 a vehicle factory, 7 a
    /// barracks, 0 nothing).
    Infiltrated { building: u32, victim: u8, by: u8, effect: u8 },
    /// A house has gained, lost, or finished charging a super weapon.
    SuperGranted { house: u8, index: u16 },
    SuperLost { house: u8, index: u16 },
    /// A super weapon was fired at a cell (the chronosphere's second click
    /// also says where from).
    SuperFired { house: u8, kind: i32, cell: (i32, i32), from: Option<(i32, i32)> },
    /// The lightning storm has begun (called by `house`); it is on its way; one
    /// could not be called (a storm is already on its way or raging); it is
    /// over.
    StormStarted { cell: (i32, i32), house: u8 },
    StormApproaching,
    StormRefused { house: u8 },
    StormEnded,
    /// A cloud of the storm forms, and a bolt comes down: which of
    /// `WeatherConClouds` and `WeatherConBolts`.
    Cloud { cell: (i32, i32), kind: u8 },
    Bolt { cell: (i32, i32), kind: u8 },
    /// A nuclear missile has left its silo; it has come down and its fireball
    /// begins.
    NukeLaunched { house: u8, at: (i32, i32, i32), cell: (i32, i32) },
    NukeLanded { at: (i32, i32, i32) },
    /// A bubble over a sinking vehicle; a sparkle over something held by a
    /// time weapon or jumping.
    Wake { at: (i32, i32, i32) },
    Sparkle { at: (i32, i32, i32) },
    /// A house's building that shows itself to all has been finished: which
    /// super weapon it holds.
    SuperDetected { owner: u8, index: u16, cell: (i32, i32) },
    /// A bridge section has gone; a bridge has been put back.
    BridgeBroken { cell: (i32, i32) },
    BridgeRepaired { house: u8, cell: (i32, i32) },
    /// A repair depot starts on a vehicle; a vehicle leaves one mended.
    DepotRepairing { house: u8 },
    UnitRepaired { house: u8 },
    /// A house is out of the game.
    HouseDefeated { house: u8 },
    /// The game is over; `World::over` has the houses left.
    GameOver,
    SuperReady { house: u8, index: u16 },
    /// A soldier has left a plane under a parachute.
    Parachuted { unit: u32, from: u32 },
    MindControlled { controller: u32, victim: u32 },
    /// An aircraft has left the ground, or come down.
    TookOff { unit: u32 },
    Landed { unit: u32 },
    /// An aircraft coming down is near the ground (its landing sound).
    Landing { unit: u32 },
    /// Someone has got into a transport, or out of it.
    Boarded { transport: u32 },
    Alighted { transport: u32 },
    /// Soldiers have gone into an empty building, or the last have left one.
    Garrisoned { building: u32, by: u8 },
    Abandoned { building: u32, by: u8 },
    Mended { building: u32, owner: u8 },
    /// A building began to be sold, and one finished.
    SaleBegan { owner: u8 },
    Sold { owner: u8 },
    /// A blast rocks a unit: from where, and how hard.
    Rocked { unit: u32, from: (i32, i32, i32), strength: f32 },
    /// A house's unit or building was hit.
    /// Something was hurt: whose, where, whether a building, and whether a
    /// miner that lived through it.
    Attacked { owner: u8, at: (i32, i32), building: bool, miner: bool },
}

/// Fire-test answers.
// The numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FireError {
    Ok,
    Facing,
    Rearm,
    Rotating,
    Illegal,
    Cant,
    Moving,
    Range,
}

/// The angle of a facing: ` × (−π/32768)`.
fn rad(facing: i32) -> f64 {
    (facing as i16 as f64 - 16383.0) * (-std::f64::consts::PI / 32768.0)
}

pub(crate) fn facing_of(angle: f64) -> i16 {
    ((angle - std::f64::consts::FRAC_PI_2) * (-32768.0 / std::f64::consts::PI)) as i32 as i16
}

/// Turns a 16-bit facing towards another by at most a step.
fn turn_towards(from: i16, to: i16, step: i32) -> i16 {
    let difference = to.wrapping_sub(from) as i32;
    if difference.abs() <= step { to } else { from.wrapping_add((step * difference.signum()) as i16) }
}

fn length3(d: (f64, f64, f64)) -> f64 {
    (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt()
}

/// The cells an explosion reaches, ring by ring.
// Ring r holds the cells
// whose larger offset plus half the smaller is r, by rows then columns; the first
// ring's own order is as given.
pub(crate) fn spread_offsets(rings: i32) -> Vec<(i32, i32)> {
    let mut cells = vec![(0, 0)];
    if rings >= 1 {
        cells.extend([(1, -1), (0, -1), (-1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]);
    }
    for ring in 2..=rings {
        for y in -ring..=ring {
            for x in -ring..=ring {
                let (ax, ay) = (x.abs(), y.abs());
                if ax.max(ay) + ax.min(ay) / 2 == ring {
                    cells.push((x, y));
                }
            }
        }
    }
    cells
}

impl World {
    /// Takes something off the map as destroyed.
    pub(crate) fn kill(&mut self, who: Aim) {
        self.dying.push((who, 0));
        self.bury();
    }

    /// How far off the ground a unit is, in leptons.
    pub fn altitude(&self, unit: &Unit) -> i32 {
        if let Some(depth) = unit.chrono.and_then(|warp| warp.sink) {
            return -depth;
        }
        if let Some(plane) = unit.plane {
            return plane.z;
        }
        if let Some(height) = unit.falling {
            return height;
        }
        if let Some(rocket) = unit.rocket {
            return rocket.z as i32 - self.ground_z(unit.x, unit.y);
        }
        unit.jet.map_or(0, |jet| jet.z - self.ground_z(unit.x, unit.y))
    }

    /// Whether something counts as in the air to a weapon.
    // 208 leptons or more off the ground.
    pub(crate) fn in_air(&self, who: Aim) -> bool {
        match who {
            Aim::Unit(id) => self.unit(id).is_some_and(|unit| self.altitude(unit) >= 208),
            _ => false,
        }
    }

    /// What happened in the last tick, handed over once.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events.0)
    }

    pub fn bullets(&self) -> &[Bullet] {
        &self.bullets
    }

    pub fn ground_z(&self, x: i32, y: i32) -> i32 {
        self.map.level(x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL)) as i32 * LEVEL_HEIGHT
    }

    pub(crate) fn fighter(&self, who: Aim) -> Option<Fighter> {
        match who {
            Aim::Unit(id) => self.unit(id).map(|unit| unit.stats.fighter),
            Aim::Building(id) => {
                let building = self.buildings.iter().find(|b| b.id == id)?;
                self.building_types.get(building.kind as usize).map(|kind| kind.fighter)
            }
            Aim::Cell(..) => None,
        }
    }

    pub fn owner_of(&self, who: Aim) -> Option<u8> {
        match who {
            Aim::Unit(id) => self.unit(id).map(|unit| unit.owner),
            Aim::Building(id) => self.buildings.iter().find(|b| b.id == id).map(|b| b.owner),
            Aim::Cell(..) => None,
        }
    }

    /// Whether something aimed at is still there to be hit.
    pub(crate) fn alive(&self, who: Aim) -> bool {
        match who {
            Aim::Unit(id) => self.unit(id).is_some_and(|unit| unit.health > 0),
            Aim::Building(id) => self.buildings.iter().any(|b| b.id == id && b.health > 0),
            Aim::Cell(x, y) => self.map.index(x, y).is_some(),
        }
    }

    /// The point a thing is hit at: a unit's own position, a building's middle,
    /// a cell's centre.
    // The centre coordinates.
    pub fn aim_point(&self, who: Aim) -> Option<(i32, i32, i32)> {
        match who {
            // On a bridge's deck it stands 416 leptons
            // (four levels) above the ground of its cell.
            Aim::Unit(id) => self.unit(id).map(|unit| (unit.x, unit.y, self.ground_z(unit.x, unit.y) + self.altitude(unit) + if unit.on_bridge { 4 * LEVEL_HEIGHT } else { 0 })),
            Aim::Building(id) => self.buildings.iter().find(|b| b.id == id).map(|b| {
                let (x, y) = (b.cell.0 * LEPTONS_PER_CELL + b.size.0 * 128, b.cell.1 * LEPTONS_PER_CELL + b.size.1 * 128);
                (x, y, self.map.level(b.cell.0, b.cell.1) as i32 * LEVEL_HEIGHT)
            }),
            Aim::Cell(x, y) => {
                let (cx, cy) = cell_centre((x, y));
                self.map.index(x, y).map(|_| (cx, cy, self.ground_z(cx, cy)))
            }
        }
    }

    /// Whether two houses will not shoot each other.
    pub fn allied(&self, a: u8, b: u8) -> bool {
        a == b || self.houses.get(a as usize).is_some_and(|house| b < 32 && house.allies & (1 << b) != 0)
    }

    /// Allies the computer houses with each other.
    // When a
    // computer house in a skirmish or network game looks to its base plan, and
    // the rules' `AlliesAllowed` is set, every computer house allies with every
    // other and makes every person's house its enemy. Both `MPBattle.ini` and
    // `MPFreeForAll.ini` set it, so in either mode the computer houses stand
    // together against the player.
    pub fn paranoid_alliances(&mut self) {
        let computers: u32 = (0..self.houses.len().min(32)).filter(|&h| self.houses[h].ai.is_some()).map(|h| 1u32 << h).sum();
        for (index, house) in self.houses.iter_mut().enumerate().take(32) {
            if house.ai.is_some() {
                house.allies = computers & !(1 << index);
            }
        }
    }

    /// Whether a house is a person's.
    pub(crate) fn human(&self, house: u8) -> bool {
        self.houses.get(house as usize).is_some_and(|house| house.human)
    }

    /// The weapon something uses on a target: 0 primary, 1 secondary.
    // The secondary only when the primary's warhead
    // does nothing to the target's armour.
    pub(crate) fn select_weapon(&self, fighter: &Fighter, target: Aim) -> usize {
        let (Some(primary), Some(secondary)) = (fighter.primary, fighter.secondary) else {
            return 0;
        };
        // A second weapon
        // marked `NeverUse` is never the one chosen (the dog's and the
        // engineer's `VirtualScanner`).
        if self.weapons.get(secondary.weapon as usize).is_some_and(|weapon| weapon.never_use) {
            return 0;
        }
        let Some(armour) = self.fighter(target).map(|t| t.armour as usize) else {
            return 0;
        };
        let verses = |mount: Mount| {
            self.weapons.get(mount.weapon as usize).and_then(|w| self.warheads.get(w.warhead as usize)).map_or(1.0, |w| w.verses[armour.min(10)])
        };
        if verses(secondary) == 0.0 {
            0
        } else if verses(primary) == 0.0 {
            1
        } else {
            // Otherwise the second weapon is for
            // things in the air, if its projectile is `AA`.
            let aa = self.weapons.get(secondary.weapon as usize).and_then(|w| self.projectiles.get(w.projectile as usize)).is_some_and(|p| p.aa);
            (aa && self.in_air(target)) as usize
        }
    }

    /// The same for something of a house, which matters for one case.
    // The second weapon on a building it would
    // charge.
    pub(crate) fn select_weapon_of(&self, house: u8, fighter: &Fighter, target: Aim) -> usize {
        if self.charges(house, fighter, target) { 1 } else { self.select_weapon(fighter, target) }
    }

    /// The weapon a unit would use on something.
    pub fn weapon_against(&self, unit: u32, target: Aim) -> Option<&Weapon> {
        let (fighter, owner) = self.unit(unit).map(|unit| (unit.stats.fighter, unit.owner))?;
        Self::mount(&fighter, self.select_weapon_of(owner, &fighter, target)).and_then(|mount| self.weapons.get(mount.weapon as usize))
    }

    pub(crate) fn mount(fighter: &Fighter, index: usize) -> Option<Mount> {
        if index == 0 { fighter.primary } else { fighter.secondary }
    }

    /// The range test from a point.
    // The minimum range in three dimensions; arcing
    // shots by distance over the ground; others in three dimensions, with a
    // building's foundation allowed for; a range of -512 always passes.
    fn in_range(&self, from: (i32, i32, i32), target: Aim, weapon: &Weapon) -> bool {
        self.in_range_for(from, target, weapon, false)
    }

    /// Cells of extra reach for a firer above its target.
    // Nothing unless both stand on the floor; the levels (a raised bridge's
    // cell counting four more) the firer is above the target, in whole
    // `ElevationIncrement`s, times `ElevationIncrementBonus`, capped at
    // `ElevationBonusCap`. Also the difference in levels, for the slant.
    fn elevation(&self, from: (i32, i32, i32), target: Aim, to: (i32, i32, i32), firer_on_floor: bool) -> (i32, i32) {
        if !firer_on_floor || self.in_air(target) {
            return (0, 0);
        }
        let level = |x: i32, y: i32| {
            let cell = (x.div_euclid(crate::LEPTONS_PER_CELL), y.div_euclid(crate::LEPTONS_PER_CELL));
            self.map.level(cell.0, cell.1) as i32 + if self.map.deck(cell.0, cell.1) { 4 } else { 0 }
        };
        let diff = (level(from.0, from.1) - level(to.0, to.1)).max(0);
        let (increment, bonus, cap) = self.elevation;
        let cells = (((diff / increment.max(1)) as f64 * bonus).min(cap)) as i32;
        (cells, diff)
    }

    // The firer's own test.
    fn in_range_for(&self, from: (i32, i32, i32), target: Aim, weapon: &Weapon, aircraft: bool) -> bool {
        if weapon.range == -512 {
            return true;
        }
        let Some(to) = self.aim_point(target) else {
            return false;
        };
        let d = ((to.0 - from.0) as f64, (to.1 - from.1) as f64, (to.2 - from.2) as f64);
        if weapon.minimum_range != 0 && (length3(d) as i32) < weapon.minimum_range {
            return false;
        }
        let projectile = self.projectiles.get(weapon.projectile as usize);
        let arcing = projectile.is_some_and(|p| p.arcing);
        let elevated = projectile.is_some_and(|p| p.subject_to_elevation);
        let on_floor = !aircraft && from.2 - self.ground_z(from.0, from.1) < 208;
        let (cells, diff) = if elevated { self.elevation(from, target, to, on_floor) } else { (0, 0) };
        if arcing {
            // A shot that arcs reaches its flat range plus the bonus, if a
            // throw at the speed for that distance could get there.
            let limit = weapon.range + cells * crate::LEPTONS_PER_CELL;
            let across = d.0.hypot(d.1) as i32;
            if across > limit {
                return false;
            }
            let gravity = projectile.map_or(self.gravity, |p| if p.floater { self.gravity / 2 } else { self.gravity }) as f64;
            let speed = ((limit as f64 * gravity * 1.2).sqrt() as i32) as f64;
            return ballistic_can_reach(speed, across as f64, d.2, gravity);
        }
        let slant = if cells > 0 || diff > 0 {
            let (a, b) = ((cells * crate::LEPTONS_PER_CELL) as i64, (LEVEL_HEIGHT * diff) as i64);
            ((a * a + b * b) as f64).sqrt() as i32
        } else {
            0
        };
        let mut limit = weapon.range + slant;
        if let Aim::Building(id) = target
            && let Some(building) = self.buildings.iter().find(|b| b.id == id) {
                limit += (building.size.0 + building.size.1) * 64;
            }
        // For a firer in the air the distance is
        // taken at the target's own height (a Kirov over its target is beside
        // it). Not original behaviour: "in the air" is judged from the height
        // of the point given.
        if aircraft || from.2 - self.ground_z(from.0, from.1) >= 208 {
            return (d.0.hypot(d.1) as i32) <= limit;
        }
        (length3(d) as i32) <= limit
    }

    /// The fire test, less facing and movement.
    // The order of the tests kept here: no target,
    // no weapon, a target on the ground needs an `AG` projectile, the rearm
    // timer, a warhead that does nothing to the armour, and range.
    pub(crate) fn fire_error(&self, from: (i32, i32, i32), fighter: &Fighter, arms: &Arms, target: Aim, index: usize) -> FireError {
        if !self.alive(target) {
            return FireError::Illegal;
        }
        let Some(weapon) = Self::mount(fighter, index).and_then(|m| self.weapons.get(m.weapon as usize)) else {
            return FireError::Cant;
        };
        // Something in the air
        // needs an `AA` projectile, something on the ground an `AG` one.
        let Some(projectile) = self.projectiles.get(weapon.projectile as usize) else {
            return FireError::Illegal;
        };
        if if self.in_air(target) { !projectile.aa } else { !projectile.ag } {
            return FireError::Illegal;
        }
        if let Some(refused) = self.warheads.get(weapon.warhead as usize).and_then(|warhead| self.special_error(warhead, target)) {
            return refused;
        }
        if self.tick < arms.rearm_at {
            return FireError::Rearm;
        }
        if let Some(theirs) = self.fighter(target) {
            let verses = self.warheads.get(weapon.warhead as usize).map_or(1.0, |w| w.verses[theirs.armour.min(10) as usize]);
            if verses == 0.0 {
                return FireError::Illegal;
            }
        }
        if !self.in_range_for(from, target, weapon, fighter.aircraft) {
            return FireError::Range;
        }
        FireError::Ok
    }

    /// The direction from a point to a target, as a 16-bit facing.
    pub(crate) fn direction_to(&self, from: (i32, i32), target: Aim) -> Option<u16> {
        let to = self.aim_point(target)?;
        Some(facing_of(((from.1 - to.1) as f64).atan2((to.0 - from.0) as f64)) as u16)
    }

    /// Where a weapon's shot starts.
    // The firing facing is taken to 32 steps; the
    // mount is turned by it about the turret's place on the body and added to
    // where the thing stands; odd shots of a burst mirror the sideways part.
    fn fire_coord(&self, at: (i32, i32, i32), body: u8, firing: u16, fighter: &Fighter, mount: Mount, burst: u32) -> (i32, i32, i32) {
        let step32 = |facing: u16| ((((facing >> 10) + 1) >> 1) & 31) as i32;
        let turned = rad(step32(firing) << 11);
        let body = rad(step32((body as u16) << 8) << 11);
        let side = if burst % 2 == 1 { -1.0 } else { 1.0 };
        let (f, l, h) = (mount.flh.0 as f64, mount.flh.1 as f64 * side, mount.flh.2);
        let offset = fighter.turret_offset as f64;
        let x = offset * body.cos() + f * turned.cos() - l * turned.sin();
        let y = offset * body.sin() + f * turned.sin() + l * turned.cos();
        (at.0 + x as i32, at.1 - y as i32, at.2 + h)
    }

    /// The speed a shot is given.
    // Shots that do not home go at the
    // square root of distance × gravity × 1.2 (gravity halved for a `Floater`);
    // homing ones at the weapon's `Speed`.
    fn shot_speed(&self, weapon: &Weapon, projectile: &Projectile, distance: i32) -> i32 {
        if projectile.rot != 0 {
            return weapon.speed;
        }
        let gravity = if projectile.floater { self.gravity as f64 * 0.5 } else { self.gravity as f64 };
        (distance as f64 * gravity * 1.2).sqrt() as i32
    }

    /// The pitch of an arcing shot, as a facing, or `None` if it cannot get there.
    fn solve_pitch(lob: bool, v: f64, x: f64, z: f64, g: f64) -> Option<i16> {
        let solve = |x: f64| -> Option<f64> {
            if z > 0.0 {
                return Some(0.78538);
            }
            let discriminant = v.powi(4) - 2.0 * v * v * z * g - g * g * x * x;
            let below = 2.0 * (1.0 + z * z / (x * x));
            if discriminant < 0.0 || below == 0.0 || !below.is_finite() {
                return None;
            }
            let r = if lob { ((v * v - z * g) - discriminant.sqrt()) / below } else { ((v * v - z * g) + discriminant.sqrt()) / below };
            (r >= 0.0).then(|| (r.sqrt() / v).clamp(-1.0, 1.0).acos())
        };
        let angle = solve(x)?;
        if lob {
            return Some(facing_of(angle));
        }
        let further = solve(x + 1.0).unwrap_or(angle);
        Some(if further < angle { facing_of(-angle) } else { facing_of(angle) })
    }

    /// After a `FireOnce` shot from a team's member: the team's target goes,
    /// and its script goes on to the next line.
    // For a foot soldier or vehicle in
    // a team, `AssignMissionTarget(null)` and the team's
    // next-mission flag set, whatever the line was. (An AI spy's
    // first line, "attack infantry", is its make-up kit; without this it took
    // one disguise after another for ever.)
    fn fired_once(&mut self, index: usize) {
        if let Some(team) = self.units[index].team {
            self.clear_team_target(team);
            if let Some(team) = self.teams.iter_mut().find(|t| t.id == team) {
                team.next_mission = true;
            }
        }
    }

    /// Fires a weapon. Returns whether a shot left.
    // The shot's
    // damage, speed, direction and pitch, where an unseen shot is put, and the
    // time until the next.
    pub(crate) fn fire(&mut self, by: Aim, at: (i32, i32, i32), body: u8, firing: u16, house: u8, fighter: &Fighter, arms: &mut Arms, target: Aim, index: usize) -> bool {
        let Some(mount) = Self::mount(fighter, index) else {
            return false;
        };
        let Some(weapon) = self.weapons.get(mount.weapon as usize).cloned() else {
            return false;
        };
        let Some(projectile) = self.projectiles.get(weapon.projectile as usize).cloned() else {
            return false;
        };
        let Some(aim) = self.aim_point(target) else {
            return false;
        };
        // A `Suicide` weapon
        // makes no shot: the firer takes its own strength in damage, and its
        // death weapon does the rest.
        if weapon.suicide {
            if let Aim::Unit(id) = by {
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                    unit.health = 0;
                    unit.died_by_suicide = true;
                }
                if !self.dying.iter().any(|(who, _)| *who == by) {
                    self.dying.push((by, 0));
                }
            }
            arms.rearm_at = self.tick + weapon.rof.max(1) as u64;
            return true;
        }
        // A `Spawner` weapon makes no shot: it
        // hands the target to the unit's spawns.
        if weapon.spawner {
            if let Aim::Unit(id) = by
                && let Some(spawner) = self.units.iter_mut().find(|u| u.id == id).and_then(|unit| unit.spawner.as_mut()) {
                    spawner.set_target(target);
                }
            arms.rearm_at = self.tick + weapon.rof.max(0) as u64 + (self.next_random() % 3) as u64;
            arms.fired_at = self.tick;
            return true;
        }
        let from = self.fire_coord(at, body, firing, fighter, mount, arms.burst);
        // A moving vehicle (not a soldier, aircraft or building) is aimed
        // ahead of, along its body's facing, by its top speed for the time the
        // shot takes (distance over nine tenths of the shot's speed). Only the
        // launch is led; the shot's fuse still counts to the target itself.
        let mut launch_at = aim;
        if let Aim::Unit(id) = target
            && let Some(them) = self.unit(id).filter(|u| !u.stats.infantry && u.plane.is_none() && u.is_moving() && u.drive.is_some()) {
                let distance = ((them.x - at.0) as f64).hypot((them.y - at.1) as f64) as i32;
                let shot = self.shot_speed(&weapon, &projectile, distance).max(1) as f64;
                let lead = (distance as f64 / (shot * 0.9) * them.speed as f64) as i32;
                let a = rad((them.facing as i32) << 8);
                launch_at = ((aim.0 as f64 + a.cos() * lead as f64) as i32, (aim.1 as f64 - a.sin() * lead as f64) as i32, aim.2);
            }
        let mut d = ((launch_at.0 - from.0) as f64, (launch_at.1 - from.1) as f64, (launch_at.2 - from.2) as f64);
        // An `Inaccurate`,
        // `Arcing` shot strays: with `FlakScatter` (and seen) by up to
        // `BallisticScatter` times its distance over the weapon's range, else
        // by a half to a whole `BallisticScatter`; in a direction drawn
        // after the distance. Height is left alone.
        if projectile.inaccurate && projectile.arcing {
            let scatter = self.ballistic_scatter.max(0);
            let stray = if projectile.flak_scatter && !projectile.inviso {
                let r = (self.next_random() % (scatter as u32 + 1)) as i64;
                (r * length3(d) as i64 / weapon.range.max(1) as i64) as f64
            } else {
                (scatter / 2 + (self.next_random() % (scatter as u32 / 2 + 1)) as i32) as f64
            };
            let angle = (self.next_random() % 0x7fff_ffff) as f64 / 2147483648.0 * std::f64::consts::TAU;
            d.0 = (d.0 + angle.cos() * stray).trunc();
            d.1 = (d.1 - angle.sin() * stray).trunc();
        }
        let flat = d.0.hypot(d.1) as i32;
        let mut speed = self.shot_speed(&weapon, &projectile, flat);
        let yaw = if projectile.rot == 0 { facing_of((-d.1).atan2(d.0)) } else { firing as i16 };
        speed = speed.min(length3(d) as i32 / 2);
        let top_speed = if projectile.rot > 0 { weapon.speed } else { speed };
        if projectile.rot > 0 {
            speed = 1;
        }
        let mut pitch: i16 = 0x3fff;
        if projectile.arcing {
            let gravity = if projectile.floater { self.gravity as f64 * 0.5 } else { self.gravity as f64 };
            match Self::solve_pitch(weapon.lobber, speed as f64, flat as f64, d.2, gravity) {
                Some(solved) => pitch = solved,
                // The shot is abandoned, and the weapon is not rearmed.
                None => return false,
            }
        } else if d.2.abs() > 200.0 {
            let mut angle = ((d.2.abs() - 20.0) as f32).atan2((flat as f32).max(0.05)) as f64;
            if d.2 < 0.0 {
                angle = -angle;
            }
            pitch = facing_of(angle);
        }
        let (yaw, pitch) = (rad(yaw as i32), rad(pitch as i32));
        let mut velocity = (yaw.cos() * speed as f64 * pitch.cos(), -yaw.sin() * speed as f64 * pitch.cos(), pitch.sin() * speed as f64);
        let mut position = (from.0 as f64, from.1 as f64, from.2 as f64);
        if projectile.inviso {
            // An unseen shot sits on its target and goes off on its next turn.
            position = (aim.0 as f64, aim.1 as f64, aim.2 as f64);
            velocity = (0.0, 0.0, 0.0);
            d = (0.0, 0.0, 0.0);
        } else if projectile.rot > 0 {
            let length = length3(velocity);
            velocity = if length > 0.0 { (velocity.0 / length, velocity.1 / length, velocity.2 / length) } else { (1.0, 0.0, 0.0) };
        }
        let id = self.next_id;
        self.next_id += 1;
        self.bullets.push(Bullet {
            id,
            weapon: mount.weapon,
            owner: Some(by),
            house,
            target,
            target_at: aim,
            position,
            velocity,
            speed: top_speed,
            // A sonic weapon's own shot does nothing.
            damage: if weapon.is_sonic { 0 } else { self.ranked_damage(by, weapon.damage) },
            arm_at: self.tick + projectile.arm.max(0) as u64,
            fuse_last: (length3(d) as i32) / 2,
            locked: true,
            lock_count: 0,
            samples: 0,
            closing: 0.0,
            born: self.tick,
        });
        if weapon.is_sonic {
            self.waves.push(Wave { owner: by, target, weapon: mount.weapon, born: self.tick });
        }
        // The time to the next shot: within a burst three to five frames, after
        // it the weapon's `ROF` plus up to two.
        arms.burst += 1;
        let delay = if (arms.burst as i32) < weapon.burst {
            3 + self.next_random() % 3
        } else {
            // A rank with ROF multiplies the wait
            // by `VeteranROF`.
            // The weapon's
            // `ROF` times the house's own, which for a computer is its
            // difficulty's.
            let house = self.owner_of(by).and_then(|owner| self.houses.get(owner as usize)).and_then(|house| house.ai.as_ref()).map(|brain| self.ai_rules.rof[brain.difficulty.min(2)]).filter(|&rof| rof > 0.0).unwrap_or(1.0);
            let wait = (weapon.rof.max(0) as f64 * house) as u32 + self.next_random() % 3;
            match by {
                Aim::Unit(id) if self.unit(id).is_some_and(|unit| unit.has_ability(crate::abilities::ability::ROF)) => (wait as f64 * self.veterancy.rof) as u32,
                _ => wait,
            }
        };
        arms.burst %= weapon.burst.max(1) as u32;
        arms.rearm_at = self.tick + delay as u64;
        arms.rearm_for = delay as u64;
        arms.fired_at = self.tick;
        self.events.push(Event::Fired { weapon: mount.weapon, at: from, to: aim, facing: (firing >> 8) as u8, by });
        if let Aim::Unit(id) = by {
            let (tick, blink) = (self.tick, weapon.fake_blink as u64);
            if let Some(unit) = self.units.iter_mut().find(|u| u.id == id).filter(|unit| unit.stats.fighter.disguise_when_still) {
                unit.blinks_until = tick + blink;
            }
        }
        true
    }

    /// A rank with FIREPOWER multiplies a shot's
    /// damage by `VeteranCombat`.
    fn ranked_damage(&self, by: Aim, damage: i32) -> i32 {
        // A firepower crate's
        // multiplier on what it does.
        let damage = match by {
            Aim::Unit(id) => self.unit(id).map_or(damage, |unit| (damage as f64 * unit.crate_firepower as f64) as i32),
            _ => damage,
        };
        match by {
            Aim::Unit(id) if self.unit(id).is_some_and(|unit| unit.has_ability(crate::abilities::ability::FIREPOWER)) => (damage as f64 * self.veterancy.combat) as i32,
            // DATA: rules.ini `PrismSupportModifier`: each beam lent adds that
            // share to the firing tower's damage.
            Aim::Building(id) => {
                let lent = self.buildings.iter().find(|b| b.id == id).map_or(0, |b| b.supports);
                (damage as f64 * (1.0 + lent as f64 * self.ability_rules.prism_modifier)) as i32
            }
            _ => damage,
        }
    }

    /// How much of a hit gets through.
    // Damage falls off in a straight line from full at
    // the middle to `PercentAtMax` at the `CellSpread` radius, is then scaled by
    // the armour's share, and capped at `MaxDamage`. Healing reaches 8 leptons.
    fn modify_damage(&self, damage: i32, warhead: &Warhead, armour: u8, distance: i32) -> i32 {
        if damage == 0 {
            return 0;
        }
        if damage < 0 {
            return if distance >= 8 { 0 } else { damage };
        }
        let at_max = damage as f32 * warhead.percent_at_max;
        let spread = (warhead.cell_spread * 256.0) as i32;
        let mut damage = damage;
        if at_max != damage as f32 && spread != 0 {
            damage = ((damage as f32 - at_max) * (spread - distance) as f32 / spread as f32 + at_max) as i32;
        }
        let damage = damage.max(0);
        ((damage as f64 * warhead.verses[armour.min(10) as usize]) as i32).min(self.max_damage)
    }

    /// The sonic waves in being.
    pub fn waves(&self) -> &[Wave] {
        &self.waves
    }

    /// Where a wave's head is now: its firer's point, and the head's.
    pub fn wave_span(&self, wave: &Wave) -> Option<((i32, i32, i32), (i32, i32, i32))> {
        let (from, to) = (self.aim_point(wave.owner)?, self.aim_point(wave.target)?);
        let done = ((self.tick - wave.born) as f64 * 0.05).min(1.0);
        Some((from, (from.0 + ((to.0 - from.0) as f64 * done) as i32, from.1 + ((to.1 - from.1) as f64 * done) as i32, from.2 + ((to.2 - from.2) as f64 * done) as i32)))
    }

    /// How far along its line a wave's tail is (0 to 1), and how many frames
    /// it has run. The tail sets off once the head is more than a tenth of
    /// the way out, and follows a twentieth of the way a frame.
    pub fn wave_tail(&self, wave: &Wave) -> (f64, u64) {
        let frames = self.tick.saturating_sub(wave.born);
        ((frames as f64 - 3.0).max(0.0) * 0.05, frames)
    }

    /// Sonic waves go out and hurt what they cross.
    // The head
    // goes a twentieth of the way a frame; every frame everything on the cells
    // of the line from the firer to the head, the firer apart, takes the
    // weapon's `AmbientDamage` with its warhead, whoever it belongs to; the
    // wave ends some 22 frames on, or when firer and target are more than 2172
    // leptons apart or either is gone.
    fn run_waves(&mut self) {
        let tick = self.tick;
        let waves = std::mem::take(&mut self.waves);
        let mut kept = Vec::new();
        for wave in waves {
            let Some((from, head)) = self.wave_span(&wave) else {
                continue;
            };
            let apart = self.aim_point(wave.target).map_or(i64::MAX, |to| (((to.0 - from.0) as f64).hypot((to.1 - from.1) as f64)) as i64);
            if tick > wave.born + 22 || apart > 2172 {
                continue;
            }
            let Some(weapon) = self.weapons.get(wave.weapon as usize).cloned() else {
                continue;
            };
            let steps = ((((head.0 - from.0) as f64).hypot((head.1 - from.1) as f64)) as i32 / 128).max(1);
            let mut cells: Vec<(i32, i32)> = Vec::new();
            for step in 0..=steps {
                let point = (from.0 + (head.0 - from.0) * step / steps, from.1 + (head.1 - from.1) * step / steps);
                let cell = (point.0.div_euclid(LEPTONS_PER_CELL), point.1.div_euclid(LEPTONS_PER_CELL));
                if !cells.contains(&cell) {
                    cells.push(cell);
                }
            }
            let mut hit: Vec<Aim> = self.units.iter().filter(|u| cells.contains(&u.cell()) && Aim::Unit(u.id) != wave.owner && u.health > 0).map(|u| Aim::Unit(u.id)).collect();
            for &cell in &cells {
                if let Some(index) = self.building_on(cell) {
                    let aim = Aim::Building(self.buildings[index].id);
                    if aim != wave.owner && !hit.contains(&aim) {
                        hit.push(aim);
                    }
                }
            }
            for victim in hit {
                self.receive_damage(victim, weapon.ambient_damage, 0, weapon.warhead, Some(wave.owner));
            }
            kept.push(wave);
        }
        self.waves = kept;
    }

    /// Hurts a unit or building. Returns whether it died.
    // At least one point reaches the armour
    // test; nothing spares one's own side; a retaliating victim turns on its
    // attacker.
    pub(crate) fn receive_damage(&mut self, victim: Aim, damage: i32, distance: i32, warhead: u16, source: Option<Aim>) -> bool {
        let Some(fighter) = self.fighter(victim) else {
            return false;
        };
        let Some(warhead) = self.warheads.get(warhead as usize).cloned() else {
            return false;
        };
        if let Aim::Unit(id) = victim {
            self.parasite_suppression(id, damage, source);
        }
        if fighter.immune {
            return false;
        }
        // A psychic warhead does nothing to what
        // is immune to psionics (Yuri's wave spares other Yuris, dogs,
        // miners, buildings).
        if warhead.psychic_damage && fighter.immune_to_psionics {
            return false;
        }
        // Nothing under the iron
        // curtain is hurt.
        let curtained = match victim {
            Aim::Unit(id) => self.unit(id).is_some_and(|u| u.curtain_until > self.tick),
            Aim::Building(id) => self.buildings.iter().any(|b| b.id == id && b.curtain_until > self.tick),
            Aim::Cell(..) => false,
        };
        if curtained {
            return false;
        }
        let mut damage = damage;
        // A soldier lying down takes the
        // warhead's `ProneDamage` share of it, at least 1.
        if let Aim::Unit(id) = victim
            && damage > 0 && self.unit(id).is_some_and(|unit| unit.prone) {
                damage = ((damage as f64 * warhead.prone_damage) as i32).max(1);
            }
        // A rank with STRONGER divides what
        // reaches it by `VeteranArmor`.
        if let Aim::Unit(id) = victim
            && damage > 0 && self.unit(id).is_some_and(|unit| unit.has_ability(crate::abilities::ability::STRONGER)) {
                damage = (damage as f64 / self.veterancy.armor) as i32;
            }
        // An
        // armour crate's multiplier divides what reaches it.
        if let Aim::Unit(id) = victim
            && damage > 0 {
                let armour = self.unit(id).map_or(1.0, |unit| unit.crate_armour);
                if armour != 1.0 {
                    damage = (damage as f64 / armour as f64) as i32;
                }
            }
        if damage > 0 {
            damage = damage.max(1);
        }
        let damage = self.modify_damage(damage, &warhead, fighter.armour, distance);
        if damage == 0 {
            return false;
        }
        let (health, strength) = match victim {
            Aim::Unit(id) => match self.units.iter_mut().find(|u| u.id == id) {
                Some(unit) => (&mut unit.health, unit.stats.strength),
                None => return false,
            },
            Aim::Building(id) => match self.buildings.iter_mut().find(|b| b.id == id) {
                Some(building) => {
                    let strength = self.building_types.get(building.kind as usize).map_or(1, |kind| kind.strength);
                    (&mut building.health, strength)
                }
                None => return false,
            },
            Aim::Cell(..) => return false,
        };
        if *health <= 0 {
            return false;
        }
        if damage < 0 {
            *health = (*health - damage).min(strength);
            return false;
        }
        *health = (*health - damage).max(0);
        let dead = *health == 0;
        self.note_attack(victim, damage, source);
        if dead {
            self.record_kill(victim, source);
            self.score_kill(victim, source);
        }
        if let (Some(owner), Some(at)) = (self.owner_of(victim), self.aim_point(victim)) {
            let miner = !dead && matches!(victim, Aim::Unit(id) if self.unit(id).is_some_and(|unit| unit.stats.harvester));
            self.events.push(Event::Attacked { owner, at: (at.0 / LEPTONS_PER_CELL, at.1 / LEPTONS_PER_CELL), building: matches!(victim, Aim::Building(_)), miner });
        }
        if dead {
            // A `Crashable` flyer killed in the air
            // falls before it is gone.
            if let Aim::Unit(id) = victim {
                let aloft = self.unit(id).is_some_and(|unit| self.altitude(unit) > 0 && unit.stats.jet.is_some_and(|jet| jet.crashable));
                if aloft {
                    if let Some(jet) = self.units.iter_mut().find(|u| u.id == id).and_then(|u| u.jet.as_mut()) {
                        jet.crashing = true;
                    }
                    return true;
                }
                // An aircraft destroyed in the air
                // falls; on the ground it is gone at once.
                if let Some(unit) = self.units.iter_mut().find(|u| u.id == id && u.plane.is_some_and(|plane| plane.z > 0)) {
                    if let Some(plane) = unit.plane.as_mut() {
                        plane.crashing = true;
                    }
                    let plane = unit.plane.and_then(|plane| plane.contact);
                    if let Some(building) = plane.and_then(|b| self.buildings.iter_mut().find(|x| x.id == b)) {
                        for slot in &mut building.docks {
                            if *slot == Some(id) {
                                *slot = None;
                            }
                        }
                    }
                    return true;
                }
            }
            self.dying.push((victim, warhead.inf_death));
            return true;
        }
        // A person's units answer only when idle with
        // no target; the attacker must be an enemy within reach or sight.
        if let (Some(attacker), Aim::Unit(id)) = (source, victim) {
            let hostile = match (self.owner_of(attacker), self.owner_of(victim)) {
                (Some(theirs), Some(ours)) => !self.allied(theirs, ours),
                _ => false,
            };
            let near = match (self.aim_point(attacker), self.aim_point(victim)) {
                (Some(a), Some(b)) => (((a.0 - b.0) as f64).hypot((a.1 - b.1) as f64) as i32) <= (fighter.sight * 256 + 128),
                _ => false,
            };
            if hostile && fighter.can_retaliate && fighter.primary.is_some()
                && let Some(unit) = self.units.iter_mut().find(|u| u.id == id) {
                    let idle = unit.arms.target.is_none() && unit.dest.is_none() && unit.work.job == Job::Idle;
                    if idle && near {
                        unit.arms.target = Some(attacker);
                        unit.arms.passive = true;
                    }
                }
        }
        if let Aim::Unit(id) = victim {
            self.frighten(id, source.is_some());
        }
        // Something on foot or
        // wheels that was hit and lived, did not turn on its attacker, and is
        // standing with nothing to do, steps aside if it is a computer's or its
        // rank brings SCATTER.
        if let Aim::Unit(id) = victim {
            let human = self.owner_of(victim).is_some_and(|owner| self.human(owner));
            let steps = self.unit(id).is_some_and(|unit| {
                unit.health > 0
                    && unit.plane.is_none()
                    && unit.arms.target.is_none()
                    && unit.dest.is_none()
                    && !unit.is_moving()
                    && unit.work.job == Job::Idle
                    && unit.stance == Stance::Standing
                    && unit.falling.is_none()
                    && (!human || unit.has_ability(crate::abilities::ability::SCATTER))
            });
            if steps {
                self.scatter_unit(id);
            }
        }
        false
    }

    /// The fragments a shot throws out where it lands.
    // Nothing where the shot's cell holds a building; otherwise one fragment
    // for each thing that is not an ally found in the rings round the cell out
    // to the fragment weapon's range, and each fragment left over goes to the
    // cell 0 to 2 away either way at random.
    fn shrapnel(&mut self, bullet: &Bullet, at: (i32, i32, i32), fragment: u16, count: i32) {
        let Some(weapon) = self.weapons.get(fragment as usize).cloned() else {
            return;
        };
        let centre = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
        if self.building_on(centre).is_some() {
            return;
        }
        let mut targets: Vec<Aim> = Vec::new();
        for (dx, dy) in spread_offsets((weapon.range / 256).clamp(0, 11)) {
            let cell = (centre.0 + dx, centre.1 + dy);
            for unit in self.units.iter().filter(|u| u.cell() == cell && u.health > 0 && !self.allied(u.owner, bullet.house)) {
                let who = Aim::Unit(unit.id);
                if who != bullet.target && !targets.contains(&who) {
                    targets.push(who);
                }
            }
            if let Some(index) = self.building_on(cell) {
                let who = Aim::Building(self.buildings[index].id);
                if !self.allied(self.buildings[index].owner, bullet.house) && self.buildings[index].owner != 0 && who != bullet.target && !targets.contains(&who) {
                    targets.push(who);
                }
            }
        }
        targets.truncate(count.max(0) as usize);
        while (targets.len() as i32) < count {
            let dx = (self.next_random() % 5) as i32 - 2;
            let dy = (self.next_random() % 5) as i32 - 2;
            targets.push(Aim::Cell(centre.0 + dx, centre.1 + dy));
        }
        for target in targets {
            let Some(aim) = self.aim_point(target) else {
                continue;
            };
            let id = self.next_id;
            self.next_id += 1;
            self.bullets.push(Bullet {
                id,
                weapon: fragment,
                owner: bullet.owner,
                house: bullet.house,
                target,
                target_at: aim,
                position: (aim.0 as f64, aim.1 as f64, aim.2 as f64),
                velocity: (0.0, 0.0, 0.0),
                speed: 0,
                damage: weapon.damage,
                arm_at: self.tick,
                fuse_last: 0,
                locked: true,
                lock_count: 0,
                samples: 0,
                closing: 0.0,
                born: self.tick,
            });
            self.events.push(Event::Forked { weapon: fragment, from: at, to: aim, house: bullet.house });
        }
    }

    /// An explosion at a point.
    // The cells within the spread, ring by ring; a
    // building is hit once for each of its cells reached, by that cell's distance
    // (none for the cell under a blast near the ground); the thing that fired
    // is spared.
    pub(crate) fn explode(&mut self, at: (i32, i32, i32), damage: i32, warhead_index: u16, source: Option<Aim>) {
        let Some(warhead) = self.warheads.get(warhead_index as usize).cloned() else {
            return;
        };
        if damage == 0 {
            return;
        }
        let spread = (warhead.cell_spread * 256.0) as i32;
        let centre = (at.0 / LEPTONS_PER_CELL, at.1 / LEPTONS_PER_CELL);
        let rings = (warhead.cell_spread + 0.99) as i32;
        let mut hits: Vec<(Aim, i32)> = Vec::new();
        let mut walls: Vec<(i32, i32)> = Vec::new();
        // A blast on a bridge cell
        // more than 208 leptons above its ground is on the deck, and hurts
        // only what is on decks; any other hurts only what is not.
        let on_deck = self.map.bridged(centre.0, centre.1) && at.2 > self.ground_z(at.0, at.1) + 208;
        for (i, (dx, dy)) in spread_offsets(rings.min(11)).into_iter().enumerate() {
            let cell = (centre.0 + dx, centre.1 + dy);
            // A wall on any
            // cell of the spread is hit by a warhead with `Wall`.
            if warhead.wall && self.map.is_wall(cell) {
                walls.push(cell);
            }
            for unit in self.units.iter().filter(|u| u.cell() == cell && u.health > 0 && u.on_bridge == on_deck) {
                let who = Aim::Unit(unit.id);
                if Some(who) == source {
                    continue;
                }
                let z = self.ground_z(unit.x, unit.y) + self.altitude(unit) + if unit.on_bridge { 4 * LEVEL_HEIGHT } else { 0 };
                let d = length3(((at.0 - unit.x) as f64, (at.1 - unit.y) as f64, (at.2 - z) as f64)) as i32;
                hits.push((who, d));
            }
            if let Some(index) = self.building_on(cell).filter(|_| !on_deck) {
                let who = Aim::Building(self.buildings[index].id);
                if Some(who) != source {
                    let (cx, cy) = cell_centre(cell);
                    let z = self.ground_z(cx, cy);
                    // The cell under the blast counts as no
                    // distance only when the blast is within two levels of
                    // it; higher up (a shot at something in the air), it is
                    // the distance less two levels.
                    let far = length3(((at.0 - cx) as f64, (at.1 - cy) as f64, (at.2 - z) as f64)) as i32;
                    let d = match i {
                        0 if at.2 - z > 2 * LEVEL_HEIGHT => far - 2 * LEVEL_HEIGHT,
                        0 => 0,
                        _ => far,
                    };
                    hits.push((who, d));
                }
            }
        }
        self.blast_bridge(at, damage, warhead.wall);
        for cell in walls {
            self.damage_wall(cell, damage);
        }
        for (who, distance) in hits {
            if distance <= spread && self.alive(who) {
                self.receive_damage(who, damage, distance, warhead_index, source);
            }
        }
        // A `Rocker` warhead carrying more than 30
        // damage rocks everything in the seven by seven cells round it, by a
        // hundredth of the damage (at most 4): away from the blast, or for what
        // stands in the blast's own cell, away from a point ten leptons towards
        // whoever fired.
        let rock = (damage as f64 * 0.01).min(4.0);
        if warhead.rocker && rock > 0.3 {
            let firer = source.and_then(|source| self.aim_point(source));
            let rocked: Vec<Event> = self
                .units
                .iter()
                .filter(|u| u.health > 0 && (u.cell().0 - centre.0).abs() <= 3 && (u.cell().1 - centre.1).abs() <= 3)
                .map(|u| {
                    let z = self.ground_z(u.x, u.y) + self.altitude(u);
                    let from = match firer {
                        Some(f) if u.cell() == centre => {
                            let d = ((f.0 - u.x) as f64, (f.1 - u.y) as f64, (f.2 - z) as f64);
                            let length = length3(d).max(f64::EPSILON);
                            (u.x + (10.0 * d.0 / length) as i32, u.y + (10.0 * d.1 / length) as i32, z + (10.0 * d.2 / length) as i32)
                        }
                        _ => at,
                    };
                    Event::Rocked { unit: u.id, from, strength: rock as f32 }
                })
                .collect();
            self.events.extend(rocked);
        }
    }

    /// An explosion with nobody behind it, for tests.
    #[doc(hidden)]
    pub fn explode_for_tests(&mut self, at: (i32, i32, i32), damage: i32, warhead: u16) {
        self.explode(at, damage, warhead, None);
    }

    /// Removes a unit at once (for tests and scripts).
    pub fn remove_unit(&mut self, id: u32) {
        self.vanish(id);
    }

    /// Takes a unit off the map with no death.
    pub(crate) fn vanish(&mut self, id: u32) {
        if !self.dying.iter().any(|(who, _)| *who == Aim::Unit(id)) {
            self.dying.push((Aim::Unit(id), VANISHED));
        }
    }

    /// A `Temporal` weapon takes hold of its target.
    // A `Warpable` target is frozen and given ten
    // times its strength to be worn down.
    fn temporal_hit(&mut self, by: Option<Aim>, target: Aim) {
        let (Some(Aim::Unit(firer)), Some(theirs)) = (by, self.fighter(target)) else {
            return;
        };
        if !theirs.warpable {
            return;
        }
        let points = match target {
            Aim::Unit(id) => self.units.iter_mut().find(|u| u.id == id).map(|u| {
                u.frozen.get_or_insert(u.stats.strength * 10);
            }),
            Aim::Building(id) => {
                let types = &self.building_types;
                self.buildings.iter_mut().find(|b| b.id == id).map(|b| {
                    b.frozen.get_or_insert(types.get(b.kind as usize).map_or(1, |k| k.strength) * 10);
                })
            }
            Aim::Cell(..) => None,
        };
        if points.is_some()
            && let Some(unit) = self.units.iter_mut().find(|u| u.id == firer) {
                unit.arms.erasing = Some(target);
            }
    }

    /// Wears down everything a `Temporal` weapon has hold of.
    // Each frame the target loses the `Damage` of each weapon holding it; at
    // nothing it is gone, with no explosion.
    fn wear_down(&mut self) {
        let mut drains: Vec<(Aim, i32)> = Vec::new();
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            let Some(held) = unit.arms.erasing else {
                continue;
            };
            let keeps = self.alive(held) && unit.arms.target == Some(held) && unit.dest.is_none() && unit.frozen.is_none();
            if !keeps {
                self.units[index].arms.erasing = None;
                continue;
            }
            let damage = unit.stats.fighter.primary.and_then(|m| self.weapons.get(m.weapon as usize)).map_or(0, |w| w.damage);
            drains.push((held, damage));
        }
        let total = |who: Aim| drains.iter().filter(|(held, _)| *held == who).map(|(_, damage)| *damage).sum::<i32>();
        let mut gone = Vec::new();
        for unit in &mut self.units {
            if let Some(points) = unit.frozen {
                let drain = total(Aim::Unit(unit.id));
                unit.frozen = (drain > 0).then_some(points - drain);
                if unit.frozen.is_some_and(|left| left <= 0) {
                    gone.push(Aim::Unit(unit.id));
                }
            }
        }
        for building in &mut self.buildings {
            if let Some(points) = building.frozen {
                let drain = total(Aim::Building(building.id));
                building.frozen = (drain > 0).then_some(points - drain);
                if building.frozen.is_some_and(|left| left <= 0) {
                    gone.push(Aim::Building(building.id));
                }
            }
        }
        for who in gone {
            self.dying.push((who, ERASED));
        }
    }

    /// Jumps a `Teleporter` to a cell.
    // The jump is at once; afterwards the unit is out of action for
    // the distance in leptons over `ChronoDistanceFactor` frames,
    // at least `ChronoMinimumDelay`.
    pub(crate) fn teleport(&mut self, index: usize, wanted: (i32, i32)) {
        let unit = &self.units[index];
        if self.tick < unit.warp_until || unit.frozen.is_some() {
            return;
        }
        let others: Vec<Other> = self.others();
        let open = entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, unit, wanted, None, wanted, -1) == code::OK;
        let cell = if open { Some(wanted) } else { nearby(&self.map, &self.claims, &self.spots, unit, wanted, crate::Near::new(false, true, Some(wanted)), self.tick) };
        let Some(cell) = cell.filter(|&cell| cell != unit.cell()) else {
            return;
        };
        let Some(at) = self.map.index(cell.0, cell.1) else {
            return;
        };
        // A parasite is put out where it
        // was; it does not go along.
        self.parasite_exit(index);
        let unit = &self.units[index];
        let from = (unit.x, unit.y, self.ground_z(unit.x, unit.y));
        let (id, owner, infantry, kind) = (unit.id, unit.owner, unit.stats.infantry, unit.stats.kind);
        let (mut x, mut y) = cell_centre(cell);
        if infantry {
            let Some(spot) = (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0) else {
                return;
            };
            if let Some(old) = self.map.index(from.0.div_euclid(LEPTONS_PER_CELL), from.1.div_euclid(LEPTONS_PER_CELL)) {
                self.spots[old] &= !(1 << spot_index(from.0, from.1));
            }
            self.spots[at] |= 1 << spot;
            self.spot_house[at] = owner;
            (x, y) = (cell.0 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].0, cell.1 * LEPTONS_PER_CELL + SPOT_OFFSETS[spot].1);
        } else {
            for claim in &mut self.claims {
                if *claim == id + 1 {
                    *claim = 0;
                }
            }
            self.claims[at] = id + 1;
        }
        let distance = (((x - from.0) as f64).hypot((y - from.1) as f64)) as i32;
        let mut delay = if self.chrono.3 { distance / self.chrono.0.max(1) } else { 0 };
        if delay <= self.chrono.1 || distance < self.chrono.2 {
            delay = self.chrono.1;
        }
        let tick = self.tick;
        let unit = &mut self.units[index];
        unit.x = x;
        unit.y = y;
        unit.dest = None;
        unit.drop_path();
        unit.head_to = None;
        unit.warp_until = tick + delay.max(0) as u64;
        let to = (x, y, self.ground_z(x, y));
        self.events.push(Event::Teleported { kind, from, to });
    }

    /// What a unit walking into a building would do there, if anything.
    // An engineer takes a `Capturable` building of
    // another house and mends a damaged one of its own.
    pub fn entry_action(&self, unit: u32, building: u32) -> Option<Entry> {
        let unit = self.unit(unit)?;
        let building = self.buildings.iter().find(|b| b.id == building && b.buildup.is_none())?;
        let kind = self.building_types.get(building.kind as usize)?;
        // A soldier into an allied `Hospital` when hurt, or into an allied
        // `Armory` when not yet elite; the building must be working and, where
        // its type counts uses (`Ammo`), have some left.
        if (kind.hospital || kind.armory) && unit.stats.infantry && self.allied(building.owner, unit.owner) {
            let wanted = if kind.hospital { unit.health < unit.stats.strength } else { unit.rank() != 0 };
            return (wanted && building.ammo != 0 && self.online(building) && unit.stance == Stance::Standing).then_some(Entry::Treat);
        }
        if unit.stats.fighter.occupier {
            return self.can_be_occupied_by(building, unit).then_some(Entry::Occupy);
        }
        // A `C4` soldier
        // over a building that is not its own side's and can be wired.
        if unit.stats.fighter.c4 {
            // No test of whose it is: what makes
            // the attack possible (an enemy, or force-fire on one's own) decides.
            return kind.can_c4.then_some(Entry::Sabotage);
        }
        // An `Agent` and an enemy's `Spyable` building.
        if unit.stats.fighter.agent {
            return (kind.spyable && !self.allied(building.owner, unit.owner)).then_some(Entry::Infiltrate);
        }
        if !unit.stats.fighter.engineer {
            return None;
        }
        // An engineer and a
        // bridge repair hut: in, if the bridge by it has a gap.
        if kind.bridge_repair_hut {
            let centre = (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2);
            return Some(if self.bridge_needs_repair(centre) { Entry::RepairBridge } else { Entry::NothingToRepair });
        }
        // To an engineer a neutral building that can
        // be occupied is like a friend's: mended, never taken.
        if self.allied(building.owner, unit.owner) || (building.owner == 0 && kind.can_be_occupied) {
            Some(if building.health < kind.strength { Entry::Repair } else { Entry::NothingToRepair })
        } else {
            kind.fighter.capturable.then_some(Entry::Capture)
        }
    }

    /// Whether a soldier may go into a building to fire from it.
    // The building can be occupied, belongs to the
    // soldier's own house or the neutral one, is not full, and is not down to red
    // health.
    pub fn can_be_occupied_by(&self, building: &Building, unit: &Unit) -> bool {
        let Some(kind) = self.building_types.get(building.kind as usize) else {
            return false;
        };
        kind.can_be_occupied
            && unit.stats.fighter.occupier
            && (building.owner == unit.owner || building.owner == 0)
            && building.occupants.len() != kind.max_occupants
            && !self.red_health(building)
    }

    /// Whether a building is down to `ConditionRed`.
    // Health over strength at or below it.
    pub fn red_health(&self, building: &Building) -> bool {
        let strength = self.building_types.get(building.kind as usize).map_or(1, |k| k.strength).max(1);
        building.health as f64 / strength as f64 <= self.condition_red
    }

    /// Puts everyone in a building out of it, alive, round one cell beside it.
    // The first cell the first of them can stand on,
    // looked for down the column past the building's far x side, then back along
    // the row past its far y side, then along the row before it and down the
    // column before it; they come out last in first and scatter. With no cell
    // they are lost.
    // Not original behaviour: the engine puts them all on the one point and lets
    // them scatter; here those the cell has no room for are put on the cells
    // round it.
    pub(crate) fn remove_occupants(&mut self, index: usize) {
        let mut occupants = std::mem::take(&mut self.buildings[index].occupants);
        if occupants.is_empty() {
            return;
        }
        let (b, (w, h)) = (self.buildings[index].cell, self.buildings[index].size);
        let order: Vec<(i32, i32)> = (-1..=h)
            .rev()
            .map(|y| (b.0 + w, b.1 + y))
            .chain((-1..w).rev().map(|x| (b.0 + x, b.1 + h)))
            .chain((0..=w).map(|x| (b.0 + x, b.1 - 1)))
            .chain((0..=h).map(|y| (b.0 - 1, b.1 + y)))
            .collect();
        let others: Vec<Other> = self.others();
        let free = |world: &Self, who: &Unit, cell: (i32, i32)| {
            entry_code(&world.map, &world.claims, (&world.spots, &world.spot_house), &others, &world.buildings, who, cell, None, cell, -1) == code::OK
        };
        let Some(exit) = order.iter().copied().find(|&cell| free(self, &occupants[0], cell)) else {
            return;
        };
        let mut cells = vec![exit];
        cells.extend(crate::path::DIRECTIONS.iter().map(|d| (exit.0 + d.0, exit.1 + d.1)));
        let mut out = Vec::new();
        while let Some(mut soldier) = occupants.pop() {
            let place = cells.iter().copied().find_map(|cell| {
                let at = self.map.index(cell.0, cell.1)?;
                if !free(self, &soldier, cell) {
                    return None;
                }
                (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0).map(|spot| (cell, at, spot))
            });
            let Some((cell, at, spot)) = place else {
                continue;
            };
            soldier.x = cell.0 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].0;
            soldier.y = cell.1 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].1;
            self.spots[at] |= 1 << spot;
            self.spot_house[at] = soldier.owner;
            soldier.arms = Arms::default();
            soldier.dest = None;
            soldier.capture = None;
            out.push(soldier.id);
            self.put_back(soldier);
        }
        for id in out {
            self.scatter_unit(id);
        }
    }

    /// Sets off the charges whose time has come.
    // The building takes its whole health in damage
    // from the absolute warhead: it is destroyed.
    pub(crate) fn detonate_charges(&mut self) {
        let due: Vec<u32> = self.buildings.iter().filter(|b| b.c4.is_some_and(|(at, _)| self.tick >= at)).map(|b| b.id).collect();
        for id in due {
            self.kill(Aim::Building(id));
        }
    }

    /// Hands occupied buildings to whoever is in them and empty ones back.
    // Every frame: at red health everyone is put
    // out; an empty one that is not the neutral house's goes back to it; a
    // neutral one with someone in it goes to the house of the first in.
    pub(crate) fn occupation(&mut self) {
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            if !self.building_types.get(building.kind as usize).is_some_and(|k| k.can_be_occupied) {
                continue;
            }
            if self.red_health(building) {
                self.remove_occupants(index);
            }
            let building = &mut self.buildings[index];
            let id = building.id;
            match building.occupants.first().map(|first| first.owner) {
                None if building.owner != 0 => {
                    let from = building.owner;
                    building.owner = 0;
                    building.arms = Arms::default();
                    self.events.push(Event::Abandoned { building: id, by: from });
                }
                Some(owner) if building.owner == 0 => {
                    building.owner = owner;
                    building.arms = Arms::default();
                    self.events.push(Event::Garrisoned { building: id, by: owner });
                }
                _ => {}
            }
        }
    }

    /// Carries out what units sent into buildings came to do, once they are there.
    // A taken building changes house and the engineer is used up; a mended one
    // is whole again and the engineer used up; with nothing to mend the engineer
    // is kept.
    pub(crate) fn enter_buildings(&mut self) {
        for index in 0..self.units.len() {
            let unit = &self.units[index];
            let Some(target) = unit.capture else {
                continue;
            };
            if unit.is_moving() {
                continue;
            }
            self.units[index].capture = None;
            self.units[index].enter_area = None;
            let unit = &self.units[index];
            let Some(at) = self.buildings.iter().position(|b| b.id == target) else {
                continue;
            };
            let building = &self.buildings[at];
            let cell = unit.cell();
            // It acts on
            // coming to rest on a cell of the building itself.
            let near = (building.cell.0..building.cell.0 + building.size.0).contains(&cell.0)
                && (building.cell.1..building.cell.1 + building.size.1).contains(&cell.1);
            let (id, owner) = (unit.id, unit.owner);
            match self.entry_action(id, target).filter(|_| near) {
                // The building is wired to go off
                // `C4Delay` later (27 frames), unless it is being sold; the
                // soldier stays on the map and is sent off the building. One
                // already wired just sends the soldier off.
                Some(Entry::Treat) => {
                    let Some(index) = self.units.iter().position(|u| u.id == id) else {
                        continue;
                    };
                    let hospital = self.building_types.get(self.buildings[at].kind as usize).is_some_and(|k| k.hospital);
                    let patient = self.lift(index);
                    let missing = (patient.stats.strength - patient.health).max(0) as u64;
                    let stay = 72 + if hospital { missing.div_ceil(20) } else { 0 };
                    let building = &mut self.buildings[at];
                    if building.ammo > 0 {
                        building.ammo -= 1;
                    }
                    building.patients.push((self.tick + stay, patient));
                    continue;
                }
                Some(Entry::Sabotage) => {
                    let building = &mut self.buildings[at];
                    if building.c4.is_none() && building.buildup.is_none() {
                        building.c4 = Some((self.tick + self.c4_delay as u64, id));
                    }
                }
                // The soldier goes off the map and
                // onto the building's list; one turned away scatters.
                Some(Entry::Occupy) => {
                    let mut soldier = self.lift(index);
                    soldier.arms = Arms::default();
                    soldier.dest = None;
                    soldier.head_to = None;
                    soldier.drop_path();
                    self.buildings[at].occupants.push(soldier);
                    // The list of units is one shorter: look at this place again.
                    return self.enter_buildings();
                }
                Some(Entry::Capture) => {
                    let from = building.owner;
                    let kind = self.building_types.get(building.kind as usize).cloned();
                    let taken = kind.as_ref().is_some_and(|k| {
                        k.factory.is_some()
                            && !self.buildings.iter().any(|b| {
                                b.owner == owner && b.primary && self.building_types.get(b.kind as usize).is_some_and(|t| t.factory == k.factory && t.naval == k.naval)
                            })
                    });
                    // Taken from a
                    // passive house, a cash building pays `ProduceCashStartup`
                    // at once and its clock starts again.
                    if let Some((startup, _, delay)) = kind.as_ref().map(|k| k.cash).filter(|cash| from == 0 && cash.0 != 0) {
                        if let Some(house) = self.houses.get_mut(owner as usize) {
                            house.credits += startup;
                        }
                        self.buildings[at].cash_at = self.tick + delay as u64;
                    }
                    let building = &mut self.buildings[at];
                    building.owner = owner;
                    building.captured = true;
                    building.primary = taken;
                    building.rally = None;
                    building.docked = None;
                    building.repairing = false;
                    building.arms = Arms::default();
                    self.events.push(Event::Captured { building: target, by: owner, from });
                    self.dying.push((Aim::Unit(id), VANISHED));
                }
                // The spy is used up.
                Some(Entry::Infiltrate) => {
                    self.infiltrate(at, owner);
                    self.dying.push((Aim::Unit(id), VANISHED));
                }
                // The
                // bridge is put back and the engineer is used up.
                Some(Entry::RepairBridge) => {
                    let centre = (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2);
                    self.repair_bridge(centre);
                    self.events.push(Event::BridgeRepaired { house: owner, cell: centre });
                    self.dying.push((Aim::Unit(id), VANISHED));
                }
                Some(Entry::Repair) => {
                    let strength = self.building_types.get(building.kind as usize).map_or(1, |k| k.strength);
                    self.buildings[at].health = strength;
                    self.events.push(Event::Mended { building: target, owner });
                    self.dying.push((Aim::Unit(id), VANISHED));
                }
                _ => {}
            }
        }
    }

    /// Moves every shot on, and sets off those that arrive.
    fn fly_bullets(&mut self) {
        let mut index = 0;
        while index < self.bullets.len() {
            let mut bullet = self.bullets[index].clone();
            let Some(weapon) = self.weapons.get(bullet.weapon as usize).cloned() else {
                self.bullets.remove(index);
                continue;
            };
            let projectile = self.projectiles.get(weapon.projectile as usize).cloned().unwrap_or_default();
            // A target that has gone leaves the cell it stood on.
            if !self.alive(bullet.target) {
                bullet.target = Aim::Cell(bullet.target_at.0.div_euclid(LEPTONS_PER_CELL), bullet.target_at.1.div_euclid(LEPTONS_PER_CELL));
            }
            if bullet.owner.is_some_and(|owner| !self.alive(owner)) {
                bullet.owner = None;
            }
            let mut hit = false;
            let mut reached_cell = false;
            let mut gone = false;
            let mut fuse = 0;
            let ground = |world: &Self, p: (f64, f64, f64)| world.ground_z(p.0 as i32, p.1 as i32) as f64;

            if projectile.rot > 0 {
                // Homing.
                let mut speed = length3(bullet.velocity);
                let top = bullet.speed as f64;
                if projectile.course_lock == 0 {
                    if top >= 40.0 || top <= speed + 0.5 {
                        bullet.locked = false;
                    }
                } else if bullet.lock_count < projectile.course_lock {
                    bullet.lock_count += 1;
                    if bullet.lock_count >= projectile.course_lock {
                        bullet.locked = false;
                    }
                }
                let mut acceleration = projectile.acceleration as f64;
                if bullet.locked && projectile.course_lock == 0 {
                    acceleration = if self.tick.is_multiple_of(2) { 1.0 } else { 0.0 };
                }
                let rescale = |v: (f64, f64, f64), to: f64| {
                    let length = length3(v);
                    if length > 0.0 { (v.0 * to / length, v.1 * to / length, v.2 * to / length) } else { (to, 0.0, 0.0) }
                };
                if speed < top {
                    speed = (speed + acceleration).min(top);
                    bullet.velocity = rescale(bullet.velocity, speed);
                } else if speed > top {
                    speed = (speed - (projectile.acceleration / 2) as f64).max(0.0);
                    bullet.velocity = rescale(bullet.velocity, speed);
                }
                let target = self.aim_point(bullet.target).unwrap_or(bullet.target_at);
                let target = (target.0 as f64, target.1 as f64, target.2 as f64);
                let phase = ((bullet.id as u64 + self.tick) % 15) as f64;
                let vary = self.missile_rot_var;
                let mut rot = (((phase / 15.0 * std::f64::consts::TAU).sin() * vary + vary + 1.0) * projectile.rot as f64) as i32;
                let away = (bullet.position.0 - target.0, bullet.position.1 - target.1, bullet.position.2 - target.2);
                if (length3(away) as i32) < 256 {
                    rot = (rot as f64 * 1.5) as i32;
                }
                let turn = if bullet.locked { 0 } else { (rot & 0xff) << 8 };
                let old = bullet.position;
                // Steering.
                let v = bullet.velocity;
                bullet.position = (old.0 + v.0.trunc(), old.1 + v.1.trunc(), old.2 + v.2.trunc());
                let to = (target.0 - bullet.position.0, target.1 - bullet.position.1, target.2 - bullet.position.2);
                let flat_speed = v.0.hypot(v.1);
                let yaw = turn_towards(facing_of((-v.1).atan2(v.0)), facing_of((-to.1).atan2(to.0)), turn);
                let mut pitch = facing_of(v.2.atan2(flat_speed));
                let flat = to.0.hypot(to.1);
                let cruise = flat > (if projectile.very_high { 6.0 } else { 3.0 }) * 256.0 && (turn >> 8) > 1;
                if cruise && !projectile.level {
                    let ahead = (bullet.position.0 + 6.0 * v.0.trunc(), bullet.position.1 + 6.0 * v.1.trunc(), 0.0);
                    let floor = ground(self, ahead);
                    let levels = if projectile.very_high { 10.0 } else { (length3(to) / 256.0).trunc().min(5.0) };
                    let over = bullet.position.2 - levels * LEVEL_HEIGHT as f64 - floor;
                    if over < -20.0 {
                        bullet.position.2 += 18.0;
                    } else if over > 20.0 {
                        bullet.position.2 -= 18.0;
                    }
                    let want: i16 = if over < -52.0 { 0x2000 } else if over > 52.0 { 0x4800 } else { 0x4000 };
                    pitch = turn_towards(pitch, want, turn / 2);
                } else if !projectile.level {
                    pitch = turn_towards(pitch, facing_of(to.2.atan2(flat)), (turn + 0x100) / 2);
                }
                let (yaw, pitch) = (rad(yaw as i32), rad(pitch as i32));
                bullet.velocity = (yaw.cos() * pitch.cos() * speed, -yaw.sin() * pitch.cos() * speed, pitch.sin() * speed);
                let near = length3((to.0, to.1, to.2 / 4.0)).trunc();
                let height = bullet.position.2 - ground(self, bullet.position);
                if near <= speed * 0.5 || height <= 0.0 {
                    hit = true;
                    if height > 0.0 {
                        bullet.position = target;
                    }
                }
                let gap = |p: (f64, f64, f64)| length3((p.0 - target.0, p.1 - target.1, p.2 - target.2)).trunc();
                let closing = gap(old) - gap(bullet.position);
                if !bullet.locked {
                    if bullet.samples < 60 {
                        bullet.samples += 1;
                        bullet.closing += closing;
                    } else {
                        bullet.closing = bullet.closing * 59.0 / 60.0 + closing;
                        if (0.0..60.0).contains(&bullet.closing) && !projectile.very_high {
                            hit = true;
                        }
                    }
                }
            } else {
                // Ballistic: gravity, then the tests on where it has got to.
                let gravity = if projectile.floater { self.gravity as f64 * 0.5 } else { self.gravity as f64 };
                let mut v = bullet.velocity;
                if !projectile.inviso {
                    v.2 -= gravity;
                }
                let p = (bullet.position.0 + v.0, bullet.position.1 + v.1, bullet.position.2 + v.2);
                let previous = bullet.position;
                bullet.position = p;
                let cell = ((p.0 as i32).div_euclid(LEPTONS_PER_CELL), (p.1 as i32).div_euclid(LEPTONS_PER_CELL));
                let target_cell = (bullet.target_at.0.div_euclid(LEPTONS_PER_CELL), bullet.target_at.1.div_euclid(LEPTONS_PER_CELL));
                let height = p.2 - ground(self, p);
                let same_building = self.building_on(cell).is_some() && self.building_on(cell) == self.building_on(target_cell);
                if (cell == target_cell || same_building) && height < 208.0 {
                    hit = true;
                    reached_cell = true;
                } else {
                    // Anything of an enemy's within half a cell is hit.
                    let near = self.units.iter().find(|u| {
                        u.cell() == cell
                            && Some(Aim::Unit(u.id)) != bullet.owner
                            && !self.allied(u.owner, bullet.house)
                            && ((u.x as f64 - p.0).hypot(u.y as f64 - p.1) as i32) < 128
                    });
                    if let Some(unit) = near {
                        hit = true;
                        // The blast is where the unit is, at its height: one
                        // in the air (a Rocketeer) is hit there, not on the
                        // ground under it, which a warhead with no spread
                        // would miss.
                        if !projectile.inaccurate {
                            let at = self.aim_point(Aim::Unit(unit.id)).map_or(ground(self, p), |at| at.2 as f64);
                            bullet.position = (unit.x as f64, unit.y as f64, at);
                        }
                    } else if self.map.index(cell.0, cell.1).is_none() {
                        gone = true;
                        bullet.position = previous;
                    } else {
                        bullet.velocity = v;
                        if length3(v) < 10.0 && height < 10.0 {
                            hit = true;
                        }
                    }
                }
            }

            if gone {
                self.bullets.remove(index);
                continue;
            }
            let height = bullet.position.2 - ground(self, bullet.position);
            if !hit && height <= -416.0 {
                hit = true;
            }
            if hit && height < 0.0 {
                bullet.position.2 -= height;
            }
            if projectile.rot > 0 || projectile.ranged {
                // The fuse: armed after a while, set off close to where the target
                // was, or once it starts to get further away.
                if self.tick >= bullet.arm_at {
                    let to = (
                        bullet.position.0 - bullet.target_at.0 as f64,
                        bullet.position.1 - bullet.target_at.1 as f64,
                        bullet.position.2 - bullet.target_at.2 as f64,
                    );
                    let d = length3(to) as i32 / 2;
                    if d < 32 {
                        fuse = 1;
                    } else if d < 256 && d > bullet.fuse_last {
                        fuse = 2;
                    } else {
                        bullet.fuse_last = d;
                    }
                }
            }
            if !hit && fuse == 0 {
                if projectile.degenerates && bullet.damage > 5 {
                    bullet.damage -= 1;
                }
                self.bullets[index] = bullet;
                index += 1;
                continue;
            }

            // Going off: on the target if close enough to it.
            let mut at = bullet.position;
            if let Some(target) = self.aim_point(bullet.target).filter(|_| !projectile.inaccurate) {
                let target = (target.0 as f64, target.1 as f64, target.2 as f64);
                let mut d = length3((at.0 - target.0, at.1 - target.1, (target.2 + at.2) / 2.0 - target.2)).trunc();
                if reached_cell {
                    d = (d / 3.0).trunc();
                }
                if (fuse == 1 || reached_cell) && (fuse == 1 || d <= (2.0 * length3(bullet.velocity)).max(128.0)) {
                    at = target;
                }
                if length3((at.0 - target.0, at.1 - target.1, at.2 - target.2)) < 42.0 {
                    at = target;
                }
            }
            let at = (at.0 as i32, at.1 as i32, at.2 as i32);
            self.bullets.remove(index);
            let special = self.warheads.get(weapon.warhead as usize).cloned().is_some_and(|warhead| self.special_hit(&warhead, bullet.owner, bullet.house, bullet.target));
            if special {
            } else if self.warheads.get(weapon.warhead as usize).is_some_and(|w| w.temporal) {
                self.temporal_hit(bullet.owner, bullet.target);
            } else {
                self.explode(at, bullet.damage, weapon.warhead, bullet.owner);
                self.irradiate(at, weapon.rad_level, weapon.warhead);
                if let Some((fragment, count)) = projectile.shrapnel {
                    self.shrapnel(&bullet, at, fragment, count);
                }
            }
            let cell = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
            let water = self.map.index(cell.0, cell.1).is_some_and(|i| self.map.land[i] == Land::Water as u8);
            self.blast_event(weapon.warhead, bullet.damage, at, water);
            let _ = bullet.born;
        }
    }

    /// The best thing in reach for a unit or building to shoot at.
    // Square rings out
    // from its own cell to its weapon's range, one thing looked at per cell, the
    // strictly better taken, and an early finish a quarter and half-way out if
    // something has been found. The tests kept here: the fire test's refusals,
    // the one-per-cent rule, the dead, friends, things too far, illegal and
    // insignificant targets, and for a person's side the neutral house and
    // buildings that do no harm.
    fn greatest_threat(&self, me: Aim, at: (i32, i32, i32), house: u8, fighter: &Fighter) -> Option<Aim> {
        self.greatest_threat_in(me, at, house, fighter, false)
    }

    /// How far a unit guarding an area looks from its post, in leptons.
    // Twice its `GuardRange`, or twice
    // its longer weapon's range without one, sixteen cells at most.
    pub(crate) fn area_range(&self, fighter: &Fighter) -> i32 {
        let range_of = |mount: Option<Mount>| mount.and_then(|m| self.weapons.get(m.weapon as usize)).map_or(0, |w| w.range);
        let reach = if fighter.guard_range > 0 { fighter.guard_range } else { range_of(fighter.primary).max(range_of(fighter.secondary)) };
        (2 * reach).clamp(0, 0x1000)
    }

    /// The same, or with `area` the search of a unit guarding an area: about
    /// the point given, out to its area range, and not only what it could
    /// shoot from where it is.
    // Scan mask 2.
    fn greatest_threat_in(&self, me: Aim, at: (i32, i32, i32), house: u8, fighter: &Fighter, area: bool) -> Option<Aim> {
        let range_of = |mount: Option<Mount>| mount.and_then(|m| self.weapons.get(m.weapon as usize)).map_or(0, |w| w.range);
        let cells = if area {
            self.area_range(fighter) / 256
        } else if fighter.guard_range > 0 {
            fighter.guard_range / 256
        } else {
            range_of(fighter.primary).max(range_of(fighter.secondary)) / 256 + 1
        };
        let centre = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
        let mut best: Option<(Aim, i32)> = None;
        let human = self.human(house);
        let blank = Arms::default();
        let evaluate = |cell: (i32, i32)| -> Option<(Aim, i32)> {
            let candidate = self
                .units
                .iter()
                .find(|u| u.cell() == cell && Aim::Unit(u.id) != me && u.health > 0 && !self.allied(u.owner, house))
                .map(|u| (Aim::Unit(u.id), u.owner, u.health as f64 / u.stats.strength.max(1) as f64))
                .or_else(|| {
                    self.building_on(cell).map(|i| &self.buildings[i]).filter(|b| Aim::Building(b.id) != me && !self.allied(b.owner, house)).map(|b| {
                        let strength = self.building_types.get(b.kind as usize).map_or(1, |k| k.strength).max(1);
                        (Aim::Building(b.id), b.owner, b.health as f64 / strength as f64)
                    })
                })?;
            let (who, owner, health) = candidate;
            // A cloaked thing on a cell
            // my house does not sense is passed over.
            if let Aim::Unit(id) = who
                && self.unit(id).is_some_and(|unit| self.hidden_from(unit, house)) {
                    return None;
                }
            // One that is disguised to my house is
            // passed over unless my type sees through disguises.
            if let Aim::Unit(id) = who
                && !fighter.detect_disguise && self.unit(id).is_some_and(|unit| self.disguised_to(unit, house)) {
                    return None;
                }
            // With both on
            // bridge cells, one on the deck and one under it are no targets for
            // each other.
            if let (Aim::Unit(mine), Aim::Unit(id)) = (me, who)
                && let (Some(a), Some(b)) = (self.unit(mine), self.unit(id)) {
                    let (ca, cb) = (a.cell(), b.cell());
                    if self.map.bridged(ca.0, ca.1) && self.map.bridged(cb.0, cb.1) && a.on_bridge != b.on_bridge {
                        return None;
                    }
                }
            let theirs = self.fighter(who)?;
            let index = self.select_weapon(fighter, who);
            let mount = Self::mount(fighter, index)?;
            let weapon = self.weapons.get(mount.weapon as usize)?;
            let verses = self.warheads.get(weapon.warhead as usize).map_or(1.0, |w| w.verses[theirs.armour.min(10) as usize]);
            match self.fire_error(at, fighter, &blank, who, index) {
                FireError::Illegal | FireError::Cant => return None,
                FireError::Range if !area => return None,
                _ => {}
            }
            if (verses * 100.0) as i32 == 1 || !theirs.legal_target {
                return None;
            }
            // A thing
            // of the passive house, or an `Insignificant` one, is no target for
            // anyone (the bridge repair hut, oil derricks, barrels).
            if theirs.insignificant || owner == 0 {
                return None;
            }
            let _ = human;
            // A person's units leave alone any
            // building without both a weapon and a `ThreatPosed` of its own.
            if human && matches!(who, Aim::Building(_)) && (theirs.primary.is_none() || theirs.threat_posed == 0) {
                return None;
            }
            // The
            // type's own weights, by default 200 for what my weapon does to it,
            // -200 for what its weapon does to me (the sign turned if it is aiming
            // at me) and -200 for its health.
            let their_index = self.select_weapon(&theirs, me);
            let their_verses = Self::mount(&theirs, their_index)
                .and_then(|m| self.weapons.get(m.weapon as usize))
                .and_then(|w| self.warheads.get(w.warhead as usize))
                .map_or(0.0, |w| w.verses[fighter.armour.min(10) as usize]);
            let aimed_at_me = match who {
                Aim::Unit(id) => self.unit(id).is_some_and(|u| u.arms.target == Some(me)),
                Aim::Building(id) => self.buildings.iter().any(|b| b.id == id && b.arms.target == Some(me)),
                Aim::Cell(..) => false,
            };
            let value = 100_000.0 + -200.0 * if aimed_at_me { -their_verses } else { their_verses } + 200.0 * verses - 200.0 * health;
            Some((who, value as i32))
        };
        for k in 0..cells.max(1) {
            let mut ring = Vec::new();
            for x in -k..=k {
                ring.push((centre.0 + x, centre.1 - k));
                ring.push((centre.0 + x, centre.1 + k));
            }
            for y in 1 - k..k {
                ring.push((centre.0 - k, centre.1 + y));
                ring.push((centre.0 + k, centre.1 + y));
            }
            for cell in ring {
                if let Some((who, value)) = evaluate(cell)
                    && best.is_none_or(|(_, most)| value > most) {
                        best = Some((who, value));
                    }
            }
            if best.is_some() && (k == cells / 4 || k == cells / 2) {
                break;
            }
        }
        best.map(|(who, _)| who)
    }

    /// Picks a cell to shoot from and sends a unit there.
    // Rings in from just under the weapon's range,
    // each tried at a spread of angles starting on the side of the target the
    // unit is on; the first cell in range that the unit can stand on is taken.
    // A candidate must be in the unit's
    // zone and hold nothing; for a vehicle a taken one is walked round (eight
    // steps, each a turn on from the last); with no cell found at all the unit
    // goes to a free cell near the last one tried and the target is dropped.
    fn approach(&mut self, index: usize, target: Aim, weapon: &Weapon) {
        const OFFSETS: [i32; 25] = [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 8, -8, 16, -16, 24, -24, 32, -32, 48, -48, 64, -64];
        let Some(to) = self.aim_point(target) else {
            return;
        };
        let unit = &self.units[index];
        let mut reach = weapon.range;
        if reach >= 0x200 {
            reach -= 0x80;
        }
        if let Aim::Building(id) = target
            && let Some(building) = self.buildings.iter().find(|b| b.id == id) {
                reach += (building.size.0 + building.size.1) * 64;
            }
        let mut reach = (reach as f64 - if unit.stats.infantry { 281.6 } else { 179.2 }) as i32;
        reach = if reach <= 0 { 0 } else { reach.max(0xff) };
        let towards = (facing_of(((to.1 - unit.y) as f64).atan2((unit.x - to.0) as f64)) as u16 >> 8) as i32;
        let others: Vec<Other> = self.others();
        let mut r = reach as f64;
        let mut last = None;
        while r > 204.8 {
            for offset in OFFSETS {
                let angle = rad(((towards + offset) & 0xff) << 8);
                let point = (to.0 + (angle.cos() * r) as i32, to.1 - (angle.sin() * r) as i32);
                let cell = (point.0.div_euclid(LEPTONS_PER_CELL), point.1.div_euclid(LEPTONS_PER_CELL));
                let (cx, cy) = cell_centre(cell);
                let from = (cx, cy, self.ground_z(cx, cy));
                if !self.in_range(from, target, weapon) || !self.map.in_playfield(cell.0, cell.1) {
                    continue;
                }
                let clear = cell == unit.cell()
                    || entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, unit, cell, None, cell, -1) == code::OK;
                let reachable = self.map.zones().is_none_or(|zones| {
                    let zone = unit.stats.movement_zone as usize;
                    let _ = zones;
                    self.map.zone_on(cell, zone, self.map.deck(cell.0, cell.1)) == self.map.zone_on(unit.cell(), zone, unit.on_bridge)
                });
                if clear && reachable {
                    if cell != unit.cell() {
                        self.send(index, cell);
                        self.units[index].arms.approach = Some(cell);
                    }
                    return;
                }
                last = Some(cell);
                if unit.stats.infantry {
                    continue;
                }
                let mut round = cell;
                for dir in 0..8 {
                    round = (round.0 + crate::DIRECTIONS[dir].0, round.1 + crate::DIRECTIONS[dir].1);
                    let (cx, cy) = cell_centre(round);
                    let from = (cx, cy, self.ground_z(cx, cy));
                    if !self.in_range(from, target, weapon) || !self.map.in_playfield(round.0, round.1) {
                        continue;
                    }
                    let zone = unit.stats.movement_zone as usize;
                    let free = round == unit.cell() || entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, unit, round, None, round, -1) == code::OK;
                    if free && self.map.zone_on(round, zone, self.map.deck(round.0, round.1)) == self.map.zone_on(unit.cell(), zone, unit.on_bridge) {
                        if round != unit.cell() {
                            self.send(index, round);
                            self.units[index].arms.approach = Some(round);
                        }
                        return;
                    }
                }
            }
            r -= 256.0;
        }
        // Nothing to stand on: a free cell near the last one tried, and the
        // target is let go.
        let Some(last) = last else {
            return;
        };
        let unit = &self.units[index];
        let near = crate::nearby(&self.map, &self.claims, &self.spots, unit, last, crate::Near::new(false, true, None), self.tick);
        self.units[index].arms.target = None;
        self.units[index].arms.attacking = false;
        self.units[index].arms.approach = None;
        match near {
            Some(cell) if cell != self.units[index].cell() => self.send(index, cell),
            _ => {}
        }
    }

    /// Whether a unit could be told to attack something, and if so whether that
    /// is in range now.
    // By the fire test without range, then range.
    pub fn attack_action(&self, unit: u32, target: Aim, force: bool) -> Option<bool> {
        let unit = self.unit(unit)?;
        let fighter = unit.stats.fighter;
        // Without force-fire the target must
        // not be an ally and must be a `LegalTarget`; with it neither is asked,
        // nor does an "illegal" answer from the fire test matter.
        if !force {
            if self.owner_of(target).is_some_and(|owner| self.allied(owner, unit.owner)) {
                return None;
            }
            if self.fighter(target).is_some_and(|theirs| !theirs.legal_target) {
                return None;
            }
        }
        let which = self.select_weapon(&fighter, target);
        Self::mount(&fighter, which)?;
        let at = (unit.x, unit.y, self.ground_z(unit.x, unit.y));
        match self.fire_error(at, &fighter, &Arms::default(), target, which) {
            FireError::Illegal if !force => None,
            FireError::Cant => None,
            FireError::Range => Some(false),
            FireError::Illegal => {
                let weapon = Self::mount(&fighter, which).and_then(|m| self.weapons.get(m.weapon as usize))?;
                Some(self.in_range(at, target, weapon))
            }
            _ => Some(true),
        }
    }

    /// Whether a building could be told to fire on something now: it has a weapon
    /// that can be used, and the target is in range.
    // As for a unit, but something
    // that cannot move must have the target in range; the weapon's shot must be
    // able to hit the ground.
    pub fn building_attack_action(&self, building: u32, target: Aim, force: bool) -> bool {
        let Some(building) = self.buildings.iter().find(|b| b.id == building && b.buildup.is_none()) else {
            return false;
        };
        let Some(kind) = self.building_types.get(building.kind as usize) else {
            return false;
        };
        if (kind.can_be_occupied && building.occupants.is_empty()) || !self.online(building) {
            return false;
        }
        if !force {
            if self.owner_of(target).is_some_and(|owner| self.allied(owner, building.owner)) {
                return false;
            }
            if self.fighter(target).is_some_and(|theirs| !theirs.legal_target) {
                return false;
            }
        }
        let which = match building.occupants.first() {
            Some(first) => usize::from(self.houses.get(first.owner as usize).is_some_and(|h| h.side == 1)),
            None => self.select_weapon(&kind.fighter, target),
        };
        let Some(weapon) = Self::mount(&kind.fighter, which).and_then(|m| self.weapons.get(m.weapon as usize)) else {
            return false;
        };
        let Some(at) = self.aim_point(Aim::Building(building.id)) else {
            return false;
        };
        match self.fire_error(at, &kind.fighter, &Arms::default(), target, which) {
            FireError::Illegal if force => self.in_range(at, target, weapon),
            FireError::Illegal | FireError::Cant | FireError::Range => false,
            _ => true,
        }
    }

    /// One tick of fighting for every unit and building, then the shots.
    pub(crate) fn fight(&mut self) {
        self.wear_down();
        self.enter_buildings();
        // Units.
        for index in 0..self.units.len() {
            // Digging in and getting up take the
            // length of their sequences.
            // A
            // type with `UndeployDelay` gets up by itself that long after it
            // was told to deploy.
            if self.units[index].stance == Stance::Deployed && self.units[index].undeploy_at.is_some_and(|at| self.tick >= at) {
                let frames = self.units[index].stats.fighter.undeploy_frames as u64;
                self.units[index].undeploy_at = None;
                self.units[index].arms.target = None;
                self.units[index].stance = Stance::Undeploying(self.tick + frames);
            }
            match self.units[index].stance {
                Stance::Deploying(until) if self.tick >= until => self.units[index].stance = Stance::Deployed,
                Stance::Undeploying(until) if self.tick >= until => self.units[index].stance = Stance::Standing,
                _ => {}
            }
            let unit = &self.units[index];
            // A gunner vehicle's one
            // weapon is the one its passenger gives it.
            let mut fighter = unit.stats.fighter;
            if let Some((mount, _)) = unit.gun {
                // An elite
                // fires the `EliteWeapon` of the same number.
                let elite = (unit.rank() == 0).then(|| self.kind_of(unit).and_then(|kind| kind.gunner_elite.get(unit.gun_mode as usize).copied())).flatten();
                fighter.primary = Some(elite.unwrap_or(mount));
                fighter.secondary = None;
            }
            // Aircraft fight by their own missions.
            if fighter.primary.is_none() || unit.tethered || unit.health <= 0 || unit.plane.is_some() {
                continue;
            }
            // One held by a `Temporal` weapon does nothing at all.
            if unit.frozen.is_some() || self.tick < unit.warp_until || self.tick < unit.paralysed_until || matches!(unit.stance, Stance::Deploying(_) | Stance::Undeploying(_)) {
                continue;
            }
            let dug_in = unit.stance == Stance::Deployed;
            // A dug-in soldier
            // whose second weapon lays radiation (the Desolator) does not look
            // for targets or shoot: he only tops up the ground under him.
            if dug_in && fighter.deploy_fire && fighter.secondary.and_then(|mount| self.weapons.get(mount.weapon as usize)).is_some_and(|weapon| weapon.rad_level > 0) {
                let arms = &mut self.units[index].arms;
                arms.target = None;
                arms.attacking = false;
                continue;
            }
            let me = Aim::Unit(unit.id);
            let at = (unit.x, unit.y, self.ground_z(unit.x, unit.y) + self.altitude(unit));
            let (owner, body) = (unit.owner, unit.facing);
            let mut arms = unit.arms;
            let moving = unit.dest.is_some();
            let infantry = unit.stats.infantry;

            // Drop a target that has gone, or gone out of sight under a cloak.
            let lost = match arms.target {
                Some(Aim::Unit(id)) => self.unit(id).is_some_and(|other| self.hidden_from(other, unit.owner)),
                _ => false,
            };
            if lost || arms.target.is_some_and(|target| !self.alive(target)) {
                arms.target = None;
                arms.attacking = false;
                arms.firing = 0;
            }
            // An idle unit looks
            // for a target every `NormalTargetingDelay` frames plus up to two; one
            // on the move only if its type has `OpportunityFire`.
            // A unit on its way into something is on another mission.
            let idle = !moving && unit.work.job == Job::Idle && unit.falling.is_none() && unit.capture.is_none() && unit.boarding.is_none();
            let scan_due = self.tick >= arms.scan_at;
            if self.tick >= arms.scan_at && fighter.can_passive_acquire && (idle || fighter.opportunity_fire) && !arms.attacking {
                arms.scan_at = self.tick + self.targeting_delay as u64 + (self.next_random() % 3) as u64;
                if arms.target.is_some() && arms.passive {
                    let index_w = self.select_weapon_of(owner, &fighter, arms.target.unwrap());
                    let ready = Arms { rearm_at: 0, ..arms };
                    if matches!(self.fire_error(at, &fighter, &ready, arms.target.unwrap(), index_w), FireError::Illegal | FireError::Range | FireError::Cant) {
                        arms.target = None;
                    }
                }
                if arms.target.is_none()
                    && let Some(found) = self.greatest_threat(me, at, owner, &fighter) {
                        arms.target = Some(found);
                        arms.passive = true;
                    }
            }
            // A unit that
            // guards an area keeps the cell it was left at as its post. With
            // no target it looks from the post out to its area range and goes
            // after what it finds; stopped further from the post than three
            // quarters of that range, it drops its target and goes back.
            if fighter.default_guard_area {
                let here = self.units[index].cell();
                // (An order clears the post; the next place it stops is it.)
                if !moving && arms.target.is_none() && self.units[index].post.is_none() && idle {
                    self.units[index].post = Some(here);
                }
                if let Some(post) = self.units[index].post {
                    let reach = self.area_range(&fighter) as i64;
                    let centre = cell_centre(post);
                    let strayed = (((at.0 - centre.0) as f64).hypot((at.1 - centre.1) as f64)) as i64 > reach * 3 / 4;
                    if strayed && !moving && arms.firing == 0 {
                        arms.target = None;
                        arms.attacking = false;
                        arms.approach = None;
                        self.send(index, post);
                    } else if arms.target.is_none() && !moving && idle && fighter.can_passive_acquire && scan_due {
                        let from = (centre.0, centre.1, self.ground_z(centre.0, centre.1));
                        if let Some(found) = self.greatest_threat_in(me, from, owner, &fighter, true) {
                            arms.target = Some(found);
                            arms.passive = true;
                            arms.attacking = true;
                        }
                    }
                }
            }

            // A
            // foot soldier on guard with nothing to shoot at, whose second
            // weapon charges, takes to the coil of its own house in one of the
            // eight cells round it. While at that
            // it keeps looking, and goes for an enemy that comes in reach.
            if !arms.target.is_some_and(|target| matches!(target, Aim::Building(_))) {
                self.units[index].charging = false;
            }
            if infantry && idle && arms.target.is_none()  {
                let assault = fighter.secondary.and_then(|mount| self.weapons.get(mount.weapon as usize)).and_then(|weapon| self.warheads.get(weapon.warhead as usize)).is_some_and(|warhead| warhead.electric_assault);
                if let Some(coil) = Some(assault).filter(|on| *on).and_then(|_| self.coil_beside(self.units[index].cell(), owner)) {
                    arms.target = Some(Aim::Building(coil));
                    arms.passive = false;
                    arms.attacking = true;
                    self.units[index].charging = true;
                }
            } else if self.units[index].charging
                && let Some(found) = self.greatest_threat(me, at, owner, &fighter) {
                    arms.target = Some(found);
                    arms.passive = true;
                    arms.attacking = false;
                    self.units[index].charging = false;
                }

            // A `HoverAttack` flyer on the ground with a
            // target takes off first.
            let grounded = {
                let unit = &self.units[index];
                unit.stats.jet.is_some_and(|jet| jet.hover_attack) && unit.jet.is_some_and(|jet| jet.state == flight::GROUNDED)
            };
            if arms.target.is_some() && grounded && !moving {
                let cell = self.units[index].cell();
                self.send(index, cell);
            }
            if let Some(target) = arms.target {
                // Dug in, a `DeployFire` soldier
                // uses its second weapon.
                let which = if dug_in && fighter.deploy_fire && fighter.secondary.is_some() { 1 } else { self.select_weapon_of(owner, &fighter, target) };
                let weapon = Self::mount(&fighter, which).and_then(|m| self.weapons.get(m.weapon as usize)).cloned();
                if let Some(weapon) = weapon {
                    let projectile = self.projectiles.get(weapon.projectile as usize).cloned().unwrap_or_default();
                    let mut error = self.fire_error(at, &fighter, &arms, target, which);
                    let want = self.direction_to((at.0, at.1), target).unwrap_or(0);
                    if error == FireError::Ok && !infantry {
                        // A vehicle's own tests.
                        if !fighter.mobile_fire && moving {
                            error = FireError::Moving;
                        } else if fighter.turret && arms.turret.remaining > 0 && projectile.rot == 0 {
                            error = FireError::Rotating;
                        } else if !weapon.omni_fire {
                            let facing = if fighter.turret { arms.turret.current() } else { self.units[index].turn.current() };
                            let tolerance = if projectile.rot != 0 { 0x1000 } else { 0x800 };
                            if (facing.wrapping_sub(want) as i16 as i32).abs() > tolerance {
                                error = FireError::Facing;
                            }
                        }
                    }
                    if error == FireError::Ok && infantry && moving {
                        error = FireError::Moving;
                    }
                    match error {
                        FireError::Ok if infantry => {
                            // The soldier turns to
                            // its target at once and starts its firing sequence;
                            // the shot leaves at the sequence's `FireUp` frame.
                            if arms.firing == 0 {
                                arms.firing = 1;
                                let unit = &mut self.units[index];
                                unit.facing = (want >> 8) as u8;
                                unit.turn = Turn::facing(unit.facing);
                            }
                        }
                        FireError::Ok => {
                            let firing = if fighter.turret { arms.turret.current() } else { self.units[index].turn.current() };
                            // A `LimboLaunch` weapon
                            // sends its firer into the target.
                            if !self.uncloak_to_fire(index) {
                                // It comes out of its cloak first.
                            } else if weapon.limbo_launch {
                                let id = self.units[index].id;
                                let weapon_index = Self::mount(&fighter, which).map_or(0, |mount| mount.weapon);
                                self.events.push(Event::Fired { weapon: weapon_index, at, to: at, facing: (firing >> 8) as u8, by: me });
                                arms.target = None;
                                arms.attacking = false;
                                self.units[index].arms = arms;
                                self.pending_bites.push((id, target, weapon_index));
                                continue;
                            } else if self.fire(me, at, body, firing, owner, &fighter, &mut arms, target, which) && weapon.fire_once {
                                // One shot and
                                // the target is let go.
                                arms.target = None;
                                arms.attacking = false;
                                self.fired_once(index);
                            }
                        }
                        FireError::Facing => {
                            // A turret turns to the
                            // target; a unit without one turns its body, but only
                            // when it is going nowhere.
                            if fighter.turret {
                                if arms.turret.remaining == 0 {
                                    arms.turret = Turn::start(arms.turret.current(), want, self.units[index].stats.rot);
                                }
                            } else if !self.units[index].is_moving() {
                                let unit = &mut self.units[index];
                                unit.turn = Turn::start(unit.turn.current(), want, unit.stats.rot);
                            }
                        }
                        FireError::Illegal | FireError::Cant => {
                            arms.target = None;
                            arms.attacking = false;
                        }
                        FireError::Range => {
                            // Out of range, a
                            // unit with nowhere to go closes in if it was told to
                            // attack or its type may approach; otherwise it lets
                            // the target go.
                            if !moving && dug_in {
                                arms.target = None;
                            } else if !moving {
                                if arms.attacking || fighter.can_approach_target {
                                    self.approach(index, target, &weapon);
                                    // (What the search settled is taken up into the copy
                                    // that is written back below.)
                                    arms.approach = self.units[index].arms.approach;
                                    if self.units[index].arms.target.is_none() {
                                        arms.target = None;
                                        arms.attacking = false;
                                    }
                                } else {
                                    arms.target = None;
                                }
                            } else if (arms.attacking || fighter.can_approach_target) && (self.tick + self.units[index].id as u64).is_multiple_of(15) {
                                // On its way, something in the
                                // air picks its place again each time the mission comes
                                // round (every 14 to 16 frames), so it follows a target
                                // that moves; a thing on the ground only once the place
                                // it chose is further from the target than the weapon's
                                // range (`ApproachTargetResetMultiplier`, 1.5, kept as
                                // the whole number 1).
                                let unit = &self.units[index];
                                let flying = self.altitude(unit) >= 208;
                                let stale = arms.approach.zip(self.aim_point(target)).is_some_and(|(cell, to)| {
                                    let (cx, cy) = cell_centre(cell);
                                    ((cx - to.0) as f64).hypot((cy - to.1) as f64) as i32 > weapon.range
                                });
                                if flying || stale {
                                    self.approach(index, target, &weapon);
                                    // (What the search settled is taken up into the copy
                                    // that is written back below.)
                                    arms.approach = self.units[index].arms.approach;
                                    if self.units[index].arms.target.is_none() {
                                        arms.target = None;
                                        arms.attacking = false;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    // The soldier's firing sequence.
                    if infantry && arms.firing > 0 {
                        // Lying down it fires from the
                        // `FireProne` sequence instead.
                        let (fire_up, fire_frames) = if self.units[index].prone { (fighter.fire_prone, fighter.fire_prone_frames) } else { (fighter.fire_up, fighter.fire_frames) };
                        if arms.firing - 1 == fire_up {
                            let ready = self.fire_error(at, &fighter, &arms, target, which) == FireError::Ok;
                            if ready && weapon.limbo_launch {
                                let id = self.units[index].id;
                                let weapon_index = Self::mount(&fighter, which).map_or(0, |mount| mount.weapon);
                                self.events.push(Event::Fired { weapon: weapon_index, at, to: at, facing: self.units[index].facing, by: me });
                                arms.target = None;
                                arms.attacking = false;
                                arms.firing = 0;
                                self.units[index].arms = arms;
                                self.pending_bites.push((id, target, weapon_index));
                                continue;
                            } else if ready {
                                let firing = self.units[index].turn.current();
                                if self.fire(me, at, self.units[index].facing, firing, owner, &fighter, &mut arms, target, which) && weapon.fire_once {
                                    arms.target = None;
                                    arms.attacking = false;
                                    self.fired_once(index);
                                }
                            } else {
                                arms.firing = 0;
                            }
                        }
                        if arms.firing > 0 {
                            arms.firing += 1;
                            if arms.firing > fire_frames.max(fire_up + 1) {
                                arms.firing = 0;
                            }
                        }
                    }
                }
            } else {
                arms.firing = 0;
            }
            // With no target the turret points where
            // the unit is going, or else follows the body.
            if fighter.turret && arms.target.is_none() && arms.turret.remaining == 0 {
                let unit = &self.units[index];
                let rest = match unit.dest {
                    Some(dest) => self.direction_to((unit.x, unit.y), Aim::Cell(dest.0, dest.1)).unwrap_or(unit.turn.current()),
                    None => unit.turn.current(),
                };
                if arms.turret.current() != rest {
                    arms.turret = Turn::start(arms.turret.current(), rest, unit.stats.rot);
                }
            }
            if arms.turret.remaining > 0 {
                arms.turret.remaining -= 1;
            }
            self.units[index].arms = arms;
        }

        // Buildings.
        // A defence looks for a target in
        // range, turns its turret if it has one, and fires when lined up; one out
        // of range is let go.
        for index in 0..self.buildings.len() {
            let building = &self.buildings[index];
            let Some(kind) = self.building_types.get(building.kind as usize) else {
                continue;
            };
            let fighter = kind.fighter;
            if fighter.primary.is_none() || building.buildup.is_some() || building.health <= 0 || building.frozen.is_some() {
                continue;
            }
            // A building that soldiers fire from does nothing with nobody
            // in it. One that needs an engineer is off until it has been
            // captured.
            let garrison = kind.can_be_occupied.then_some(building.occupants.len());
            if garrison == Some(0) || (kind.needs_engineer && building.owner == 0) {
                self.buildings[index].arms.target = None;
                continue;
            }
            // The secondary weapon if the first one
            // in is of a Soviet house, else the primary.
            let garrison_weapon = garrison.map(|_| {
                let house = building.occupants[0].owner as usize;
                usize::from(self.houses.get(house).is_some_and(|h| h.side == 1))
            });
            // Fired from the next of the building's
            // `MuzzleFlash` points, given in screen pixels.
            let muzzle = garrison.and_then(|_| kind.muzzles.get(building.muzzle % kind.muzzles.len().max(1)).copied());
            // A defence without
            // power cannot fire and lets its target go.
            if !self.online(building) {
                self.buildings[index].arms.target = None;
                continue;
            }
            let rot = kind.turret_rot;
            let delayed_fire = kind.delayed_fire;
            let me = Aim::Building(building.id);
            let Some(mut at) = self.aim_point(me) else {
                continue;
            };
            let centre = at;
            if let Some((px, py)) = muzzle {
                let (px, py) = (px as f64, py as f64);
                at.0 = building.cell.0 * LEPTONS_PER_CELL + ((py / 15.0 + px / 30.0) * 128.0) as i32;
                at.1 = building.cell.1 * LEPTONS_PER_CELL + ((py / 15.0 - px / 30.0) * 128.0) as i32;
            }
            let owner = building.owner;
            let mut arms = building.arms;
            if arms.target.is_some_and(|target| !self.alive(target)) {
                arms.target = None;
            }
            if self.tick >= arms.scan_at {
                arms.scan_at = self.tick + self.targeting_delay as u64 + (self.next_random() % 3) as u64;
                if arms.target.is_none() {
                    arms.target = self.greatest_threat(me, centre, owner, &fighter);
                    arms.passive = true;
                }
            }
            // A charged shot is let go when its time
            // is up, if the target is still there to be fired on.
            if let Some((_, due, which)) = self.buildings[index].charge {
                if self.tick >= due {
                    self.buildings[index].charge = None;
                    // A tower
                    // lending its beam draws it from its own firing point to
                    // the firing point of the tower it helps, as that was
                    // when it was asked; nothing is shot and nothing heard;
                    // it is then out of action for `PrismSupportDelay`.
                    if let Some(to) = self.buildings[index].supporting.take() {
                        let weapon = fighter.secondary.or(fighter.primary).map_or(0, |mount| mount.weapon);
                        self.buildings[index].last_supports = 0;
                        self.events.push(Event::Fired { weapon, at, to, facing: 0, by: me });
                        arms.rearm_at = self.tick + self.ability_rules.prism_delay as u64;
                        self.buildings[index].arms = arms;
                        continue;
                    }
                    self.buildings[index].last_supports = self.buildings[index].supports;
                    if let Some(target) = arms.target
                        && self.fire_error(centre, &fighter, &arms, target, which) == FireError::Ok {
                            let want = self.direction_to((at.0, at.1), target).unwrap_or(0);
                            self.fire(me, at, 0x40, want, owner, &fighter, &mut arms, target, which);
                        }
                    // The count of beams lent
                    // starts again with each shot.
                    self.buildings[index].supports = 0;
                }
                self.buildings[index].arms = arms;
                continue;
            }
            if let Some(target) = arms.target {
                // A charged building answers its
                // second weapon before anything else is asked.
                let charged = self.buildings[index].charged && fighter.secondary.is_some();
                let which = garrison_weapon.unwrap_or_else(|| if charged { 1 } else { self.select_weapon(&fighter, target) });
                let mut error = self.fire_error(centre, &fighter, &arms, target, which);
                // A building that charges starts
                // its charge instead of firing.
                if error == FireError::Ok && delayed_fire > 0 {
                    // A prism tower about to fire
                    // first takes, one each time round and up to
                    // `PrismSupportMax`, the nearest other tower of its house
                    // that is loaded, idle and within its second weapon's
                    // range, which charges to lend its beam; then it charges.
                    if Some(self.buildings[index].kind) == self.ability_rules.prism_type && self.buildings[index].supports < self.ability_rules.prism_max {
                        let reach = fighter.secondary.and_then(|mount| self.weapons.get(mount.weapon as usize)).map_or(0, |weapon| weapon.range) as i64;
                        let (kind, here) = (self.buildings[index].kind, self.buildings[index].cell);
                        let helper = self
                            .buildings
                            .iter()
                            .enumerate()
                            .filter(|(at, other)| {
                                *at != index && other.owner == owner && other.kind == kind && other.buildup.is_none() && other.charge.is_none() && other.arms.target.is_none() && self.tick >= other.arms.rearm_at
                            })
                            .map(|(at, other)| {
                                let (dx, dy) = (((other.cell.0 - here.0) * LEPTONS_PER_CELL) as f64, ((other.cell.1 - here.1) * LEPTONS_PER_CELL) as f64);
                                (at, dx.hypot(dy) as i64)
                            })
                            .filter(|&(_, d)| d <= reach)
                            .min_by_key(|&(at, d)| (d, at))
                            .map(|(at, _)| at);
                        if let Some(helper) = helper {
                            self.buildings[index].supports += 1;
                            self.buildings[helper].charge = Some((self.tick, self.tick + delayed_fire as u64, 1));
                            self.buildings[helper].supporting = Some(at);
                            self.buildings[index].arms = arms;
                            continue;
                        }
                    }
                    self.buildings[index].charge = Some((self.tick, self.tick + delayed_fire as u64, which));
                    self.buildings[index].arms = arms;
                    continue;
                }
                let want = self.direction_to((at.0, at.1), target).unwrap_or(0);
                // A building's turret must point exactly at the target; within
                // one step of its turning rate it is put there.
                if error == FireError::Ok && fighter.turret {
                    let off = (arms.turret.current().wrapping_sub(want) as i16 as i32).abs();
                    if arms.turret.remaining == 0 && off <= rot.clamp(1, 127) * 256 {
                        arms.turret = Turn { desired: want, ..Turn::default() };
                    } else {
                        error = FireError::Facing;
                    }
                }
                match error {
                    FireError::Ok => {
                        let firing = if fighter.turret { arms.turret.current() } else { want };
                        if self.fire(me, at, 0x40, firing, owner, &fighter, &mut arms, target, which) {
                            // The wait for
                            // the next shot is divided by the number inside, and
                            // the next shot comes from the next point.
                            if let Some(count) = garrison {
                                let wait = arms.rearm_at.saturating_sub(self.tick) / count.max(1) as u64;
                                arms.rearm_at = self.tick + wait;
                                self.buildings[index].muzzle = (self.buildings[index].muzzle + 1) % count.max(1);
                            }
                        }
                    }
                    FireError::Facing => {
                        if arms.turret.remaining == 0 {
                            arms.turret = Turn::start(arms.turret.current(), want, rot);
                        }
                    }
                    FireError::Illegal | FireError::Cant | FireError::Range => arms.target = None,
                    _ => {}
                }
            }
            if arms.turret.remaining > 0 {
                arms.turret.remaining -= 1;
            }
            self.buildings[index].arms = arms;
        }

        // Biters take hold once every unit has had its turn (the list of units
        // changes as they go in).
        self.run_leaps();
        self.run_waves();
        for (id, target, weapon) in std::mem::take(&mut self.pending_bites) {
            self.infest(id, target, weapon);
        }
        self.fly_bullets();
        self.run_debris();
        self.run_sinking();
        self.bury();
    }

    /// Takes the dead off the map.
    // A dead vehicle is removed in the same
    // call; a soldier's death is one of several by the warhead's `InfDeath`.
    fn bury(&mut self) {
        for (who, death) in std::mem::take(&mut self.dying) {
            match who {
                Aim::Unit(id) => {
                    let Some(index) = self.units.iter().position(|u| u.id == id) else {
                        continue;
                    };
                    self.leave_dock(index);
                    let unit = self.units.remove(index);
                    for claim in &mut self.claims {
                        if *claim == id + 1 {
                            *claim = 0;
                        }
                    }
                    if unit.stats.infantry {
                        let mut ground = Vec::new();
                        self.release_spots(&unit, &mut ground);
                    }
                    for building in &mut self.buildings {
                        if building.unload.is_some_and(|unload| unload.unit == id) {
                            // The factory's own mission sees that it has gone.
                        }
                        if building.docked == Some(id) {
                            building.docked = None;
                        }
                    }
                    let at = (unit.x, unit.y, self.ground_z(unit.x, unit.y));
                    let (kind, owner, facing, infantry) = (unit.stats.kind, unit.owner, unit.facing, unit.stats.infantry);
                    if death == VANISHED {
                        continue;
                    }
                    if death == ERASED {
                        self.events.push(Event::Erased { at });
                        continue;
                    }
                    // Nothing is thrown and nothing goes off
                    // in the water.
                    let wet = self.wet_death(at, self.altitude(&unit), unit.on_bridge);
                    let centre = self.aim_point_of(&unit);
                    if !wet {
                        let leavings = self.unit_types.get((kind as usize).wrapping_sub(1)).map(|kind| kind.leavings.clone()).unwrap_or_default();
                        self.throw_debris(&leavings, centre);
                    }
                    let crashes = unit.stats.jet.is_some_and(|jet| jet.crashable) || unit.plane.is_some();
                    let sinks = !infantry && self.sinks(&unit);
                    let shell = unit.clone();
                    self.empty_the_dead(unit, wet);
                    // A ship goes down; anything else in the water
                    // leaves a wake and the last splash.
                    let (death, explosion) = if infantry {
                        self.infantry_death(&shell, death, at, wet)
                    } else if sinks {
                        self.start_sinking(&shell);
                        (death, crate::smudge::NO_ANIM)
                    } else if wet {
                        let wake = self.remains_rules.wake;
                        self.events.push(Event::Detonated { warhead: 0, damage: 0, at, water: true, anim: wake });
                        if let Some(&splash) = self.splash_anims.last() {
                            self.events.push(Event::Detonated { warhead: 0, damage: 0, at: (at.0, at.1, at.2 + 5), water: true, anim: splash });
                        }
                        (death, crate::smudge::NO_ANIM)
                    } else {
                        let list = self.unit_types.get((kind as usize).wrapping_sub(1)).map(|kind| kind.explosions.clone()).unwrap_or_default();
                        (death, self.death_anim(&list, at))
                    };
                    self.events.push(Event::UnitDied { id, kind, owner, at, facing, infantry, death, explosion });
                    if !infantry && !crashes {
                        self.vehicle_crew(&shell);
                    }
                    self.crate_from_unit(kind, (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL)));
                }
                Aim::Building(id) => {
                    let Some(index) = self.buildings.iter().position(|b| b.id == id) else {
                        continue;
                    };
                    // Those inside are put out first.
                    self.remove_occupants(index);
                    let building = self.demolish(index);
                    if death == ERASED {
                        let at = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128, 0);
                        self.events.push(Event::Erased { at });
                        continue;
                    }
                    self.building_mark(building.cell, building.size);
                    // One that
                    // `Explodes` goes off at its centre.
                    if let Some(kind) = self.building_types.get(building.kind as usize).filter(|kind| kind.explodes) {
                        let (own, percent, primary, strength) = (kind.death_weapon, kind.death_percent, kind.fighter.primary.map(|mount| mount.weapon), kind.strength);
                        let (x, y) = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128);
                        let point = (x, y, self.ground_z(x, y));
                        self.death_blast(point, own, percent, primary, strength, Aim::Building(id));
                    }
                    let centre = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128);
                    let centre = (centre.0, centre.1, self.ground_z(centre.0, centre.1));
                    let leavings = self.building_types.get(building.kind as usize).map(|kind| kind.leavings.clone()).unwrap_or_default();
                    self.throw_debris(&leavings, centre);
                    let list = self.building_types.get(building.kind as usize).map(|kind| kind.explosions.clone()).unwrap_or_default();
                    let mut explosions = Vec::new();
                    for dy in 0..building.size.1 {
                        for dx in 0..building.size.0 {
                            let (x, y) = ((building.cell.0 + dx) * LEPTONS_PER_CELL + 128, (building.cell.1 + dy) * LEPTONS_PER_CELL + 128);
                            let point = (x, y, self.map.level(building.cell.0 + dx, building.cell.1 + dy) as i32 * LEVEL_HEIGHT);
                            explosions.push(self.death_anim(&list, point));
                        }
                    }
                    self.events.push(Event::BuildingDied { id, kind: building.kind, owner: building.owner, cell: building.cell, size: building.size, explosions });
                    self.building_survivors(&building);
                    self.crate_from_building(building.kind, (building.cell.0 + building.size.0 / 2, building.cell.1 + building.size.1 / 2));
                }
                Aim::Cell(..) => {}
            }
        }
    }

    /// Lets go of the spot or spots a dead soldier held.
    fn release_spots(&mut self, unit: &Unit, _scatter: &mut Vec<(i32, i32)>) {
        let free = |world: &mut Self, x: i32, y: i32| {
            let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
            if let Some(index) = world.map.index(cell.0, cell.1) {
                world.spots[index] &= !(1 << spot_index(x, y));
            }
        };
        match unit.head_to {
            Some((x, y)) => free(self, x, y),
            None => free(self, unit.x, unit.y),
        }
    }
}

/// Whether a throw at a speed can reach a point so far away and so much higher.
// The usual ballistic discriminant; a distance of
// nothing counts as 0.001; it fails only if no root, or both roots negative.
fn ballistic_can_reach(speed: f64, x: f64, z: f64, gravity: f64) -> bool {
    let x = if x == 0.0 { 0.001 } else { x };
    let v2 = speed * speed;
    let discriminant = v2 * v2 - 2.0 * v2 * z * gravity - gravity * gravity * x * x;
    if discriminant < 0.0 {
        return false;
    }
    let denominator = 2.0 * (1.0 + z * z / (x * x));
    let root = discriminant.sqrt();
    (v2 - z * gravity + root) / denominator >= 0.0 || (v2 - z * gravity - root) / denominator >= 0.0
}
