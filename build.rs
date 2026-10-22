//! Sets `game_files` when the game's folder is there (`RA2_DIR`, or the Steam
//! depot copied into the project root), so the tests that need it run, and are
//! ignored without it.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(game_files)");
    println!("cargo::rerun-if-env-changed=RA2_DIR");
    let dir = std::env::var_os("RA2_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("app_2229850/depot_2229851"));
    println!("cargo::rerun-if-changed={}", dir.display());
    if dir.is_dir() {
        println!("cargo::rustc-cfg=game_files");
    }
}
