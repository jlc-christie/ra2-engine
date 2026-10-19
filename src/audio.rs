//! Sound effects, the announcer and music, from the game's own files.
//!
//! DATA: `sound.ini` names the samples and volume of each sound, `eva.ini` the
//! announcer's lines per side, `theme.ini` the music tracks; `RA2.INI [Audio]`
//! holds the player's three volume settings.
//!
//! The rules for sounds and the announcer follow the original game.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::audio::Volume;
use bevy::prelude::*;
use ra2_formats::audio::{AudioIndex, Pcm};
use ra2_formats::ini::Ini;

use crate::files::GameFiles;
use crate::shell::Screen;

/// Something to be heard.
#[derive(Message, Clone, Debug)]
pub enum Cue {
    /// A `sound.ini` section, such as a unit's `VoiceSelect`.
    Sound(String),
    /// The same at a share of its volume.
    SoundAt(String, f32),
    /// A sound made at a point on the map (scene pixels): how loud it is depends
    /// on how far that is from the view.
    SoundOn(String, Vec2),
    /// An `eva.ini` section, such as `EVA_ConstructionComplete`.
    Eva(&'static str),
    /// Leaving the game: the music and sounds stop and this line is spoken at
    /// once, over anything the announcer was saying.
    Farewell(&'static str),
}

/// An `[AudioVisual]` or `[General]` key of rules.ini that names a sound.
pub fn rules_sound(files: &GameFiles, key: &str) -> Option<String> {
    let rules = Ini::parse(files.get("rules.ini")?);
    ["AudioVisual", "General"]
        .iter()
        .find_map(|section| rules.get(section, key))
        .and_then(|value| value.split(',').next())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// Every name in an `[AudioVisual]` (or `[General]`) list in rules.ini.
pub fn rules_list(files: &GameFiles, key: &str) -> Vec<String> {
    let Some(rules) = files.get("rules.ini").map(Ini::parse) else {
        return Vec::new();
    };
    ["AudioVisual", "General"]
        .iter()
        .find_map(|section| rules.get(section, key))
        .map(|value| value.split(',').map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()).collect())
        .unwrap_or_default()
}

#[derive(Resource)]
struct Audio {
    index: Option<AudioIndex>,
    sounds: Option<Ini>,
    eva: Option<Ini>,
    themes: Option<Ini>,
    /// `GUIMainButtonSound`.
    button_sound: Option<String>,
    sound_volume: f32,
    voice_volume: f32,
    score_volume: f32,
    decoded: HashMap<String, Handle<AudioSource>>,
    /// Advances with each random pick.
    seed: u32,
    /// Position in the in-game play list.
    track: usize,
    /// Sounds asked for since the last service pass, and when that was.
    pending: Vec<(String, f32, Option<Vec2>)>,
    serviced: f64,
    /// The announcer: lines waiting (name, priority, queued rather than in the
    /// single ordinary slot), and when the last line ended.
    eva_waiting: Vec<(&'static str, u8, bool)>,
    /// A line asked for while nothing was playing or waiting: it starts first.
    eva_first: Option<&'static str>,
    eva_playing: Option<&'static str>,
    eva_ended: f64,
    /// The game is being left: no more in-game music.
    leaving: bool,
}

/// A sound effect that is playing: its `sound.ini` section and when it started.
#[derive(Component)]
struct Playing(String, f64);

/// The samples still to come after the one an entity is playing.
#[derive(Component)]
struct Chain {
    rest: Vec<Handle<AudioSource>>,
    level: f32,
    speed: f32,
}

/// Starts the next sample of a sound made of several once the last has ended.
fn play_chains(mut commands: Commands, mut done: Query<(Entity, &mut Chain), Without<AudioPlayer>>) {
    for (entity, mut chain) in &mut done {
        if chain.rest.is_empty() {
            commands.entity(entity).despawn();
            continue;
        }
        let next = chain.rest.remove(0);
        let settings = if chain.rest.is_empty() { PlaybackSettings::DESPAWN } else { PlaybackSettings::REMOVE };
        commands.entity(entity).insert((AudioPlayer::new(next), settings.with_volume(Volume::Linear(chain.level)).with_speed(chain.speed)));
        if chain.rest.is_empty() {
            commands.entity(entity).remove::<Chain>();
        }
    }
}

/// The announcer's line that is playing.
#[derive(Component)]
pub struct Speaking;

/// Marks the entity playing music, with which it is: 0 the menus', 1 the
/// in-game play list, 2 the score screen's.
#[derive(Component)]
struct Music(u8);

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Cue>();
        // RA2_MUTE=1 turns all sound off, for automated runs.
        if std::env::var_os("RA2_MUTE").is_some() {
            return;
        }
        app.add_systems(Startup, setup)
            .add_systems(Update, (follow_prefs, play_cues, play_chains, play_music).run_if(resource_exists::<Audio>))
            .add_observer(on_button);
    }
}

/// The three volumes follow the Options screen, the music that is playing
/// at once.
// The volume sliders take effect as they move.
fn follow_prefs(prefs: Res<crate::prefs::Prefs>, mut audio: ResMut<Audio>, mut music: Query<&mut AudioSink, With<Music>>) {
    if !prefs.is_changed() {
        return;
    }
    audio.sound_volume = prefs.sound;
    audio.voice_volume = prefs.voice;
    audio.score_volume = prefs.music;
    for mut sink in &mut music {
        sink.set_volume(Volume::Linear(prefs.music));
    }
}

fn setup(mut commands: Commands, files: Res<GameFiles>, prefs: Res<crate::prefs::Prefs>) {
    let ini = |name: &str| files.get(name).map(Ini::parse);
    let volume = |key: &str, _default: f32| match key {
        "SoundVolume" => prefs.sound,
        "VoiceVolume" => prefs.voice,
        _ => prefs.music,
    };
    commands.insert_resource(Audio {
        index: files.get("audio.idx").and_then(AudioIndex::parse),
        sounds: ini("sound.ini"),
        eva: ini("eva.ini"),
        themes: ini("theme.ini"),
        button_sound: rules_sound(&files, "GUIMainButtonSound"),
        sound_volume: volume("SoundVolume", 0.7),
        voice_volume: volume("VoiceVolume", 0.8),
        score_volume: volume("ScoreVolume", 0.6),
        decoded: HashMap::new(),
        seed: 0x2545_f491,
        track: 0,
        pending: Vec::new(),
        serviced: 0.0,
        eva_waiting: Vec::new(),
        eva_first: None,
        eva_playing: None,
        eva_ended: -1.0,
        leaving: false,
    });
}

impl Audio {
    fn random(&mut self, count: usize) -> usize {
        // Not part of the simulation, so any generator will do.
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 16) as usize % count.max(1)
    }

    /// A sample by name: from the sound bank, or else a wave file of that name.
    fn sample(
        &mut self,
        files: &GameFiles,
        sources: &mut Assets<AudioSource>,
        name: &str,
    ) -> Option<Handle<AudioSource>> {
        let name = name.to_lowercase();
        if let Some(handle) = self.decoded.get(&name) {
            return Some(handle.clone());
        }
        let banked = || self.index.as_ref()?.decode(&name, files.get("audio.bag")?);
        let pcm = banked().or_else(|| Pcm::from_wav(files.get(&format!("{name}.wav"))?))?;
        let handle = sources.add(AudioSource { bytes: Arc::from(pcm.to_wav()) });
        self.decoded.insert(name, handle.clone());
        Some(handle)
    }
}

/// Starts sounds and announcer lines.
// New sounds start at most
// once every 33 ms; a sound's volume is its entry's `Volume` (80 if none) times the
// caller's share, less a random `VShift`; pitch varies by `FShift`; only `random`
// in `Control` picks among the samples, otherwise the first is used; with `Limit`
// (5 if none) of an entry already playing a new one is dropped, unless it has
// `interrupt`, when the oldest is stopped instead.
// The announcer speaks one line at a time,
// the next starting half a second after the last ended. A line asked for again
// while playing or waiting is ignored. `Type=QUEUE` lines wait their turn; an
// ordinary line waits in a single place, which a later one takes only if its
// priority is higher, and is otherwise dropped.
fn play_cues(
    mut commands: Commands,
    mut cues: MessageReader<Cue>,
    mut audio: ResMut<Audio>,
    files: Res<GameFiles>,
    time: Res<Time<Real>>,
    setup: Option<Res<crate::game::GameSetup>>,
    centre: Option<Res<crate::game::ViewCentre>>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut sources: ResMut<Assets<AudioSource>>,
    playing: Query<(Entity, &Playing)>,
    speaking: Query<Entity, With<Speaking>>,
    music: Query<Entity, With<Music>>,
) {
    let now = time.elapsed_secs_f64();
    let audio = &mut *audio;
    for cue in cues.read() {
        match cue {
            Cue::Sound(section) => audio.pending.push((section.clone(), 1.0, None)),
            Cue::SoundAt(section, share) => audio.pending.push((section.clone(), *share, None)),
            Cue::SoundOn(section, point) => audio.pending.push((section.clone(), 1.0, Some(*point))),
            Cue::Farewell(line) => {
                for entity in playing.iter().map(|(entity, _)| entity).chain(&speaking).chain(&music) {
                    commands.entity(entity).despawn();
                }
                audio.leaving = true;
                audio.eva_waiting = vec![(line, 3, true)];
                audio.eva_first = Some(*line);
                audio.eva_playing = None;
                audio.eva_ended = f64::NEG_INFINITY;
            }
            Cue::Eva(line) => {
                if audio.eva_playing == Some(*line) || audio.eva_waiting.iter().any(|(waiting, ..)| waiting == line) {
                    continue;
                }
                let get = |key: &str| audio.eva.as_ref().and_then(|eva| eva.get(line, key)).map(|v| v.trim().to_uppercase());
                let priority = match get("Priority").as_deref() {
                    Some("LOW") => 0,
                    Some("IMPORTANT") => 2,
                    Some("CRITICAL") => 3,
                    _ => 1,
                };
                let queued = matches!(get("Type").as_deref(), Some("QUEUE" | "QUEUED_INTERRUPT")) || priority == 3;
                // Each request
                // tries to start the next line at once, so one made while
                // nothing plays or waits is the one that starts, whatever is
                // asked for after it in the same frame ("player defeated"
                // before "you are victorious").
                if audio.eva_playing.is_none() && audio.eva_waiting.is_empty() && audio.eva_first.is_none() {
                    audio.eva_first = Some(*line);
                }
                if queued {
                    audio.eva_waiting.push((line, priority, true));
                } else {
                    // The one ordinary place.
                    match audio.eva_waiting.iter().position(|(_, _, queued)| !queued) {
                        None => audio.eva_waiting.push((line, priority, false)),
                        Some(at) if audio.eva_waiting[at].1 < priority => audio.eva_waiting[at] = (line, priority, false),
                        Some(_) => {}
                    }
                }
            }
        }
    }

    // The announcer.
    if speaking.is_empty() {
        if audio.eva_playing.take().is_some() {
            audio.eva_ended = now;
        }
        if now - audio.eva_ended >= 0.5 && !audio.eva_waiting.is_empty() {
            // Critical lines, then the ordinary place, then the queue by priority.
            let rank = |&(_, priority, queued): &(&str, u8, bool)| match (priority, queued) {
                (3, _) => 0,
                (_, false) => 1,
                (priority, true) => 5 - priority,
            };
            let first = audio.eva_first.take().and_then(|first| audio.eva_waiting.iter().position(|(line, ..)| *line == first));
            let at = first.unwrap_or_else(|| (0..audio.eva_waiting.len()).min_by_key(|&at| rank(&audio.eva_waiting[at])).unwrap_or(0));
            let (line, ..) = audio.eva_waiting.remove(at);
            // `Allied=` for the first side, `Russian=` for the others.
            let soviet = setup.as_ref().is_some_and(|setup| setup.options.country >= 5);
            let key = if soviet { "Russian" } else { "Allied" };
            let name = audio.eva.as_ref().and_then(|eva| eva.get(line, key)).map(|name| name.trim().to_owned());
            if let Some(handle) = name.and_then(|name| audio.sample(&files, &mut sources, &name)) {
                commands.spawn((
                    Speaking,
                    AudioPlayer::new(handle),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(audio.voice_volume)),
                ));
                audio.eva_playing = Some(line);
            }
        }
    }

    // Sound effects, at the service rate.
    if now - audio.serviced < 0.033 || audio.pending.is_empty() {
        return;
    }
    audio.serviced = now;
    let mut live: Vec<(Entity, String, f64)> = playing.iter().map(|(entity, sound)| (entity, sound.0.clone(), sound.1)).collect();
    // The middle of the map view and half its size, in scene pixels.
    let view = centre.map(|centre| (centre.0, Vec2::new(window.width() - 168.0, window.height() - 32.0) / 2.0));
    for (section, share, point) in std::mem::take(&mut audio.pending) {
        let Some(sounds) = &audio.sounds else {
            continue;
        };
        let get = |key: &str| sounds.get(&section, key).or_else(|| sounds.get("Defaults", key));
        let number = |key: &str, default: f32| get(key).and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok()).unwrap_or(default);
        // DATA: `Sounds` lists the samples; "$" and "#" mark where they are kept.
        let names: Vec<String> = sounds
            .get(&section, "Sounds")
            .unwrap_or("")
            .split_whitespace()
            .map(|name| name.trim_start_matches(['$', '#']).to_owned())
            .collect();
        if names.is_empty() {
            continue;
        }
        let control = get("Control").unwrap_or("").to_lowercase();
        let has = |word: &str| control.split_whitespace().any(|w| w == word);
        let (random, interrupt) = (has("random"), has("interrupt"));
        // A sound made on the map is at
        // full strength while its place is in the view (unless its `Type` has
        // LOCAL, when it fades from the middle of the view), and beyond that
        // fades to nothing over its `Range` (10 cells unless given, 60 pixels a
        // cell), counting distance up and down the screen double. A GLOBAL sound
        // is never quieter than its `MinVolume`. Under a twentieth it is not
        // played.
        // DATA: `[Defaults]` of sound.ini for keys an entry does not give.
        let share = match (point, view.as_ref()) {
            (Some(point), Some((centre, half))) => {
                let default = |key: &str| sounds.get("Defaults", key).map(|v| v.to_lowercase());
                let kind = get("Type").map(|v| v.to_lowercase()).or_else(|| default("Type")).unwrap_or_default();
                let flag = |word: &str| kind.split_whitespace().any(|w| w == word);
                let range = get("Range").or(sounds.get("Defaults", "Range")).and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok()).unwrap_or(10.0) * 60.0;
                let floor = get("MinVolume").or(sounds.get("Defaults", "MinVolume")).and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok()).unwrap_or(20.0) / 100.0;
                let (mut ax, mut ay) = ((point.x - centre.x).abs(), (point.y - centre.y).abs());
                if !flag("local") {
                    ax = (ax - half.x).max(0.0);
                    ay = (ay - half.y).max(0.0);
                }
                ay *= 2.0;
                let mut heard = if range > 0.0 && ax < range && ay < range { (range - ax.max(ay)) / range } else { 0.0 };
                if flag("global") {
                    heard = heard.max(floor.clamp(0.0, 1.0));
                }
                if heard < 0.05 {
                    continue;
                }
                share * heard
            }
            _ => share,
        };
        let limit = number("Limit", 5.0) as usize;
        let volume = (number("Volume", 80.0) / 100.0).clamp(0.0, 1.0);
        let shift = number("VShift", 0.0).clamp(0.0, 100.0) as usize;
        let pitch: Vec<i32> = get("FShift")
            .map(|v| v.split_whitespace().filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default();

        let mut same: Vec<(Entity, f64)> = live.iter().filter(|(_, entry, _)| *entry == section).map(|&(e, _, t)| (e, t)).collect();
        if limit > 0 && same.len() >= limit {
            if !interrupt {
                continue;
            }
            same.sort_by(|a, b| a.1.total_cmp(&b.1));
            let oldest = same[0].0;
            commands.entity(oldest).despawn();
            live.retain(|(entity, ..)| *entity != oldest);
        }
        // With `attack` in `Control` the first `Attack` samples (at least one)
        // are lead-ins, and with `decay` the last `Decay` are lead-outs; one of
        // each is played before and after one of the samples between.
        let lead_in = if has("attack") { (number("Attack", 0.0) as usize).max(1).min(names.len()) } else { 0 };
        let lead_out = if has("decay") { (number("Decay", 0.0) as usize).max(1).min(names.len() - lead_in) } else { 0 };
        let body = lead_in..(names.len() - lead_out).max(lead_in + 1).min(names.len());
        let pick = body.start + if random { audio.random(body.len().max(1)) } else { 0 };
        let pick = pick.min(names.len() - 1);
        let mut sequence: Vec<usize> = Vec::new();
        if lead_in > 0 {
            sequence.push(audio.random(lead_in));
        }
        sequence.push(pick);
        if lead_out > 0 {
            sequence.push(names.len() - lead_out + audio.random(lead_out));
        }
        let quieter = 1.0 - audio.random(shift + 1) as f32 / 100.0;
        let speed = match pitch[..] {
            [low, high] if high > low => 1.0 + (low + audio.random((high - low + 1) as usize) as i32) as f32 / 100.0,
            [only, ..] => 1.0 + only as f32 / 100.0,
            [] => 1.0,
        };
        let mut handles: Vec<Handle<AudioSource>> = Vec::new();
        for &index in &sequence {
            match audio.sample(&files, &mut sources, &names[index]) {
                Some(handle) => handles.push(handle),
                None => warn!("sound {} not found", names[index]),
            }
        }
        if handles.is_empty() {
            continue;
        }
        let first = handles.remove(0);
        let level = volume * share * quieter * audio.sound_volume;
        let settings = if handles.is_empty() { PlaybackSettings::DESPAWN } else { PlaybackSettings::REMOVE };
        let mut spawned = commands.spawn((
            Playing(section.clone(), now),
            AudioPlayer::new(first),
            settings.with_volume(Volume::Linear(level)).with_speed(speed.max(0.1)),
        ));
        if !handles.is_empty() {
            spawned.insert(Chain { rest: handles, level, speed: speed.max(0.1) });
        }
        let entity = spawned.id();
        live.push((entity, section, now));
    }
}

/// Keeps music going: the menu theme in the menus, the play list in a game.
fn play_music(
    mut commands: Commands,
    mut audio: ResMut<Audio>,
    screen: Res<State<Screen>>,
    files: Res<GameFiles>,
    mut sources: ResMut<Assets<AudioSource>>,
    playing: Query<(Entity, &Music)>,
) {
    // The score screen starts the
    // theme named SCORE.
    let which = match *screen.get() {
        Screen::InGame => 1,
        Screen::Score => 2,
        _ => 0,
    };
    let in_game = which == 1;
    if !in_game {
        audio.leaving = false;
    } else if audio.leaving {
        return;
    }
    let mut current = false;
    for (entity, music) in &playing {
        if music.0 == which {
            current = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    if current {
        return;
    }
    let Some(themes) = &audio.themes else {
        return;
    };
    let theme = if in_game {
        let list: Vec<String> = themes
            .section("Themes")
            .iter()
            .map(|(_, name)| name.clone())
            .filter(|name| !name.is_empty())
            .filter(|name| themes.get(name, "Normal").is_none_or(|v| v.trim().eq_ignore_ascii_case("yes")))
            .collect();
        if list.is_empty() {
            return;
        }
        if audio.track == 0 {
            audio.track = 1 + audio.random(list.len());
        }
        audio.track += 1;
        list[audio.track % list.len()].clone()
    } else if which == 2 {
        "SCORE".to_owned()
    } else {
        "INTRO".to_owned()
    };
    let Some(sound) = audio.themes.as_ref().and_then(|themes| themes.get(&theme, "Sound")).map(str::to_owned)
    else {
        return;
    };
    let Some(handle) = audio.sample(&files, &mut sources, &sound) else {
        warn!("music {sound} not found");
        // Leave a silent marker so a missing track is not retried every frame.
        commands.spawn(Music(which));
        return;
    };
    let settings = if in_game { PlaybackSettings::DESPAWN } else { PlaybackSettings::LOOP };
    commands.spawn((
        Music(which),
        AudioPlayer::new(handle),
        settings.with_volume(Volume::Linear(audio.score_volume)),
    ));
}

/// Every button makes the menu click.
// DATA: rules.ini `GUIMainButtonSound`.
fn on_button(
    activate: On<bevy::ui_widgets::Activate>,
    on_press: Query<(), With<crate::shell::ClicksOnPress>>,
    audio: Option<Res<Audio>>,
    mut cues: MessageWriter<Cue>,
) {
    // The menus' big buttons have clicked already, as they went down.
    if on_press.contains(activate.entity) {
        return;
    }
    if let Some(sound) = audio.and_then(|audio| audio.button_sound.clone()) {
        cues.write(Cue::Sound(sound));
    }
}
