//! What a click would do: the engine's "what action" answer for one selected thing
//! and whatever the mouse is over, from which both the cursor and the order come.
//!
//! The checks behave as the original's do; the step numbers in the comments
//! count them. Only the branches for things this game has so far are
//! here.
//!

use ra2_sim::combat::{Aim, Entry, Stance};
use ra2_sim::World;

use crate::control::Target;
use crate::cursor;
use crate::terrain::local;

/// The engine's action numbers (those that can come up here).
// The numbers, and the cursor each
// shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None = 0,
    Move = 1,
    NoMove = 2,
    Enter = 3,
    /// On itself: deploy, unload, make primary, come out.
    Deploy = 4,
    Attack = 5,
    Harvest = 6,
    Select = 7,
    ToggleSelect = 8,
    /// Go into a building: capture it, or garrison it.
    Capture = 9,
    /// Plant a charge in a building.
    Sabotage = 0x10,
    /// An attack with a `SabotageCursor` weapon: an attack under another cursor.
    SabotageFire = 0x40,
    /// Ivan: stick a bomb on it, or the refusal when it has one.
    IvanBomb = 0x35,
    NoIvanBomb = 0x36,
    /// An engineer taking a bomb off.
    Disarm = 0x39,
    GuardArea = 0x1a,
    /// An engineer mending a building.
    EngineerRepair = 0x1d,
    NoDeploy = 0x1e,
    NoEnter = 0x1f,
    NoEngineerRepair = 0x20,
}

/// The modifier keys as the engine reads them: Ctrl forces fire, Alt forces a
/// move, Shift selects.
#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    /// Shift and Ctrl together
    /// cancel each other.
    fn settled(self) -> Self {
        if self.shift && self.ctrl { Self { ctrl: false, shift: false, ..self } } else { self }
    }
}

/// The cursor an action shows.
// Attack shows the in-range cursor only when exactly
// one thing is selected and its target is in range.
pub fn cursor_of(action: Action, in_range_alone: bool) -> usize {
    match action {
        Action::None => cursor::NORMAL,
        Action::Move => cursor::MOVE,
        Action::NoMove => cursor::NO_MOVE,
        Action::Enter | Action::Capture => cursor::ENTER,
        Action::Deploy => cursor::DEPLOY,
        Action::Attack if in_range_alone => cursor::ATTACK,
        Action::Attack | Action::Harvest => cursor::HARVEST,
        Action::Select | Action::ToggleSelect => cursor::SELECT,
        Action::Sabotage | Action::SabotageFire => cursor::SABOTAGE,
        Action::IvanBomb => cursor::IVAN_BOMB,
        Action::NoIvanBomb => cursor::NO_MOVE,
        Action::Disarm => cursor::DISARM,
        Action::GuardArea => cursor::GUARD_AREA,
        Action::EngineerRepair => cursor::ENGINEER_REPAIR,
        Action::NoDeploy => cursor::NO_DEPLOY,
        Action::NoEnter => cursor::NO_ENTER,
        Action::NoEngineerRepair => cursor::NO_REPAIR,
    }
}

fn aim_of(target: Target) -> Aim {
    match target {
        Target::Unit(id) => Aim::Unit(id),
        Target::Building(id) => Aim::Building(id),
    }
}

fn owner_of(world: &World, target: Target) -> Option<u8> {
    match target {
        Target::Unit(id) => world.unit(id).map(|unit| unit.owner),
        Target::Building(id) => world.building(id).map(|b| b.owner),
    }
}

/// Which of the selection answers for the cursor.
// The highest rank wins: an armed thing that is not
// a building and does damage, then an armed thing that is not a building, then an
// armed building, then anything else; equals go to the one nearest the point, and
// exact ties to the earlier in the list.
pub fn best(world: &World, selection: &[Target], at: (i32, i32)) -> Option<Target> {
    let rank = |target: Target| -> (i32, i64) {
        match target {
            Target::Unit(id) => world.unit(id).map_or((0, 0), |unit| {
                let armed = unit.stats.fighter.primary.is_some() || unit.gun.is_some();
                let (dx, dy) = ((unit.x - at.0) as i64, (unit.y - at.1) as i64);
                (if armed { 5 } else { 2 }, ((dx * dx + dy * dy) as f64).sqrt() as i64)
            }),
            Target::Building(id) => world.building(id).map_or((0, 0), |b| {
                let armed = world.building_types.get(b.kind as usize).is_some_and(|kind| kind.fighter.primary.is_some());
                let (dx, dy) = ((b.cell.0 * 256 + b.size.0 * 128 - at.0) as i64, (b.cell.1 * 256 + b.size.1 * 128 - at.1) as i64);
                (if armed { 3 } else { 2 }, ((dx * dx + dy * dy) as f64).sqrt() as i64)
            }),
        }
    };
    let mut chosen: Option<(Target, (i32, i64))> = None;
    for &target in selection {
        let score = rank(target);
        if chosen.is_none_or(|(_, held)| score.0 > held.0 || (score.0 == held.0 && score.1 < held.1)) {
            chosen = Some((target, score));
        }
    }
    chosen.map(|(target, _)| target)
}

/// Whether an attack by a unit on something is in range now.
pub fn in_range(world: &World, me: Target, target: Aim, mods: Mods) -> bool {
    match me {
        Target::Unit(id) => world.attack_action(id, target, mods.settled().ctrl) == Some(true),
        // A building is only ever offered an attack that is in range.
        Target::Building(id) => world.building_attack_action(id, target, mods.settled().ctrl),
    }
}

/// The answer of one selected thing for an object under the mouse. `alone` says
/// that it is the only thing selected, `selected` that the object is itself in the
/// selection.
pub fn for_object(world: &World, me: Target, object: Target, alone: bool, selected: bool, mods: Mods) -> Action {
    let mods = mods.settled();
    if owner_of(world, me) != Some(local()) {
        return Action::None;
    }
    let base = techno_object(world, me, object, alone, selected, mods);
    if base == Action::ToggleSelect {
        return base;
    }
    match me {
        Target::Unit(id) => match world.unit(id) {
            Some(unit) if unit.stats.infantry => infantry_object(world, unit, object, base, mods),
            Some(unit) => vehicle_object(world, unit, object, base),
            None => Action::None,
        },
        Target::Building(id) => building_object(world, id, base),
    }
}

/// The answer every kind starts from.
fn techno_object(world: &World, me: Target, object: Target, alone: bool, selected: bool, mods: Mods) -> Action {
    // Step 4.
    if mods.shift {
        return Action::ToggleSelect;
    }
    // Step 5.
    if object == me && alone {
        return Action::Deploy;
    }
    let mine = owner_of(world, me);
    let theirs = owner_of(world, object);
    let allied = mine == theirs;
    let movable = match me {
        Target::Unit(_) => true,
        Target::Building(id) => world.has_rally_point(id) || world.can_undeploy(id),
    };
    // Steps 6 and 7.
    if allied && mods.ctrl && mods.alt && movable && matches!(me, Target::Unit(_)) {
        return Action::GuardArea;
    }
    if mods.alt && movable {
        return Action::Move;
    }
    // Step 8: the attack test. Something that cannot move needs the target in
    // range.
    match me {
        Target::Unit(id) => {
            if world.attack_action(id, aim_of(object), mods.ctrl).is_some() {
                return Action::Attack;
            }
        }
        Target::Building(id) => {
            if object != me && world.building_attack_action(id, aim_of(object), mods.ctrl) {
                return Action::Attack;
            }
        }
    }
    // Step 9.
    let armed = match me {
        Target::Unit(id) => world.unit(id).is_some_and(|unit| unit.stats.fighter.primary.is_some() || unit.gun.is_some()),
        Target::Building(id) => world
            .buildings()
            .iter()
            .find(|b| b.id == id)
            .and_then(|b| world.building_types.get(b.kind as usize))
            .is_some_and(|kind| kind.fighter.primary.is_some()),
    };
    if armed && theirs != mine {
        return Action::None;
    }
    // Not original behaviour: only the player's own things can be selected yet.
    if theirs == Some(local()) && !selected {
        return Action::Select;
    }
    Action::None
}

/// What a foot soldier adds.
fn infantry_object(world: &World, unit: &ra2_sim::Unit, object: Target, base: Action, mods: Mods) -> Action {
    let fighter = unit.stats.fighter;
    let mut a = base;
    // Step 1: an engineer over something with a bomb on it takes it off.
    if fighter.engineer && world.bomb_on(aim_of(object)).is_some() {
        return Action::Disarm;
    }
    if let Target::Building(building) = object {
        // Step 2: an engineer's building rules come before everything, whatever
        // the base answer and whether or not Ctrl is held.
        if fighter.engineer {
            match world.entry_action(unit.id, building) {
                Some(Entry::Repair | Entry::RepairBridge) => return Action::EngineerRepair,
                Some(Entry::NothingToRepair) => return Action::NoEngineerRepair,
                Some(Entry::Capture) => return Action::Capture,
                _ => {}
            }
        }
        // Steps 7 and 8: a hospital for the hurt, an armory for those not yet
        // elite. (Tested here, ahead of the occupier's rule, because the
        // simulation answers both through one question.)
        if !mods.ctrl && world.entry_action(unit.id, building) == Some(Entry::Treat) {
            return Action::Enter;
        }
        // Step 3: a building it can go into and fire from, unless forcing fire.
        if !mods.ctrl && world.entry_action(unit.id, building) == Some(Entry::Occupy) {
            return Action::Capture;
        }
    }
    // An `Agent` over an enemy's `Spyable` building
    // goes in.
    if fighter.agent && !mods.ctrl
        && let Target::Building(building) = object
            && world.entry_action(unit.id, building) == Some(Entry::Infiltrate) {
                return Action::Capture;
            }
    // Step 4: an engineer never shows the attack cursor.
    if fighter.engineer && a == Action::Attack {
        return Action::NoMove;
    }
    // Step 9: a `C4` soldier's attack on a building that can be wired is a
    // walk-in.
    // The engine's attack test
    // asks only that the soldier be armed, not that its weapon can touch the
    // target; ours asks the weapon's fire test, so an armed C4 soldier whose
    // weapon cannot fire on a building (the Psi Commando's mind control) is
    // let through here.
    let armed = fighter.primary.is_some() || fighter.secondary.is_some();
    let attack = a == Action::Attack || (armed && matches!(a, Action::NoMove | Action::None));
    if attack && fighter.c4
        && let Target::Building(building) = object
            && world.entry_action(unit.id, building) == Some(Entry::Sabotage) {
                return Action::Sabotage;
            }
    // An attack with a weapon
    // that has `SabotageCursor` (the chrono commando's and the SEAL's charge)
    // shows the charge-planting cursor and is an attack all the same.
    if a == Action::Attack && world.weapon_against(unit.id, aim_of(object)).is_some_and(|weapon| weapon.sabotage_cursor) {
        return Action::SabotageFire;
    }
    // Step 11: Ivan's attack is a bomb, or a refusal where there is one already.
    if a == Action::Attack && fighter.ivan {
        return if world.bomb_on(aim_of(object)).is_none() { Action::IvanBomb } else { Action::NoIvanBomb };
    }
    // Step 12: dug in.
    if fighter.deployer && unit.stance != Stance::Standing {
        match a {
            Action::Move => return Action::NoMove,
            Action::Attack
                if (!fighter.deploy_fire || world.attack_action(unit.id, aim_of(object), mods.ctrl) != Some(true)) => {
                    return Action::NoMove;
                }
            _ => {}
        }
    }
    // Step 13: on itself.
    if a == Action::Deploy && !fighter.deployer {
        a = Action::None;
    }
    // Step 14: a friendly transport.
    if a != Action::NoEnter
        && let Some(entered) = transport(world, unit, object, a) {
            a = entered;
        }
    finish(world, a, object)
}

/// What a vehicle adds.
fn vehicle_object(world: &World, unit: &ra2_sim::Unit, object: Target, base: Action) -> Action {
    let mut a = base;
    // Step 2: on itself.
    if a == Action::Deploy {
        let passengers = world.unit_types.get((unit.stats.kind as usize).wrapping_sub(1)).map_or(0, |kind| kind.passengers);
        a = if unit.stats.deploys_into.is_some() {
            if world.can_deploy(unit.id) { Action::Deploy } else { Action::NoDeploy }
        } else if passengers <= 0 {
            Action::None
        } else if unit.cargo.is_empty() || world.altitude(unit) != 0 {
            Action::NoDeploy
        } else {
            Action::Deploy
        };
    }
    // Step 3: a friendly building that will take it.
    if let Target::Building(id) = object {
        let takes = world.building(id).is_some_and(|b| {
            b.owner == unit.owner
                && unit.stats.harvester
                && b.docked.is_none()
                && b.buildup.is_none()
                && world.building_types.get(b.kind as usize).is_some_and(|kind| kind.refinery)
        });
        if takes {
            a = Action::Enter;
        }
    }
    // An aircraft over one of its side's pad
    // buildings with room for it.
    if let (true, Target::Building(id)) = (unit.plane.is_some(), object)
        && world.can_dock(unit.id, id) {
            return Action::Enter;
        }
    // Step 5.
    if let Some(entered) = transport(world, unit, object, a) {
        return entered;
    }
    finish(world, a, object)
}

/// A friendly transport's answer to something that might get in.
// Asked unless the answer
// so far is to attack or guard; a moving transport, or a soldier that hovers,
// gets the refusal.
fn transport(world: &World, unit: &ra2_sim::Unit, object: Target, a: Action) -> Option<Action> {
    let Target::Unit(other) = object else {
        return None;
    };
    if matches!(a, Action::Attack | Action::GuardArea) || other == unit.id {
        return None;
    }
    let carrier = world.unit(other).filter(|carrier| carrier.owner == unit.owner)?;
    let passengers = world.unit_types.get((carrier.stats.kind as usize).wrapping_sub(1)).map_or(0, |kind| kind.passengers);
    if passengers <= 0 {
        return None;
    }
    if unit.stats.jet.is_some_and(|jet| jet.balloon_hover) || carrier.is_moving() {
        return Some(Action::NoEnter);
    }
    Some(if world.can_board(unit.id, other) { Action::Enter } else { Action::NoEnter })
}

/// No answer becomes
/// "cannot move there"; an attack on something `Immune` likewise.
fn finish(world: &World, a: Action, object: Target) -> Action {
    match a {
        Action::None => Action::NoMove,
        Action::Attack => {
            let immune = match object {
                Target::Unit(id) => world.unit(id).is_some_and(|unit| unit.stats.fighter.immune),
                Target::Building(id) => world
                    .buildings()
                    .iter()
                    .find(|b| b.id == id)
                    .and_then(|b| world.building_types.get(b.kind as usize))
                    .is_some_and(|kind| kind.fighter.immune),
            };
            if immune { Action::NoMove } else { Action::Attack }
        }
        other => other,
    }
}

/// What a building adds. On itself: one with
/// soldiers in it lets them out; a factory that is not the primary one becomes it;
/// anything else has no answer.
fn building_object(world: &World, id: u32, base: Action) -> Action {
    let Some(building) = world.building(id) else {
        return Action::None;
    };
    let factory = world.building_types.get(building.kind as usize).is_some_and(|kind| kind.factory.is_some());
    match base {
        Action::Deploy if !building.occupants.is_empty() && !factory => Action::Deploy,
        Action::Deploy if !factory => Action::None,
        Action::Deploy if building.primary => Action::None,
        // Alt on an object with a factory selected
        // would set the rally point there; without Alt it selects.
        Action::Move | Action::NoMove if !world.has_rally_point(id) && !world.can_undeploy(id) => Action::Select,
        other => other,
    }
}

/// The answer of one selected thing for a cell with nothing on it to point at.
// Where the engine's own notes disagree: one reading has the hover always show
// "move" for a unit and only the click test the ground; the cursor table and the
// game as played show "cannot move" over ground it cannot enter, which is kept.
pub fn for_cell(world: &World, me: Target, cell: (i32, i32), mods: Mods) -> Action {
    let mods = mods.settled();
    if owner_of(world, me) != Some(local()) {
        return Action::None;
    }
    match me {
        Target::Unit(id) => {
            let Some(unit) = world.unit(id) else {
                return Action::None;
            };
            let fighter = unit.stats.fighter;
            let dug_in = unit.stats.infantry && fighter.deployer && unit.stance != Stance::Standing;
            // Ctrl and Alt together: guard the area.
            if mods.ctrl && mods.alt && (fighter.primary.is_some() || fighter.engineer) {
                return Action::GuardArea;
            }
            // Ctrl: fire on the ground itself.
            if mods.ctrl
                && let Some(in_range) = world.attack_action(id, Aim::Cell(cell.0, cell.1), true) {
                    if dug_in && !(fighter.deploy_fire && in_range) {
                        return Action::NoMove;
                    }
                    return Action::Attack;
                }
            if !world.map.in_playfield(cell.0, cell.1) {
                return Action::NoMove;
            }
            // Over the shroud, "move" for a type
            // with `MoveToShroud`, else "cannot move".
            if world.map.shrouded(cell.0, cell.1) {
                return if dug_in { Action::NoMove } else { Action::Move };
            }
            let a = if mods.shift || unit.plane.is_some() || world.can_move_to(id, cell) { Action::Move } else { Action::NoMove };
            if dug_in && a == Action::Move {
                return Action::NoMove;
            }
            // A miner over ore it can reach digs.
            if a == Action::Move && unit.stats.harvester && world.map.ore_at(cell.0, cell.1).is_some() {
                return Action::Harvest;
            }
            a
        }
        Target::Building(id) => {
            // An armed building fires on the ground
            // only with Ctrl, and in range.
            if mods.ctrl && world.building_attack_action(id, Aim::Cell(cell.0, cell.1), true) {
                return Action::Attack;
            }
            // Factories, cloning vats and repair depots answer "move" for any
            // cell: the click sets the rally point. A building that packs up
            // answers by the ground.
            if world.has_rally_point(id) {
                Action::Move
            } else if world.can_undeploy(id) {
                if world.map.speed_percent(cell.0, cell.1, ra2_sim::SpeedType::Track) > 0 { Action::Move } else { Action::NoMove }
            } else {
                Action::None
            }
        }
    }
}
