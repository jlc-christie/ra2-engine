//! The player's choices on the Options screen, kept between runs.
//!
//! The original keeps them in
//! RA2.INI as `[Options]` `DetailLevel`, `Difficulty`, `ScrollRate`,
//! `UnitActionLines`, `ShowHidden`, `ToolTips` and `[Audio]` `SoundVolume`,
//! `VoiceVolume`, `ScoreVolume`; here they go in our `settings.ini` under the
//! same names (see `net::Settings`).

use bevy::prelude::*;

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    /// 0 lower or 1 higher (the slider has two places).
    pub detail: i32,
    /// The campaign's difficulty: 0 easiest to 2 hardest.
    pub difficulty: i32,
    /// 0 fastest to 6 slowest (the slider shows 6 less this).
    pub scroll_rate: i32,
    pub target_lines: bool,
    pub show_hidden: bool,
    pub tooltips: bool,
    /// Each from 0 to 1, in tenths on the screen.
    pub sound: f32,
    pub voice: f32,
    pub music: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { detail: 1, difficulty: 1, scroll_rate: 3, target_lines: true, show_hidden: true, tooltips: true, sound: 0.7, voice: 0.8, music: 0.6 }
    }
}

impl Prefs {
    /// From a settings file's keys, each missing one as the default.
    pub fn from_keys(get: &dyn Fn(&str) -> Option<String>) -> Self {
        let base = Self::default();
        let int = |key: &str, default: i32, most: i32| get(key).and_then(|v| v.parse::<i32>().ok()).map_or(default, |v| v.clamp(0, most));
        let flag = |key: &str, default: bool| get(key).map_or(default, |v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true" | "1"));
        let level = |key: &str, default: f32| get(key).and_then(|v| v.parse::<f32>().ok()).map_or(default, |v| v.clamp(0.0, 1.0));
        Self {
            detail: int("DetailLevel", base.detail, 1),
            difficulty: int("Difficulty", base.difficulty, 2),
            scroll_rate: int("ScrollRate", base.scroll_rate, 6),
            target_lines: flag("UnitActionLines", base.target_lines),
            show_hidden: flag("ShowHidden", base.show_hidden),
            tooltips: flag("ToolTips", base.tooltips),
            sound: level("SoundVolume", base.sound),
            voice: level("VoiceVolume", base.voice),
            music: level("ScoreVolume", base.music),
        }
    }

    /// The `[Options]` and `[Audio]` sections.
    pub fn to_keys(self) -> String {
        let yes = |on: bool| if on { "yes" } else { "no" };
        format!(
            "\n[Options]\nDetailLevel={}\nDifficulty={}\nScrollRate={}\nUnitActionLines={}\nShowHidden={}\nToolTips={}\n\n[Audio]\nSoundVolume={:.6}\nVoiceVolume={:.6}\nScoreVolume={:.6}\n",
            self.detail,
            self.difficulty,
            self.scroll_rate,
            yes(self.target_lines),
            yes(self.show_hidden),
            yes(self.tooltips),
            self.sound,
            self.voice,
            self.music
        )
    }

    /// Keeps them in the settings file, with everything else there as it was.
    pub fn save(&self) {
        crate::net::Settings { prefs: *self, ..crate::net::Settings::read() }.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_survive_the_settings_file() {
        let prefs = Prefs { detail: 0, difficulty: 2, scroll_rate: 5, target_lines: false, show_hidden: true, tooltips: false, sound: 0.3, voice: 1.0, music: 0.0 };
        let text = prefs.to_keys();
        let get = |key: &str| text.lines().filter_map(|line| line.split_once('=')).find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, value)| value.to_owned());
        assert_eq!(Prefs::from_keys(&get), prefs);
        // Missing keys take the defaults; values out of range are held in it.
        let none = |_: &str| None;
        assert_eq!(Prefs::from_keys(&none), Prefs::default());
        let wild = |key: &str| (key == "ScrollRate").then(|| "40".to_owned());
        assert_eq!(Prefs::from_keys(&wild).scroll_rate, 6);
    }
}
