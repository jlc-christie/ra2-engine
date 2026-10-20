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

/// Selects the original rules, art, maps and interface. Expansion-specific
/// simulation behavior is implemented separately, and remains incomplete.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edition {
    #[default]
    RedAlert2,
    YuriAssets,
}

impl Edition {
    fn from_env() -> std::io::Result<Self> {
        match std::env::var("RA2_EDITION").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("") | Ok("ra2") => Ok(Self::RedAlert2),
            Ok("yr") => Ok(Self::YuriAssets),
            _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "RA2_EDITION must be ra2 or yr")),
        }
    }

    fn asset_name<'a>(self, name: &'a str) -> &'a str {
        if self == Self::YuriAssets {
            if let Some((_, md)) = MD_ALIASES.iter().find(|(base, _)| name.eq_ignore_ascii_case(base)) {
                return md;
            }
        }
        name
    }
}

// Highest priority first. In the installed 1.001 data, expandmd01 replaces
// rulesmd.ini and soundmd.ini from localmd.mix.
const YR_ARCHIVES: &[&str] = &["expandmd01.mix", "langmd.mix", "ra2md.mix", "multimd.mix", "mapsmd03.mix", "thememd.mix"];
const YR_NESTED: &[&str] = &[
    "localmd.mix", "cachemd.mix", "ntrlmd.mix", "cameomd.mix",
    "isotemmd.mix", "isosnomd.mix", "isourbmd.mix",
    "temmd.mix", "snomd.mix", "urbmd.mix",
    "conqmd.mix", "isogenmd.mix", "genermd.mix", "audiomd.mix", "loadmd.mix",
    "isodesmd.mix", "isolunmd.mix", "isoubnmd.mix",
    "isodes.mix", "isolun.mix", "isoubn.mix",
    "desert.mix", "lunar.mix", "urbann.mix",
];
// Despite its name, SIDEC02MD contains Yuri's sidebar, not the Soviet one.
// Allied and Soviet sidebars still come from the original SIDEC01/02 archives.
const YR_SIDEBARS: &[&str] = &["sidec02md.mix"];
const MD_ALIASES: &[(&str, &str)] = &[
    ("rules.ini", "rulesmd.ini"),
    ("art.ini", "artmd.ini"),
    ("ai.ini", "aimd.ini"),
    ("sound.ini", "soundmd.ini"),
    ("eva.ini", "evamd.ini"),
    ("theme.ini", "thememd.ini"),
    ("ra2.csf", "ra2md.csf"),
    ("missions.pkt", "missionsmd.pkt"),
    ("mpmodes.ini", "mpmodesmd.ini"),
    // The direct-to-game debug entry point defaults to MPBattle.ini.
    ("mpbattle.ini", "mpbattlemd.ini"),
    ("temperat.ini", "temperatmd.ini"),
    ("snow.ini", "snowmd.ini"),
    ("urban.ini", "urbanmd.ini"),
    ("desert.ini", "desertmd.ini"),
    ("lunar.ini", "lunarmd.ini"),
    ("urbann.ini", "urbannmd.ini"),
    ("ui.ini", "uimd.ini"),
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
    edition: Edition,
    archives: Vec<Mix>,
    /// Each side's sidebar archive (missing ones left empty), and the side
    /// whose art is looked in first.
    sidebars: Vec<Option<Mix>>,
    sidebar_overlays: Vec<Option<Mix>>,
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
    pub fn open() -> std::io::Result<Self> {
        Self::open_at(&dev_dir())
    }

    /// Opens the game's archives in `dir`. An error names the file at fault.
    pub fn open_at(dir: &Path) -> std::io::Result<Self> {
        Self::open_for_edition(dir, Edition::from_env()?)
    }

    /// Opens an explicit edition without changing process-wide environment.
    /// YR overlays the expansion's rules, art, maps, theatres and Yuri sidebar.
    pub fn open_for_edition(dir: &Path, edition: Edition) -> std::io::Result<Self> {
        let dir = dir.to_path_buf();
        let (mut archives, sidebar_overlays) = if edition == Edition::YuriAssets {
            let (archives, mut yuri) = Self::read_archives(&dir, YR_ARCHIVES, YR_NESTED, YR_SIDEBARS)?;
            (archives, vec![None, None, yuri.pop().flatten()])
        } else {
            (Vec::new(), SIDEBARS.iter().map(|_| None).collect())
        };
        let (base, sidebars) = Self::read_archives(&dir, ARCHIVES, NESTED, SIDEBARS)?;
        archives.extend(base);
        let files = Self { dir, edition, archives, sidebars, sidebar_overlays, side: Default::default() };
        if edition == Edition::YuriAssets {
            let missing: Vec<_> = MD_ALIASES.iter().filter(|(base, _)| files.get(base).is_none()).map(|(_, md)| *md).collect();
            if !missing.is_empty() {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("experimental YR assets are incomplete: {}", missing.join(", "))));
            }
        }
        Ok(files)
    }

    fn read_archives(dir: &Path, names: &[&str], nested_names: &[&str], sidebar_names: &[&str]) -> std::io::Result<(Vec<Mix>, Vec<Option<Mix>>)> {
        let mut archives = Vec::new();
        let mut sidebars: Vec<Option<Mix>> = sidebar_names.iter().map(|_| None).collect();
        for name in names {
            let path = find(&dir, name).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("{name} not found"))
            })?;
            let named = |error: std::io::Error| std::io::Error::new(error.kind(), format!("{name}: {error}"));
            let mix = Mix::open(path).map_err(named)?;
            for nested in nested_names {
                if let Some(inner) = mix.nested(nested) {
                    archives.push(inner.map_err(named)?);
                }
            }
            for (slot, nested) in sidebars.iter_mut().zip(sidebar_names) {
                if slot.is_none() {
                    if let Some(inner) = mix.nested(nested) {
                        *slot = Some(inner.map_err(named)?);
                    }
                }
            }
            archives.push(mix);
        }
        Ok((archives, sidebars))
    }

    /// The original's executable, if it is there and its tables are where
    /// they are read from.
    pub fn engine_exe(&self) -> Option<Vec<u8>> {
        // This experimental mode still runs our RA2 simulation/cursor code.
        // Its shared tables may come only from the digest-checked RA2 game.exe;
        // gamemd.exe has different addresses and is never interpreted here.
        std::fs::read(find(&self.dir, EXE)?).ok().filter(|exe| exe_tables_match(exe))
    }

    /// The folder the files were found in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn edition(&self) -> Edition {
        self.edition
    }

    /// Reads a file that sits in the game directory outside the archives.
    pub fn loose(&self, relative: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(relative)).ok()
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let name = self.edition.asset_name(name);
        let side = self.side.load(std::sync::atomic::Ordering::Relaxed);
        let shared = if side == 2 { 1 } else { side };
        let sidebar = |at: usize| self.sidebars.get(at).and_then(Option::as_ref).and_then(|mix| mix.get(name));
        let overlay = |at: usize| self.sidebar_overlays.get(at).and_then(Option::as_ref).and_then(|mix| mix.get(name));
        overlay(side).or_else(|| sidebar(shared)).or_else(|| sidebar(0)).or_else(|| self.archives.iter().find_map(|mix| mix.get(name)))
    }

    /// YR's audio bank adds samples to the original bank. Each index must be
    /// decoded against its own bag, then missing samples fall through to RA2.
    pub fn audio_banks(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.archives.iter().filter_map(|mix| mix.get("audio.idx").zip(mix.get("audio.bag")))
    }

    /// Chooses whose sidebar art is used: 0 Allied, 1 Soviet, 2 Yuri (YR only).
    pub fn set_side(&self, side: usize) {
        let last = if self.edition == Edition::YuriAssets { 2 } else { 1 };
        self.side.store(side.min(last), std::sync::atomic::Ordering::Relaxed);
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
    let edition = match Edition::from_env() {
        Ok(edition) => edition,
        Err(error) => return failed(Problem::Damaged(error.to_string())),
    };
    match std::fs::read_dir(dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return failed(Problem::NoPermission),
        Err(_) => return failed(Problem::Gone),
    }
    let yr_archives = if edition == Edition::YuriAssets { YR_ARCHIVES } else { &[] };
    let mut problems: Vec<Problem> = ARCHIVES.iter().chain(yr_archives).filter(|name| find(dir, name).is_none()).map(|name| Problem::Missing(name)).collect();
    if !problems.is_empty() {
        return Check { problems, warnings: Vec::new(), files: None };
    }
    let mut warnings = Vec::new();
    if edition == Edition::YuriAssets {
        warnings.push("Experimental Yuri's Revenge: expansion rules, three sides and six theatres; expansion gameplay is not yet complete".into());
    }
    let simpler = "vehicles will turn on the spot and the shroud's edges and the cursor will be plainer";
    match find(dir, EXE).map(std::fs::read) {
        Some(Ok(exe)) if exe_tables_match(&exe) => {}
        Some(Ok(_)) => warnings.push(format!("{EXE} is a version other than 1.006, so {simpler}")),
        _ => warnings.push(format!("{EXE} couldn't be read, so {simpler}")),
    }
    let files = match GameFiles::open_for_edition(dir, edition) {
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
    if edition == Edition::YuriAssets && files.sidebar_overlays.get(2).is_none_or(Option::is_none) {
        missing.push("sidec02md.mix");
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

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("ra2-edition-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A minimal unencrypted MIX, so edition selection is also tested without
    /// requiring commercial game files or modifying an installation.
    fn write_mix(path: &Path, entries: &[(&str, &[u8])]) {
        let mut bytes = Vec::new();
        bytes.extend(0u32.to_le_bytes());
        bytes.extend((entries.len() as u16).to_le_bytes());
        bytes.extend((entries.iter().map(|(_, data)| data.len()).sum::<usize>() as u32).to_le_bytes());
        let mut offset = 0;
        for (name, data) in entries {
            bytes.extend(ra2_formats::mix::mix_id(name).to_le_bytes());
            bytes.extend((offset as u32).to_le_bytes());
            bytes.extend((data.len() as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, data) in entries { bytes.extend_from_slice(data) }
        std::fs::write(path, bytes).unwrap();
    }

    fn edition_fixture(dir: &Path, omit: Option<&str>) {
        for name in ARCHIVES.iter().chain(YR_ARCHIVES) { write_mix(&dir.join(name), &[]) }
        write_mix(&dir.join("ra2.mix"), &[("rules.ini", b"[MTNK]\nImage=BASE\n"), ("art.ini", b"base art")]);
        let entries: Vec<_> = MD_ALIASES.iter().filter(|(_, md)| Some(*md) != omit).map(|(_, md)| (*md, b"unpatched YR data".as_slice())).collect();
        write_mix(&dir.join("ra2md.mix"), &entries);
        write_mix(&dir.join("expandmd01.mix"), &[("rulesmd.ini", b"[MTNK]\nImage=GTNK\n")]);
    }

    #[test]
    fn yr_patches_override_md_data_while_ra2_ignores_both() {
        let dir = TestDir::new();
        edition_fixture(&dir.0, None);
        let base = GameFiles::open_for_edition(&dir.0, Edition::RedAlert2).unwrap();
        let yr = GameFiles::open_for_edition(&dir.0, Edition::YuriAssets).unwrap();
        assert_eq!(base.get("rules.ini"), Some(b"[MTNK]\nImage=BASE\n".as_slice()));
        assert_eq!(yr.get("RULES.INI"), Some(b"[MTNK]\nImage=GTNK\n".as_slice()));
        assert_eq!(yr.get("rules.ini"), yr.get("rulesmd.ini"));
    }

    #[test]
    fn incomplete_yr_data_does_not_fall_back_to_ra2() {
        let dir = TestDir::new();
        edition_fixture(&dir.0, Some("artmd.ini"));
        let error = GameFiles::open_for_edition(&dir.0, Edition::YuriAssets).err().expect("missing MD art must fail even when base art exists");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(error.to_string().contains("artmd.ini"));
        std::fs::remove_file(dir.0.join("langmd.mix")).unwrap();
        let error = GameFiles::open_for_edition(&dir.0, Edition::YuriAssets).err().expect("a missing YR archive must fail");
        assert!(error.to_string().contains("langmd.mix"));
        assert!(GameFiles::open_for_edition(&dir.0, Edition::RedAlert2).is_ok());
    }

    #[test]
    fn installed_yr_rules_strings_map_and_shared_visuals_load() {
        use ra2_formats::{csf::Csf, ini::Ini, map::Map, shp::Shp, vxl::{Hva, Vxl}};
        let dir = dev_dir();
        if find(&dir, "ra2md.mix").is_none() { return }
        let files = GameFiles::open_for_edition(&dir, Edition::YuriAssets).unwrap();
        assert_eq!(files.edition(), Edition::YuriAssets);
        let rules = Ini::parse(files.get("rules.ini").unwrap());
        assert_eq!(rules.get("MTNK", "Image"), Some("GTNK"));
        let patch = Mix::open(find(&dir, "expandmd01.mix").unwrap()).unwrap();
        assert_eq!(files.get("rules.ini"), patch.get("rulesmd.ini"));
        let local = Mix::open(find(&dir, "ra2md.mix").unwrap()).unwrap().nested("localmd.mix").unwrap().unwrap();
        assert_ne!(files.get("rules.ini"), local.get("rulesmd.ini"));
        let strings = Csf::parse(files.get("ra2.csf").unwrap()).unwrap();
        assert!(strings.get("GUI:GameControls").is_some());
        let map = Map::parse(files.get("xmp02t2.map").unwrap()).unwrap();
        assert_eq!((&*map.theater, map.width, map.height), ("TEMPERATE", 100, 100));
        let art = Ini::parse(files.get("art.ini").unwrap());
        assert_eq!(art.get("GISequence", "Deployed"), Some("292,1,1"));
        let gi = Shp::parse(files.get("gi.shp").unwrap()).unwrap();
        assert!(gi.frames.get(292).is_some());
        let model = rules.get("MTNK", "Image").unwrap().to_lowercase();
        let vxl = Vxl::parse(files.get(&format!("{model}.vxl")).unwrap()).unwrap();
        let hva = files.get(&format!("{model}.hva")).and_then(Hva::parse);
        for facing in (0..=255).step_by(8) {
            let shadow = crate::voxel::shadow(&vxl, hva.as_ref(), facing, false, 0, None).unwrap();
            assert!(shadow.rgba.chunks_exact(4).any(|p| p[3] != 0), "YR Grizzly shadow is empty at {facing}");
        }
    }

    #[test]
    fn yr_sidebar_selection_keeps_soviet_art_and_adds_yuri_overlays() {
        let dir = dev_dir();
        if find(&dir, "ra2md.mix").is_none() { return }
        let files = GameFiles::open_for_edition(&dir, Edition::YuriAssets).unwrap();
        let base = Mix::open(find(&dir, "ra2.mix").unwrap()).unwrap();
        let soviet = base.nested("sidec02.mix").unwrap().unwrap();
        let md = Mix::open(find(&dir, "ra2md.mix").unwrap()).unwrap();
        let yuri = md.nested("sidec02md.mix").unwrap().unwrap();
        files.set_side(1);
        for name in ["sidebar.pal", "side1.shp", "radar.shp"] {
            assert!(soviet.get(name).is_some(), "missing Soviet {name}");
            assert_eq!(files.get(name), soviet.get(name), "Soviets must keep their own {name}");
        }
        files.set_side(2);
        for name in ["radary.shp", "radaryuri.pal", "bkgdlgy.shp", "uibkgdy.pal"] {
            assert!(yuri.get(name).is_some(), "missing Yuri {name}");
            assert_eq!(files.get(name), yuri.get(name));
        }
        assert_eq!(files.get("side1.shp"), soviet.get("side1.shp"), "shared sidebar frames use the Soviet family");
    }

    #[test]
    fn the_steam_copy_passes_its_check() {
        let dir = dev_dir();
        if !dir.exists() {
            eprintln!("game files not found; skipping");
            return;
        }
        let check = check(&dir);
        assert_eq!(check.problems, Vec::new());
        if check.files.as_ref().is_some_and(|files| files.edition() == Edition::YuriAssets) {
            assert_eq!(check.warnings.len(), 1);
            assert!(check.warnings[0].starts_with("Experimental Yuri's Revenge:"));
        } else {
            assert_eq!(check.warnings, Vec::<String>::new());
        }
        assert!(check.ok());
        // Found from the folder above it too. The development depot may be a
        // symlink to Steam; search deliberately traverses real directories.
        let actual = dir.canonicalize().unwrap();
        assert_eq!(search(actual.parent().unwrap(), 2, 100).as_deref(), Some(actual.as_path()));
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
