use bevy::prelude::*;

mod action;
mod aidata;
mod armoury;
mod audio;
mod bar;
mod beams;
mod colors;
mod control;
mod cursor;
mod debug;
mod depth;
mod endgame;
mod files;
#[cfg(test)]
mod fuzz;
mod game;
mod home;
mod hud;
mod lighting;
mod net;
mod notice;
mod pause;
mod saves;
mod schedule;
mod radar;
#[cfg(test)]
mod scenario;
mod setup;
mod shell;
mod shroud;
mod steam;
mod terrain;
mod units;
mod prefs;
#[cfg(test)]
mod replay;
mod video;
mod voxel;

fn main() {
    home::log_to_file();
    let files = match setup::locate() {
        Ok(files) => files,
        Err(reason) => {
            if std::env::var_os("RA2_DIR").is_some() {
                panic!("could not open the Red Alert 2 data files in RA2_DIR: {reason:?}");
            }
            // The screen that asks for them runs in a window of its own; the
            // game starts afresh once they are chosen, since a program can
            // only open its windows' event loop once.
            if setup::run(reason) == setup::Outcome::Play {
                restart();
            }
            return;
        }
    };
    eprintln!("the game's files are in {}", files.dir().display());
    let video = video::Video::from_settings();
    App::new()
        .insert_resource(files)
        .insert_resource(video)
        .insert_resource(crate::net::Settings::read().prefs)
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(video.window()),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugins((shell::ShellPlugin, game::GamePlugin, hud::HudPlugin, units::UnitsPlugin, debug::DebugPlugin, audio::AudioPlugin, cursor::CursorPlugin, control::ControlPlugin, pause::PausePlugin, radar::RadarPlugin, shroud::ShroudPlugin, beams::BeamsPlugin))
        .add_plugins((schedule::SchedulePlugin, video::VideoPlugin, bar::BarPlugin, endgame::EndGamePlugin, saves::SavesPlugin, net::NetPlugin, notice::NoticePlugin, depth::DepthPlugin))
        .run();
}

/// Starts this program again in place of this run.
fn restart() {
    let exe = std::env::current_exe().expect("this program's own path");
    let mut command = std::process::Command::new(exe);
    command.args(std::env::args_os().skip(1)).env_remove("RA2_SETUP");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        panic!("could not start the game: {error}");
    }
    #[cfg(not(unix))]
    {
        command.spawn().expect("could not start the game");
    }
}
