//! Saving a game to a file and loading it again.
//!
//! Not original behaviour: the file format. The original writes a Windows
//! compound file of its objects; ours is
//! a header, the match's setup, and the simulation's snapshot
//! (`ra2_sim::World::snapshot`). The rules the player sees are meant to be the
//! original's.
//!
//! The lists of saves are shown by the pause menu (`pause.rs`).
//!

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::game::{GameSetup, MatchSetup};
use crate::shell::Screen;
use crate::units::Sim;

const MAGIC: &[u8; 8] = b"RA2RSAV1";
/// Bumped whenever the header or anything in it changes shape. The
/// snapshot's own number is folded in, so a save the game can no longer
/// load is left out of the lists.
const FORMAT: u32 = 6 << 16 | ra2_sim::snapshot::FORMAT;

/// Where saves are kept: never in the game's own folder (see `home`).
pub fn folder() -> PathBuf {
    std::env::var_os("RA2_SAVES").map_or_else(|| crate::home::data_dir().join("saves"), PathBuf::from)
}

#[derive(Serialize, Deserialize)]
pub struct SaveFile {
    format: u32,
    /// What the list shows.
    pub description: String,
    pub tick: u64,
    /// A digest of the rules and the map it was saved against.
    digest: u64,
    pub setup: MatchSetup,
    /// The house the player sat at, and where the view was.
    pub local: u8,
    pub camera: (f32, f32),
    /// The simulation.
    pub state: Vec<u8>,
    /// The game from its start, for playing it through again.
    pub recording: Option<Recording>,
}

/// A game from its start: the simulation as it began, and every order
/// carried out since, with the tick it was carried out at. The computer
/// players and all chance are inside the simulation, so these play the game
/// through again exactly (`replay`).
/// Not original behaviour: the original keeps no such record.
#[derive(Serialize, Deserialize, Clone, Default, Resource)]
pub struct Recording {
    pub start: Vec<u8>,
    pub start_tick: u64,
    pub orders: Vec<(u64, ra2_sim::Order)>,
}

/// Plays a saved game's recording through, headless, up to a tick (or as far
/// as the save if `to` is past it). For finding out how a game came to be as
/// it was saved.
pub fn replay(files: &crate::files::GameFiles, save: &SaveFile, to: u64) -> Result<ra2_sim::World, String> {
    let recording = save.recording.as_ref().ok_or("the save has no recording")?;
    let mut world = crate::game::match_world(files, &save.setup).ok_or("the map could not be set up")?;
    world.restore(&recording.start)?;
    let to = to.min(save.tick);
    let mut orders = recording.orders.iter().peekable();
    while world.current_tick() < to {
        let tick = world.current_tick();
        while let Some((_, order)) = orders.next_if(|(at, _)| *at <= tick) {
            world.order(*order);
        }
        world.tick();
    }
    Ok(world)
}

pub fn write(path: &Path, save: &SaveFile) -> Result<(), String> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(bincode::serialize(save).map_err(|error| error.to_string())?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

pub fn read(path: &Path) -> Result<SaveFile, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let body = bytes.strip_prefix(MAGIC).ok_or("not a saved game")?;
    let save: SaveFile = bincode::deserialize(body).map_err(|error| error.to_string())?;
    if save.format != FORMAT {
        return Err(format!("saved by another version (format {})", save.format));
    }
    Ok(save)
}

/// A save as the lists show it.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub description: String,
    pub modified: std::time::SystemTime,
}

/// The saves there are, newest first.
// Every save in the folder
// by the file's modified time, newest first; no fixed slots; one written by
// another version is not listed at all.
pub fn list() -> Vec<Entry> {
    use std::io::Read;
    let mut found: Vec<Entry> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("sav")))
        .filter_map(|entry| {
            // Only the front of the file is read: the format and what it is called.
            let mut reader = std::io::BufReader::new(std::fs::File::open(entry.path()).ok()?);
            let mut magic = [0u8; 8];
            reader.read_exact(&mut magic).ok()?;
            let (format, description): (u32, String) = bincode::deserialize_from(&mut reader).ok()?;
            (&magic == MAGIC && format == FORMAT).then(|| Entry { path: entry.path(), description, modified: entry.metadata().and_then(|data| data.modified()).unwrap_or(std::time::UNIX_EPOCH) })
        })
        .collect();
    found.sort_by(|a, b| b.modified.cmp(&a.modified));
    found
}

/// A date and time for the list.
pub fn stamp(time: std::time::SystemTime) -> String {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs()) as i64;
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days to a date in the civil calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (day, month) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}  {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

/// The rules and map a game is played against, boiled down to a number, so
/// that a save is not loaded against others.
fn digest(files: &crate::files::GameFiles, setup: &MatchSetup) -> u64 {
    let map = [format!("{}.map", setup.map), format!("{}.mpr", setup.map)];
    let names = ["rules.ini", "art.ini", "ai.ini", setup.mode_rules.as_str(), map[0].as_str(), map[1].as_str()];
    names.iter().filter_map(|name| files.get(&name.to_lowercase())).flatten().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x100_0000_01b3))
}

/// Which of the pause menu's lists is up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Load,
    Save,
    Delete,
}

/// What has been asked for and not yet done, and the list the pause menu shows.
#[derive(Resource, Default)]
pub struct SaveLoad {
    pub save: Option<PathBuf>,
    pub load: Option<PathBuf>,
    /// What the next save is to be called.
    pub description: String,
    /// The list: the saves, the row picked, what has been typed as a
    /// description, and whether it needs drawing again.
    pub entries: Vec<Entry>,
    pub picked: usize,
    pub text: String,
    pub dirty: bool,
    /// A save asked for from the list is being written.
    pub saving: bool,
    /// The row last clicked and when, to tell a double click.
    pub clicked: Option<(usize, f64)>,
    /// A question or notice over the list, waiting for its answer.
    pub ask: Option<Ask>,
}

/// What a box over a list of saves is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Saving needs a description.
    MustDescribe,
    /// Write over this save?
    Overwrite(PathBuf),
    /// Delete this save?
    Delete(PathBuf, String),
    /// The game was saved.
    Saved,
}

/// The name of a new save.
// `SAVE` and four hex
// digits at random, tried again until no such file exists.
pub fn new_name() -> PathBuf {
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |since| since.as_nanos() as u64);
    loop {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let path = folder().join(format!("SAVE{:04X}.SAV", (seed >> 33) & 0x7fff));
        if !path.exists() {
            return path;
        }
    }
}

/// What a save is called when nothing else is typed: the scenario's name.
pub fn default_description(setup: &MatchSetup) -> String {
    if setup.map_name.is_empty() { setup.map.clone() } else { setup.map_name.clone() }
}

/// A save read from disk, waiting for the game it belongs to to be set up.
#[derive(Resource)]
pub struct PendingLoad(pub SaveFile);

pub struct SavesPlugin;

impl Plugin for SavesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SaveLoad>()
            .add_systems(Startup, |mut asked: ResMut<SaveLoad>| {
                // RA2_LOAD=path starts by loading that save.
                asked.load = std::env::var_os("RA2_LOAD").map(PathBuf::from);
            })
            .add_systems(Update, (save.run_if(resource_exists::<Sim>), load, type_description.run_if(resource_exists::<Sim>)))
            // A game is loaded over a running one by leaving it and coming
            // straight back.
            .add_systems(OnEnter(Screen::Loading), |mut next: ResMut<NextState<Screen>>| next.set(Screen::InGame));
    }
}

fn save(
    mut asked: ResMut<SaveLoad>,
    sim: Res<Sim>,
    recording: Option<Res<Recording>>,
    setup: Option<Res<MatchSetup>>,
    camera: Single<&Transform, With<Camera2d>>,
    files: Res<crate::files::GameFiles>,
    mut status: MessageWriter<crate::bar::Status>,
    mut at_tick: Local<Option<(u64, PathBuf)>>,
) {
    // RA2_SAVE_AT=tick,path saves at a tick, for trying this without a mouse.
    if at_tick.is_none() {
        let spec = std::env::var("RA2_SAVE_AT").unwrap_or_default();
        *at_tick = Some(match spec.split_once(',') {
            Some((tick, path)) => (tick.trim().parse().unwrap_or(u64::MAX), PathBuf::from(path.trim())),
            None => (u64::MAX, PathBuf::new()),
        });
    }
    if let Some((tick, path)) = at_tick.as_mut().filter(|(tick, _)| sim.0.current_tick() >= *tick) {
        *tick = u64::MAX;
        asked.save = Some(path.clone());
    }
    let (Some(path), Some(setup)) = (asked.save.take(), setup) else {
        return;
    };
    let file = SaveFile {
        format: FORMAT,
        // With nothing
        // typed, the scenario's name.
        description: Some(std::mem::take(&mut asked.description)).filter(|text| !text.trim().is_empty()).unwrap_or_else(|| default_description(&setup)),
        tick: sim.0.current_tick(),
        digest: digest(&files, &setup),
        setup: setup.clone(),
        local: crate::terrain::local(),
        camera: (camera.translation.x, camera.translation.y),
        state: sim.0.snapshot(),
        // The orders since the game was set up (or loaded) follow those the
        // recording already held.
        recording: recording.map(|recording| Recording { orders: recording.orders.iter().chain(sim.2.iter()).copied().collect(), ..recording.clone() }),
    };
    // Afterwards the
    // player is told the game was saved (`TXT_GAME_WAS_SAVED`).
    match write(&path, &file) {
        Ok(()) => {
            info!("saved {} at tick {} ({} bytes of state)", path.display(), file.tick, file.state.len());
            status.write(crate::bar::Status::Key("TXT_GAME_WAS_SAVED"));
        }
        Err(error) => {
            error!("could not save {}: {error}", path.display());
            status.write(crate::bar::Status::Text(format!("Could not save: {error}")));
        }
    }
}

fn load(mut commands: Commands, mut asked: ResMut<SaveLoad>, mut setup: Option<ResMut<GameSetup>>, state: Res<State<Screen>>, mut next: ResMut<NextState<Screen>>, files: Res<crate::files::GameFiles>, mut status: MessageWriter<crate::bar::Status>, mut notices: MessageWriter<crate::notice::Notice>) {
    let Some(setup) = setup.as_mut() else {
        return;
    };
    let Some(path) = asked.load.take() else {
        return;
    };
    match read(&path) {
        // Saved against other rules or another map file: the numbers in it
        // would mean other things.
        Ok(file) if file.digest != digest(&files, &file.setup) => {
            error!("{} was saved with different game files", path.display());
            status.write(crate::bar::Status::Text("That game was saved with different game files".to_owned()));
            notices.write(crate::notice::Notice("That game was saved with different game files.".to_owned()));
        }
        Ok(file) => {
            info!("loading {} (tick {})", path.display(), file.tick);
            // The match is set up again exactly as it was, every choice that
            // was left to chance already settled; then the saved state is
            // laid over it (`game::load_map`).
            **setup = file.setup.as_game_setup();
            commands.insert_resource(PendingLoad(file));
            next.set(if *state.get() == Screen::InGame { Screen::Loading } else { Screen::InGame });
        }
        Err(error) => {
            error!("could not load {}: {error}", path.display());
            notices.write(crate::notice::Notice("That saved game could not be loaded.".to_owned()));
        }
    }
}

/// Typing while the Save list is up goes to the description.
fn type_description(mut keys: MessageReader<bevy::input::keyboard::KeyboardInput>, mut asked: ResMut<SaveLoad>, menu: Res<crate::pause::Menu>) {
    use bevy::input::keyboard::Key;
    let saving = *menu == crate::pause::Menu::Saves(Kind::Save) && asked.ask.is_none();
    for event in keys.read() {
        if !saving || !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                asked.text.pop();
            }
            // At most 79 characters.
            Key::Space if asked.text.len() < 79 => asked.text.push(' '),
            Key::Character(typed) => {
                for c in typed.chars().filter(|c| c.is_ascii_graphic()) {
                    if asked.text.len() < 79 {
                        asked.text.push(c);
                    }
                }
            }
            _ => continue,
        }
        asked.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_file_comes_back_as_it_was_written() {
        let setup = MatchSetup { map: "mp03t4".into(), map_name: "Anytown".into(), options: crate::game::Options::default(), max_players: 4, mode_rules: "MPBattle.ini".into() };
        let file = SaveFile { format: FORMAT, description: "a game".into(), tick: 1234, digest: 7, setup, local: 1, camera: (10.0, -20.0), state: (0..=255).collect(), recording: None };
        let path = std::env::temp_dir().join(format!("ra2-rust-save-test-{}.sav", std::process::id()));
        write(&path, &file).unwrap();
        let back = read(&path).unwrap();
        assert_eq!((back.tick, back.local, back.camera, &back.state, back.setup.map.as_str()), (1234, 1, (10.0, -20.0), &file.state, "mp03t4"));
        // The list reads only the front of it.
        let front: (u32, String) = bincode::deserialize(&std::fs::read(&path).unwrap()[8..]).unwrap();
        assert_eq!(front, (FORMAT, "a game".to_owned()));
        assert!(stamp(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_791_878_400)).starts_with("2026-10-13"));
        // Anything else is refused.
        std::fs::write(&path, b"not a save").unwrap();
        assert!(read(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
