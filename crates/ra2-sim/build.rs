//! Sets `game_files` when the game's executable is there (`RA2_GAME_EXE`, or
//! the Steam depot copied into the project root), so the tests that read its
//! tables run, and are ignored without it.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(game_files)");
    println!("cargo::rerun-if-env-changed=RA2_GAME_EXE");
    let exe = std::env::var("RA2_GAME_EXE").unwrap_or_else(|_| "../../app_2229850/depot_2229851/game.exe".into());
    println!("cargo::rerun-if-changed={exe}");
    if std::path::Path::new(&exe).is_file() {
        println!("cargo::rustc-cfg=game_files");
    }
}
