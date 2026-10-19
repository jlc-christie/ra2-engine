//! Access to the original game's data files.

use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use ra2_formats::mix::Mix;

const ARCHIVES: &[&str] = &[
    "language.mix",
    "ra2.mix",
    "multi.mix",
    "maps01.mix",
    "maps02.mix",
    "theme.mix",
];

/// Archives nested inside the top-level ones that are searched.
const NESTED: &[&str] = &[
    "local.mix",
    "cache.mix",
    "neutral.mix",
    "cameo.mix",
    "isotemp.mix",
    "isosnow.mix",
    "isourb.mix",
    "temperat.mix",
    "snow.mix",
    "urban.mix",
    // The theatre set-up also opens these small ones.
    "tem.mix",
    "sno.mix",
    "urb.mix",
    "conquer.mix",
    "isogen.mix",
    "generic.mix",
    // Sounds and the announcer, inside language.mix.
    "audio.mix",
    // The loading screens, inside ra2.mix.
    "load.mix",
];

/// The sidebar art of each side, in side order.
// The sidebar archive is `SIDEC%02d.MIX` with the
// player's side number plus one.
const SIDEBARS: &[&str] = &["sidec01.mix", "sidec02.mix"];

#[derive(Resource, Clone)]
pub struct GameFiles {
    dir: PathBuf,
    archives: Vec<Mix>,
    /// Each side's sidebar archive (missing ones left empty), and the side
    /// whose art is looked in first.
    sidebars: Vec<Option<Mix>>,
    side: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// Finds a file in `dir` ignoring case, since the depot mixes upper and lower case.
pub fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|entry| entry.path())
}

impl GameFiles {
    /// Opens the game directory named by `RA2_DIR`, or the Steam depot copied into
    /// the project root (for the tests and tools; the game itself goes through
    /// `setup::locate`).
    #[cfg(test)]
    pub fn open() -> std::io::Result<Self> {
        Self::open_at(&dev_dir())
    }

    /// Opens the game's archives in `dir`. An error names the file at fault.
    pub fn open_at(dir: &Path) -> std::io::Result<Self> {
        let dir = dir.to_path_buf();
        let mut archives = Vec::new();
        let mut sidebars: Vec<Option<Mix>> = SIDEBARS.iter().map(|_| None).collect();
        for name in ARCHIVES {
            let path = find(&dir, name).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("{name} not found"))
            })?;
            let named = |error: std::io::Error| std::io::Error::new(error.kind(), format!("{name}: {error}"));
            let mix = Mix::open(path).map_err(named)?;
            for nested in NESTED {
                if let Some(inner) = mix.nested(nested) {
                    archives.push(inner.map_err(named)?);
                }
            }
            for (slot, nested) in sidebars.iter_mut().zip(SIDEBARS) {
                if slot.is_none()
                    && let Some(inner) = mix.nested(nested) {
                        *slot = Some(inner.map_err(named)?);
                    }
            }
            archives.push(mix);
        }
        Ok(Self { dir, archives, sidebars, side: Default::default() })
    }

    /// The original's executable, if it is there and its tables are where
    /// they are read from.
    pub fn engine_exe(&self) -> Option<Vec<u8>> {
        std::fs::read(find(&self.dir, EXE)?).ok().filter(|exe| exe_tables_match(exe))
    }

    /// The folder the files were found in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Reads a file that sits in the game directory outside the archives.
    pub fn loose(&self, relative: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(relative)).ok()
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let side = self.side.load(std::sync::atomic::Ordering::Relaxed);
        let sidebar = |at: usize| self.sidebars.get(at).and_then(Option::as_ref).and_then(|mix| mix.get(name));
        sidebar(side).or_else(|| sidebar(0)).or_else(|| self.archives.iter().find_map(|mix| mix.get(name)))
    }

    /// Chooses whose sidebar art is used: the player's side (0 Allied, 1 Soviet).
    pub fn set_side(&self, side: usize) {
        self.side.store(side.min(SIDEBARS.len() - 1), std::sync::atomic::Ordering::Relaxed);
    }
}

/// The game's folder for the tests and tools: `RA2_DIR`, or the Steam depot
/// copied into the project root.
pub fn dev_dir() -> PathBuf {
    std::env::var_os("RA2_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("app_2229850/depot_2229851"))
}

/// The original's executable, whose tables are read (drive tracks, the
/// shroud's edges, the cursor's frames; see `tracks.rs`, `shroud.rs`,
/// `cursor.rs`). Not needed: without it those are simpler.
pub const EXE: &str = "game.exe";
/// Where those tables are in 1.006 (addresses, lengths), and a digest of their
/// bytes there. Any copy whose bytes match is used, whatever its version
/// says; one that doesn't match is not read, rather than misread.
const EXE_TABLES: &[(usize, usize)] = &[(0x7a_00a0, 14 * 16), (0x7a_c3bc, 256), (0x7e_2af8, 79 * 24)];
const EXE_TABLES_DIGEST: u64 = 0x9a67_f488_91f7_4a49;

/// Whether an executable holds the tables where 1.006 has them.
fn exe_tables_match(exe: &[u8]) -> bool {
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    for &(at, len) in EXE_TABLES {
        let Some(bytes) = exe.get(at - 0x40_0000..at - 0x40_0000 + len) else {
            return false;
        };
        for &byte in bytes {
            digest = (digest ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    digest == EXE_TABLES_DIGEST
}
/// Not needed, but without it vehicles are lit flat.
pub const NORMALS: &str = "FinalAlert2/voxel_normal_tables.bin";

/// Files inside the archives that are read whatever is played, each checked
/// before the game starts so that a copy lacking one is found out then rather
/// than part way into a game. (Art for particular units, maps and so on are
/// not listed; the game does without what it lacks of those.)
const NEEDED_INSIDE: &[&str] = &[
    "rules.ini",
    "art.ini",
    "ai.ini",
    "sound.ini",
    "theme.ini",
    "ra2.csf",
    "game.fnt",
    "missions.pkt",
    "mpmodes.ini",
    "palette.pal",
    "anim.pal",
    "mouse.shp",
    "mousepal.pal",
    "shroud.shp",
    "voxels.vpl",
    "audio.idx",
    "audio.bag",
    "temperat.ini",
    "snow.ini",
    "urban.ini",
    "isotem.pal",
    "unittem.pal",
    "pips.shp",
];

/// What is wrong with a folder chosen as the game's.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// The folder is not there (moved, deleted, or on a drive not plugged in).
    Gone,
    /// The system would not let it be read.
    NoPermission,
    /// A file that must be in the folder is not.
    Missing(&'static str),
    /// A file is there but could not be read as it should be.
    Damaged(String),
    /// Files that must be in the archives are not.
    MissingInside(Vec<&'static str>),
}

/// The outcome of looking over a folder.
pub struct Check {
    pub problems: Vec<Problem>,
    /// Things that are not right but that the game can do without.
    pub warnings: Vec<String>,
    pub files: Option<GameFiles>,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.problems.is_empty() && self.files.is_some()
    }
}

/// Looks over a folder for everything the game reads, so that a copy that
/// can't be played is found out before the game starts.
/// Not original behaviour.
pub fn check(dir: &Path) -> Check {
    let failed = |problem: Problem| Check { problems: vec![problem], warnings: Vec::new(), files: None };
    match std::fs::read_dir(dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return failed(Problem::NoPermission),
        Err(_) => return failed(Problem::Gone),
    }
    let mut problems: Vec<Problem> = ARCHIVES.iter().filter(|name| find(dir, name).is_none()).map(|name| Problem::Missing(name)).collect();
    if !problems.is_empty() {
        return Check { problems, warnings: Vec::new(), files: None };
    }
    let mut warnings = Vec::new();
    let simpler = "vehicles will turn on the spot and the shroud's edges and the cursor will be plainer";
    match find(dir, EXE).map(std::fs::read) {
        Some(Ok(exe)) if exe_tables_match(&exe) => {}
        Some(Ok(_)) => warnings.push(format!("{EXE} is a version other than 1.006, so {simpler}")),
        _ => warnings.push(format!("{EXE} couldn't be read, so {simpler}")),
    }
    let files = match GameFiles::open_at(dir) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return failed(Problem::NoPermission),
        Err(error) => {
            problems.push(Problem::Damaged(error.to_string()));
            return Check { problems, warnings, files: None };
        }
    };
    let mut missing: Vec<&'static str> = NEEDED_INSIDE.iter().copied().filter(|name| files.get(name).is_none()).collect();
    for (slot, name) in files.sidebars.iter().zip(SIDEBARS) {
        if slot.is_none() {
            missing.push(name);
        }
    }
    if !missing.is_empty() {
        problems.push(Problem::MissingInside(missing));
    }
    if files.loose(NORMALS).is_none() {
        warnings.push(format!("{NORMALS} is missing; vehicles will be lit flat"));
    }
    Check { problems, warnings, files: Some(files) }
}

/// Whether a folder looks like the game's at a glance (its main archive is
/// there), without opening anything.
pub fn looks_like_game(dir: &Path) -> bool {
    find(dir, "ra2.mix").is_some()
}

/// The first folder at or below `root` that looks like the game's, looking
/// no more than `depth` folders down and over no more than `budget` folders.
/// Hidden folders are passed over.
pub fn search(root: &Path, depth: usize, mut budget: usize) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0)]);
    while let Some((dir, level)) = queue.pop_front() {
        if looks_like_game(&dir) {
            return Some(dir);
        }
        if level >= depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut below: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        below.sort();
        for path in below {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            queue.push_back((path, level + 1));
        }
    }
    None
}

/// Places the game is often found on a Mac, looked through when the game is
/// first run. Only folders macOS lets a program read without asking are
/// looked in (not Documents, Desktop, Downloads or other programs' data), so
/// that the first thing the player sees isn't a request for permission.
pub fn detect() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    // Fetched with Steam's console (see `steam`), where Steam puts it.
    if let Some(dir) = crate::steam::depot_dir().filter(|dir| check(dir).ok()) {
        return Some(dir);
    }
    let mut roots: Vec<(PathBuf, usize)> = vec![
        (home.join("Library/Application Support/Steam/steamapps/common"), 2),
        (home.join("Games"), 3),
        (home.join("Applications"), 1),
        (PathBuf::from("/Applications"), 1),
    ];
    // Windows set-ups run by CrossOver, and wrappers made by Porting Kit,
    // Wineskin and the like.
    if let Ok(bottles) = std::fs::read_dir(home.join("Library/Application Support/CrossOver/Bottles")) {
        roots.extend(bottles.filter_map(Result::ok).map(|bottle| (bottle.path().join("drive_c"), 5)));
    }
    for apps in [home.join("Applications"), PathBuf::from("/Applications")] {
        let Ok(entries) = std::fs::read_dir(&apps) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.ends_with(".app") && (name.contains("red alert") || name.contains("ra2") || name.contains("command")) {
                roots.push((entry.path(), 8));
            }
        }
    }
    roots.into_iter().find_map(|(root, depth)| search(&root, depth, 4000).filter(|dir| check(dir).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steam_copy_passes_its_check() {
        let dir = dev_dir();
        if !dir.exists() {
            eprintln!("game files not found; skipping");
            return;
        }
        let check = check(&dir);
        assert_eq!(check.problems, Vec::new());
        assert_eq!(check.warnings, Vec::<String>::new());
        assert!(check.ok());
        // Found from the folder above it too.
        assert_eq!(search(dir.parent().unwrap(), 2, 100).as_deref(), Some(dir.as_path()));
    }

    #[test]
    fn a_folder_that_is_gone_or_not_the_game_is_told_apart() {
        assert_eq!(check(Path::new("/no/such/folder")).problems, vec![Problem::Gone]);
        let empty = std::env::temp_dir().join(format!("ra2-check-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let check = check(&empty);
        assert!(check.problems.contains(&Problem::Missing("ra2.mix")));
        assert!(!check.problems.contains(&Problem::Missing(EXE)), "game.exe is not needed");
        assert!(!check.ok());
        let _ = std::fs::remove_dir_all(&empty);
    }
}
