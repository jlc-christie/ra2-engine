//! The in-game options menu: Esc stops the game and shows it.
//!
//! The game does not run while the menu is up (there is no pause flag; the menu is
//! a modal loop); the map is replaced by the side's backdrop and the sidebar by a
//! blank one; the six buttons, their order and where they go; the abort question.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

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
    /// "What would you like to do?" after Abort Mission.
    Abort,
    /// The list of saved games, to load, save over or delete.
    Saves(crate::saves::Kind),
    /// Abort chosen: the game stays stopped while the announcer says so.
    Leaving,
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

#[derive(Clone, Copy, PartialEq, Eq)]
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
    /// Shown, but not built yet.
    Nothing,
}

/// The buttons of a menu: slot, string table key, and what it does.
fn buttons(menu: Menu) -> &'static [(usize, &'static str, Action)] {
    match menu {
        Menu::Options => &[
            (0, "GUI:GameControls", Action::Nothing),
            (1, "GUI:LoadGame", Action::Open(Kind::Load)),
            (2, "GUI:SaveGame", Action::Open(Kind::Save)),
            (3, "GUI:DeleteGame", Action::Open(Kind::Delete)),
            (4, "GUI:AbortMission", Action::AskAbort),
            (5, "GUI:ResumeMission", Action::Resume),
        ],
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
    let corner = Vec2::new(window.width() - BUTTON_INSET, FIRST_BUTTON_Y + BUTTON_SIZE.y * slot as f32);
    Rect::from_corners(corner, corner + BUTTON_SIZE)
}

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Menu>()
            .add_systems(OnEnter(Screen::InGame), |mut menu: ResMut<Menu>, mut saves: ResMut<crate::saves::SaveLoad>| {
                // RA2_MENU=options, abort, load, save or delete starts with that
                // menu up, for checking it.
                *menu = match std::env::var("RA2_MENU").as_deref() {
                    Ok("options") => Menu::Options,
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
            .add_systems(Update, run_menu.run_if(in_state(Screen::InGame)).in_set(crate::schedule::GameSet::Input));
    }
}

fn run_menu(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    mut saves: ResMut<crate::saves::SaveLoad>,
    mut shown: Local<(Menu, Vec2)>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pointer: Res<Pointer>,
    window: Single<&Window, With<PrimaryWindow>>,
    (files, rules, game_text): (Res<GameFiles>, Res<crate::files::Rules>, Res<crate::files::GameText>),
    mut images: ResMut<Assets<Image>>,
    mut wanted: ResMut<WantedCursor>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut cues: MessageWriter<Cue>,
    parts: Query<Entity, With<MenuPart>>,
    time: Res<Time<Real>>,
    (setup, mut notices, speaking, mut left): (
        Option<Res<crate::game::MatchSetup>>,
        MessageWriter<crate::notice::Notice>,
        Query<(), With<crate::audio::Speaking>>,
        Local<f64>,
    ),
) {
    let mut now = *menu;
    // Once the save is written a box says so.
    if matches!(now, Menu::Saves(Kind::Save)) && saves.ask.is_none() && saves.saving && saves.save.is_none() {
        saves.saving = false;
        saves.ask = Some(Ask::Saved);
        saves.dirty = true;
    }
    match now {
        Menu::Closing => now = Menu::Closed,
        // Up to five seconds for the line to finish.
        Menu::Leaving => {
            let waited = time.elapsed_secs_f64() - *left;
            if waited >= 5.0 || (waited >= 0.25 && speaking.is_empty()) {
                now = Menu::Closed;
                next_screen.set(Screen::MainMenu);
            }
        }
        // Esc is always the options menu.
        Menu::Closed if keys.just_pressed(KeyCode::Escape) => now = Menu::Options,
        Menu::Options | Menu::Abort if keys.just_pressed(KeyCode::Escape) => now = Menu::Closing,
        Menu::Saves(_) if keys.just_pressed(KeyCode::Escape) => now = Menu::Options,
        Menu::Options | Menu::Abort | Menu::Saves(_) => {
            wanted.0 = cursor::NORMAL;
            // A click or Enter that answers a box does nothing else.
            let entered_with_ask = saves.ask.is_some();
            // A box over the list takes every click until it is answered.
            if let (Menu::Saves(kind), Some(ask)) = (now, saves.ask.clone()) {
                let two = matches!(ask, Ask::Overwrite(_) | Ask::Delete(..));
                let enter = keys.just_pressed(KeyCode::Enter);
                let hit = pointer.0.filter(|_| mouse.just_pressed(MouseButton::Left)).and_then(|at| (0..if two { 2 } else { 1 }).find(|&n| ask_button(&window, n, two).contains(at)));
                if let Some(answer) = hit.or(enter.then_some(0)) {
                    cues.write_batch(rules.sound("GUIMainButtonSound").map(Cue::Sound));
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
            if mouse.just_pressed(MouseButton::Left) && saves.ask.is_none() && !entered_with_ask {
                let hit = buttons(now)
                    .iter()
                    .find(|(slot, ..)| pointer.0.is_some_and(|at| button_rect(&window, *slot).contains(at)));
                if let Some(&(_, key, action)) = hit {
                    if action != Action::Nothing {
                        cues.write_batch(rules.sound("GUIMainButtonSound").map(Cue::Sound));
                    }
                    match action {
                        Action::Resume => now = Menu::Closing,
                        Action::AskAbort => now = Menu::Abort,
                        Action::Quit => {
                            now = Menu::Leaving;
                            *left = time.elapsed_secs_f64();
                            cues.write(Cue::Farewell("EVA_BattleControlTerminated"));
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
                        Action::Nothing => {
                            notices.write(crate::notice::Notice::not_built(&spaced(key)));
                        }
                    }
                }
            }
        }
        Menu::Closed => {}
    }
    if now != *menu {
        *menu = now;
    }
    // A new window size lays the menu out again.
    let size = Vec2::new(window.width(), window.height());
    if (now, size) == *shown && !saves.dirty {
        return;
    }
    saves.dirty = false;
    *shown = (now, size);
    for entity in &parts {
        commands.entity(entity).despawn();
    }
    if !matches!(now, Menu::Options | Menu::Abort | Menu::Saves(_)) {
        return;
    }

    let font = game_text.font.as_ref();
    let text = |key: &str| game_text.text(key);
    let (width, height) = (window.width(), window.height());
    let side = width - SIDEBAR_WIDTH;
    commands
        .spawn((
            MenuPart,
            DespawnOnExit(Screen::InGame),
            GlobalZIndex(100),
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
                let palette = if name.starts_with("bkgd") { "uibkgd.pal" } else { hud::PALETTE };
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
            // The sidebar with its buttons and cameos blanked.
            piece(parent, &mut images, "side1.shp", 0, side, hud::CONTROLS_Y);
            let rows = hud::strip_rows(height);
            for row in 0..rows {
                piece(parent, &mut images, "side2b.shp", 0, side, FIRST_BUTTON_Y + hud::ROW_HEIGHT * row as f32);
            }

            if let Some(font) = &font {
                // The title, centred in the sidebar near the top.
                let (image, size) = hud::text_image(font, &mut images, &text("GUI:GameOptions"));
                parent.spawn((
                    hud::absolute((side + (SIDEBAR_WIDTH - size.x) / 2.0).round(), 2.0, size),
                    ImageNode::new(image),
                ));
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
                        frame(parent, button);
                        let (image, size) = hud::text_image(font, &mut images, &say(key, fallback));
                        parent.spawn((hud::absolute((button.center().x - size.x / 2.0).round(), (button.center().y - size.y / 2.0).round(), size), ImageNode::new(image)));
                    }
                }
            }
            for &(slot, key, action) in buttons(now) {
                let rect = button_rect(&window, slot);
                // Frame 0 is the button at rest.
                piece(parent, &mut images, "sidebttn.shp", 0, rect.min.x, rect.min.y);
                if let Some(font) = &font {
                    let (image, size) = hud::text_image(font, &mut images, &text(key));
                    let at = rect.center() - size / 2.0;
                    let mut label = ImageNode::new(image);
                    if action == Action::Nothing {
                        label.color = Color::srgba(1.0, 1.0, 1.0, 0.35);
                    }
                    parent.spawn((hud::absolute(at.x.round(), at.y.round(), size), label));
                }
            }
        });
}

/// A string table key as words: "GUI:GameControls" as "Game Controls".
fn spaced(key: &str) -> String {
    let name = key.rsplit(':').next().unwrap_or(key);
    let mut words = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            words.push(' ');
        }
        words.push(c);
    }
    words
}
