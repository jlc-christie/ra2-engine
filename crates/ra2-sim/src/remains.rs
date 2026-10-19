//! What is left when something dies: the debris it throws, the people who get
//! out, and ships that go down instead of blowing up.

use crate::combat::{Aim, Event};
use crate::path::code;
use crate::smudge::NO_ANIM;
use crate::{entry_code, Land, Other, Unit, UnitStats, World, LEPTONS_PER_CELL};

/// How a thrown piece flies, bounces and ends.
// DATA: art.ini for an animation, `[VoxelAnims]` entries in rules.ini for a
// model: `Elasticity`, `MaxXYVel`, `MinZVel`, `MaxZVel`, `Damage`,
// `DamageRadius`, `Warhead`, `ExpireAnim`, `BounceAnim`, `TrailerAnim`,
// `TrailerSeperation`, and for a model `Duration`.
#[derive(Clone, Debug, Default)]
pub struct Bouncer {
    pub elasticity: f64,
    pub max_xy: f64,
    pub min_z: f64,
    pub max_z: f64,
    pub damage: i32,
    pub radius: i32,
    pub warhead: Option<u16>,
    /// Animations by number.
    pub expire: u16,
    pub bounce: u16,
    pub trailer: u16,
    pub trailer_every: i32,
    /// Frames a model flies before it ends; none for an animation, which ends
    /// where it first comes down.
    pub duration: Option<i32>,
    pub spin: (f64, f64),
}

impl Bouncer {
    /// An animation type's defaults: `MaxZVel` is not read for animations and
    /// keeps its 3.5.
    pub fn anim() -> Self {
        Self { elasticity: 0.8, max_xy: 15.0, min_z: 3.5, max_z: 3.5, expire: NO_ANIM, bounce: NO_ANIM, trailer: NO_ANIM, ..Self::default() }
    }
}

/// A `[VoxelAnims]` entry thrown as debris.
#[derive(Clone, Debug, Default)]
pub struct VoxelDebris {
    /// The model's name.
    pub image: String,
    pub motion: Bouncer,
}

/// What a dying thing throws: `MinDebris`, `MaxDebris`, `DebrisTypes` (by
/// number in the rules' voxel debris), `DebrisMaximums` and `DebrisAnims`.
#[derive(Clone, Debug, Default)]
pub struct Leavings {
    pub min: i32,
    pub max: i32,
    pub types: Vec<u16>,
    pub maximums: Vec<i32>,
    pub anims: Vec<u16>,
}

/// The rules of what is left.
#[derive(Clone, Debug)]
pub struct Rules {
    /// `CrewEscape`: the chance a vehicle's crew gets out.
    pub crew_escape: f64,
    /// `ShipSinkingWeight`.
    pub ship_sinking_weight: f64,
    /// `Wake`, `MetallicDebris`, `InfantryExplode`.
    pub wake: u16,
    pub metallic_debris: Vec<u16>,
    pub infantry_explode: u16,
    /// By animation number: how each one thrown flies, if it is a `Bouncer`.
    pub bouncers: Vec<Option<Bouncer>>,
    pub voxels: Vec<VoxelDebris>,
}

impl Default for Rules {
    fn default() -> Self {
        Self { crew_escape: 0.5, ship_sinking_weight: 3.0, wake: NO_ANIM, metallic_debris: Vec::new(), infantry_explode: NO_ANIM, bouncers: Vec::new(), voxels: Vec::new() }
    }
}

/// A piece in the air: an animation or a model, where it is, how it moves,
/// and for a model the frames it has left.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Debris {
    pub id: u32,
    pub anim: u16,
    pub voxel: Option<u16>,
    pub position: (f64, f64, f64),
    pub velocity: (f64, f64, f64),
    pub spin: f64,
    pub left: i32,
    pub born: u64,
}

/// A ship going down.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Sinking {
    pub id: u32,
    pub kind: u16,
    pub owner: u8,
    pub x: i32,
    pub y: i32,
    pub facing: u8,
    /// How far under the water it is, in leptons.
    pub depth: i32,
}

/// The pieces and wrecks the world is showing.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Wreckage {
    pub debris: Vec<Debris>,
    pub sinking: Vec<Sinking>,
    /// Parasites put out of their hosts this tick, and where.
    released: Vec<(Unit, (i32, i32))>,
    next: u32,
}

/// The pull on a thrown piece each frame, and the height under which it has
/// stopped.
const BOUNCE_GRAVITY: f64 = 1.4;
const STOP_HEIGHT: f64 = 2.5;
/// A sinking ship goes down this far a frame, and is gone below this depth.
const SINK_STEP: i32 = 5;
const SUNK: i32 = 400;

enum Contact {
    Flying,
    Bounced,
    Stopped,
}

impl World {
    /// A number from `lo` to `hi`, both included, either way round.
    fn ranged(&mut self, lo: i32, hi: i32) -> i32 {
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        lo + self.random_below((hi - lo + 1) as u32) as i32
    }

    /// Whether a point counts as in the water for a death: low enough, off any
    /// deck, over a water cell.
    pub(crate) fn wet_death(&self, at: (i32, i32, i32), height: i32, on_bridge: bool) -> bool {
        let cell = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
        height <= 10 && !on_bridge && self.map.index(cell.0, cell.1).is_some_and(|i| self.map.land[i] == Land::Water as u8)
    }

    /// Throws a dying thing's debris from its centre.
    // A number from `MinDebris` to one less than `MaxDebris`. With
    // `DebrisTypes`, those models in turn, each up to its `DebrisMaximums`
    // entry at a time, until the number is used up; otherwise that many of its
    // `DebrisAnims`, or of `MetallicDebris` when it names none, each one picked
    // at random, thrown from 20 leptons up.
    pub(crate) fn throw_debris(&mut self, leavings: &crate::remains::Leavings, centre: (i32, i32, i32)) {
        if leavings.max <= 0 {
            return;
        }
        let mut count = self.ranged(leavings.min, leavings.max - 1);
        if !leavings.types.is_empty() {
            let mut at = 0;
            let mut rounds = 0;
            while count > 0 && rounds < 64 {
                let most = leavings.maximums.get(at).copied().unwrap_or(0);
                let some = (self.random_below(u32::MAX >> 1) as i32 % (most + 1).max(1)).min(count);
                for _ in 0..some {
                    self.throw_voxel(leavings.types[at], centre);
                }
                count -= some;
                at = (at + 1) % leavings.types.len();
                rounds += 1;
            }
            return;
        }
        let list = if leavings.anims.is_empty() { self.remains_rules.metallic_debris.clone() } else { leavings.anims.clone() };
        if list.is_empty() {
            return;
        }
        for _ in 0..count {
            let anim = list[self.ranged(0, list.len() as i32 - 1) as usize];
            self.throw_anim(anim, (centre.0, centre.1, centre.2 + 20));
        }
    }

    /// The speed a thrown piece leaves with: across, at random up to
    /// `MaxXYVel` each way, and up, from `MinZVel` by up to `MaxZVel` less
    /// `MinZVel`.
    fn throw_speed(&mut self, rule: &Bouncer) -> (f64, f64, f64) {
        let up = self.next_random() as i32;
        let side = self.next_random() as i32;
        let across = self.next_random() as i32;
        let width = (rule.max_xy * 2.0) as i32;
        let rise = (rule.max_z - rule.min_z + 1.0) as i32;
        let x = if width != 0 { (across % width) as f64 - rule.max_xy } else { -rule.max_xy };
        let y = if width != 0 { (side % width) as f64 - rule.max_xy } else { -rule.max_xy };
        let z = if rise != 0 { (up % rise) as f64 + rule.min_z } else { rule.min_z };
        (x, y, z)
    }

    /// Throws one animation; one that is not a `Bouncer` just plays there.
    fn throw_anim(&mut self, anim: u16, at: (i32, i32, i32)) {
        let Some(rule) = self.remains_rules.bouncers.get(anim as usize).cloned().flatten() else {
            self.events.push(Event::Detonated { warhead: 0, damage: 0, at, water: false, anim });
            return;
        };
        let velocity = self.throw_speed(&rule);
        let id = self.wreckage.next;
        self.wreckage.next = id.wrapping_add(1);
        let born = self.tick;
        self.wreckage.debris.push(Debris { id, anim, voxel: None, position: (at.0 as f64, at.1 as f64, (at.2 + 10) as f64), velocity, spin: 0.0, left: 0, born });
    }

    /// Throws one model.
    fn throw_voxel(&mut self, voxel: u16, at: (i32, i32, i32)) {
        let Some(rule) = self.remains_rules.voxels.get(voxel as usize).map(|v| v.motion.clone()) else {
            return;
        };
        let velocity = self.throw_speed(&rule);
        let turn = self.next_random() as i32;
        let range = (rule.spin.1 - rule.spin.0 + 1.0) as i32;
        let spin = if range != 0 { (turn % range) as f64 + rule.spin.0 } else { rule.spin.0 };
        let id = self.wreckage.next;
        self.wreckage.next = id.wrapping_add(1);
        let born = self.tick;
        let left = rule.duration.unwrap_or(0);
        self.wreckage.debris.push(Debris { id, anim: NO_ANIM, voxel: Some(voxel), position: (at.0 as f64, at.1 as f64, (at.2 + 10) as f64), velocity, spin, left, born });
    }

    /// One frame of a thrown piece's flight.
    // Gravity of 1.4, then the move. It
    // has come down when it is below the ground, or less than 150 over it in a
    // cell with a building; within 100 under the ground it is put back on it,
    // and every part of its speed is multiplied by `Elasticity`, the upward
    // part turned about. Less than 2.5 over the ground it has stopped, which
    // it always is once it has come down: nothing ever bounces.
    fn bounce(&self, piece: &mut Debris, elasticity: f64) -> Contact {
        piece.velocity.2 -= BOUNCE_GRAVITY;
        piece.position = (piece.position.0 + piece.velocity.0, piece.position.1 + piece.velocity.1, piece.position.2 + piece.velocity.2);
        let (x, y) = (piece.position.0 as i32, piece.position.1 as i32);
        let ground = self.ground_z(x, y) as f64;
        let cell = (x.div_euclid(LEPTONS_PER_CELL), y.div_euclid(LEPTONS_PER_CELL));
        let roof = piece.position.2 >= ground && piece.position.2 - 150.0 < ground && self.building_on(cell).is_some_and(|b| self.buildings[b].health > 0);
        let mut contact = Contact::Flying;
        if piece.position.2 < ground || roof {
            if piece.position.2 > ground - 100.0 {
                piece.position.2 = ground;
            }
            piece.velocity = (piece.velocity.0 * elasticity, piece.velocity.1 * elasticity, -piece.velocity.2 * elasticity);
            contact = Contact::Bounced;
        }
        if piece.position.2 - ground < STOP_HEIGHT {
            Contact::Stopped
        } else {
            contact
        }
    }

    /// Thrown pieces fly and come to an end.
    // A piece ends where it comes down, a model also
    // when its `Duration` runs out. Were one to bounce, the `BounceAnim` would
    // play and whatever stands in the cell within `DamageRadius` (along each
    // axis added) would take `Damage` by the `Warhead`.
    pub(crate) fn run_debris(&mut self) {
        for (biter, cell) in std::mem::take(&mut self.wreckage.released) {
            self.release_parasite(biter, cell);
        }
        let mut flying = Vec::new();
        for mut piece in std::mem::take(&mut self.wreckage.debris) {
            let rule = match piece.voxel {
                Some(voxel) => self.remains_rules.voxels.get(voxel as usize).map(|v| v.motion.clone()),
                None => self.remains_rules.bouncers.get(piece.anim as usize).cloned().flatten(),
            };
            let Some(rule) = rule else {
                continue;
            };
            if piece.voxel.is_some() {
                if piece.left != 0 {
                    piece.left -= 1;
                }
                if piece.left <= 0 {
                    self.debris_ends(&piece, &rule);
                    continue;
                }
            }
            let contact = self.bounce(&mut piece, rule.elasticity);
            let at = (piece.position.0 as i32, piece.position.1 as i32, piece.position.2 as i32);
            match contact {
                Contact::Flying => {}
                Contact::Bounced => {
                    let water = self.wet_death(at, 0, false);
                    if piece.voxel.is_some() && water {
                        piece.left = 0;
                    } else {
                        if rule.bounce != NO_ANIM {
                            self.events.push(Event::Detonated { warhead: 0, damage: 0, at, water: false, anim: rule.bounce });
                        }
                        self.bounce_damage(at, &rule);
                    }
                }
                Contact::Stopped => piece.left = 0,
            }
            if piece.voxel.is_none() && !matches!(contact, Contact::Flying) {
                self.debris_ends(&piece, &rule);
                continue;
            }
            flying.push(piece);
        }
        flying.extend(std::mem::take(&mut self.wreckage.debris));
        self.wreckage.debris = flying;
    }

    fn bounce_damage(&mut self, at: (i32, i32, i32), rule: &Bouncer) {
        let Some(warhead) = rule.warhead.filter(|_| rule.damage != 0) else {
            return;
        };
        let cell = (at.0.div_euclid(LEPTONS_PER_CELL), at.1.div_euclid(LEPTONS_PER_CELL));
        let hit: Vec<(Aim, i32)> = self
            .units
            .iter()
            .filter(|u| u.cell() == cell)
            .map(|u| (Aim::Unit(u.id), (u.x - at.0).abs() + (u.y - at.1).abs()))
            .chain(self.building_on(cell).map(|b| {
                let building = &self.buildings[b];
                let centre = (building.cell.0 * LEPTONS_PER_CELL + building.size.0 * 128, building.cell.1 * LEPTONS_PER_CELL + building.size.1 * 128);
                (Aim::Building(building.id), (centre.0 - at.0).abs() + (centre.1 - at.1).abs())
            }))
            .filter(|&(_, distance)| distance <= rule.radius)
            .collect();
        for (victim, distance) in hit {
            self.receive_damage(victim, rule.damage, distance, warhead, None);
        }
    }

    /// A thrown piece's end: on water a wake and the first splash; elsewhere
    /// its `ExpireAnim`, which also deals its `Damage` there as an explosion of
    /// its `Warhead`.
    fn debris_ends(&mut self, piece: &Debris, rule: &Bouncer) {
        let at = (piece.position.0 as i32, piece.position.1 as i32, piece.position.2 as i32);
        let ground = self.ground_z(at.0, at.1);
        if self.wet_death(at, at.2 - ground, false) {
            let wake = self.remains_rules.wake;
            let up = if piece.voxel.is_some() { 10 } else { 3 };
            self.events.push(Event::Detonated { warhead: 0, damage: 0, at, water: true, anim: wake });
            if let Some(&splash) = self.splash_anims.first() {
                self.events.push(Event::Detonated { warhead: 0, damage: 0, at: (at.0, at.1, at.2 + up), water: true, anim: splash });
            }
            return;
        }
        if rule.expire == NO_ANIM {
            return;
        }
        if self.marks_ground(at) {
            self.mark_of(rule.expire, (at.0, at.1));
        }
        self.events.push(Event::Detonated { warhead: rule.warhead.unwrap_or(0), damage: rule.damage, at, water: false, anim: rule.expire });
        if let Some(warhead) = rule.warhead {
            self.explode(at, rule.damage, warhead, None);
        }
    }

    /// A unit's centre, whether or not it is still on the list.
    pub(crate) fn aim_point_of(&self, unit: &Unit) -> (i32, i32, i32) {
        let deck = if unit.on_bridge { 4 * crate::combat::LEVEL_HEIGHT } else { 0 };
        (unit.x, unit.y, self.ground_z(unit.x, unit.y) + self.altitude(unit) + deck)
    }

    pub fn debris(&self) -> &[Debris] {
        &self.wreckage.debris
    }

    pub fn sinking(&self) -> &[Sinking] {
        &self.wreckage.sinking
    }

    /// Whether a dying vehicle goes down instead of blowing up: a surface ship
    /// (`Naval`, not `Underwater` or `Organic`) of at least
    /// `ShipSinkingWeight`, on water, not in the middle of a jump.
    pub(crate) fn sinks(&self, unit: &Unit) -> bool {
        let Some(kind) = (unit.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)) else {
            return false;
        };
        let cell = unit.cell();
        kind.naval
            && !kind.underwater
            && !unit.stats.fighter.organic
            && kind.weight >= self.remains_rules.ship_sinking_weight
            && self.map.index(cell.0, cell.1).is_some_and(|i| self.map.land[i] == Land::Water as u8)
            && self.tick >= unit.warp_until
    }

    pub(crate) fn start_sinking(&mut self, unit: &Unit) {
        self.wreckage.sinking.push(Sinking { id: unit.id, kind: unit.stats.kind, owner: unit.owner, x: unit.x, y: unit.y, facing: unit.facing, depth: 0 });
    }

    /// Ships go down 5 leptons a frame and are gone once more than 400 under;
    /// every fourth frame a wake shows somewhere within 170 leptons of them.
    pub(crate) fn run_sinking(&mut self) {
        let tick = self.tick;
        let mut afloat = Vec::new();
        for mut ship in std::mem::take(&mut self.wreckage.sinking) {
            ship.depth += SINK_STEP;
            if ship.depth > SUNK {
                continue;
            }
            if tick.is_multiple_of(4) {
                let dy = self.ranged(-170, 170);
                let dx = self.ranged(-170, 170);
                let at = (ship.x + dx, ship.y + dy, self.ground_z(ship.x, ship.y));
                let wake = self.remains_rules.wake;
                self.events.push(Event::Detonated { warhead: 0, damage: 0, at, water: true, anim: wake });
            }
            afloat.push(ship);
        }
        self.wreckage.sinking = afloat;
    }

    /// The soldier who gets out of something: the side's crew (`AlliedCrew`,
    /// `SovietCrew`), a `Technician` for any other side, and for an armed
    /// thing a technician one time in seven or so (15 in 100).
    fn crew_kind(&mut self, owner: u8, armed: bool) -> Option<u16> {
        let side = self.houses.get(owner as usize).map_or(0, |h| h.side);
        let (allied, soviet, _, technician) = self.crew_types;
        let crew = match side {
            0 => allied,
            1 => soviet,
            _ => technician,
        };
        if armed && self.random_below(100) < 15 {
            return technician;
        }
        crew
    }

    /// Puts a soldier of a type down on a cell, if he can stand there.
    fn put_survivor(&mut self, kind: u16, owner: u8, cell: (i32, i32)) -> Option<u32> {
        let stats = self.unit_types.get(kind as usize).map(|t| UnitStats { kind: kind + 1, infantry: true, ..t.stats })?;
        let id = self.next_id;
        let mut unit = Unit::new(id, owner, cell, 0, stats);
        unit.facing = 0;
        let others: Vec<Other> = self.others();
        if entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, &unit, cell, None, cell, -1) != code::OK {
            return None;
        }
        let at = self.map.index(cell.0, cell.1)?;
        let spot = (2..=4usize).find(|spot| self.spots[at] & (1 << spot) == 0)?;
        unit.x = cell.0 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].0;
        unit.y = cell.1 * LEPTONS_PER_CELL + crate::SPOT_OFFSETS[spot].1;
        self.spots[at] |= 1 << spot;
        self.spot_house[at] = owner;
        self.next_id += 1;
        self.put_back(unit);
        Some(id)
    }

    /// A destroyed vehicle's crew: for a `Crewed` type that carries no
    /// passengers, a `CrewEscape` chance of one soldier on its spot, hurt to
    /// between 5 and half his strength, who scatters.
    pub(crate) fn vehicle_crew(&mut self, dead: &Unit) {
        let Some(kind) = (dead.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)) else {
            return;
        };
        if !kind.crewed || kind.passengers != 0 {
            return;
        }
        let chance = self.random_below(0x7fff_ffff) as f64 * 4.656_612_875e-10;
        if chance >= self.remains_rules.crew_escape {
            return;
        }
        let Some(crew) = self.crew_kind(dead.owner, dead.stats.fighter.primary.is_some()) else {
            return;
        };
        let Some(id) = self.put_survivor(crew, dead.owner, dead.cell()) else {
            return;
        };
        let strength = self.unit(id).map_or(1, |u| u.stats.strength);
        let health = self.ranged(5, strength / 2);
        if let Some(unit) = self.unit_at(id).map(|at| &mut self.units[at]) {
            unit.health = health.clamp(1, strength);
            unit.on_bridge = dead.on_bridge;
        }
        self.scatter_unit(id);
    }

    /// The crew who come out of a building as it is destroyed.
    // One for each `AlliedSurvivorDivisor` or `SovietSurvivorDivisor` (by the
    // side it was built for) credits of the refund, from one to five, the
    // divisor doubled for a captured building. Cell by cell of the foundation,
    // while some are left, one comes out at a chance of one in two if a charge
    // was planted in it, otherwise one in three (far less for a captured one),
    // hurt to between 5 and his full strength; he goes for the planter if that
    // is an enemy. Each clear cell is marked, scorched or cratered at even
    // odds.
    pub(crate) fn building_survivors(&mut self, building: &crate::Building) {
        let Some(kind) = self.building_types.get(building.kind as usize).cloned() else {
            return;
        };
        let defeated = self.houses.get(building.owner as usize).is_some_and(|h| h.defeated);
        let count = self.crew_count(building, &kind);
        if count == 0 {
            return;
        }
        let planter = building.c4.map(|(_, by)| by).filter(|&by| self.unit(by).is_some());
        let mut odds = if building.c4.is_some() { 1 } else { 2 };
        if building.captured {
            odds += 6;
        }
        let mut left = count;
        for dy in 0..building.size.1 {
            for dx in 0..building.size.0 {
                let cell = (building.cell.0 + dx, building.cell.1 + dy);
                if !defeated && left > 0 && self.ranged(0, odds) == 1
                    && let Some(crew) = self.building_crew_kind(building, &kind)
                        && let Some(id) = self.put_survivor(crew, building.owner, cell) {
                            left -= 1;
                            let strength = self.unit(id).map_or(1, |u| u.stats.strength);
                            let health = self.ranged(5, strength);
                            if let Some(unit) = self.unit_at(id).map(|at| &mut self.units[at]) {
                                unit.health = health.clamp(1, strength);
                            }
                            self.scatter_unit(id);
                            let enemy = planter.filter(|&by| self.unit(by).is_some_and(|u| !self.allied(building.owner, u.owner)));
                            if let (Some(by), Some(unit)) = (enemy, self.unit_at(id).map(|at| &mut self.units[at])) {
                                unit.arms.target = Some(Aim::Unit(by));
                                unit.arms.attacking = true;
                            }
                        }
                if self.map.passable(cell.0, cell.1) {
                    let burn = self.random_below(100) < 50;
                    let dx = self.ranged(-0x80, 0x80);
                    let dy = self.ranged(-0x80, 0x80);
                    let (x, y) = (cell.0 * LEPTONS_PER_CELL + 0x80 + dx, cell.1 * LEPTONS_PER_CELL + 0x80 + dy);
                    let centre = (x.div_euclid(LEPTONS_PER_CELL) * LEPTONS_PER_CELL + 0x80, y.div_euclid(LEPTONS_PER_CELL) * LEPTONS_PER_CELL + 0x80);
                    self.smudge(burn, centre, 100, 100, false);
                }
            }
        }
    }

    /// How many people a building holds.
    pub(crate) fn crew_count(&self, building: &crate::Building, kind: &crate::BuildingType) -> i32 {
        let side = self.houses.get(building.owner as usize).map_or(0, |house| house.side);
        let mut divisor = if side == 1 { self.survivor_divisors.1 } else { self.survivor_divisors.0 };
        if !kind.crewed || divisor == 0 {
            return 0;
        }
        if building.captured {
            divisor *= 2;
        }
        (self.refund(building.kind) / divisor).clamp(1, 5)
    }

    /// Who comes out of a building: from a construction yard not taken by an
    /// engineer, an `Engineer` one time in four; otherwise as from a vehicle.
    // The one in four is drawn for every building.
    pub(crate) fn building_crew_kind(&mut self, building: &crate::Building, kind: &crate::BuildingType) -> Option<u16> {
        if !building.captured && self.random_below(100) < 25 && kind.construction_yard {
            return self.crew_types.2;
        }
        self.crew_kind(building.owner, kind.fighter.primary.is_some())
    }

    /// How a soldier's death shows: a death sequence (1, 2), one of the
    /// special deaths (3 to 7), or an animation chosen here.
    // On water, a wake and the first splash. A `JumpJet`
    // soldier, or one coming down under a parachute, blows up
    // (`InfantryExplode`). Otherwise the warhead's `InfDeath`: with
    // `DeathAnims` the one by that number (the first if it names none there);
    // a `NotHuman` soldier always plays his first death sequence.
    pub(crate) fn infantry_death(&mut self, unit: &Unit, death: i32, at: (i32, i32, i32), wet: bool) -> (i32, u16) {
        if wet {
            let wake = self.remains_rules.wake;
            self.events.push(Event::Detonated { warhead: 0, damage: 0, at, water: true, anim: wake });
            if let Some(&splash) = self.splash_anims.first() {
                self.events.push(Event::Detonated { warhead: 0, damage: 0, at: (at.0, at.1, at.2 + 3), water: true, anim: splash });
            }
            return (0, NO_ANIM);
        }
        if unit.stats.jet.is_some() {
            return (0, self.remains_rules.infantry_explode);
        }
        let death = if unit.falling.is_some() { 3 } else { death };
        let kind = (unit.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k));
        if let Some(anims) = kind.map(|k| &k.death_anims).filter(|list| !list.is_empty()) {
            let at = if (0..anims.len() as i32).contains(&death) && anims[death as usize] != NO_ANIM { death as usize } else { 0 };
            return (0, anims[at]);
        }
        if kind.is_some_and(|k| k.not_human) {
            return (1, NO_ANIM);
        }
        (death, NO_ANIM)
    }

    /// A hit on something with a parasite in it, before armour: by anybody
    /// but the parasite and above its `SuppressionThreshold`, the parasite
    /// would not survive leaving for twice the damage less the threshold in
    /// frames; mending (negative damage) throws it out with no chance.
    pub(crate) fn parasite_suppression(&mut self, host: u32, damage: i32, source: Option<Aim>) {
        let Some(index) = self.unit_at(host) else {
            return;
        };
        let Some(biter) = self.units[index].parasite.first() else {
            return;
        };
        let threshold = (biter.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).map_or(0, |k| k.suppression_threshold);
        let own = source == Some(Aim::Unit(biter.id));
        let tick = self.tick;
        if !own && damage > threshold {
            self.units[index].parasite[0].suppressed_until = tick + (2 * damage - threshold).max(0) as u64;
        }
        if damage < 0 {
            self.units[index].parasite[0].suppressed_until = tick + 50;
            self.parasite_exit(index);
        }
    }

    /// A parasite is put out of the thing it was inside, which lives on: by
    /// the host's cell, or the nearest place it can stand; it is lost if it
    /// was recently hurt by somebody else's damage to the host, if the host is
    /// in the air, or (for one that does not swim) the host is on water or
    /// beach, or a vehicle parasite's host is on a building.
    // After that it stays put for three times its weapon's `ROF`.
    // It is put down once every unit has had its turn.
    pub(crate) fn parasite_exit(&mut self, host: usize) {
        if let Some(left) = self.take_parasite(host) {
            self.wreckage.released.push(left);
        }
    }

    /// Takes a parasite out of its host, with the cell it is to be put down
    /// on; none if it does not survive leaving.
    fn take_parasite(&mut self, host: usize) -> Option<(Unit, (i32, i32))> {
        let mut biter = self.units[host].parasite.pop()?;
        let naval = (biter.stats.kind as usize).checked_sub(1).and_then(|k| self.unit_types.get(k)).is_some_and(|k| k.naval);
        if !naval && self.tick < biter.suppressed_until {
            return None;
        }
        let cell = self.units[host].cell();
        let aloft = self.altitude(&self.units[host]) > 0;
        let land = self.map.index(cell.0, cell.1).map(|i| self.map.land[i]);
        let mut ok = !aloft;
        if ok && !naval {
            if !biter.stats.infantry && self.building_on(cell).is_some() {
                ok = false;
            }
            if land.is_some_and(|land| land == Land::Water as u8 || land == Land::Beach as u8 || land == Land::Rock as u8) {
                ok = false;
            }
        }
        if !ok {
            return None;
        }
        let weapon = biter.bite.map(|(weapon, _)| weapon);
        let rest = weapon.and_then(|w| self.weapons.get(w as usize)).map_or(0, |w| 3 * w.rof.max(0) as u64);
        biter.bite = None;
        biter.arms.target = None;
        biter.arms.attacking = false;
        biter.dest = None;
        biter.head_to = None;
        biter.drop_path();
        biter.paralysed_until = self.tick + rest;
        (biter.x, biter.y) = (self.units[host].x, self.units[host].y);
        biter.on_bridge = self.units[host].on_bridge;
        Some((biter, cell))
    }

    /// Puts a parasite that has left its host down on a cell, or the nearest
    /// one it can stand on; it is lost if there is none.
    fn release_parasite(&mut self, biter: Unit, cell: (i32, i32)) {
        let others: Vec<Other> = self.others();
        let open = entry_code(&self.map, &self.claims, (&self.spots, &self.spot_house), &others, &self.buildings, &biter, cell, None, cell, -1) == code::OK;
        let spot = if open { Some(cell) } else { crate::nearby(&self.map, &self.claims, &self.spots, &biter, cell, crate::Near::new(false, true, Some(cell)), self.tick) };
        if let Some(spot) = spot {
            self.set_down(biter, spot);
        }
    }
}
