//! Weapons, warheads and projectiles from rules.ini and art.ini, as the simulation
//! wants them and as the game shows and sounds them.
//!
//! DATA: the keys. Which
//! keys there are, their defaults and how each value is read

use ra2_formats::ini::Ini;
use ra2_sim::combat::{ARMOURS, Fighter, Mount, Projectile, Warhead, Weapon};

/// What a weapon shows and sounds like.
#[derive(Clone, Default)]
pub struct WeaponLook {
    pub name: String,
    /// `Report`: sounds, one of which plays as it fires.
    pub report: Vec<String>,
    /// `Anim`: muzzle flashes, one or one per direction.
    pub anims: Vec<String>,
    /// The projectile's `Image`, if it can be seen.
    pub image: Option<String>,
    pub is_laser: bool,
    pub laser_inner: [u8; 3],
    pub laser_outer: [u8; 3],
    pub laser_duration: u32,
    pub house_colour: bool,
    pub electric_bolt: bool,
    pub alternate_colour: bool,
    /// `IsRadBeam`: the beam's colour, `ChronoBeamColor` for a `Temporal`
    /// warhead, else `RadColor`.
    pub rad_beam: Option<[u8; 3]>,
}

#[derive(Clone, Default)]
pub struct WarheadLook {
    /// `AnimList`: explosions, by how much damage was carried.
    pub anims: Vec<String>,
}

#[derive(Default, bevy::prelude::Resource, Clone)]
pub struct Armoury {
    pub weapons: Vec<Weapon>,
    pub warheads: Vec<Warhead>,
    pub projectiles: Vec<Projectile>,
    pub weapon_looks: Vec<WeaponLook>,
    pub warhead_looks: Vec<WarheadLook>,
    warhead_names: Vec<String>,
    projectile_names: Vec<String>,
    /// `[CombatDamage] SplashList`.
    pub splashes: Vec<String>,
    /// What a soldier's death shows by the warhead's `InfDeath` from 3 to 7,
    /// and the bodies left by the other two.
    // DATA: `[AudioVisual]` `InfantryExplode`, `FlamingInfantry`,
    // `InfantryHeadPop`, `InfantryNuked`, `DeadBodies`; the fifth kind is the
    // second entry of `[Animations]`.
    pub infantry_deaths: [Option<String>; 5],
    pub dead_bodies: Vec<String>,
    /// By type name in lower case: `Explosion` animations and `DieSound` sounds.
    pub explosions: std::collections::HashMap<String, Vec<String>>,
    pub die_sounds: std::collections::HashMap<String, Vec<String>>,
    /// By type name in lower case: `CrushSound`.
    pub crush_sounds: std::collections::HashMap<String, String>,
    /// By type name in lower case: what it throws as it dies, its
    /// `DeathAnims`, and a soldier's own `DeadBodies`.
    pub leavings: std::collections::HashMap<String, Leavings>,
    pub death_anims: std::collections::HashMap<String, Vec<String>>,
    pub own_bodies: std::collections::HashMap<String, Vec<String>>,
    /// `[AudioVisual]` `MetallicDebris` and `Wake`.
    pub metallic_debris: Vec<String>,
    pub wake: Option<String>,
    /// `[VoxelAnims]`: the models that can be thrown as debris.
    pub voxel_debris: Vec<VoxelDebrisInfo>,
}

/// What a type throws as it dies, by name.
// DATA: `MinDebris`, `MaxDebris` (raised to `MinDebris`), `DebrisTypes`,
// `DebrisMaximums`, `DebrisAnims`. Keys are matched exactly, so the many
// `Maxdebris=` lines in rules.ini are never seen, nor is `DebrisAnim=`, which
// no type reads.
#[derive(Clone, Default)]
pub struct Leavings {
    pub min: i32,
    pub max: i32,
    pub types: Vec<String>,
    pub maximums: Vec<i32>,
    pub anims: Vec<String>,
}

/// A `[VoxelAnims]` entry.
// DATA: `Image`, `Elasticity`, `MinAngularVelocity`, `MaxAngularVelocity`,
// `Duration`, `MinZVel`, `MaxZVel`, `MaxXYVel`, `BounceAnim`, `ExpireAnim`,
// `TrailerAnim`, `Damage`, `DamageRadius`, `Warhead`.
#[derive(Clone, Default)]
pub struct VoxelDebrisInfo {
    pub name: String,
    pub image: String,
    pub motion: ra2_sim::remains::Bouncer,
    pub expire: Option<String>,
    pub bounce: Option<String>,
    pub trailer: Option<String>,
}

fn yes(value: Option<&str>, default: bool) -> bool {
    value.map_or(default, |v| matches!(v.trim().to_lowercase().as_str(), "yes" | "true" | "1"))
}

fn list(value: Option<&str>) -> Vec<String> {
    value.map(|v| v.split(',').map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).collect()).unwrap_or_default()
}

/// A distance in cells as the engine stores it: leptons, truncated.
fn leptons(value: Option<&str>) -> Option<i32> {
    value.and_then(|v| v.trim().parse::<f64>().ok()).map(|cells| (cells * 256.0) as i32)
}

fn triple(value: Option<&str>) -> Option<(i32, i32, i32)> {
    let mut numbers = value?.split(',').filter_map(|n| n.trim().parse::<i32>().ok());
    Some((numbers.next()?, numbers.next()?, numbers.next()?))
}

impl Armoury {
    pub fn load(rules: &Ini) -> Self {
        let mut armoury = Self {
            splashes: list(rules.get("CombatDamage", "SplashList")),
            infantry_deaths: {
                let name = |key: &str| ["AudioVisual", "General", "CombatDamage"].iter().find_map(|section| rules.get(section, key)).map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).filter(|v| !v.is_empty());
                [name("InfantryExplode"), name("FlamingInfantry"), rules.section("Animations").get(1).map(|(_, v)| v.trim().to_owned()), name("InfantryHeadPop"), name("InfantryNuked")]
            },
            dead_bodies: list(["AudioVisual", "General"].iter().find_map(|section| rules.get(section, "DeadBodies"))),
            metallic_debris: list(["AudioVisual", "General"].iter().find_map(|section| rules.get(section, "MetallicDebris"))),
            wake: ["AudioVisual", "General"].iter().find_map(|section| rules.get(section, "Wake")).map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).filter(|v| !v.is_empty()),
            ..Self::default()
        };
        for (_, name) in rules.section("VoxelAnims") {
            let name = name.split(';').next().unwrap_or("").trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let get = |key: &str| rules.get_exact(&name, key).map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).filter(|v| !v.is_empty());
            let real = |key: &str, default: f64| get(key).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
            let whole = |key: &str, default: i32| get(key).and_then(|v| v.parse::<i32>().ok()).unwrap_or(default);
            let warhead = get("Warhead").map(|w| armoury.warhead(rules, &w));
            armoury.voxel_debris.push(VoxelDebrisInfo {
                image: get("Image").unwrap_or_else(|| name.clone()).to_lowercase(),
                motion: ra2_sim::remains::Bouncer {
                    elasticity: real("Elasticity", 0.8),
                    max_xy: real("MaxXYVel", 15.0),
                    min_z: real("MinZVel", 3.5),
                    max_z: real("MaxZVel", 3.5),
                    damage: whole("Damage", 0),
                    radius: whole("DamageRadius", 0),
                    warhead,
                    duration: Some(whole("Duration", 0)),
                    spin: (real("MinAngularVelocity", 0.0), real("MaxAngularVelocity", 0.0)),
                    ..ra2_sim::remains::Bouncer::anim()
                },
                expire: get("ExpireAnim"),
                bounce: get("BounceAnim"),
                trailer: get("TrailerAnim"),
                name,
            });
        }
        for section in ["InfantryTypes", "VehicleTypes", "AircraftTypes", "BuildingTypes"] {
            for (_, kind) in rules.section(section) {
                let name = kind.trim().to_lowercase();
                armoury.explosions.insert(name.clone(), list(rules.get(kind.trim(), "Explosion")));
                if let Some(sound) = rules.get(kind.trim(), "CrushSound").map(|v| v.split(';').next().unwrap_or("").trim().to_owned()).filter(|v| !v.is_empty()) {
                    armoury.crush_sounds.insert(name.clone(), sound);
                }
                let exact = |key: &str| rules.get_exact(kind.trim(), key).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
                let min = exact("MinDebris").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0).max(0);
                armoury.leavings.insert(
                    name.clone(),
                    Leavings {
                        min,
                        max: exact("MaxDebris").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0).max(min),
                        types: list(exact("DebrisTypes").as_deref()),
                        maximums: list(exact("DebrisMaximums").as_deref()).iter().map(|v| v.parse().unwrap_or(0)).collect(),
                        anims: list(exact("DebrisAnims").as_deref()),
                    },
                );
                armoury.death_anims.insert(name.clone(), list(exact("DeathAnims").as_deref()));
                armoury.own_bodies.insert(name.clone(), list(exact("DeadBodies").as_deref()));
                armoury.die_sounds.insert(name, list(rules.get(kind.trim(), "DieSound")));
                for key in ["Primary", "Secondary", "ElitePrimary", "EliteSecondary"] {
                    if let Some(name) = rules.get(kind.trim(), key) {
                        armoury.weapon(rules, name.trim());
                    }
                }
                let count = rules.get(kind.trim(), "WeaponCount").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                for index in 1..=count {
                    if let Some(name) = rules.get(kind.trim(), &format!("Weapon{index}")) {
                        armoury.weapon(rules, name.trim());
                    }
                }
            }
        }
        armoury
    }

    /// The index of a weapon by name, reading it in if it is new.
    /// The flight rules of the V3's rocket and the dreadnought's missile.
    // DATA: `[General]` `V3Rocket...` and `DMisl...` keys, and `[CombatDamage]`
    // `V3Warhead` and `DMislWarhead`.
    pub fn rocket_rules(&mut self, rules: &Ini, unit: &dyn Fn(&str) -> Option<u16>) -> [ra2_sim::spawns::RocketRules; 2] {
        ["V3Rocket", "DMisl"].map(|prefix| {
            let get = |key: &str| rules.get("General", &format!("{prefix}{key}")).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
            let number = |key: &str, default: f64| get(key).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
            let warhead = rules
                .get("CombatDamage", if prefix == "V3Rocket" { "V3Warhead" } else { "DMislWarhead" })
                .map(|v| v.split(';').next().unwrap_or("").trim().to_owned())
                .map_or(0, |name| self.warhead(rules, &name));
            ra2_sim::spawns::RocketRules {
                kind: get("Type").and_then(|name| unit(&name)),
                pause_frames: number("PauseFrames", 0.0) as u32,
                tilt_frames: number("TiltFrames", 60.0) as u32,
                pitch_initial: number("PitchInitial", 0.0) as f32,
                pitch_final: number("PitchFinal", 0.5) as f32,
                turn_rate: number("TurnRate", 0.05) as f32,
                acceleration: number("Acceleration", 0.4),
                altitude: number("Altitude", 768.0) as i32,
                damage: number("Damage", 200.0) as i32,
                body_length: number("BodyLength", 256.0) as i32,
                lazy_curve: get("LazyCurve").is_some_and(|v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "1")),
                warhead,
            }
        })
    }

    /// The rules of Ivan's bombs.
    // DATA: `[CombatDamage]` (or `[General]`) `IvanDamage`, `IvanWarhead`,
    // `IvanTimedDelay`.
    pub fn ability_rules(&mut self, rules: &Ini, unit: &dyn Fn(&str) -> Option<u16>, building: &dyn Fn(&str) -> Option<u16>) -> ra2_sim::abilities::Rules {
        let get = |key: &str| ["CombatDamage", "General", "AudioVisual"].iter().find_map(|section| rules.get(section, key)).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
        let defaults = ra2_sim::abilities::Rules::default();
        let death_weapon = rules.get("CombatDamage", "DeathWeapon").and_then(|name| self.weapon(rules, name.split(';').next()?.trim()));
        ra2_sim::abilities::Rules {
            death_weapon,
            // DATA: `[General]` `AlliedDisguise`, `SovietDisguise`,
            // `SpyPowerBlackout`, `SpyMoneyStealPercent`; `[AI]` `BuildTech`.
            disguises: ["AlliedDisguise", "SovietDisguise"].map(|key| get(key).and_then(|name| unit(&name)).map(|index| index + 1)),
            // DATA: `[General]` `DefaultMirageDisguises`, `InfantryBlinkDisguiseTime`.
            mirage_looks: get("DefaultMirageDisguises").map_or(0, |list| list.split(',').filter(|name| !name.trim().is_empty()).count() as u16),
            c4_warhead: get("C4Warhead").map(|name| self.warhead(rules, &name)),
            infantry_blink: get("InfantryBlinkDisguiseTime").and_then(|v| v.parse().ok()).unwrap_or(defaults.infantry_blink),
            spy_blackout: get("SpyPowerBlackout").and_then(|v| v.parse().ok()).unwrap_or(defaults.spy_blackout),
            spy_steal: get("SpyMoneyStealPercent").and_then(|v| v.parse().ok()).unwrap_or(defaults.spy_steal),
            prism_type: get("PrismType").and_then(|name| building(&name)),
            prism_max: get("PrismSupportMax").and_then(|v| v.parse().ok()).unwrap_or(defaults.prism_max),
            prism_modifier: get("PrismSupportModifier").and_then(|v| v.trim_end_matches('%').trim().parse::<f64>().ok()).map_or(defaults.prism_modifier, |percent| percent / 100.0),
            prism_delay: get("PrismSupportDelay").and_then(|v| v.parse().ok()).unwrap_or(defaults.prism_delay),
            build_tech: rules.get("AI", "BuildTech").map(|list| list.split(';').next().unwrap_or("").split(',').filter_map(|name| building(name.trim())).collect()).unwrap_or_default(),
            ivan_damage: get("IvanDamage").and_then(|v| v.parse().ok()).unwrap_or(defaults.ivan_damage),
            ivan_warhead: get("IvanWarhead").map_or(0, |name| self.warhead(rules, &name)),
            ivan_delay: get("IvanTimedDelay").and_then(|v| v.parse().ok()).unwrap_or(defaults.ivan_delay),
        }
    }

    fn weapon(&mut self, rules: &Ini, name: &str) -> Option<u16> {
        if name.is_empty() || name.eq_ignore_ascii_case("none") {
            return None;
        }
        if let Some(index) = self.weapon_looks.iter().position(|look| look.name.eq_ignore_ascii_case(name)) {
            return Some(index as u16);
        }
        let get = |key: &str| rules.get(name, key);
        let number = |key: &str| get(key).and_then(|v| v.trim().parse::<i32>().ok());
        let projectile_name = get("Projectile").map(str::trim).unwrap_or("");
        let warhead_name = get("Warhead").map(str::trim).unwrap_or("");
        let projectile = self.projectile(rules, projectile_name);
        let warhead = self.warhead(rules, warhead_name);
        let colour = |key: &str| triple(get(key)).map_or([0; 3], |(r, g, b)| [r as u8, g as u8, b as u8]);
        let defaults = Weapon::default();
        self.weapons.push(Weapon {
            damage: number("Damage").unwrap_or(0),
            rof: number("ROF").unwrap_or(0),
            range: leptons(get("Range")).unwrap_or(0),
            minimum_range: leptons(get("MinimumRange")).unwrap_or(0),
            // `Speed` 0 to 100 becomes 0 to 255.
            speed: number("Speed").map_or(0, |v| (v.clamp(0, 100) * 256 / 100).min(255)),
            burst: number("Burst").unwrap_or(1),
            projectile,
            warhead,
            omni_fire: yes(get("OmniFire"), false),
            cell_rangefinding: yes(get("CellRangefinding"), false),
            lobber: yes(get("Lobber"), false),
            bright: yes(get("Bright"), false),
            fire_once: yes(get("FireOnce"), false),
            limbo_launch: yes(get("LimboLaunch"), false),
            spawner: yes(get("Spawner"), false),
            fake_blink: number("DisguiseFakeBlinkTime").unwrap_or(0).max(0) as u32,
            area_fire: yes(get("AreaFire"), false),
            is_sonic: yes(get("IsSonic"), false),
            sabotage_cursor: yes(get("SabotageCursor"), false),
            never_use: yes(get("NeverUse"), false),
            ambient_damage: get("AmbientDamage").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0),
            rad_level: get("RadLevel").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0),
            suicide: yes(get("Suicide"), false),
            ..defaults
        });
        self.weapon_looks.push(WeaponLook {
            name: name.to_owned(),
            report: list(get("Report")),
            anims: list(get("Anim")),
            image: (!yes(rules.get(projectile_name, "Inviso"), false))
                .then(|| rules.get(projectile_name, "Image").map(|image| image.trim().to_lowercase()))
                .flatten()
                .filter(|image| !image.eq_ignore_ascii_case("none")),
            is_laser: yes(get("IsLaser"), false),
            laser_inner: colour("LaserInnerColor"),
            laser_outer: colour("LaserOuterColor"),
            laser_duration: number("LaserDuration").unwrap_or(10).max(1) as u32,
            house_colour: yes(get("IsHouseColor"), false),
            electric_bolt: yes(get("IsElectricBolt"), false),
            alternate_colour: yes(get("IsAlternateColor"), false),
            rad_beam: yes(get("IsRadBeam"), false).then(|| {
                let temporal = self.warheads.get(warhead as usize).is_some_and(|w| w.temporal);
                let (section, key) = if temporal { ("AudioVisual", "ChronoBeamColor") } else { ("Radiation", "RadColor") };
                triple(rules.get(section, key).map(|v| v.split(';').next().unwrap_or(""))).map_or(if temporal { [128, 200, 255] } else { [0, 255, 0] }, |(r, g, b)| [r as u8, g as u8, b as u8])
            }),
        });
        Some(self.weapons.len() as u16 - 1)
    }

    fn warhead(&mut self, rules: &Ini, name: &str) -> u16 {
        if let Some(index) = self.warhead_names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            return index as u16;
        }
        let get = |key: &str| rules.get(name, key);
        let mut warhead = Warhead::default();
        // Eleven shares in armour order; one with a
        // per cent sign is a whole number of hundredths, otherwise a fraction.
        if let Some(verses) = get("Verses") {
            for (slot, token) in warhead.verses.iter_mut().zip(verses.split(',')) {
                let token = token.trim();
                *slot = if token.contains('%') {
                    token.trim_end_matches('%').trim().parse::<i32>().unwrap_or(0) as f64 * 0.01
                } else {
                    token.parse::<f64>().unwrap_or(0.0)
                };
            }
        }
        let float = |key: &str| get(key).and_then(|v| v.trim().parse::<f32>().ok());
        warhead.cell_spread = float("CellSpread").unwrap_or(0.0);
        warhead.percent_at_max = float("PercentAtMax").unwrap_or(1.0);
        warhead.prone_damage = get("ProneDamage")
            .and_then(|v| {
                let v = v.trim();
                if v.contains('%') { v.trim_end_matches('%').parse::<f64>().ok().map(|p| p * 0.01) } else { v.parse::<f64>().ok() }
            })
            .unwrap_or(1.0);
        warhead.inf_death = get("InfDeath").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        warhead.wall = yes(get("Wall"), false);
        warhead.wood = yes(get("Wood"), false);
        warhead.conventional = yes(get("Conventional"), false);
        warhead.rocker = yes(get("Rocker"), false);
        warhead.temporal = yes(get("Temporal"), false);
        warhead.ivan_bomb = yes(get("IvanBomb"), false);
        warhead.bomb_disarm = yes(get("BombDisarm"), false);
        warhead.mind_control = yes(get("MindControl"), false);
        warhead.parasite = yes(get("Parasite"), false);
        warhead.makes_disguise = yes(get("MakesDisguise"), false);
        warhead.electric_assault = yes(get("ElectricAssault"), false);
        warhead.psychic_damage = yes(get("PsychicDamage"), false);
        let anims = list(get("AnimList"));
        warhead.anims = anims.len();
        self.warheads.push(warhead);
        self.warhead_looks.push(WarheadLook { anims });
        self.warhead_names.push(name.to_owned());
        self.warheads.len() as u16 - 1
    }

    fn projectile(&mut self, rules: &Ini, name: &str) -> u16 {
        if let Some(index) = self.projectile_names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            return index as u16;
        }
        let get = |key: &str| rules.get(name, key);
        let number = |key: &str| get(key).and_then(|v| v.trim().parse::<i32>().ok());
        let defaults = Projectile::default();
        self.projectiles.push(Projectile {
            rot: number("ROT").unwrap_or(0),
            arcing: yes(get("Arcing"), false),
            inviso: yes(get("Inviso"), false),
            aa: yes(get("AA"), false),
            ag: yes(get("AG"), true),
            arm: number("Arm").unwrap_or(0),
            acceleration: number("Acceleration").unwrap_or(3),
            floater: yes(get("Floater"), false),
            inaccurate: yes(get("Inaccurate"), false),
            ranged: yes(get("Ranged"), false),
            degenerates: yes(get("Degenerates"), false),
            level: yes(get("Level"), false),
            very_high: yes(get("VeryHigh"), false),
            course_lock: number("CourseLockDuration").unwrap_or(0),
            subject_to_elevation: yes(get("SubjectToElevation"), false),
            flak_scatter: yes(get("FlakScatter"), false),
            vertical: yes(get("Vertical"), false),
            detonation_altitude: get("DetonationAltitude").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0),
            ..defaults
        });
        self.projectile_names.push(name.to_owned());
        let index = self.projectiles.len() - 1;
        // DATA: `ShrapnelWeapon` and `ShrapnelCount` (the Prism Tank's forking
        // beams). Looked up once the projectile is listed, since weapons name
        // projectiles in turn.
        let count = number("ShrapnelCount").unwrap_or(0);
        if let Some(fragment) = get("ShrapnelWeapon").map(|v| v.trim().to_owned()).filter(|v| !v.is_empty() && count != 0) {
            let weapon = self.weapon(rules, &fragment);
            self.projectiles[index].shrapnel = weapon.map(|weapon| (weapon, count));
        }
        index as u16
    }

    /// Every animation named by a weapon, warhead or dying thing.
    pub fn animations(&self) -> Vec<String> {
        self.weapon_looks
            .iter()
            .flat_map(|look| look.anims.iter())
            .chain(self.warhead_looks.iter().flat_map(|look| look.anims.iter()))
            .chain(self.splashes.iter())
            .chain(self.explosions.values().flatten())
            .chain(self.infantry_deaths.iter().flatten())
            .chain(self.dead_bodies.iter())
            .chain(self.own_bodies.values().flatten())
            .chain(self.leavings.values().flat_map(|l| l.anims.iter()))
            .chain(self.death_anims.values().flatten())
            .chain(self.metallic_debris.iter())
            .chain(self.wake.iter())
            .chain(self.voxel_debris.iter().flat_map(|v| v.expire.iter().chain(v.bounce.iter()).chain(v.trailer.iter())))
            .cloned()
            .chain(["WARPOUT".to_owned(), "WARPAWAY".to_owned()])
            .collect()
    }

    /// A gunner vehicle's weapons (`Weapon1` onward) with the turret each uses.
    // The weapons by
    // number, their mounts from art.ini `Weapon<n>FLH`, and the turret of each from
    // the thirteen `<X>TurretIndex` / `<X>TurretWeapon` pairs.
    pub fn gunner_weapons(&mut self, rules: &Ini, art: Option<&Ini>, name: &str, image: &str) -> Vec<(Mount, u8)> {
        const MODES: [&str; 13] = [
            "Normal", "Repair", "MachineGun", "Flak", "Pistol", "Sniper", "Shock", "Explode", "BrainBlast", "RadCannon", "Chrono",
            "TerroristExplode", "Cow",
        ];
        let number = |key: &str| rules.get(name, key).and_then(|v| v.split(';').next()?.trim().parse::<i32>().ok());
        let count = number("WeaponCount").unwrap_or(0).max(0) as usize;
        let mut turrets = vec![0u8; count];
        for (order, mode) in MODES.iter().enumerate() {
            let default = if order < 4 { order as i32 } else { 0 };
            let index = number(&format!("{mode}TurretIndex")).unwrap_or(default);
            if let Some(slot) = number(&format!("{mode}TurretWeapon")).and_then(|weapon| turrets.get_mut(weapon as usize)) {
                *slot = index.max(0) as u8;
            }
        }
        (1..=count)
            .filter_map(|index| {
                let weapon = self.weapon(rules, rules.get(name, &format!("Weapon{index}"))?.split(';').next()?.trim())?;
                let flh = triple(art.and_then(|art| art.get(image, &format!("Weapon{index}FLH")))).unwrap_or((0, 0, 0));
                Some((Mount { weapon, flh }, turrets[index - 1]))
            })
            .collect()
    }

    /// A gunner vehicle's weapons when elite: `EliteWeapon<n>`, each falling
    /// back to the plain one.
    // The elite
    // list beside the plain one, from `EliteWeapon<n>FLH` or the plain mount.
    pub fn gunner_elite(&mut self, rules: &Ini, art: Option<&Ini>, name: &str, image: &str) -> Vec<Mount> {
        let count = rules.get(name, "WeaponCount").and_then(|v| v.split(';').next()?.trim().parse::<usize>().ok()).unwrap_or(0);
        (1..=count)
            .filter_map(|index| {
                let plain = rules.get(name, &format!("Weapon{index}"))?;
                let named = rules.get(name, &format!("EliteWeapon{index}")).unwrap_or(plain);
                let weapon = self.weapon(rules, named.split(';').next()?.trim()).or_else(|| self.weapon(rules, plain.split(';').next()?.trim()))?;
                let flh = triple(art.and_then(|art| art.get(image, &format!("EliteWeapon{index}FLH"))))
                    .or_else(|| triple(art.and_then(|art| art.get(image, &format!("Weapon{index}FLH")))))
                    .unwrap_or((0, 0, 0));
                Some(Mount { weapon, flh })
            })
            .collect()
    }

    /// An elite's weapons: `ElitePrimary` and `EliteSecondary`.
    pub fn elite(&mut self, rules: &Ini, art: Option<&Ini>, name: &str, image: &str) -> (Option<Mount>, Option<Mount>) {
        let mut mount = |armoury: &mut Self, key: &str, flh: [&str; 2]| -> Option<Mount> {
            let weapon = armoury.weapon(rules, rules.get(name, key)?.trim())?;
            let point = flh.iter().find_map(|key| triple(art.and_then(|art| art.get(image, key))));
            Some(Mount { weapon, flh: point.unwrap_or((0, 0, 0)) })
        };
        (mount(self, "ElitePrimary", ["ElitePrimaryFireFLH", "PrimaryFireFLH"]), mount(self, "EliteSecondary", ["EliteSecondaryFireFLH", "SecondaryFireFLH"]))
    }

    /// A type's `DeathWeapon`.
    /// The `[Radiation]` rules.
    pub fn radiation(&mut self, rules: &Ini) -> ra2_sim::radiation::Rules {
        let get = |key: &str| rules.get("Radiation", key).map(|v| v.split(';').next().unwrap_or("").trim().to_owned());
        let defaults = ra2_sim::radiation::Rules::default();
        ra2_sim::radiation::Rules {
            duration_multiple: get("RadDurationMultiple").and_then(|v| v.parse().ok()).unwrap_or(defaults.duration_multiple),
            application_delay: get("RadApplicationDelay").and_then(|v| v.parse().ok()).unwrap_or(defaults.application_delay),
            level_max: get("RadLevelMax").and_then(|v| v.parse().ok()).unwrap_or(defaults.level_max),
            level_delay: get("RadLevelDelay").and_then(|v| v.parse().ok()).unwrap_or(defaults.level_delay),
            light_delay: get("RadLightDelay").and_then(|v| v.parse().ok()).unwrap_or(defaults.light_delay),
            level_factor: get("RadLevelFactor").and_then(|v| v.parse().ok()).unwrap_or(defaults.level_factor),
            warhead: self.warhead(rules, &get("RadSiteWarhead").unwrap_or_else(|| "RadSite".to_owned())),
        }
    }

    /// A weapon or a warhead by its rules.ini name.
    pub fn weapon_named(&mut self, rules: &Ini, name: &str) -> Option<u16> {
        self.weapon(rules, name)
    }

    pub fn warhead_named(&mut self, rules: &Ini, name: &str) -> u16 {
        self.warhead(rules, name)
    }

    pub fn death_weapon(&mut self, rules: &Ini, name: &str) -> Option<u16> {
        self.weapon(rules, rules.get(name, "DeathWeapon")?.split(';').next()?.trim())
    }

    /// The numbers of experience.
    pub fn veterancy(rules: &Ini) -> ra2_sim::abilities::Veterancy {
        let number = |key: &str, default: f64| rules.get("General", key).and_then(|v| v.split(';').next()?.trim().parse::<f64>().ok()).unwrap_or(default);
        let defaults = ra2_sim::abilities::Veterancy::default();
        ra2_sim::abilities::Veterancy {
            ratio: number("VeteranRatio", defaults.ratio),
            combat: number("VeteranCombat", defaults.combat),
            speed: number("VeteranSpeed", defaults.speed),
            armor: number("VeteranArmor", defaults.armor),
            rof: number("VeteranROF", defaults.rof),
            cap: number("VeteranCap", defaults.cap),
            heal_every: ((number("RepairRate", 0.016) * 900.0) as u64).max(1),
        }
    }

    /// How a type fights: its rules section, its art section, and for a foot
    /// soldier its sequence section.
    pub fn fighter(&mut self, rules: &Ini, art: Option<&Ini>, name: &str, image: &str, sequence: Option<&str>) -> Fighter {
        let get = |key: &str| rules.get(name, key);
        let art_get = |key: &str| art.and_then(|art| art.get(image, key));
        let defaults = Fighter::default();
        let is_building = rules.section("BuildingTypes").iter().any(|(_, entry)| entry.trim().eq_ignore_ascii_case(name));
        let mut mount = |armoury: &mut Self, key: &str, flh: &str| -> Option<Mount> {
            let weapon = armoury.weapon(rules, get(key)?.trim())?;
            Some(Mount { weapon, flh: triple(art_get(flh)).unwrap_or((0, 0, 0)) })
        };
        // A type with `TurretCount` above 0
        // (the Prism Tank, the IFV) takes its weapons from `Weapon1`, `Weapon2`...
        // and their art keys; others from `Primary` and `Secondary`.
        let many = get("TurretCount").and_then(|v| v.split(';').next()?.trim().parse::<i32>().ok()).unwrap_or(0) > 0;
        let (primary, secondary) = if many {
            (mount(self, "Weapon1", "Weapon1FLH"), mount(self, "Weapon2", "Weapon2FLH"))
        } else {
            (mount(self, "Primary", "PrimaryFireFLH"), mount(self, "Secondary", "SecondaryFireFLH"))
        };
        let armour = get("Armor").and_then(|a| ARMOURS.iter().position(|n| n.eq_ignore_ascii_case(a.trim()))).unwrap_or(0) as u8;
        // DATA: a sequence entry is "first frame, frames, frames per facing".
        let frames = |action: &str| {
            sequence
                .and_then(|section| art?.get(section, action))
                .and_then(|entry| entry.split(',').nth(1)?.trim().parse::<u16>().ok())
                .unwrap_or(0)
        };
        let fire_frames = frames("FireUp");
        let flag = |key: &str, default: bool| yes(get(key), default);
        Fighter {
            armour,
            primary,
            secondary,
            turret: yes(get("Turret"), false),
            turret_offset: art_get("TurretOffset").or(get("TurretOffset")).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            mobile_fire: yes(get("MobileFire"), defaults.mobile_fire),
            opportunity_fire: yes(get("OpportunityFire"), false),
            can_passive_acquire: yes(get("CanPassiveAquire"), true),
            default_guard_area: yes(get("DefaultToGuardArea"), false),
            can_retaliate: yes(get("CanRetaliate"), true),
            can_approach_target: yes(get("CanApproachTarget"), true),
            guard_range: leptons(get("GuardRange")).unwrap_or(0),
            sight: get("Sight").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            legal_target: yes(get("LegalTarget"), true),
            insignificant: yes(get("Insignificant"), false),
            immune: yes(get("Immune"), false),
            fire_up: art_get("FireUp").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            fire_frames,
            fire_prone: art_get("FireProne").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            fire_prone_frames: frames("FireProne"),
            down_frames: frames("Down"),
            up_frames: frames("Up"),
            fearless: flag("Fearless", false),
            fraidycat: flag("Fraidycat", false),
            // DATA: on unless the type says no.
            crawls: flag("Crawls", true),
            engineer: yes(get("Engineer"), false),
            occupier: yes(get("Occupier"), false),
            c4: yes(get("C4"), false),
            aircraft: rules.section("AircraftTypes").iter().any(|(_, entry)| entry.trim().eq_ignore_ascii_case(name)),
            can_disguise: yes(get("CanDisguise"), false),
            perma_disguise: yes(get("PermaDisguise"), false),
            disguise_when_still: yes(get("DisguiseWhenStill"), false),
            ivan: yes(get("Ivan"), false),
            agent: yes(get("Agent"), false),
            detect_disguise: yes(get("DetectDisguise"), false),
            // On for every
            // building unless it says otherwise, off for the rest.
            immune_to_psionics: yes(get("ImmuneToPsionics"), is_building),
            immune_to_radiation: yes(get("ImmuneToRadiation"), false),
            parasiteable: yes(get("Parasiteable"), !is_building),
            organic: yes(get("Organic"), false),
            threat_posed: get("ThreatPosed").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(0),
            capturable: yes(get("Capturable"), false),
            warpable: yes(get("Warpable"), true),
            deployer: yes(get("Deployer"), false),
            deploy_fire: yes(get("DeployFire"), false),
            undeploy_delay: get("UndeployDelay").and_then(|v| v.split(';').next()?.trim().parse().ok()).unwrap_or(-1),
            deploy_frames: frames("Deploy"),
            undeploy_frames: frames("Undeploy"),
        }
    }
}
