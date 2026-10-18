//! The screen: its size, and filling the screen or not.
//!
//! The original keeps the size it runs
//! at as `[Video] ScreenWidth` and `ScreenHeight`, and always fills the screen,
//! changing the display's mode to that size.
//! Not original behaviour: here the game runs in a window by default; with
//! `Fullscreen=yes` (or Alt+Enter, or RA2_FULLSCREEN=1) it fills the screen
//! without changing the display's mode, and is scaled so that it is as near
//! the size asked for as the screen's shape allows. RA2_RES=WxH asks for a size
//! for one run. A window can be resized freely; everything is laid out for
//! the size it has.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode, WindowResolution};

/// The size the game starts at when nothing is asked for.
pub const DEFAULT: (u32, u32) = (800, 600);

/// What was asked for.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Video {
    pub screen: Option<(u32, u32)>,
    pub fullscreen: bool,
}

impl Video {
    pub fn from_settings() -> Self {
        let kept = crate::net::Settings::read();
        let parse = |text: &str| {
            let (w, h) = text.trim().split_once(['x', 'X'])?;
            Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)).filter(|&(w, h)| w >= 200 && h >= 200)
        };
        let screen = std::env::var("RA2_RES").ok().and_then(|text| parse(&text)).or(kept.screen);
        let fullscreen = std::env::var("RA2_FULLSCREEN").map_or(kept.fullscreen, |value| value.trim() != "0") && !screenshot_run();
        Self { screen, fullscreen }
    }

    /// The window the game opens with.
    pub fn window(&self) -> Window {
        let (width, height) = self.screen.unwrap_or(DEFAULT);
        Window {
            title: crate::home::APP_NAME.into(),
            resolution: WindowResolution::new(width, height),
            mode: if self.fullscreen { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed },
            // Screenshots are drawn off screen, so a run that takes one stays
            // out of the way of whoever is using the machine.
            visible: !screenshot_run(),
            focused: !screenshot_run(),
            ..default()
        }
    }
}

fn screenshot_run() -> bool {
    std::env::var_os("RA2_SHOT").is_some()
}

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (toggle, fit, resize_for_checks).chain())
            // The menus may be drawn larger on a big screen; the game's sidebar
            // and bar are always drawn at their own size.
            .add_systems(OnEnter(crate::shell::Screen::InGame), |mut scale: ResMut<UiScale>| scale.0 = 1.0);
    }
}

/// Puts the window into the state asked for.
pub fn apply(video: &Video, window: &mut Window) {
    if video.fullscreen {
        window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Current);
    } else {
        window.mode = WindowMode::Windowed;
        window.resolution.set_scale_factor_override(None);
        let (width, height) = video.screen.unwrap_or(DEFAULT);
        window.resolution.set(width as f32, height as f32);
    }
}

/// Alt+Enter: fills the screen, or goes back to a window; kept for next time.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut video: ResMut<Video>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    if !(alt && keys.just_pressed(KeyCode::Enter)) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    video.fullscreen = !video.fullscreen;
    apply(&video, &mut window);
    let kept = crate::net::Settings::read();
    crate::net::Settings { fullscreen: video.fullscreen, ..kept }.save();
}

/// Filling the screen at a size asked for: the scale that brings the game as
/// near that size as the screen's shape allows (the screen's own scale when
/// no size was asked for).
fn fit(video: Res<Video>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if !video.fullscreen || window.mode == WindowMode::Windowed {
        return;
    }
    let Some((width, height)) = video.screen else {
        return;
    };
    let (pw, ph) = (window.physical_width() as f32, window.physical_height() as f32);
    if pw < 1.0 || ph < 1.0 {
        return;
    }
    let scale = (pw / width as f32).min(ph / height as f32).max(0.25);
    if window.resolution.scale_factor_override().is_none_or(|now| (now - scale).abs() > 0.001) {
        window.resolution.set_scale_factor_override(Some(scale));
    }
}

/// RA2_RESIZE=frame:WxH resizes the window at that frame, for checking that
/// everything is laid out again.
fn resize_for_checks(mut frame: Local<u32>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    *frame += 1;
    let Ok(text) = std::env::var("RA2_RESIZE") else {
        return;
    };
    let Some((at, size)) = text.split_once(':') else {
        return;
    };
    let Some((w, h)) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))) else {
        return;
    };
    if at.parse::<u32>().ok() == Some(*frame) {
        if let Ok(mut window) = windows.single_mut() {
            window.resolution.set(w, h);
        }
    }
}
