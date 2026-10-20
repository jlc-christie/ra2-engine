//! The in-game options menu: Esc stops the game and shows it.
//!
//! The game does not run while the menu is up (there is no pause flag; the menu is
//! a modal loop); the map is replaced by the side's backdrop and the sidebar by a
//! blank one; the six buttons, their order and where they go; the abort question.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ra2_formats::csf::Csf;
use ra2_formats::fnt::Font;
use ra2_formats::pcx::Pcx;

use crate::audio::Cue;
use crate::cursor::{self, Pointer, WantedCursor};
use crate::files::GameFiles;
use crate::hud::{self, SIDEBAR_WIDTH};
use crate::saves::{Ask, Kind, SaveLoad};
use crate::shell::Screen;

/// Which menu is up, if any.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Menu {
    #[default]
    Closed,
    /// Just closed: the game carries on from the next frame, so that the click
    /// that closed the menu is not also taken as a click on the map.
    Closing,
    Options,
    Players,
    /// The settings that can be changed while a match is paused.
    Controls,
    /// "What would you like to do?" after Abort Mission.
    Abort,
    /// The list of saved games, to load, save over or delete.
    Saves(crate::saves::Kind),
}

/// Run condition: the game is not stopped behind a menu.
pub(crate) fn playing(menu: Res<Menu>) -> bool {
    *menu == Menu::Closed
}

#[derive(Component)]
struct MenuPart;

/// Top of the first button, and the buttons' spacing and size.
// The first cameo row's top, and SIDEBTTN.SHP's size;
// the buttons start 147 pixels from the right edge.
const FIRST_BUTTON_Y: f32 = 227.0;
const BUTTON_SIZE: Vec2 = Vec2::new(125.0, 25.0);
const BUTTON_INSET: f32 = 147.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Resume,
    AskAbort,
    Quit,
    /// Open one of the lists of saves.
    Open(crate::saves::Kind),
    /// Do what the list is for to the row picked.
    Do,
    /// From a list back to the options.
    Back,
    Controls,
}

/// The buttons of a menu: slot, string table key, and what it does.
fn buttons(menu: Menu) -> &'static [(usize, &'static str, Action)] {
    match menu {
        Menu::Options => &[
            (0, "GUI:GameControls", Action::Controls),
            (1, "GUI:LoadGame", Action::Open(Kind::Load)),
            (2, "GUI:SaveGame", Action::Open(Kind::Save)),
            (3, "GUI:DeleteGame", Action::Open(Kind::Delete)),
            (4, "GUI:AbortMission", Action::AskAbort),
            (5, "GUI:ResumeMission", Action::Resume),
        ],
        Menu::Players => &[(5, "GUI:Continue", Action::Resume)],
        Menu::Controls => &[(5, "GUI:Back", Action::Back)],
        // In skirmish, Quit and Resume only.
        Menu::Abort => &[(0, "GUI:Leave", Action::Quit), (5, "GUI:ResumeMission", Action::Resume)],
        // The action button and Back.
        Menu::Saves(Kind::Load) => &[(0, "GUI:Load", Action::Do), (5, "GUI:Back", Action::Back)],
        Menu::Saves(Kind::Save) => &[(0, "GUI:Save", Action::Do), (5, "GUI:Back", Action::Back)],
        Menu::Saves(Kind::Delete) => &[(0, "GUI:Delete", Action::Do), (5, "GUI:Back", Action::Back)],
        _ => &[],
    }
}

/// The height of a row of a list of saves.
const LIST_ROW: f32 = 18.0;

/// The list box of a list of saves.
// At (24,50), 266 wide
// and 187 high, or 157 in the Save dialog, which has the name box under it.
fn list_box(window: &Window, kind: Kind) -> Rect {
    dialog_rect(window, 24.0, 50.0, 266.0, if kind == Kind::Save { 157.0 } else { 187.0 })
}

/// A rectangle of a dialog template, moved with the screen size as the engine
/// moves ordinary controls: kept centred as it is at 640x480.
fn dialog_rect(window: &Window, x: f32, y: f32, width: f32, height: f32) -> Rect {
    let at = crate::shell::dialog_rect(x, y, width, height);
    let shift = ((Vec2::new(window.width(), window.height()) - Vec2::new(800.0, 600.0)) / 2.0).round();
    Rect::from_corners(at.min + shift, at.max + shift)
}

/// Where a row of a list of saves is.
fn list_row(window: &Window, kind: Kind, row: usize) -> Rect {
    let list = list_box(window, kind);
    let corner = Vec2::new(list.min.x + 1.0, list.min.y + 1.0 + LIST_ROW * row as f32);
    Rect::from_corners(corner, Vec2::new(list.max.x - 1.0, corner.y + LIST_ROW))
}

/// The box a question is asked in, and its buttons.
fn ask_box(window: &Window) -> Rect {
    let centre = Vec2::new((window.width() - SIDEBAR_WIDTH) / 2.0, window.height() / 2.0 - 20.0);
    Rect::from_center_size(centre, Vec2::new(380.0, 130.0))
}

fn ask_button(window: &Window, n: usize, two: bool) -> Rect {
    let area = ask_box(window);
    let x = if two { area.min.x + 50.0 + 160.0 * n as f32 } else { area.center().x - 60.0 };
    Rect::from_corners(Vec2::new(x, area.max.y - 36.0), Vec2::new(x + 120.0, area.max.y - 12.0))
}

/// Asks for the save to be written, under what the name box says.
fn write(saves: &mut SaveLoad, path: std::path::PathBuf) {
    saves.description = saves.text.trim().to_owned();
    saves.save = Some(path);
    saves.saving = true;
}

/// The action button of a list of saves, or a double click or Enter.
// Saving needs a
// description, asks before writing over a save, and names a new one itself;
// deleting asks first; loading just loads.
fn act(kind: Kind, saves: &mut SaveLoad, now: &mut Menu) {
    // In the Save list the first row is the empty slot.
    let picked = if kind == Kind::Save { saves.picked.checked_sub(1) } else { Some(saves.picked) };
    let entry = picked.and_then(|row| saves.entries.get(row)).cloned();
    saves.dirty = true;
    match (kind, entry) {
        (Kind::Load, Some(entry)) => {
            saves.load = Some(entry.path);
            *now = Menu::Closing;
        }
        (Kind::Save, _) if saves.text.trim().is_empty() => saves.ask = Some(Ask::MustDescribe),
        (Kind::Save, Some(entry)) => saves.ask = Some(Ask::Overwrite(entry.path)),
        (Kind::Save, None) => write(saves, crate::saves::new_name()),
        (Kind::Delete, Some(entry)) => saves.ask = Some(Ask::Delete(entry.path, entry.description)),
        _ => {}
    }
}

fn button_rect(window: &Window, slot: usize) -> Rect {
    // The return/continue button occupies the bottom of the blank strip.
    // Its location follows the strip when the window is resized.
    let y = if slot == 5 { FIRST_BUTTON_Y + hud::ROW_HEIGHT * hud::strip_rows(window.height()) as f32 - BUTTON_SIZE.y } else { FIRST_BUTTON_Y + BUTTON_SIZE.y * slot as f32 };
    let corner = Vec2::new(window.width() - BUTTON_INSET, y);
    Rect::from_corners(corner, corner + BUTTON_SIZE)
}

/// The mouse-down target is kept until release. Dragging away cancels a button,
/// and the original pressed art remains visible for as long as it is held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Hit {
    Top(Menu),
    Side(usize),
    Ask(usize),
    Slider(usize),
    Check(usize),
}

#[derive(Resource, Default)]
pub(crate) struct ButtonPress {
    menu: Menu,
    target: Option<Hit>,
}

impl ButtonPress {
    pub(crate) fn top(&self) -> Option<Menu> {
        match self.target { Some(Hit::Top(menu)) => Some(menu), _ => None }
    }

    fn release(&mut self, menu: Menu, hit: Option<Hit>) -> Option<Hit> {
        self.target.take().filter(|target| self.menu == menu && Some(*target) == hit)
    }
}

const CONTROL_SLIDERS: &[(&str, f32, f32, f32)] = &[
    ("GUI:VisualDetails", 25.0, 45.0, 120.0),
    ("GUI:ScrollRate", 170.0, 175.0, 120.0),
    ("GUI:MusicVolume", 25.0, 250.0, 85.0),
    ("GUI:SoundVolume", 122.0, 250.0, 85.0),
    ("GUI:VoiceVolume", 220.0, 250.0, 85.0),
];

fn control_slider(window: &Window, index: usize) -> Rect {
    let &(_, x, y, width) = &CONTROL_SLIDERS[index];
    dialog_rect(window, x, y, width, 13.0)
}

fn control_check(window: &Window, index: usize) -> Rect {
    dialog_rect(window, 25.0, 160.0 + 15.0 * index as f32, 130.0, 12.0)
}

fn slider_value(prefs: &crate::prefs::Prefs, index: usize) -> (i32, i32) {
    match index {
        0 => (prefs.detail, 1),
        1 => (6 - prefs.scroll_rate, 6),
        2 => ((prefs.music * 10.0).round() as i32, 10),
        3 => ((prefs.sound * 10.0).round() as i32, 10),
        _ => ((prefs.voice * 10.0).round() as i32, 10),
    }
}

fn hit_at(window: &Window, menu: Menu, ask: Option<&Ask>, at: Vec2) -> Option<Hit> {
    if menu == Menu::Closed { return hud::top_button_at(window, at).map(Hit::Top) }
    if let Some(ask) = ask {
        let two = matches!(ask, Ask::Overwrite(_) | Ask::Delete(..));
        return (0..if two { 2 } else { 1 }).find(|&n| ask_button(window, n, two).contains(at)).map(Hit::Ask);
    }
    if menu == Menu::Controls {
        if let Some(index) = (0..CONTROL_SLIDERS.len()).find(|&i| control_slider(window, i).contains(at)) { return Some(Hit::Slider(index)) }
        if let Some(index) = (0..3).find(|&i| control_check(window, i).contains(at)) { return Some(Hit::Check(index)) }
    }
    buttons(menu).iter().find(|(slot, ..)| button_rect(window, *slot).contains(at)).map(|(slot, ..)| Hit::Side(*slot))
}

fn pcx_image(files: &GameFiles, images: &mut Assets<Image>, name: &str) -> Option<(Handle<Image>, Vec2)> {
    let pic = Pcx::parse(files.get(name)?)?;
    let data = pic.rgb.chunks_exact(3).flat_map(|rgb| [rgb[0], rgb[1], rgb[2], if rgb == [255, 0, 255] { 0 } else { 255 }]).collect();
    Some((images.add(hud::rgba_image(pic.width, pic.height, data)), Vec2::new(pic.width as f32, pic.height as f32)))
}

/// Pale yellow lettering from the imported font, with no outline or shadow.
/// The same style is used for every language and button interaction state.
fn button_text_image(font: &Font, images: &mut Assets<Image>, text: &str) -> (Handle<Image>, Vec2) {
    hud::text_image_coloured(font, images, text, [255, 244, 176])
}

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Menu>()
            .init_resource::<ButtonPress>()
            .add_systems(OnEnter(Screen::InGame), |mut menu: ResMut<Menu>, mut saves: ResMut<crate::saves::SaveLoad>, mut press: ResMut<ButtonPress>| {
                press.target = None;
                // RA2_MENU=options, abort, load, save or delete starts with that
                // menu up, for checking it.
                *menu = match std::env::var("RA2_MENU").as_deref() {
                    Ok("options") => Menu::Options,
                    Ok("players") => Menu::Players,
                    Ok("controls") => Menu::Controls,
                    Ok("abort") => Menu::Abort,
                    Ok("load") => Menu::Saves(Kind::Load),
                    Ok("save") => Menu::Saves(Kind::Save),
                    Ok("delete") => Menu::Saves(Kind::Delete),
                    _ => Menu::Closed,
                };
                if matches!(*menu, Menu::Saves(_)) {
                    saves.entries = crate::saves::list();
                    saves.picked = 0;
                }
            })
            .add_systems(PreUpdate, run_menu.after(crate::cursor::track_pointer).run_if(in_state(Screen::InGame)));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_menu(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    mut saves: ResMut<crate::saves::SaveLoad>,
    mut shown: Local<(Menu, Vec2, Option<Hit>, Option<Hit>)>,
    (keys, mouse, pointer): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>, Res<Pointer>),
    window: Single<&Window, With<PrimaryWindow>>,
    files: Res<GameFiles>,
    mut images: ResMut<Assets<Image>>,
    mut wanted: ResMut<WantedCursor>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut cues: MessageWriter<Cue>,
    parts: Query<Entity, With<MenuPart>>,
    time: Res<Time<Real>>,
    setup: Option<Res<crate::game::MatchSetup>>,
    live: (Option<Res<crate::units::Sim>>, Res<crate::endgame::HouseNames>, Option<Res<crate::units::UnitView>>),
    (mut press, mut prefs): (ResMut<ButtonPress>, ResMut<crate::prefs::Prefs>),
) {
    let mut now = *menu;
    let hit = pointer.0.and_then(|at| hit_at(&window, now, saves.ask.as_ref(), at));
    if mouse.just_pressed(MouseButton::Left) {
        press.menu = now;
        press.target = hit;
        if hit.is_some() {
            cues.write_batch(crate::audio::rules_sound(&files, "GUIMainButtonSound").map(Cue::Sound));
        }
    }
    let activated = if mouse.just_released(MouseButton::Left) {
        press.release(now, hit)
    } else { None };
    if now == Menu::Controls {
        if let (Some(Hit::Slider(index)), Some(at)) = (press.target, pointer.0) {
            let area = control_slider(&window, index);
            let (_, most) = slider_value(&prefs, index);
            let value = (((at.x - area.min.x) / area.width()).clamp(0.0, 1.0) * most as f32).round() as i32;
            let before = *prefs;
            match index {
                0 => prefs.detail = value,
                1 => prefs.scroll_rate = 6 - value,
                2 => prefs.music = value as f32 / 10.0,
                3 => prefs.sound = value as f32 / 10.0,
                _ => prefs.voice = value as f32 / 10.0,
            }
            if *prefs != before { saves.dirty = true }
        }
        if let Some(Hit::Check(index)) = activated {
            match index { 0 => prefs.tooltips = !prefs.tooltips, 1 => prefs.target_lines = !prefs.target_lines, _ => prefs.show_hidden = !prefs.show_hidden }
            saves.dirty = true;
        }
        if mouse.just_released(MouseButton::Left) { prefs.save() }
    }
    // Once the save is written a box says so.
    if matches!(now, Menu::Saves(Kind::Save)) && saves.ask.is_none() && saves.saving && saves.save.is_none() {
        saves.saving = false;
        saves.ask = Some(Ask::Saved);
        saves.dirty = true;
    }
    match now {
        Menu::Closing => now = Menu::Closed,
        // Esc is always the options menu.
        Menu::Closed if keys.just_pressed(KeyCode::Escape) => now = Menu::Options,
        Menu::Closed if matches!(activated, Some(Hit::Top(_))) => {
            if let Some(Hit::Top(menu)) = activated { now = menu }
        }
        Menu::Options | Menu::Players | Menu::Abort if keys.just_pressed(KeyCode::Escape) => now = Menu::Closing,
        Menu::Controls | Menu::Saves(_) if keys.just_pressed(KeyCode::Escape) => now = Menu::Options,
        Menu::Options | Menu::Players | Menu::Controls | Menu::Abort | Menu::Saves(_) => {
            wanted.0 = cursor::NORMAL;
            // A click or Enter that answers a box does nothing else.
            let entered_with_ask = saves.ask.is_some();
            // A box over the list takes every click until it is answered.
            if let (Menu::Saves(kind), Some(ask)) = (now, saves.ask.clone()) {
                let enter = keys.just_pressed(KeyCode::Enter);
                let answer = match activated { Some(Hit::Ask(n)) => Some(n), _ => None };
                if let Some(answer) = answer.or(enter.then_some(0)) {
                    saves.ask = None;
                    saves.dirty = true;
                    match (ask, answer) {
                        // Yes: the save is written over.
                        (Ask::Overwrite(path), 0) => write(&mut saves, path),
                        // Yes: the file goes; the list stays up while it has
                        // rows.
                        (Ask::Delete(path, _), 0) => {
                            let _ = std::fs::remove_file(&path);
                            saves.entries = crate::saves::list();
                            saves.picked = 0;
                            if saves.entries.is_empty() {
                                now = Menu::Options;
                            }
                        }
                        // The game was saved: back to the options.
                        (Ask::Saved, _) => now = Menu::Options,
                        _ => {}
                    }
                }
                let _ = kind;
            } else if let (Menu::Saves(kind), true, Some(at)) = (now, mouse.just_pressed(MouseButton::Left), pointer.0) {
                // A click on a row picks it.
                let rows = saves.entries.len() + usize::from(kind == Kind::Save);
                if let Some(row) = (0..rows).find(|&row| list_row(&window, kind, row).contains(at)) {
                    // In the Load list a double
                    // click loads.
                    let again = saves.clicked.is_some_and(|(last, at)| last == row && time.elapsed_secs_f64() - at < 0.4);
                    saves.clicked = Some((row, time.elapsed_secs_f64()));
                    saves.picked = row;
                    saves.dirty = true;
                    // Picking a save puts its
                    // description in the name box; the empty slot, the
                    // scenario's name.
                    if kind == Kind::Save {
                        saves.text = match row.checked_sub(1).and_then(|row| saves.entries.get(row)) {
                            Some(entry) => entry.description.clone(),
                            None => setup.as_ref().map(|setup| crate::saves::default_description(setup)).unwrap_or_default(),
                        };
                    }
                    if again && kind == Kind::Load {
                        act(kind, &mut saves, &mut now);
                    }
                }
            }
            // Enter in the name box is Save.
            if now == Menu::Saves(Kind::Save) && saves.ask.is_none() && keys.just_pressed(KeyCode::Enter) && !entered_with_ask {
                act(Kind::Save, &mut saves, &mut now);
            }
            if let Some(Hit::Side(slot)) = activated.filter(|_| saves.ask.is_none() && !entered_with_ask) {
                if let Some(&(_, _, action)) = buttons(now).iter().find(|(at, ..)| *at == slot) {
                    match action {
                        Action::Resume => now = Menu::Closing,
                        Action::AskAbort => now = Menu::Abort,
                        Action::Quit => {
                            now = Menu::Closed;
                            next_screen.set(Screen::MainMenu);
                        }
                        // Load and Delete are there to press only when there
                        // is a save; Save always.
                        Action::Open(kind) => {
                            saves.entries = crate::saves::list();
                            saves.picked = 0;
                            saves.ask = None;
                            // The name
                            // box starts with the scenario's name.
                            saves.text = setup.as_ref().map(|setup| crate::saves::default_description(setup)).unwrap_or_default();
                            if kind == Kind::Save || !saves.entries.is_empty() {
                                now = Menu::Saves(kind);
                            }
                        }
                        Action::Back => now = Menu::Options,
                        Action::Do => {
                            if let Menu::Saves(kind) = now {
                                act(kind, &mut saves, &mut now);
                            }
                        }
                        Action::Controls => now = Menu::Controls,
                    }
                }
            }
        }
        Menu::Closed => {}
    }
    if now != *menu {
        *menu = now;
        press.target = None;
    }
    // A new window size lays the menu out again.
    let size = Vec2::new(window.width(), window.height());
    let pressed = press.target.filter(|target| Some(*target) == hit);
    if (now, size, pressed, hit) == *shown && !saves.dirty {
        return;
    }
    saves.dirty = false;
    *shown = (now, size, pressed, hit);
    for entity in &parts {
        commands.entity(entity).despawn();
    }
    if !matches!(now, Menu::Options | Menu::Players | Menu::Controls | Menu::Abort | Menu::Saves(_)) {
        return;
    }

    let font = files.get("game.fnt").and_then(Font::parse);
    let strings = files.get("ra2.csf").and_then(Csf::parse);
    let text = |key: &str| strings.as_ref().and_then(|strings| strings.get(key)).unwrap_or(key).to_owned();
    let (width, height) = (window.width(), window.height());
    let side = width - SIDEBAR_WIDTH;
    let player_side = live.0.as_ref().and_then(|sim| sim.0.houses.get(crate::terrain::local() as usize)).map_or(0, |house| house.side);
    commands
        .spawn((
            MenuPart,
            DespawnOnExit(Screen::InGame),
            GlobalZIndex(100),
            // Input is handled in game coordinates; the artwork must not add
            // a picking layer over repair, sell or the top buttons.
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(width),
                height: Val::Px(height),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .with_children(|parent| {
            let piece = |parent: &mut ChildSpawnerCommands,
                         images: &mut Assets<Image>,
                         name: &str,
                         frame: usize,
                         x: f32,
                         y: f32| {
                // DATA: the backdrop has a palette of its own beside it in the
                // side's archive.
                let (name, palette) = hud::sidebar_background(&files, name, player_side);
                match hud::load(&files, images, name, frame, palette) {
                    Some((image, size)) => {
                        parent.spawn((hud::absolute(x, y, size), ImageNode::new(image)));
                    }
                    None => warn!("menu piece {name} not found"),
                }
            };
            // The map area: black, then the side's backdrop for this screen size.
            // The bottom bar is drawn over the backdrop, so the backdrop is
            // cut off above it.
            let mut map_area = parent.spawn((
                Node { overflow: Overflow::clip(), ..hud::absolute(0.0, 0.0, Vec2::new(side, height - hud::BOTTOM_BAR_HEIGHT)) },
                BackgroundColor(Color::BLACK),
            ));
            let backdrop = if width <= 640.0 {
                "bkgdsm.shp"
            } else if width <= 800.0 {
                "bkgdmd.shp"
            } else {
                "bkgdlg.shp"
            };
            map_area.with_children(|area| piece(area, &mut images, backdrop, 0, 0.0, 0.0));
            // The sidebar with its credits, buttons and cameos blanked.
            piece(parent, &mut images, "credits.shp", 0, side, 0.0);
            piece(parent, &mut images, "side1.shp", 0, side, hud::CONTROLS_Y);
            let rows = hud::strip_rows(height);
            for row in 0..rows {
                piece(parent, &mut images, "side2b.shp", 0, side, FIRST_BUTTON_Y + hud::ROW_HEIGHT * row as f32);
            }

            if let Some(font) = &font {
                if now == Menu::Abort {
                    // The question's box, moved with the
                    // screen size as the engine moves ordinary controls.
                    let (image, size) = hud::text_image(font, &mut images, &text("GUI:AskAbortMission"));
                    let corner = Vec2::new(66.0 + (width - 640.0) / 2.0, 222.0 + (height - 480.0) / 2.0);
                    let centre = corner + Vec2::new(345.0, 31.0) / 2.0;
                    parent.spawn((
                        hud::absolute((centre.x - size.x / 2.0).round(), (centre.y - size.y / 2.0).round(), size),
                        ImageNode::new(image),
                    ));
                }
            }
            if let (Menu::Players, Some(font), Some(sim)) = (now, &font, &live.0) {
                // The original diplomacy screen has flags, players and kills.
                // Keep the dialog centred while the backdrop stays at (0, 0).
                let rules = files.get("rules.ini").map(ra2_formats::ini::Ini::parse);
                let left = ((side - 436.0) / 2.0).round().max(8.0);
                let top = ((height - 600.0) / 2.0 + 110.0).round().max(20.0);
                let line = |parent: &mut ChildSpawnerCommands, images: &mut Assets<Image>, words: &str, column: f32, y: f32, colour| {
                    let (image, size) = hud::text_image_coloured(font, images, words, colour);
                    parent.spawn((hud::absolute(left + column, y, size), ImageNode::new(image)));
                };
                for (column, key) in [(62.0, "GUI:Player"), (376.0, "GUI:Kills")] {
                    line(parent, &mut images, &text(key), column, top, [255, 255, 0]);
                }
                for (id, house) in sim.0.houses.iter().enumerate().skip(1) {
                    let y = top + 26.0 + 32.0 * (id - 1) as f32;
                    if let Some((image, size)) = rules.as_ref().and_then(|rules| pcx_image(&files, &mut images, crate::shell::country_flag(rules, house.country as usize))) {
                        parent.spawn((hud::absolute(left, y - 5.0, size), ImageNode::new(image)));
                    }
                    let name = if house.human { live.1.0.get(id).map(String::as_str).unwrap_or("Player").to_owned() } else { text("TXT_COMPUTER") };
                    let colour = live.2.as_ref().map_or(hud::TEXT, |view| view.house_colour(id as u8));
                    line(parent, &mut images, &name, 62.0, y, colour);
                    line(parent, &mut images, &house.score.kills.to_string(), 376.0, y, colour);
                }
                if let Some(setup) = setup.as_ref().filter(|setup| !setup.map_name.is_empty()) {
                    let (image, size) = hud::text_image_coloured(font, &mut images, &setup.map_name, [255, 255, 0]);
                    parent.spawn((hud::absolute((856.0 - size.x - 117.0).min(side - size.x).max(0.0), 716.0_f32.min(height - hud::BOTTOM_BAR_HEIGHT - size.y), size), ImageNode::new(image)));
                }
            }
            if let (Menu::Controls, Some(font)) = (now, &font) {
                let label = |parent: &mut ChildSpawnerCommands, images: &mut Assets<Image>, key: &str, area: Rect| {
                    let (image, size) = hud::text_image(font, images, &text(key));
                    parent.spawn((hud::absolute(area.min.x, area.min.y, size), ImageNode::new(image)));
                };
                for (key, x, y, width) in [
                    ("GUI:DisplayOptions", 10.0, 10.0, 140.0),
                    ("GUI:UIOptions", 10.0, 140.0, 298.0),
                    ("GUI:AudioOptions", 10.0, 215.0, 298.0),
                ] {
                    label(parent, &mut images, key, dialog_rect(&window, x, y, width, 10.0));
                }
                for (index, &(key, x, y, width)) in CONTROL_SLIDERS.iter().enumerate() {
                    label(parent, &mut images, key, dialog_rect(&window, x, y - 15.0, width, 10.0));
                    let area = control_slider(&window, index);
                    let (value, most) = slider_value(&prefs, index);
                    let track = Rect::from_corners(Vec2::new(area.min.x, area.center().y - 3.0), Vec2::new(area.max.x, area.center().y + 3.0));
                    parent.spawn((
                        Node { border: UiRect::all(Val::Px(1.0)), ..hud::absolute(track.min.x, track.min.y, track.size()) },
                        BorderColor::all(Color::srgb_u8(120, 170, 255)),
                        BackgroundColor(Color::srgb_u8(0, 12, 50)),
                    ));
                    let down = pressed == Some(Hit::Slider(index));
                    if let Some((image, size)) = pcx_image(&files, &mut images, "trakgrip.pcx") {
                        let x = area.min.x + (area.width() - size.x) * value as f32 / most as f32;
                        parent.spawn((hud::absolute(x.round(), (area.center().y - size.y / 2.0).round(), size), ImageNode::new(image).with_color(if down { Color::srgb(0.65, 0.8, 1.0) } else { Color::WHITE })));
                    }
                }
                for (index, (key, on)) in [("GUI:Tooltips", prefs.tooltips), ("GUI:TargetLines", prefs.target_lines), ("GUI:ShowHidden", prefs.show_hidden)].into_iter().enumerate() {
                    let area = control_check(&window, index);
                    if let Some((image, size)) = pcx_image(&files, &mut images, if on { "cce_i.pcx" } else { "cue_i.pcx" }) {
                        let offset = if pressed == Some(Hit::Check(index)) { Vec2::ONE } else { Vec2::ZERO };
                        parent.spawn((hud::absolute(area.min.x + offset.x, area.min.y + offset.y, size), ImageNode::new(image)));
                    }
                    label(parent, &mut images, key, Rect::from_corners(area.min + Vec2::new(26.0, 0.0), area.max));
                }
            }
            // A list of saves, laid out as the original's dialogs are. The
            // heading centred over the list; the list with the description,
            // the date and the time in columns, the row picked filled in;
            // in the Save dialog the name box under it.
            if let (Menu::Saves(kind), Some(font)) = (now, &font) {
                let line = |parent: &mut ChildSpawnerCommands, images: &mut Assets<Image>, words: &str, x: f32, y: f32| {
                    if !words.is_empty() {
                        let (image, size) = hud::text_image(font, images, words);
                        parent.spawn((hud::absolute(x.round(), y.round(), size), ImageNode::new(image)));
                    }
                };
                let frame = |parent: &mut ChildSpawnerCommands, area: Rect| {
                    parent.spawn((
                        Node { border: UiRect::all(Val::Px(1.0)), ..hud::absolute(area.min.x - 1.0, area.min.y - 1.0, area.size() + 2.0) },
                        BorderColor::all(Color::srgb_u8(120, 170, 255)),
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                    ));
                };
                let heading = match kind {
                    Kind::Load => "GUI:SelectMission",
                    Kind::Save => "GUI:SaveMission",
                    Kind::Delete => "GUI:DeleteMission",
                };
                let over = dialog_rect(&window, 24.0, 16.0, 266.0, 24.0);
                let (image, size) = hud::text_image(font, &mut images, &text(heading));
                parent.spawn((hud::absolute((over.center().x - size.x / 2.0).round(), over.min.y, size), ImageNode::new(image)));
                let list = list_box(&window, kind);
                frame(parent, list);
                let empty = Some(text("TXT_EMPTY_SLOT")).filter(|words| words != "TXT_EMPTY_SLOT").unwrap_or_else(|| "[Empty slot]".to_owned());
                let rows: Vec<(String, String)> = (kind == Kind::Save).then(|| (empty, String::new())).into_iter().chain(saves.entries.iter().map(|entry| (entry.description.clone(), crate::saves::stamp(entry.modified)))).collect();
                let fit = ((list.height() - 2.0) / LIST_ROW) as usize;
                for (row, (name, when)) in rows.iter().enumerate().take(fit) {
                    let rect = list_row(&window, kind, row);
                    if row == saves.picked {
                        parent.spawn((hud::absolute(rect.min.x, rect.min.y, rect.size()), BackgroundColor(Color::srgb_u8(40, 90, 200))));
                    }
                    let short: String = name.chars().take(34).collect();
                    line(parent, &mut images, &short, rect.min.x + 2.0, rect.min.y + 2.0);
                    line(parent, &mut images, when, rect.min.x + rect.width() * 255.0 / 399.0, rect.min.y + 2.0);
                }
                if kind == Kind::Save {
                    let edit = dialog_rect(&window, 25.0, 223.0, 266.0, 14.0);
                    frame(parent, edit);
                    let shown: String = saves.text.chars().rev().take(48).collect::<Vec<_>>().into_iter().rev().collect();
                    line(parent, &mut images, &format!("{shown}_"), edit.min.x + 3.0, edit.min.y + 3.0);
                }
                // A question or notice over it all.
                if let Some(ask) = &saves.ask {
                    let say = |key: &str, fallback: &str| Some(text(key)).filter(|words| words != key).unwrap_or_else(|| fallback.to_owned());
                    let (words, two) = match ask {
                        Ask::MustDescribe => (vec![say("TXT_MUSTENTER_DESCRIPTION", "You must enter a description.")], false),
                        Ask::Overwrite(_) => (vec![say("TXT_CONFIRM_SAVE", "Overwrite the existing saved game?")], true),
                        Ask::Delete(_, name) => (vec![say("TXT_DELETE_FILE_QUERY", "Delete this saved game?"), String::new(), name.clone()], true),
                        Ask::Saved => (vec![say("TXT_GAME_WAS_SAVED", "The game was saved.")], false),
                    };
                    let area = ask_box(&window);
                    parent.spawn((
                        Node { border: UiRect::all(Val::Px(2.0)), ..hud::absolute(area.min.x, area.min.y, area.size()) },
                        BorderColor::all(Color::srgb_u8(120, 170, 255)),
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.1, 0.95)),
                    ));
                    for (n, words) in words.iter().enumerate() {
                        let (image, size) = hud::text_image(font, &mut images, words);
                        if !words.is_empty() {
                            parent.spawn((hud::absolute((area.center().x - size.x / 2.0).round(), area.min.y + 18.0 + 18.0 * n as f32, size), ImageNode::new(image)));
                        }
                    }
                    let labels: &[(&str, &str)] = if two { &[("TXT_YES", "Yes"), ("TXT_NO", "No")] } else { &[("TXT_OK", "OK")] };
                    for (n, (key, fallback)) in labels.iter().enumerate() {
                        let button = ask_button(&window, n, two);
                        let down = pressed == Some(Hit::Ask(n));
                        parent.spawn((
                            Node { border: UiRect::all(Val::Px(if down { 2.0 } else { 1.0 })), ..hud::absolute(button.min.x, button.min.y, button.size()) },
                            BorderColor::all(if down { Color::srgb_u8(35, 65, 130) } else { Color::srgb_u8(120, 170, 255) }),
                            BackgroundColor(if down { Color::srgb_u8(15, 35, 95) } else { Color::srgb_u8(0, 10, 40) }),
                        ));
                        let (image, size) = button_text_image(font, &mut images, &say(key, fallback));
                        let at = button.center() - size / 2.0 + if down { Vec2::ONE } else { Vec2::ZERO };
                        parent.spawn((hud::absolute(at.x.round(), at.y.round(), size), ImageNode::new(image)));
                    }
                }
            }
            for &(slot, key, _) in buttons(now) {
                let rect = button_rect(&window, slot);
                let down = pressed == Some(Hit::Side(slot));
                let hovered = hit == Some(Hit::Side(slot));
                piece(parent, &mut images, "sidebttn.shp", if down { 2 } else if hovered { 1 } else { 0 }, rect.min.x, rect.min.y);
                if let Some(font) = &font {
                    let (image, size) = button_text_image(font, &mut images, &text(key));
                    let at = rect.center() - size / 2.0 + if down { Vec2::ONE } else { Vec2::ZERO };
                    parent.spawn((hud::absolute(at.x.round(), at.y.round(), size), ImageNode::new(image)));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::window::WindowResolution;

    fn window(width: u32, height: u32) -> Window {
        Window { resolution: WindowResolution::new(width, height).with_scale_factor_override(1.0), ..default() }
    }

    #[test]
    fn top_menu_buttons_leave_repair_and_sell_clicks_to_the_sidebar() {
        for width in [800, 1920] {
            let window = window(width, 1080);
            let side = window.width() - SIDEBAR_WIDTH;
            assert_eq!(hit_at(&window, Menu::Closed, None, Vec2::new(side + 47.0, 29.0)), Some(Hit::Top(Menu::Players)));
            assert_eq!(hit_at(&window, Menu::Closed, None, Vec2::new(side + 119.0, 29.0)), Some(Hit::Top(Menu::Options)));
            for x in [side + 52.0, side + 116.0] {
                assert_eq!(hit_at(&window, Menu::Closed, None, Vec2::new(x, 180.0)), None);
            }
        }
    }

    #[test]
    fn continue_moves_with_the_bottom_of_the_sidebar() {
        for height in [480, 600, 1080] {
            let window = window(1920, height);
            let area = button_rect(&window, 5);
            assert!(area.min.y > button_rect(&window, 4).max.y);
            assert!(area.max.y < window.height());
            assert_eq!(hit_at(&window, Menu::Players, None, area.center()), Some(Hit::Side(5)));
        }
        assert_eq!(buttons(Menu::Players), &[(5, "GUI:Continue", Action::Resume)]);
    }

    #[test]
    fn buttons_activate_only_when_released_on_the_pressed_target() {
        let target = Some(Hit::Top(Menu::Options));
        let mut press = ButtonPress { menu: Menu::Closed, target };
        assert_eq!(press.top(), Some(Menu::Options));
        assert_eq!(press.release(Menu::Closed, target), target);
        assert_eq!(press.top(), None);
        // A drag off the button, or a menu change while holding, cancels it.
        press.target = target;
        assert_eq!(press.release(Menu::Closed, None), None);
        press.target = target;
        assert_eq!(press.release(Menu::Options, target), None);
        assert_eq!(press.release(Menu::Closed, target), None);
    }
}
