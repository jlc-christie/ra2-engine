//! The menu shell, drawn with the original art, font and strings at the original
//! 800x600 layout. Navigation works; everything that would start or change a game
//! only logs.
//!
//! Control positions come from the dialog templates in the original game, which are laid out
//! for 640x480; they are converted to pixels and centred in the 800x600 left panel.
//!

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input::mouse::MouseWheel;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;
use ra2_formats::csf::Csf;
use ra2_formats::fnt::Font;
use ra2_formats::ini::Ini;
use ra2_formats::pal::Palette;
use ra2_formats::pcx::Pcx;
use ra2_formats::shp::Shp;

use crate::files::GameFiles;
use crate::game::GameSetup;

// The shell is laid out for at most 800x600 and the
// sidebar is 168 px wide.
const SCREEN: Vec2 = Vec2::new(800.0, 600.0);
const SIDEBAR_X: f32 = 632.0;
const SIDEBAR_WIDTH: f32 = 168.0;
// The slots start at 199 and are 42
// high; there are (600 - 199) / 42 = 9 of them and the bottom piece goes below the
// last, at 577, so only its top rows show.
const SIDEBAR_TOP_HEIGHT: f32 = 199.0;
const SLOT_HEIGHT: f32 = 42.0;
const SLOT_COUNT: usize = 9;
/// The slot of each dialog's last button (Exit Game, Main Menu, Back, Cancel).
// It is put in the bottom slot whatever its template
// position.
const LAST_BUTTON_SLOT: usize = 8;
// Buttons are right-aligned and 0x9c px from the edge.
const BUTTON_WIDTH: f32 = 156.0;

/// Offset that centres the 640x480 dialog's left panel (472x448) in the 800x600 one.
// Ordinary controls move by half the difference between the screen size and the
// design size.
const PANEL_OFFSET: Vec2 = Vec2::new(80.0, 60.0);

// The engine draws text and borders itself, probably from a named colour scheme in
// rules.ini.
const TEXT: [u8; 3] = [255, 255, 0];
// The shell's colours are constants: text 255,255,0,
// border and highlight 255,0,0, disabled text and border 159,0,0.
const TEXT_DISABLED: [u8; 3] = [159, 0, 0];
const EDGE: Color = Color::srgb_u8(255, 0, 0);
const SELECTED: Color = Color::srgb_u8(255, 0, 0);

const LIST_ROW_HEIGHT: f32 = 18.0;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Screen {
    #[default]
    MainMenu,
    SinglePlayer,
    Skirmish,
    ChooseMap,
    /// Create or join a network game.
    Network,
    /// The games on the meeting-place server.
    Internet,
    InGame,
    /// Passed through on the way from one game into a loaded one.
    Loading,
    /// The player's country's picture, shown while a skirmish loads.
    LoadScreen,
    /// The figures after a skirmish.
    Score,
    /// The Options screen: display, game, interface and audio settings.
    Options,
    /// The list of saved games, to pick one to load.
    LoadGame,
}

#[derive(Component, Clone, Copy)]
enum MenuAction {
    Goto(Screen),
    /// Commits the highlighted map and returns to the skirmish screen.
    UseMap,
    /// Starts a game on the selected map.
    StartGame,
    /// Create a network game: on to the skirmish screen to set it up.
    HostGame,
    /// Join the network game at the address typed.
    JoinGame,
    /// Give up creating or joining, back to the Network screen.
    LeaveLobby,
    /// Typing on the Network and Internet screens goes to this box.
    Focus(crate::net::Focus),
    /// Load the save picked in the list.
    LoadSaved,
    /// Internet: create a game on the server, join the one picked, pick one
    /// from the list, ask for the list again.
    HostInternet,
    JoinInternet,
    PickGame(u32),
    Refresh,
    Exit,
    /// Not implemented yet: says so on screen (see `notice`).
    Mock(&'static str),
    /// Says something on screen and does nothing else.
    Tell(&'static str),
}

/// One of the sidebar's eight slots: which it is, whether it holds a button, every
/// frame of SDBTNANM.SHP, and when the mouse came to rest on it.
#[derive(Component)]
struct Slot {
    index: usize,
    frames: std::sync::Arc<Vec<Handle<Image>>>,
    was_pressed: bool,
}

/// Set when a screen is only being drawn again after an option changed, so that
/// its buttons do not slide in again.
// The slide runs when a dialog
// appears or goes, never for something done to one of its controls.
#[derive(Resource, Default)]
struct Redraw(bool);

/// A slot button's words, hidden while the buttons slide.
#[derive(Component)]
struct SlotText;

/// Where a slot button's words sit when the button is up.
#[derive(Component)]
struct SlotTextHome(f32, f32);

/// A button that makes its sound as it goes down, not when it is let go.
#[derive(Component)]
pub(crate) struct ClicksOnPress;

/// The sidebar buttons' slide: when the slide in began, and the action waiting on
/// a slide out with when that began.
#[derive(Resource, Default)]
struct Slide {
    started: f64,
    leaving: Option<(MenuAction, f64)>,
}

/// A step of the slide is a redraw and a 30 ms sleep.
const SLIDE_STEP: f64 = 0.032;

#[derive(Component)]
struct CheckBox {
    on: bool,
    checked: Handle<Image>,
    unchecked: Handle<Image>,
}

/// A control on the skirmish screen that does something when clicked.
#[derive(Component, Clone, Copy)]
enum OptionControl {
    /// The arrow of a combo box: opens or closes its list.
    Combo(Combo),
    /// Anywhere outside an open list.
    CloseCombo,
    /// The arrows of an open list's scroll bar.
    ScrollCombo(i32),
    ShortGame,
    McvRepacks,
    Crates,
    SuperWeapons,
    /// The Options screen's check boxes.
    Tooltips,
    TargetLines,
    ShowHidden,
    FullScreen,
}

/// RA2_COUNTRY, RA2_COLOUR, RA2_GAMESPEED, RA2_SHORT and RA2_OPPONENTS set those options when starting
/// straight into a game, for checking them.
fn options_from_env() -> crate::game::Options {
    let number = |name: &str| std::env::var(name).ok().and_then(|n| n.trim().parse::<i32>().ok());
    let mut options = crate::game::Options::default();
    options.country = number("RA2_COUNTRY").unwrap_or(options.country);
    options.colour = number("RA2_COLOUR").unwrap_or(options.colour);
    options.speed = number("RA2_GAMESPEED").unwrap_or(options.speed);
    // RA2_SHORT=0 or 1 sets Short Game.
    options.short_game = number("RA2_SHORT").map_or(options.short_game, |n| n != 0);
    // RA2_OPPONENTS=n plays against n easy computers.
    if let Some(count) = number("RA2_OPPONENTS") {
        for (index, row) in options.opponents.iter_mut().enumerate() {
            row.level = if (index as i32) < count { 2 } else { -1 };
        }
    }
    options
}

/// The countries a player can be, in `[Countries]` order: string table key and
/// flag picture.
const COUNTRIES: [(&str, &str); 9] = [
    ("Name:Americans", "usai.pcx"),
    ("Name:Alliance", "japi.pcx"),
    ("Name:French", "frai.pcx"),
    ("Name:Germans", "geri.pcx"),
    ("Name:British", "gbri.pcx"),
    ("Name:Africans", "djbi.pcx"),
    ("Name:Arabs", "arbi.pcx"),
    ("Name:Confederation", "lati.pcx"),
    ("Name:Russians", "rusi.pcx"),
];

/// The swatch of each colour a player can have.
// Gold, red, blue, green, orange, sky blue,
// purple, pink.
const SWATCHES: [[u8; 3]; 8] = [
    [221, 226, 13], [255, 25, 25], [42, 116, 226], [62, 209, 46],
    [255, 160, 25], [50, 215, 230], [149, 40, 189], [255, 154, 235],
];

#[derive(Component)]
struct MapEntry(usize);

/// A row of the game mode list.
#[derive(Component)]
struct ModeEntry(usize);

#[derive(Component)]
struct MapListContent {
    rows: usize,
    visible_rows: usize,
}

#[derive(Resource, Default)]
struct MapChoice {
    /// The map the skirmish will use.
    selected: usize,
    /// The map highlighted on the Choose Map screen.
    highlighted: usize,
    scroll: usize,
    /// The game mode chosen in the other list.
    mode: usize,
}

#[derive(Clone)]
struct Pic {
    image: Handle<Image>,
    size: Vec2,
}

struct MultiMap {
    /// File name without extension.
    id: String,
    /// Display name.
    name: String,
    /// `MaxPlayers` of its missions.pkt entry.
    max_players: usize,
}

#[derive(Resource)]
struct Shell {
    files: GameFiles,
    font: Font,
    strings: Csf,
    /// The maps offered for standard game modes, in the game's order.
    maps: Vec<MultiMap>,
    /// Display names of the game modes that allow computer players, and the
    /// rules file of each.
    modes: Vec<String>,
    mode_rules: Vec<String>,
    pics: HashMap<String, Pic>,
}

pub struct ShellPlugin;

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        // RA2_SCREEN picks the screen to open on, for checking a layout directly.
        let first = match std::env::var("RA2_SCREEN").as_deref() {
            Ok("single-player") => Screen::SinglePlayer,
            Ok("skirmish") => Screen::Skirmish,
            Ok("choose-map") => Screen::ChooseMap,
            Ok("network") => Screen::Network,
            Ok("internet") => Screen::Internet,
            Ok("game") => Screen::InGame,
            Ok("score") => Screen::Score,
            Ok("options") => Screen::Options,
            Ok("loading") => Screen::LoadScreen,
            Ok("load-game") => Screen::LoadGame,
            _ => Screen::MainMenu,
        };
        let shell = Shell::load(app.world().resource::<GameFiles>().clone());
        // RA2_MAP names the map file to load when starting directly in a game.
        let map = std::env::var("RA2_MAP")
            .ok()
            .or_else(|| shell.maps.first().map(|map| map.id.clone()))
            .unwrap_or_default();
        app.insert_resource(shell)
            .insert_resource(GameSetup { map, map_name: String::new(), options: options_from_env(), max_players: 8, mode_rules: std::env::var("RA2_MODE").unwrap_or_else(|_| "MPBattle.ini".into()) })
            .insert_state(first)
            .init_resource::<MapChoice>()
            .insert_resource(OpenCombo(match std::env::var("RA2_COMBO").as_deref() {
                // RA2_COMBO=country or colour starts with that list open, for checking it.
                Ok("country") => Some((Combo::Country(0), 0)),
                Ok("colour") => Some((Combo::Colour(0), 0)),
                Ok("level") => Some((Combo::Level(1), 0)),
                Ok("resolution") => Some((Combo::Resolution, 0)),
                _ => None,
            }))
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(Screen::MainMenu), spawn_main_menu)
            .add_systems(OnEnter(Screen::Score), spawn_score)
            .add_systems(OnEnter(Screen::SinglePlayer), spawn_single_player)
            .add_systems(OnEnter(Screen::Skirmish), spawn_skirmish)
            .add_systems(OnEnter(Screen::ChooseMap), spawn_choose_map)
            .add_systems(OnEnter(Screen::Network), (spawn_network, start_slide))
            .add_systems(OnEnter(Screen::Internet), (spawn_internet, start_slide))
            .add_systems(Update, network_screens.run_if(not(in_state(Screen::InGame))))
            .init_resource::<Slide>()
            .init_resource::<Redraw>()
            .add_systems(OnEnter(Screen::MainMenu), start_slide)
            .add_systems(OnEnter(Screen::SinglePlayer), start_slide)
            .add_systems(OnEnter(Screen::LoadGame), (spawn_load_game, start_slide))
            .add_systems(OnEnter(Screen::Skirmish), start_slide)
            .add_systems(OnEnter(Screen::ChooseMap), start_slide)
            .add_systems(OnEnter(Screen::Options), (spawn_options, start_slide))
            .add_systems(OnEnter(Screen::LoadScreen), spawn_load_screen)
            .add_systems(Update, (fill_load_bar, leave_load_screen).run_if(in_state(Screen::LoadScreen)))
            .add_systems(
                Update,
                (
                    fit_scale.run_if(not(in_state(Screen::InGame))),
                    slot_frames.run_if(not(in_state(Screen::InGame))),
                    finish_leaving.run_if(not(in_state(Screen::InGame))),
                    highlight_combo_rows,
                    run_meter,
                    drag_sliders.run_if(|screen: Option<Res<State<Screen>>>| screen.is_some_and(|s| matches!(s.get(), Screen::Skirmish | Screen::Options))),
                    map_list_scroll,
                    map_entry_colors,
                    mode_entry_colors,
                    follow_highlight.run_if(in_state(Screen::ChooseMap)),
                    (save_entry_colors, save_list_scroll).run_if(in_state(Screen::LoadGame)),
                ),
            )
            .add_observer(on_menu_action)
            .add_observer(on_check_box)
            .add_observer(on_option)
            .add_observer(on_map_entry)
            .add_observer(on_mode_entry)
            .add_observer(on_save_entry);
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Scales the 800x600 shell by the largest whole number that fits the window.
// Not original behaviour: the original centres the unscaled shell. Scaling is a deliberate addition for modern displays.
fn fit_scale(windows: Query<&Window, With<PrimaryWindow>>, mut scale: ResMut<UiScale>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let fit = (window.width() / SCREEN.x).min(window.height() / SCREEN.y);
    let wanted = fit.floor().max(1.0);
    if scale.0 != wanted {
        scale.0 = wanted;
    }
}

fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

impl Shell {
    fn load(files: GameFiles) -> Self {
        let font = Font::parse(files.get("game.fnt").expect("game.fnt")).expect("parse game.fnt");
        let strings = Csf::parse(files.get("ra2.csf").expect("ra2.csf")).expect("parse ra2.csf");

        let missions = Ini::parse(files.get("missions.pkt").expect("missions.pkt"));
        let maps = missions
            .section("MultiMaps")
            .iter()
            .filter(|(_, id)| {
                missions
                    .get(id, "GameMode")
                    .is_some_and(|modes| modes.split(',').any(|m| m.trim() == "standard"))
            })
            .filter_map(|(_, id)| {
                let name = strings.get(missions.get(id, "Description")?)?;
                Some(MultiMap {
                    id: id.to_lowercase(),
                    name: name.to_owned(),
                    max_players: missions.get(id, "MaxPlayers").and_then(|v| v.trim().parse().ok()).unwrap_or(8),
                })
            })
            .collect();

        // Each entry is `id=name, tooltip, rules, map filter, computer players allowed`.
        let mp_modes = Ini::parse(files.get("mpmodes.ini").expect("mpmodes.ini"));
        let (modes, mode_rules) = mp_modes
            .sections()
            .flat_map(|section| mp_modes.section(section))
            .filter_map(|(_, value)| {
                let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                (fields.last() == Some(&"true") && fields.len() >= 3).then(|| (fields[0], fields[2].to_owned()))
            })
            .filter_map(|(label, rules)| Some((strings.get(label)?.to_owned(), rules)))
            .unzip();

        Self {
            files,
            font,
            strings,
            maps,
            modes,
            mode_rules,
            pics: HashMap::new(),
        }
    }

    fn string(&self, label: &str) -> String {
        self.strings.get(label).unwrap_or(label).to_owned()
    }

    fn cached(
        &mut self,
        images: &mut Assets<Image>,
        key: String,
        make: impl FnOnce(&Self) -> (u32, u32, Vec<u8>),
    ) -> Pic {
        if let Some(pic) = self.pics.get(&key) {
            return pic.clone();
        }
        let (width, height, data) = make(self);
        let pic = Pic {
            image: images.add(rgba_image(width, height, data)),
            size: Vec2::new(width as f32, height as f32),
        };
        self.pics.insert(key, pic.clone());
        pic
    }

    fn shp(&mut self, images: &mut Assets<Image>, name: &str, palette: &str, frame: usize) -> Pic {
        self.cached(images, format!("{name}/{palette}/{frame}"), |shell| {
            let shp = shell.files.get(name).and_then(Shp::parse);
            let shp = shp.unwrap_or_else(|| panic!("missing or bad {name}"));
            let pal = shell.files.get(palette).and_then(Palette::parse);
            let pal = pal.unwrap_or_else(|| panic!("missing or bad {palette}"));
            (shp.width as u32, shp.height as u32, shp.frame_rgba(frame, &pal))
        })
    }

    /// A shape drawn in a house colour: palette entries 16 to 31 become the
    /// colour's shades.
    fn shp_in_colour(&mut self, images: &mut Assets<Image>, name: &str, palette: &str, frame: usize, hsv: [u8; 3]) -> Pic {
        self.cached(images, format!("{name}/{palette}/{frame}/{hsv:?}"), |shell| {
            let shp = shell.files.get(name).and_then(Shp::parse);
            let shp = shp.unwrap_or_else(|| panic!("missing or bad {name}"));
            let pal = shell.files.get(palette).and_then(Palette::parse);
            let mut pal = pal.unwrap_or_else(|| panic!("missing or bad {palette}"));
            pal.0[16..32].copy_from_slice(&crate::colors::remap_shades(hsv));
            (shp.width as u32, shp.height as u32, shp.frame_rgba(frame, &pal))
        })
    }

    /// A dialog widget image. Pure magenta is transparent.
    fn pcx(&mut self, images: &mut Assets<Image>, name: &str) -> Pic {
        self.cached(images, name.to_owned(), |shell| {
            let pcx = shell.files.get(name).and_then(Pcx::parse);
            let pcx = pcx.unwrap_or_else(|| panic!("missing or bad {name}"));
            let rgba = pcx
                .rgb
                .as_chunks::<3>().0.iter()
                .flat_map(|c| match c {
                    [255, 0, 255] => [0, 0, 0, 0],
                    _ => [c[0], c[1], c[2], 255],
                })
                .collect();
            (pcx.width, pcx.height, rgba)
        })
    }

    /// The stored picture of a map, with its size.
    fn preview(&mut self, images: &mut Assets<Image>, map: usize) -> Option<(Handle<Image>, i32, i32)> {
        let id = self.maps.get(map)?.id.clone();
        let key = format!("preview/{id}");
        if !self.pics.contains_key(&key) {
            let data = self.files.get(&format!("{id}.map")).or_else(|| self.files.get(&format!("{id}.mpr")))?;
            let (w, h, rgb) = ra2_formats::map::preview(data)?;
            let rgba: Vec<u8> = rgb.as_chunks::<3>().0.iter().take((w * h) as usize).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
            let image = Image::new(
                bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                rgba,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            self.pics.insert(key.clone(), Pic { image: images.add(image), size: Vec2::new(w as f32, h as f32) });
        }
        let pic = self.pics.get(&key)?;
        Some((pic.image.clone(), pic.size.x as i32, pic.size.y as i32))
    }

    fn text(&mut self, images: &mut Assets<Image>, text: &str, color: [u8; 3]) -> Pic {
        self.cached(images, format!("text/{color:?}/{text}"), |shell| {
            let (width, mask) = shell.font.rasterize(text);
            let rgba = mask
                .iter()
                .flat_map(|&a| [color[0], color[1], color[2], a])
                .collect();
            (width as u32, shell.font.height as u32, rgba)
        })
    }
}

fn place(x: f32, y: f32, width: f32, height: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x),
        top: Val::Px(y),
        width: Val::Px(width),
        height: Val::Px(height),
        ..default()
    }
}

fn picture(pic: &Pic, x: f32, y: f32) -> impl Bundle {
    (
        place(x, y, pic.size.x, pic.size.y),
        ImageNode::new(pic.image.clone()),
        Pickable::IGNORE,
    )
}

/// Converts a rectangle from the 640x480 dialog templates (in dialog units of the
/// 8-point system font: 1.5 px wide, 1.625 px tall) to 800x600 screen pixels.
// DATA: rectangles are the RT_DIALOG templates in the original game (226, 256, 258, 107).
pub(crate) fn dialog_rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    let min = Vec2::new((x * 1.5).round(), (y * 1.625).round()) + PANEL_OFFSET;
    Rect::from_corners(min, min + Vec2::new((width * 1.5).round(), (height * 1.625).round()))
}

enum Align {
    Left,
    Center,
    Right,
}

/// Draws one screen's widgets. Borrowed for the duration of a spawn.
struct Painter<'a> {
    shell: &'a mut Shell,
    images: &'a mut Assets<Image>,
}

impl Painter<'_> {
    /// Text within a box, vertically centred.
    fn label(
        &mut self,
        p: &mut ChildSpawnerCommands,
        text: &str,
        color: [u8; 3],
        area: Rect,
        align: Align,
    ) {
        let pic = self.shell.text(self.images, text, color);
        let x = match align {
            Align::Left => area.min.x,
            Align::Center => area.min.x + ((area.width() - pic.size.x) / 2.0).round(),
            Align::Right => area.max.x - pic.size.x,
        };
        let y = area.min.y + ((area.height() - pic.size.y) / 2.0).round();
        p.spawn(picture(&pic, x, y));
    }

    /// Text centred across a box and at its top, as the engine draws static text.
    fn label_top(&mut self, p: &mut ChildSpawnerCommands, text: &str, color: [u8; 3], area: Rect) {
        let pic = self.shell.text(self.images, text, color);
        p.spawn(picture(&pic, area.min.x + ((area.width() - pic.size.x) / 2.0).floor(), area.min.y));
    }

    /// The left panel shared by every menu screen: backdrop and the strip below it.
    fn backdrop(&mut self, p: &mut ChildSpawnerCommands) {
        let main = self.shell.shp(self.images, "mnscrnl.shp", "shell.pal", 0);
        let lower = self.shell.shp(self.images, "lwscrnl.shp", "shell.pal", 0);
        p.spawn(picture(&main, 0.0, 0.0));
        p.spawn(picture(&lower, 0.0, main.size.y));
    }

    /// The right-hand column: a top piece, eight button slots and a bottom piece.
    /// `buttons` gives the slot, label and action of each button; other slots show
    /// the blank cover plate.
    fn sidebar(
        &mut self,
        p: &mut ChildSpawnerCommands,
        title: &str,
        title_top: f32,
        top_frame: usize,
        buttons: &[(usize, &str, MenuAction)],
    ) {
        let top = self.shell.shp(self.images, "sdtp.shp", "shell.pal", top_frame);
        let slot = self.shell.shp(self.images, "sdbtnbkgd.shp", "shell.pal", 0);
        let bottom = self.shell.shp(self.images, "sdbtm.shp", "shell.pal", 0);
        // SDBTNANM.SHP's seventeen frames:
        // the empty plate (0), the button plate (1), the button's face normal,
        // lit and pressed (2, 3, 4), and the two slides (5 to 10, 11 to 16).
        let frames: std::sync::Arc<Vec<Handle<Image>>> =
            std::sync::Arc::new((0..17).map(|frame| self.shell.shp(self.images, "sdbtnanm.shp", "sdbtnanm.pal", frame).image).collect());
        let gone = frames[10].clone();
        let button_x = SIDEBAR_X + SIDEBAR_WIDTH - BUTTON_WIDTH;

        p.spawn(picture(&top, SIDEBAR_X, 0.0));
        // On screens
        // whose top piece is the warning meter, SDWRNANM.SHP (91 frames, in
        // SHELL2.PAL) runs over it at (671, 47), a frame every 91 ms, for ever,
        // starting again whenever a screen is made.
        if top_frame == 0 {
            let frames: Vec<Handle<Image>> = (0..91).map(|frame| self.shell.shp(self.images, "sdwrnanm.shp", "shell2.pal", frame)).map(|pic| pic.image).collect();
            let first = self.shell.shp(self.images, "sdwrnanm.shp", "shell2.pal", 0);
            p.spawn((picture(&first, 671.0, 47.0), Meter { frames, began: None }));
        }
        let title = self.shell.string(title);
        // Centred in the sidebar at its
        // template height, the text at the top of its rectangle.
        self.label_top(p, &title, TEXT, Rect::new(635.0, title_top, 797.0, title_top + 17.0));

        for index in 0..SLOT_COUNT {
            let y = SIDEBAR_TOP_HEIGHT + SLOT_HEIGHT * index as f32;
            p.spawn(picture(&slot, SIDEBAR_X, y));
            let slot_state = || Slot { index, frames: frames.clone(), was_pressed: false };
            let Some((_, label, action)) = buttons.iter().find(|(slot, ..)| *slot == index)
            else {
                p.spawn((place(button_x, y, BUTTON_WIDTH, SLOT_HEIGHT), ImageNode::new(gone.clone()), slot_state()));
                continue;
            };
            let text = self.shell.string(label);
            let text = self.shell.text(self.images, &text, TEXT);
            p.spawn((
                Button,
                Hovered::default(),
                *action,
                slot_state(),
                ClicksOnPress,
                place(button_x, y, BUTTON_WIDTH, SLOT_HEIGHT),
                ImageNode::new(gone.clone()),
            ))
            .with_child((
                // The words are centred in
                // 154 pixels, rounding down, and one below the middle of 41.
                picture(&text, ((154.0 - text.size.x) / 2.0).floor(), 1.0 + ((41.0 - text.size.y) / 2.0).floor()),
                SlotText,
                SlotTextHome(((154.0 - text.size.x) / 2.0).floor(), 1.0 + ((41.0 - text.size.y) / 2.0).floor()),
                Visibility::Hidden,
            ));
        }
        let bottom_y = SIDEBAR_TOP_HEIGHT + SLOT_HEIGHT * SLOT_COUNT as f32;
        p.spawn(picture(&bottom, SIDEBAR_X, bottom_y));
        // The version, centred in a box at (635, 584) 162 by 16.
        let version = format!("{} 1.006", self.shell.string("GUI:Version"));
        self.label(p, &version, TEXT, Rect::new(635.0, 584.0, 797.0, 600.0), Align::Center);
    }

    /// A list of text rows. The map list is selectable and scrolls with the mouse
    /// wheel; the game type list only shows its first row as selected.
    fn list_box(
        &mut self,
        p: &mut ChildSpawnerCommands,
        area: Rect,
        rows: &[String],
        is_map_list: bool,
    ) {
        let texts: Vec<Pic> = rows
            .iter()
            .map(|row| self.shell.text(self.images, row, TEXT))
            .collect();
        let inner_width = area.width() - 2.0;
        let content_height = LIST_ROW_HEIGHT * rows.len() as f32;
        p.spawn((
            Node {
                border: UiRect::all(Val::Px(1.0)),
                overflow: Overflow::clip(),
                ..place(area.min.x, area.min.y, area.width(), area.height())
            },
            BackgroundColor(Color::BLACK),
            BorderColor::all(EDGE),
        ))
        .with_children(|frame| {
            let mut content = frame.spawn(place(0.0, 0.0, inner_width, content_height));
            if is_map_list {
                content.insert(MapListContent {
                    rows: rows.len(),
                    visible_rows: ((area.height() - 2.0) / LIST_ROW_HEIGHT) as usize,
                });
            }
            content.with_children(|content| {
                for (index, text) in texts.iter().enumerate() {
                    let y = LIST_ROW_HEIGHT * index as f32;
                    let mut row = content.spawn(place(0.0, y, inner_width, LIST_ROW_HEIGHT));
                    if is_map_list {
                        row.insert((Button, MapEntry(index), BackgroundColor(Color::NONE)));
                    } else {
                        row.insert((Button, ModeEntry(index), BackgroundColor(Color::NONE)));
                    }
                    let text_y = ((LIST_ROW_HEIGHT - text.size.y) / 2.0).round();
                    row.with_child(picture(text, 3.0, text_y));
                }
            });
        });
    }

    /// The game mode and the selected map's name under the preview window in the
    /// sidebar's top piece.
    // The mode in control 0x6ec
    // at (651,167,135,16) and below it the name in 0x5a8 at (651,189,135,33), on up
    // to two lines.
    fn map_summary(&mut self, p: &mut ChildSpawnerCommands, map: usize, mode: usize) {
        let name = self.shell.maps.get(map).map(|m| m.name.clone()).unwrap_or_default();
        let mode = self.shell.modes.get(mode).cloned().unwrap_or_default();
        self.label_top(p, &mode, TEXT, Rect::new(651.0, 167.0, 786.0, 183.0));
        // Broken at a space where the whole name is wider than the control.
        let whole = self.shell.text(self.images, &name, TEXT);
        let mut lines = vec![name.clone()];
        if whole.size.x > 135.0 {
            let words: Vec<&str> = name.split(' ').collect();
            for cut in (1..words.len()).rev() {
                let first = words[..cut].join(" ");
                if self.shell.text(self.images, &first, TEXT).size.x <= 135.0 {
                    lines = vec![first, words[cut..].join(" ")];
                    break;
                }
            }
        }
        for (row, line) in lines.iter().enumerate() {
            let y = 189.0 + 16.0 * row as f32;
            self.label_top(p, line, TEXT, Rect::new(651.0, y, 786.0, y + 16.0));
        }
    }

    /// The picture of the selected map.
    // In control 0x468 at
    // (644,37,144,112): black, with the picture scaled by the smaller of the two
    // ratios (in thousandths) and centred.
    fn map_preview(&mut self, p: &mut ChildSpawnerCommands, map: usize) {
        let (cx, cy, cw, ch) = (644, 37, 144, 112);
        p.spawn((place(cx as f32, cy as f32, cw as f32, ch as f32), BackgroundColor(Color::BLACK)));
        match self.shell.preview(self.images, map) {
            Some((handle, w, h)) => {
                let (x, y, dw, dh) = preview_rect(w, h);
                p.spawn((place(x as f32, y as f32, dw as f32, dh as f32), ImageNode::new(handle), PreviewPicture(map)));
            }
            // A map without a stored picture shows
            // black.
            None => {
                p.spawn((place(cx as f32, cy as f32, 1.0, 1.0), ImageNode::default(), Visibility::Hidden, PreviewPicture(map)));
            }
        }
    }
}

/// The map picture on screen and which map it shows.
#[derive(Component)]
struct PreviewPicture(usize);

/// Where a stored map picture of a size goes in the preview control.
fn preview_rect(w: i32, h: i32) -> (i32, i32, i32, i32) {
    let (cx, cy, cw, ch) = (644, 37, 144, 112);
    let scale = (ch * 1000 / h.max(1)).min(cw * 1000 / w.max(1));
    (cx + cw / 2 - w * scale / 2000, cy + ch / 2 - h * scale / 2000, w * scale / 1000, h * scale / 1000)
}

/// On the Choose Map screen the picture follows the map highlighted in the list.
// The preview control is redrawn from the list's
// current row.
fn follow_highlight(
    choice: Res<MapChoice>,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
    mut pictures: Query<(&mut PreviewPicture, &mut ImageNode, &mut Node, &mut Visibility)>,
) {
    for (mut picture, mut image, mut node, mut visibility) in &mut pictures {
        if picture.0 == choice.highlighted {
            continue;
        }
        picture.0 = choice.highlighted;
        match shell.preview(&mut images, choice.highlighted) {
            Some((handle, w, h)) => {
                let (x, y, dw, dh) = preview_rect(w, h);
                image.image = handle;
                (node.left, node.top, node.width, node.height) = (Val::Px(x as f32), Val::Px(y as f32), Val::Px(dw as f32), Val::Px(dh as f32));
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}

fn spawn_screen(
    commands: &mut Commands,
    screen: Screen,
    shell: &mut Shell,
    images: &mut Assets<Image>,
    build: impl FnOnce(&mut Painter, &mut ChildSpawnerCommands),
) {
    let mut painter = Painter { shell, images };
    commands
        .spawn((
            DespawnOnExit(screen),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
        ))
        .with_children(|outer| {
            outer
                .spawn(Node {
                    width: Val::Px(SCREEN.x),
                    height: Val::Px(SCREEN.y),
                    flex_shrink: 0.0,
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|p| {
                    painter.backdrop(p);
                    build(&mut painter, p);
                });
        });
}

fn spawn_main_menu(
    mut commands: Commands,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
) {
    spawn_screen(&mut commands, Screen::MainMenu, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:MainMenu",
            9.0,
            0,
            &[
                (0, "GUI:SinglePlayer", MenuAction::Goto(Screen::SinglePlayer)),
                (1, "GUI:WWOnline", MenuAction::Goto(Screen::Internet)),
                (2, "GUI:Network", MenuAction::Goto(Screen::Network)),
                (3, "GUI:MoviesAndCredits", MenuAction::Mock("Movies & Credits")),
                (4, "GUI:Options", MenuAction::Goto(Screen::Options)),
                (LAST_BUTTON_SLOT, "GUI:ExitGame", MenuAction::Exit),
            ],
        );
    });
}

/// What the loading screen shows for each country, in `[Countries]` order:
/// the picture's name, the country's name, its special unit and the line about it.
const LOAD_SCREENS: [(&str, &str, &str, &str); 9] = [
    ("ustates", "Name:Americans", "Name:Para", "LoadBrief:USA"),
    ("korea", "Name:Alliance", "Name:BEAGLE", "LoadBrief:Korea"),
    ("france", "Name:French", "Name:GTGCAN", "LoadBrief:French"),
    ("germany", "Name:Germans", "Name:TNKD", "LoadBrief:Germans"),
    ("ukingdom", "Name:British", "Name:SNIPE", "LoadBrief:British"),
    ("libya", "Name:Africans", "Name:DTRUCK", "LoadBrief:Lybia"),
    ("iraq", "Name:Arabs", "Name:DESO", "LoadBrief:Iraq"),
    ("cuba", "Name:Confederation", "Name:TERROR", "LoadBrief:Cuba"),
    ("russia", "Name:Russians", "Name:TTNK", "LoadBrief:Russia"),
];

/// Not original behaviour: the original shows the screen only as long as the
/// map takes to load, which is now too quick to read it.
const LOAD_SCREEN_SECONDS: f64 = 2.5;

#[derive(Resource)]
struct LoadScreenShown(f64);

/// The loading screen: the country's picture, its name at the bottom right,
/// its special unit, the line about that unit and "Loading..." at the left, in
/// the side's load colour on darkened boxes.
fn spawn_load_screen(
    mut commands: Commands,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
    setup: Res<GameSetup>,
    time: Res<Time<Real>>,
    lobby: Res<crate::net::Lobby>,
    rules: Res<crate::files::Rules>,
) {
    let country = match setup.options.country {
        country @ 0..=8 => country as usize,
        _ => {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.subsec_nanos());
            let country = nanos as usize % LOAD_SCREENS.len();
            commands.insert_resource(crate::game::DrawnCountry(country));
            country
        }
    };
    commands.insert_resource(LoadScreenShown(time.elapsed_secs_f64()));
    let (picture_name, name, unit, brief) = LOAD_SCREENS[country];
    let rules = Some(&rules.0);
    let scheme = if country < 5 { "AlliedLoad" } else { "SovietLoad" };
    let hsv = rules.as_ref().and_then(|rules| rules.get("Colors", scheme)).and_then(|value| {
        let parts: Vec<u8> = value.split(',').filter_map(|part| part.split(';').next()?.trim().parse().ok()).collect();
        <[u8; 3]>::try_from(parts.get(..3)?).ok()
    });
    let colour = crate::colors::hsv_to_rgb(hsv.unwrap_or([164, 255, 255]));
    let painter = Painter { shell: &mut shell, images: &mut images };
    let line_height = painter.shell.font.height as f32;
    let darken = |p: &mut ChildSpawnerCommands, x: f32, y: f32, width: f32, height: f32| {
        p.spawn((place(x - 4.0, y - 4.0, width + 8.0, height + 8.0), BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62))));
    };
    commands
        .spawn((
            DespawnOnExit(Screen::LoadScreen),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
        ))
        .with_children(|outer| {
            outer
                .spawn(Node { width: Val::Px(SCREEN.x), height: Val::Px(SCREEN.y), flex_shrink: 0.0, overflow: Overflow::clip(), ..default() })
                .with_children(|p| {
                    let back = painter.shell.shp(painter.images, &format!("ls800{picture_name}.shp"), "mpls.pal", 0);
                    p.spawn(picture(&back, 0.0, 0.0));

                    let name = painter.shell.string(name);
                    let width = painter.shell.font.width(&name) as f32;
                    darken(p, 533.0 + 200.0 - width, 545.0, width, line_height);
                    let pic = painter.shell.text(painter.images, &name, colour);
                    p.spawn(picture(&pic, 533.0 + 200.0 - pic.size.x, 545.0));

                    let unit = painter.shell.string(unit).to_uppercase();
                    let pic = painter.shell.text(painter.images, &unit, colour);
                    p.spawn(picture(&pic, 20.0, 90.0));

                    let brief = painter.shell.string(brief);
                    let lines = wrap(&painter.shell.font, &brief, 398);
                    darken(p, 20.0, 158.0, 398.0, line_height * lines.len() as f32);
                    for (row, line) in lines.iter().enumerate() {
                        let pic = painter.shell.text(painter.images, line, colour);
                        p.spawn(picture(&pic, 20.0, 158.0 + line_height * row as f32));
                    }

                    let loading = painter.shell.string("GUI:LoadingEx");
                    let pic = painter.shell.text(painter.images, &loading, colour);
                    darken(p, 20.0, 300.0, pic.size.x, line_height);
                    p.spawn(picture(&pic, 20.0, 300.0));

                    // The player's row at (16, 321): a darkened box; the bar,
                    // outlined in the player's colour, with PROGBARM.SHP over
                    // as much of it as is loaded; then the name, 21 pixels
                    // past the bar. The row is the taller of the bar plus 6 and
                    // the text, plus 4.
                    let player_name = if lobby.name.is_empty() { painter.shell.string("GUI:Player") } else { lobby.name.clone() };
                    let player_hsv = rules
                        .as_ref()
                        .and_then(|rules| rules.get("Colors", crate::terrain::PLAYER_COLOURS[setup.options.colour.clamp(0, 7) as usize]))
                        .and_then(|value| <[u8; 3]>::try_from(value.split(',').filter_map(|part| part.trim().parse().ok()).collect::<Vec<u8>>().get(..3)?).ok());
                    let player_colour = player_hsv.map_or([255, 255, 255], crate::colors::hsv_to_rgb);
                    // The bar's art is all house-colour shades.
                    let bar = match player_hsv {
                        Some(hsv) => painter.shell.shp_in_colour(painter.images, "progbarm.shp", "mpls.pal", 0, hsv),
                        None => painter.shell.shp(painter.images, "progbarm.shp", "mpls.pal", 0),
                    };
                    let name = painter.shell.text(painter.images, &player_name, player_colour);
                    let row = (bar.size.y + 6.0).max(name.size.y) + 4.0;
                    let (x, y) = (16.0, 321.0);
                    p.spawn((place(x, y, bar.size.x + name.size.x + 36.0, row + 1.0), BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62))));
                    let (bar_x, bar_y) = (x + 5.0, y + ((row - 6.0 - bar.size.y) / 2.0).floor());
                    p.spawn((
                        Node { border: UiRect::all(Val::Px(1.0)), ..place(bar_x, bar_y, bar.size.x + 6.0, bar.size.y + 6.0) },
                        BorderColor::all(Color::srgb_u8(player_colour[0], player_colour[1], player_colour[2])),
                    ));
                    // Not original behaviour: the map loads at once here, so
                    // the bar fills over the time the screen is shown.
                    p.spawn((Node { overflow: Overflow::clip(), ..place(bar_x + 3.0, bar_y + 3.0, 0.0, bar.size.y) }, LoadBar(bar.size.x)))
                        .with_child(picture(&bar, 0.0, 0.0));
                    p.spawn(picture(&name, x + 21.0 + bar.size.x, y + ((row - name.size.y) / 2.0).floor()));
                });
        });
}

/// Breaks text into lines no wider than `width`, at spaces and at the
/// strings' own " \n " breaks.
fn wrap(font: &Font, text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if !line.is_empty() && font.width(&candidate) > width {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

/// The loading bar's filled part, and the bar's full width.
#[derive(Component)]
struct LoadBar(f32);

fn fill_load_bar(shown: Option<Res<LoadScreenShown>>, time: Res<Time<Real>>, mut bars: Query<(&LoadBar, &mut Node)>) {
    let done = shown.map_or(1.0, |shown| ((time.elapsed_secs_f64() - shown.0) / LOAD_SCREEN_SECONDS).clamp(0.0, 1.0)) as f32;
    for (bar, mut node) in &mut bars {
        node.width = Val::Px((bar.0 * done).round());
    }
}

fn leave_load_screen(shown: Option<Res<LoadScreenShown>>, time: Res<Time<Real>>, mut next_screen: ResMut<NextState<Screen>>) {
    if shown.is_none_or(|shown| time.elapsed_secs_f64() - shown.0 >= LOAD_SCREEN_SECONDS) {
        next_screen.set(Screen::InGame);
    }
}

/// The score screen after a skirmish.
// The title, "Game: n" and the time; the headings; a row for each house in
// its colour, best first; Continue goes back to the menus.
fn spawn_score(mut commands: Commands, mut shell: ResMut<Shell>, mut images: ResMut<Assets<Image>>, board: Res<crate::endgame::ScoreBoard>) {
    let board = board.clone();
    spawn_screen(&mut commands, Screen::Score, &mut shell, &mut images, |painter, p| {
        // The score dialog's main panel is MPASCRNL.SHP in MPASCRN.PAL when
        // the player's side is the first (the Allies), else MPSSCRNL.SHP in
        // MPSSCRN.PAL (the S shapes on a screen 640 wide).
        let (shape, palette) = if board.soviet { ("mpsscrnl.shp", "mpsscrn.pal") } else { ("mpascrnl.shp", "mpascrn.pal") };
        let back = painter.shell.shp(painter.images, shape, palette, 0);
        p.spawn(picture(&back, 0.0, 0.0));
        // The rectangle from the game line to the last used row's last
        // column is darkened (blend level 0x9f), grown by 4 across and 8
        // down.
        let last = board.rows.len().clamp(1, 8) as f32 - 1.0;
        let from = dialog_rect(13.0, 38.0, 1.0, 1.0).min;
        let to = dialog_rect(255.0, 88.0 + 20.0 * last, 45.0, 10.0).max;
        p.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(from.x - 4.0),
                top: Val::Px(from.y - 8.0),
                width: Val::Px(to.x - from.x + 8.0),
                height: Val::Px(to.y - from.y + 16.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
        ));
        painter.sidebar(p, "GUI:SkirmishScore", 9.0, 0, &[(LAST_BUTTON_SLOT, "GUI:Continue", MenuAction::Goto(Screen::Skirmish))]);
        let game = painter.shell.string("TXT_GAME").replacen("%d", &board.games.to_string(), 1);
        let (hours, minutes, seconds) = (board.seconds.min(359_999) / 3600, board.seconds.min(359_999) / 60 % 60, board.seconds % 60);
        let time = format!("{} {hours:02}:{minutes:02}:{seconds:02}", painter.shell.string("TXT_TIME_FORMAT_HOURS").split('%').next().unwrap_or("Time:").trim());
        painter.label(p, &game, TEXT, dialog_rect(13.0, 38.0, 115.0, 10.0), Align::Left);
        painter.label(p, &time, TEXT, dialog_rect(195.0, 38.0, 105.0, 10.0), Align::Right);
        const COLUMNS: [(&str, f32, f32); 5] =
            [("GUI:Player", 13.0, 75.0), ("GUI:Kills", 94.0, 45.0), ("GUI:Losses", 149.0, 45.0), ("GUI:Built", 204.0, 45.0), ("GUI:Score", 255.0, 45.0)];
        for (index, (key, x, width)) in COLUMNS.iter().enumerate() {
            let text = painter.shell.string(key);
            painter.label(p, &text, TEXT, dialog_rect(*x, 63.0, *width, 10.0), if index == 0 { Align::Left } else { Align::Right });
        }
        for (row, entry) in board.rows.iter().take(8).enumerate() {
            let y = 88.0 + 20.0 * row as f32;
            let texts = [entry.name.clone(), entry.kills.to_string(), entry.losses.to_string(), entry.built.to_string(), entry.score.to_string()];
            for (index, (_, x, width)) in COLUMNS.iter().enumerate() {
                painter.label(p, &texts[index], entry.colour, dialog_rect(*x, y, *width, 10.0), if index == 0 { Align::Left } else { Align::Right });
            }
        }
    });
}

fn spawn_single_player(
    mut commands: Commands,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
) {
    spawn_screen(&mut commands, Screen::SinglePlayer, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:SinglePlayerMenu",
            9.0,
            0,
            &[
                (0, "GUI:NewCampaign", MenuAction::Mock("New Campaign")),
                // There to
                // press only when a save exists.
                (1, "GUI:LoadSavedGame", if !crate::saves::list().is_empty() { MenuAction::Goto(Screen::LoadGame) } else { MenuAction::Tell("There are no saved games yet.") }),
                (2, "GUI:Skirmish", MenuAction::Goto(Screen::Skirmish)),
                (LAST_BUTTON_SLOT, "GUI:MainMenu", MenuAction::Goto(Screen::MainMenu)),
            ],
        );
    });
}

/// The Load Saved Game screen: every save, newest first, the first picked.
// The heading and list sit where
// the in-game Load dialog has them; a double click loads.
fn spawn_load_game(mut commands: Commands, mut shell: ResMut<Shell>, mut images: ResMut<Assets<Image>>, mut saves: ResMut<crate::saves::SaveLoad>) {
    saves.entries = crate::saves::list();
    saves.picked = 0;
    saves.clicked = None;
    let rows: Vec<(String, String)> = saves.entries.iter().map(|entry| (entry.description.chars().take(34).collect(), crate::saves::stamp(entry.modified))).collect();
    spawn_screen(&mut commands, Screen::LoadGame, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:LoadSavedGame",
            9.0,
            0,
            &[(0, "GUI:Load", MenuAction::LoadSaved), (LAST_BUTTON_SLOT, "GUI:Back", MenuAction::Goto(Screen::SinglePlayer))],
        );
        let heading = painter.shell.string("GUI:SelectMission");
        painter.label_top(p, &heading, TEXT, dialog_rect(24.0, 16.0, 266.0, 24.0));
        let area = dialog_rect(24.0, 50.0, 266.0, 187.0);
        let texts: Vec<(Pic, Pic)> = rows.iter().map(|(name, when)| (painter.shell.text(painter.images, name, TEXT), painter.shell.text(painter.images, when, TEXT))).collect();
        let inner_width = area.width() - 2.0;
        p.spawn((
            Node { border: UiRect::all(Val::Px(1.0)), overflow: Overflow::clip(), ..place(area.min.x, area.min.y, area.width(), area.height()) },
            BackgroundColor(Color::BLACK),
            BorderColor::all(EDGE),
        ))
        .with_children(|frame| {
            frame
                .spawn((
                    place(0.0, 0.0, inner_width, LIST_ROW_HEIGHT * texts.len() as f32),
                    SaveListContent { rows: texts.len(), visible_rows: ((area.height() - 2.0) / LIST_ROW_HEIGHT) as usize },
                ))
                .with_children(|content| {
                    for (index, (name, when)) in texts.iter().enumerate() {
                        let text_y = ((LIST_ROW_HEIGHT - name.size.y) / 2.0).round();
                        content
                            .spawn((place(0.0, LIST_ROW_HEIGHT * index as f32, inner_width, LIST_ROW_HEIGHT), Button, SaveEntry(index), BackgroundColor(Color::NONE)))
                            .with_children(|row| {
                                row.spawn(picture(name, 3.0, text_y));
                                row.spawn(picture(when, (inner_width * 255.0 / 399.0).round(), text_y));
                            });
                    }
                });
        });
    });
}

/// A row of the list of saves, by its place in `SaveLoad::entries`.
#[derive(Component)]
struct SaveEntry(usize);

/// The rows of the list of saves, moved up as it scrolls.
#[derive(Component)]
struct SaveListContent {
    rows: usize,
    visible_rows: usize,
}

fn on_save_entry(activate: On<Activate>, entries: Query<&SaveEntry>, mut saves: ResMut<crate::saves::SaveLoad>, time: Res<Time<Real>>) {
    let Ok(entry) = entries.get(activate.entity) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let again = saves.clicked.is_some_and(|(last, at)| last == entry.0 && now - at < 0.4);
    saves.clicked = Some((entry.0, now));
    saves.picked = entry.0;
    if again {
        saves.load = saves.entries.get(entry.0).map(|entry| entry.path.clone());
    }
}

fn save_entry_colors(mut entries: Query<(&SaveEntry, &mut BackgroundColor)>, saves: Res<crate::saves::SaveLoad>) {
    for (entry, mut background) in &mut entries {
        let color = if saves.picked == entry.0 { SELECTED } else { Color::NONE };
        if background.0 != color {
            background.0 = color;
        }
    }
}

fn save_list_scroll(mut wheel: MessageReader<MouseWheel>, mut lists: Query<(Ref<SaveListContent>, &mut Node)>, mut scroll: Local<usize>) {
    let lines: f32 = wheel.read().map(|event| event.y).sum();
    let step = lines.signum() * lines.abs().ceil();
    for (list, mut node) in &mut lists {
        if list.is_added() {
            *scroll = 0;
        }
        let max = list.rows.saturating_sub(list.visible_rows) as f32;
        *scroll = (*scroll as f32 - step).clamp(0.0, max) as usize;
        let top = Val::Px(-LIST_ROW_HEIGHT * *scroll as f32);
        if node.top != top {
            node.top = top;
        }
    }
}

/// A box to type in: its outline (yellow when typing goes to it), what it
/// holds, and a click on it takes the typing.
fn text_box(painter: &mut Painter, p: &mut ChildSpawnerCommands, field: Rect, text: &str, which: crate::net::Focus, focus: crate::net::Focus) {
    let focused = which == focus;
    painter.outline(p, field, if focused { Color::srgb_u8(255, 255, 0) } else { EDGE });
    let shown = if focused { format!("{text}_") } else { text.to_owned() };
    painter.label(p, &shown, TEXT, Rect::new(field.min.x + 4.0, field.min.y, field.max.x, field.max.y), Align::Left);
    p.spawn((place(field.min.x, field.min.y, field.width(), field.height()), Button, MenuAction::Focus(which)));
}

/// The Network screen: create a game, or join one at an address.
// Not original behaviour: the original's network lobby (dialog 187) lists the
// games it hears on the local network; here the address is typed.
fn spawn_network(mut commands: Commands, mut shell: ResMut<Shell>, mut images: ResMut<Assets<Image>>, lobby: Res<crate::net::Lobby>) {
    use crate::net::Focus;
    let (address, status, name) = (lobby.address.clone(), lobby.status.clone(), lobby.name.clone());
    let focus = if matches!(lobby.focus, Focus::Name) { Focus::Name } else { Focus::Address };
    spawn_screen(&mut commands, Screen::Network, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:Network",
            9.0,
            0,
            &[
                (0, "GUI:CreateGame", MenuAction::HostGame),
                (1, "GUI:JoinGame", MenuAction::JoinGame),
                (LAST_BUTTON_SLOT, "GUI:Back", MenuAction::Goto(Screen::MainMenu)),
            ],
        );
        let lines = [
            (60.0, "Create Game: set the game up, then other players join you."),
            (80.0, "Join Game: type the address of the player who created the game."),
            (120.0, "Your name:"),
            (180.0, "Address and port:"),
            (240.0, "Click a box (or press Tab) and type to change it."),
        ];
        for (y, text) in lines {
            painter.label(p, text, TEXT, Rect::new(40.0, y, 600.0, y + 18.0), Align::Left);
        }
        text_box(painter, p, Rect::new(40.0, 142.0, 360.0, 164.0), &name, Focus::Name, focus);
        text_box(painter, p, Rect::new(40.0, 202.0, 360.0, 224.0), &address, Focus::Address, focus);
        painter.label(p, &status, TEXT, Rect::new(40.0, 280.0, 620.0, 298.0), Align::Left);
    });
}

/// The Internet screen: the games waiting on the meeting-place server.
// Not original behaviour: the original's Internet play went through Westwood
// Online (dialogs 271 and 285), which no longer exists.
fn spawn_internet(mut commands: Commands, mut shell: ResMut<Shell>, mut images: ResMut<Assets<Image>>, lobby: Res<crate::net::Lobby>) {
    use crate::net::Focus;
    let lobby = &*lobby;
    let focus = if matches!(lobby.focus, Focus::Address) { Focus::Server } else { lobby.focus };
    let map_name = |id: &str| shell.maps.iter().find(|map| map.id == id).map_or_else(|| id.to_owned(), |map| map.name.clone());
    let rows: Vec<(u32, String, String, String, bool)> = lobby.games.iter().map(|game| (game.id, game.name.clone(), map_name(&game.map), format!("{}/{}", game.players, game.max_players), game.locked)).collect();
    spawn_screen(&mut commands, Screen::Internet, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:WWOnline",
            9.0,
            0,
            &[
                (0, "GUI:CreateGame", MenuAction::HostInternet),
                (1, "GUI:JoinGame", MenuAction::JoinInternet),
                (2, "GUI:Refresh", MenuAction::Refresh),
                (LAST_BUTTON_SLOT, "GUI:Back", MenuAction::Goto(Screen::MainMenu)),
            ],
        );
        for (x, y, text) in [(40.0, 30.0, "Server:"), (330.0, 30.0, "Your name:"), (40.0, 84.0, "Password (for a game you create, or to join a locked one):")] {
            painter.label(p, text, TEXT, Rect::new(x, y, 620.0, y + 18.0), Align::Left);
        }
        text_box(painter, p, Rect::new(40.0, 50.0, 310.0, 72.0), &lobby.server, Focus::Server, focus);
        text_box(painter, p, Rect::new(330.0, 50.0, 600.0, 72.0), &lobby.name, Focus::Name, focus);
        text_box(painter, p, Rect::new(40.0, 104.0, 310.0, 126.0), &"*".repeat(lobby.password.len()), Focus::Password, focus);
        // The list: name, map, players, and whether a password is wanted.
        let list = Rect::new(40.0, 170.0, 600.0, 500.0);
        for (x, text) in [(44.0, "Game"), (270.0, "Map"), (470.0, "Players"), (540.0, "Locked")] {
            painter.label(p, text, TEXT, Rect::new(x, 148.0, 620.0, 166.0), Align::Left);
        }
        painter.outline(p, list, EDGE);
        if rows.is_empty() {
            painter.label(p, "No games are waiting for players.", TEXT_DISABLED, Rect::new(list.min.x + 4.0, list.min.y + 2.0, list.max.x, list.min.y + 20.0), Align::Left);
        }
        for (n, (id, name, map, players, locked)) in rows.iter().take(18).enumerate() {
            let y = list.min.y + 2.0 + LIST_ROW_HEIGHT * n as f32;
            let row = Rect::new(list.min.x + 1.0, y, list.max.x - 1.0, y + LIST_ROW_HEIGHT);
            let picked = lobby.picked == Some(*id);
            p.spawn((place(row.min.x, row.min.y, row.width(), row.height()), Button, MenuAction::PickGame(*id), BackgroundColor(if picked { SELECTED } else { Color::NONE })));
            for (x, text) in [(44.0, name.as_str()), (270.0, map.as_str()), (470.0, players.as_str()), (540.0, if *locked { "Yes" } else { "" })] {
                painter.label(p, text, TEXT, Rect::new(x, y, 620.0, y + LIST_ROW_HEIGHT), Align::Left);
            }
        }
        painter.label(p, &lobby.status, TEXT, Rect::new(40.0, 510.0, 620.0, 528.0), Align::Left);
    });
}

/// Typing changes the address on the Network screen; the lobby's map and mode
/// are kept in step between the machines; and a network screen is drawn again
/// when what it shows has changed.
fn network_screens(
    mut keys: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut lobby: ResMut<crate::net::Lobby>,
    shell: Res<Shell>,
    setup: Res<GameSetup>,
    mut choice: ResMut<MapChoice>,
    state: Res<State<Screen>>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut redraw: ResMut<Redraw>,
    mut shown: Local<Option<(String, String, crate::game::Options, usize, usize, String, crate::net::Focus, [String; 8], (String, usize, Vec<ra2_net::directory::GameInfo>, Option<u32>))>>,
    mut was: Local<Option<crate::net::LobbyMode>>,
    mut asked_browse: Local<bool>,
    mut kept: Local<Option<(String, String, String)>>,
) {
    use bevy::input::keyboard::Key;
    use crate::net::LobbyMode;
    // The game is starting: the screen is on its way out, and drawing it
    // again would call the move to the game off.
    if lobby.starting {
        return;
    }
    let screen = *state.get();
    for event in keys.read() {
        // On the skirmish screen the player's name box takes typing once it
        // has been clicked (not for a player who joined: theirs is already told).
        let naming = screen == Screen::Skirmish && lobby.focus == crate::net::Focus::Name && lobby.mode != LobbyMode::Joining;
        if !(matches!(screen, Screen::Network | Screen::Internet) || naming) || !event.state.is_pressed() {
            continue;
        }
        use crate::net::Focus;
        // Each screen has its own boxes; Tab goes round them.
        let boxes: &[Focus] = if naming { &[Focus::Name] } else if screen == Screen::Internet { &[Focus::Server, Focus::Name, Focus::Password] } else { &[Focus::Address, Focus::Name] };
        if !boxes.contains(&lobby.focus) {
            lobby.focus = boxes[0];
        }
        let focus = lobby.focus;
        let (text, limit, extra): (&mut String, usize, &[char]) = match focus {
            Focus::Address => (&mut lobby.address, 40, &['.', ':', '-']),
            Focus::Server => (&mut lobby.server, 40, &['.', ':', '-']),
            Focus::Name => (&mut lobby.name, 16, &[' ', '_', '-', '.']),
            Focus::Password => (&mut lobby.password, 24, &['_', '-', '.', '!', '?']),
        };
        match &event.logical_key {
            Key::Tab => {
                let at = boxes.iter().position(|which| *which == focus).unwrap_or(0);
                lobby.focus = boxes[(at + 1) % boxes.len()];
            }
            Key::Backspace => {
                text.pop();
            }
            Key::Space if extra.contains(&' ') && text.len() < limit => text.push(' '),
            Key::Character(typed) => {
                for c in typed.chars().filter(|c| c.is_ascii_alphanumeric() || extra.contains(c)) {
                    if text.len() < limit {
                        text.push(c);
                    }
                }
            }
            _ => {}
        }
    }
    // The server's list is wanted while its screen is up, and the connection
    // while a game made or joined through it is being arranged.
    let browse = screen == Screen::Internet || (lobby.via_server && lobby.mode != LobbyMode::Off && screen != Screen::MainMenu);
    if browse && !*asked_browse {
        lobby.browse = true;
    } else if !browse && *asked_browse {
        lobby.browse = false;
    }
    *asked_browse = browse;
    match lobby.mode {
        // The host's choice of map and mode is the game's.
        LobbyMode::Hosting if crate::net::auto_host() => {}
        LobbyMode::Hosting => {
            if let Some(map) = shell.maps.get(choice.selected)
                && (lobby.map != map.id || lobby.max_players != map.max_players) {
                    lobby.map = map.id.clone();
                    lobby.max_players = map.max_players;
                }
            if let Some(rules) = shell.mode_rules.get(choice.mode).filter(|rules| **rules != lobby.mode_rules) {
                lobby.mode_rules = rules.clone();
            }
        }
        // A player who joined is shown the host's.
        LobbyMode::Joining => {
            if let Some(index) = shell.maps.iter().position(|map| map.id == lobby.map).filter(|index| *index != choice.selected) {
                choice.selected = index;
                choice.highlighted = index;
            }
            if let Some(index) = shell.mode_rules.iter().position(|rules| *rules == lobby.mode_rules).filter(|index| *index != choice.mode) {
                choice.mode = index;
            }
        }
        // Thrown out of a lobby (refused, or the host gone): back to the
        // Network screen, which says why.
        LobbyMode::Off => {
            if *was == Some(LobbyMode::Joining) && screen == Screen::Skirmish {
                next_screen.set(if lobby.via_server { Screen::Internet } else { Screen::Network });
            }
        }
    }
    *was = Some(lobby.mode);
    if !matches!(screen, Screen::Skirmish | Screen::Network | Screen::Internet) && lobby.focus == crate::net::Focus::Name {
        lobby.focus = crate::net::Focus::Address;
    }
    let now = (lobby.address.clone(), lobby.status.clone(), setup.options, choice.selected, choice.mode, lobby.name.clone(), lobby.focus, lobby.names.clone(), (lobby.server.clone(), lobby.password.len(), lobby.games.clone(), lobby.picked));
    // The name and the addresses are kept for next time as they change.
    let keep = (lobby.name.clone(), lobby.address.clone(), lobby.server.clone());
    if kept.as_ref() != Some(&keep) {
        if kept.is_some() {
            crate::net::Settings::write(&lobby);
        }
        *kept = Some(keep);
    }
    let lobby_screen = matches!(screen, Screen::Network | Screen::Internet | Screen::Skirmish);
    if !lobby_screen {
        *shown = Some(now);
        return;
    }
    if shown.as_ref() != Some(&now) {
        *shown = Some(now);
        redraw.0 = true;
        next_screen.set(screen);
    }
}

/// The warning meter at the top of the menus' column: its pictures, and when it
/// began to run.
#[derive(Component)]
struct Meter {
    frames: Vec<Handle<Image>>,
    began: Option<f64>,
}

fn run_meter(time: Res<Time<Real>>, mut meters: Query<(&mut Meter, &mut ImageNode)>) {
    let now = time.elapsed_secs_f64();
    for (mut meter, mut image) in &mut meters {
        let began = *meter.began.get_or_insert(now);
        let frame = ((now - began) / 0.091) as usize % meter.frames.len().max(1);
        if let Some(handle) = meter.frames.get(frame)
            && image.image != *handle {
                image.image = handle.clone();
            }
    }
}

/// Which of the skirmish screen's drop-down lists is open, and how far it is
/// scrolled.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
struct OpenCombo(Option<(Combo, usize)>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Combo {
    /// A computer row's level; then the country and the colour of a row, the
    /// player's being row 0.
    Level(usize),
    Country(usize),
    Colour(usize),
    /// The Options screen's list of screen sizes.
    Resolution,
}

/// The values behind the level list's items: none, easy, medium, brutal.
// In this order, with these labels.
const LEVELS: [(i32, &str); 4] = [(-1, "GUI:None"), (2, "GUI:AIEasy"), (1, "GUI:AINormal"), (0, "GUI:AIHard")];

/// A row's country and colour choices (row 0 is the player).
fn row_choice(options: &mut crate::game::Options, row: usize) -> (&mut i32, &mut i32) {
    match row.checked_sub(1) {
        None => (&mut options.country, &mut options.colour),
        Some(index) => {
            let slot = &mut options.opponents[index.min(6)];
            (&mut slot.country, &mut slot.colour)
        }
    }
}

/// The values a row's colour list offers: random, then every colour no other
/// row has.
// Each list holds Random
// and the colours that are free or the row's own, and every pick refills all.
fn colour_values(options: &crate::game::Options, row: usize) -> Vec<i32> {
    let mut options = *options;
    let taken: Vec<i32> = (0..8).filter(|&other| other != row).map(|other| *row_choice(&mut options, other).1).collect();
    std::iter::once(-2).chain((0..8).filter(|colour| !taken.contains(colour))).collect()
}

/// A row of an open drop-down list: which list, and the item's place in it.
#[derive(Component)]
struct ComboRow(Combo, usize);

/// The part of a slider that takes the mouse, and its grip.
#[derive(Component, Clone, Copy)]
struct SliderTrack(Slider);
#[derive(Component)]
struct SliderGrip(Slider, f32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slider {
    Speed,
    Credits,
    UnitCount,
    /// The Options screen's.
    Detail,
    Difficulty,
    ScrollRate,
    Music,
    Sound,
    Voice,
}

impl Slider {
    /// Lowest value, range, and step.
    // Speed and unit count in ones; credits from
    // `MinMoney` to `MaxMoney` in steps of `MoneyIncrement`.
    fn span(self) -> (i32, i32, i32) {
        match self {
            Slider::Speed => (0, 6, 1),
            Slider::Credits => (5000, 5000, 100),
            Slider::UnitCount => (0, 10, 1),
            // Detail in two places, difficulty in
            // three, the scroll rate in seven, each volume in eleven (tenths).
            Slider::Detail => (0, 1, 1),
            Slider::Difficulty => (0, 2, 1),
            Slider::ScrollRate => (0, 6, 1),
            Slider::Music | Slider::Sound | Slider::Voice => (0, 10, 1),
        }
    }

    /// The Options screen's sliders: their value, and setting it.
    // The scroll rate's slider shows 6 less it;
    // a volume is the slider's place times a tenth.
    fn pref_value(self, prefs: &crate::prefs::Prefs) -> Option<i32> {
        Some(match self {
            Slider::Detail => prefs.detail,
            Slider::Difficulty => prefs.difficulty,
            Slider::ScrollRate => 6 - prefs.scroll_rate,
            Slider::Music => (prefs.music * 10.0).round() as i32,
            Slider::Sound => (prefs.sound * 10.0).round() as i32,
            Slider::Voice => (prefs.voice * 10.0).round() as i32,
            _ => return None,
        })
    }

    fn set_pref(self, prefs: &mut crate::prefs::Prefs, value: i32) {
        match self {
            Slider::Detail => prefs.detail = value,
            Slider::Difficulty => prefs.difficulty = value,
            Slider::ScrollRate => prefs.scroll_rate = 6 - value,
            Slider::Music => prefs.music = value as f32 * 0.1,
            Slider::Sound => prefs.sound = value as f32 * 0.1,
            Slider::Voice => prefs.voice = value as f32 * 0.1,
            _ => {}
        }
    }

    fn value(self, options: &crate::game::Options) -> i32 {
        match self {
            // The slider's right end is the fastest, which is speed number 0.
            Slider::Speed => 6 - options.speed,
            Slider::Credits => options.credits,
            Slider::UnitCount => options.unit_count,
            _ => 0,
        }
    }

    fn set(self, options: &mut crate::game::Options, value: i32) {
        match self {
            Slider::Speed => options.speed = 6 - value,
            Slider::Credits => options.credits = value,
            Slider::UnitCount => options.unit_count = value,
            _ => {}
        }
    }
}

/// Height of a closed combo box, and of a row of its list.
// 24, whatever the template says; rows are the font's
// height plus 6.
const COMBO_HEIGHT: f32 = 24.0;
const COMBO_ROW: f32 = 22.0;
/// Width of the value box at a slider's right end.
const SLIDER_VALUE_BOX: f32 = 50.0;
const DISABLED_EDGE: Color = Color::srgb_u8(159, 0, 0);

impl Painter<'_> {
    /// A one-pixel outline round a rectangle, outside it, with nothing inside.
    // The controls have no fill of their own; the
    // dialog's art shows through.
    fn outline(&mut self, p: &mut ChildSpawnerCommands, area: Rect, color: Color) {
        p.spawn((
            Node {
                border: UiRect::all(Val::Px(1.0)),
                ..place(area.min.x - 1.0, area.min.y - 1.0, area.width() + 2.0, area.height() + 2.0)
            },
            BorderColor::all(color),
            Pickable::IGNORE,
        ));
    }

    /// A closed combo box.
    // The box, the arrow 19 in from the right and one
    // down, the text two in and centred, and in a colour list the swatch in place
    // of the text.
    fn combo(
        &mut self,
        p: &mut ChildSpawnerCommands,
        corner: Vec2,
        width: f32,
        text: &str,
        swatch: Option<Color>,
        enabled: bool,
        open: bool,
    ) -> Rect {
        let area = Rect::from_corners(corner, corner + Vec2::new(width, COMBO_HEIGHT));
        self.outline(p, area, if enabled { EDGE } else { DISABLED_EDGE });
        let arrow = self.shell.pcx(self.images, if open { "uparrowp.pcx" } else { "dnarrowr.pcx" });
        p.spawn(picture(&arrow, area.max.x - 19.0, area.min.y + 1.0));
        match swatch {
            Some(color) => {
                p.spawn((place(area.min.x + 2.0, area.min.y + 2.0, width - 24.0, 20.0), BackgroundColor(color), Pickable::IGNORE));
            }
            None if !text.is_empty() => {
                let color = if enabled { TEXT } else { TEXT_DISABLED };
                let text_area = Rect::new(area.min.x + 2.0, area.min.y, area.max.x - 20.0, area.max.y);
                self.label(p, text, color, text_area, Align::Left);
            }
            None => {}
        }
        area
    }

    /// The open list under a combo box.
    // Just under the box and as wide; rows 22
    // high; the dialog's art behind; text three in; a colour list's rows filled
    // with the colour, inset by two; a scroll bar 20 wide when there are more
    // items than rows.
    fn drop_list(
        &mut self,
        p: &mut ChildSpawnerCommands,
        combo: Combo,
        area: Rect,
        items: &[(String, Option<Color>)],
        rows: usize,
        scroll: usize,
    ) {
        // Anything else on the screen is out of reach while the list is open; a
        // click outside closes it.
        p.spawn((place(0.0, 0.0, SCREEN.x, SCREEN.y), Button, OptionControl::CloseCombo, GlobalZIndex(5)));
        let rows = rows.min(items.len()).max(1);
        let scrolls = items.len() > rows;
        let list = Rect::new(area.min.x, area.max.y + 1.0, area.max.x, area.max.y + 1.0 + rows as f32 * COMBO_ROW);
        let backdrop = self.shell.shp(self.images, "mnscrnl.shp", "shell.pal", 0);
        let up = self.shell.pcx(self.images, "uparrowr.pcx");
        let down = self.shell.pcx(self.images, "dnarrowr.pcx");
        let grip = self.shell.pcx(self.images, "sbgripm.pcx");
        let grip_top = self.shell.pcx(self.images, "sbgript.pcx");
        let grip_bottom = self.shell.pcx(self.images, "sbgripb.pcx");
        let row_width = list.width() - if scrolls { 20.0 } else { 0.0 };
        let texts: Vec<Option<Pic>> = items
            .iter()
            .map(|(text, swatch)| swatch.is_none().then(|| self.shell.text(self.images, text, TEXT)))
            .collect();
        p.spawn((
            Node {
                border: UiRect::all(Val::Px(1.0)),
                overflow: Overflow::clip(),
                ..place(list.min.x - 1.0, list.min.y - 1.0, list.width() + 2.0, list.height() + 2.0)
            },
            BorderColor::all(EDGE),
            GlobalZIndex(6),
        ))
        .with_children(|inner| {
            inner.spawn(picture(&backdrop, -list.min.x, -list.min.y));
            for row in 0..rows {
                let index = scroll + row;
                let Some((_, swatch)) = items.get(index) else {
                    break;
                };
                inner
                    .spawn((
                        place(0.0, row as f32 * COMBO_ROW, row_width, COMBO_ROW),
                        Button,
                        Hovered::default(),
                        ComboRow(combo, index),
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|line| {
                        if let Some(color) = swatch {
                            line.spawn((
                                place(2.0, 2.0, row_width - 4.0, COMBO_ROW - 4.0),
                                BackgroundColor(*color),
                                Pickable::IGNORE,
                            ));
                        }
                        if let Some(text) = &texts[index] {
                            line.spawn(picture(text, 3.0, ((COMBO_ROW - text.size.y) / 2.0).round()));
                        }
                    });
            }
            if scrolls {
                let x = list.width() - 19.0;
                let most = items.len() - rows;
                // Grip height and place, by the engine's formula.
                let track = list.height() - 44.0;
                let height = (track - 0.2 * ((most + 1) as f32).ln() * track).max(14.0).floor();
                let top = 22.0 + ((track - height).max(1.0) * scroll as f32 / most as f32).floor();
                // The middle art tiled down the grip,
                // with the cap pieces at its top and bottom.
                inner
                    .spawn((
                        Node { overflow: Overflow::clip(), ..place(x, top, 18.0, height) },
                        Pickable::IGNORE,
                    ))
                    .with_children(|bar| {
                        let mut y = 0.0;
                        while y < height {
                            bar.spawn(picture(&grip, 0.0, y));
                            y += grip.size.y.max(1.0);
                        }
                        bar.spawn(picture(&grip_top, 0.0, 0.0));
                        bar.spawn(picture(&grip_bottom, 0.0, height - grip_bottom.size.y));
                    });
                inner.spawn((
                    place(x, 0.0, 18.0, 22.0),
                    ImageNode::new(up.image.clone()),
                    Button,
                    OptionControl::ScrollCombo(-1),
                ));
                inner.spawn((
                    place(x, list.height() - 22.0, 18.0, 22.0),
                    ImageNode::new(down.image.clone()),
                    Button,
                    OptionControl::ScrollCombo(1),
                ));
            }
        });
    }

    /// A slider: the outlined run the grip moves in, and the box showing its value.
    // There is no trough art; the three "trough"
    // pictures are the value box's background, 50 wide at the right end.
    fn value_slider(&mut self, p: &mut ChildSpawnerCommands, area: Rect, slider: Slider, value: i32) {
        self.slider(p, area, slider, value, true);
    }

    /// A slider, with or without the box showing its value.
    // Sliders sent message 0x4ae (the skirmish screen's, the volumes) have
    // the value box; 0x4ac (detail, difficulty, scroll rate) have their
    // meaning in a label above instead.
    fn slider(&mut self, p: &mut ChildSpawnerCommands, area: Rect, slider: Slider, value: i32, boxed: bool) {
        let (low, range, step) = slider.span();
        let run = Rect::new(area.min.x, area.min.y, area.max.x - if boxed { SLIDER_VALUE_BOX } else { 0.0 }, area.max.y);
        self.outline(p, run, EDGE);
        let grip = self.shell.pcx(self.images, "trakgrip.pcx");
        if boxed {
            let left = self.shell.pcx(self.images, "trofl.pcx");
            let middle = self.shell.pcx(self.images, "trofm.pcx");
            let right = self.shell.pcx(self.images, "trofr.pcx");
            let box_x = area.max.x - 49.0;
            p.spawn((
                Node { overflow: Overflow::clip(), ..place(box_x, area.min.y - 1.0, SLIDER_VALUE_BOX, 24.0) },
                Pickable::IGNORE,
            ))
            .with_children(|value_box| {
                value_box.spawn(picture(&middle, 0.0, 0.0));
                value_box.spawn(picture(&left, 0.0, 0.0));
                value_box.spawn(picture(&right, SLIDER_VALUE_BOX - right.size.x, 0.0));
            });
            self.outline(p, Rect::new(run.max.x + 2.0, area.min.y, area.max.x, area.max.y), EDGE);
            let shown = (value / step) * step;
            self.label(p, &shown.to_string(), TEXT, Rect::new(box_x, area.min.y, area.max.x, area.max.y), Align::Center);
        }
        // The grip: one in from the left, plus its share of the travel.
        let travel = (run.width() - 13.0).max(1.0);
        let x = 1.0 + (travel * (value - low) as f32 / range.max(1) as f32).floor();
        p.spawn((place(run.min.x + x, area.min.y, grip.size.x, grip.size.y), ImageNode::new(grip.image.clone()), Pickable::IGNORE, SliderGrip(slider, run.min.x)));
        p.spawn((
            place(run.min.x, run.min.y, run.width(), run.height()),
            Button,
            bevy::ui::RelativeCursorPosition::default(),
            SliderTrack(slider),
        ));
    }
}

fn spawn_skirmish(
    mut commands: Commands,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
    choice: Res<MapChoice>,
    setup: Res<GameSetup>,
    open: Res<OpenCombo>,
    lobby: Res<crate::net::Lobby>,
) {
    // In a network game this screen is the lobby: the host's is the game being
    // set up, and each player who joined sees the same with only their own
    // row's country and colour to change.
    let networked = lobby.mode != crate::net::LobbyMode::Off;
    let editable = lobby.may_edit_game();
    let status = lobby.status.clone();
    let options = setup.options;
    let open = open.0;
    let max_players = shell.maps.get(choice.selected).map_or(8, |map| map.max_players);
    let back = if networked { MenuAction::LeaveLobby } else { MenuAction::Goto(Screen::SinglePlayer) };
    let buttons: Vec<(usize, &str, MenuAction)> = if editable {
        vec![(1, "GUI:StartGame", MenuAction::StartGame), (2, "GUI:ChooseMap", MenuAction::Goto(Screen::ChooseMap)), (LAST_BUTTON_SLOT, "GUI:Back", back)]
    } else {
        vec![(LAST_BUTTON_SLOT, "GUI:Back", back)]
    };
    spawn_screen(&mut commands, Screen::Skirmish, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            if networked { "GUI:Lobby" } else { "GUI:SkirmishGame" },
            3.0,
            1,
            &buttons,
        );
        if networked {
            painter.label(p, &status, TEXT, Rect::new(20.0, 548.0, 620.0, 566.0), Align::Left);
        }
        painter.map_summary(p, choice.selected, choice.mode);
        painter.map_preview(p, choice.selected);

        // Eight player rows: name or AI level, flag, country, colour.
        // The first row is the player, with random or any of the nine countries,
        // and random or any of the eight colours no other row has; the other
        // seven are computer players, each none, easy, medium or brutal; a row
        // set to none shows random for both and its lists do not open.
        let player = painter.shell.string("GUI:Player");
        let random = painter.shell.string("GUI:RandomEx");
        let marks = painter.shell.string("GUI:RandomAsSymbols");
        let levels: Vec<(String, Option<Color>)> = LEVELS.iter().map(|(_, key)| (painter.shell.string(key), None)).collect();
        let countries: Vec<(String, Option<Color>)> = std::iter::once((random.clone(), None))
            .chain(COUNTRIES.iter().map(|(key, _)| (painter.shell.string(key), None)))
            .collect();
        let colour_items = |values: &[i32]| -> Vec<(String, Option<Color>)> {
            values
                .iter()
                .map(|&value| match usize::try_from(value) {
                    Ok(colour) => (String::new(), Some(Color::srgb_u8(SWATCHES[colour % 8][0], SWATCHES[colour % 8][1], SWATCHES[colour % 8][2]))),
                    Err(_) => (marks.clone(), None),
                })
                .collect()
        };
        let mut lists = Vec::new();
        for row in 0..8 {
            let y = 11.0 + 16.0 * row as f32;
            let mut copy = options;
            let (country, colour) = {
                let (country, colour) = row_choice(&mut copy, row);
                (*country, *colour)
            };
            // DATA: missions.pkt `MaxPlayers`.
            let allowed = row < max_players;
            let level = if row == 0 { 0 } else if allowed { options.opponents[row - 1].level } else { -1 };
            let active = level >= 0;
            let name = dialog_rect(35.0, y, 100.0, 12.0);
            // A row taken by a player on another machine shows who, where a
            // computer row shows its level.
            let human_row = row > 0 && level == crate::net::HUMAN_ROW;
            if human_row || (row == 0 && networked) {
                let area = Rect::from_corners(name.min, name.min + Vec2::new(name.width(), COMBO_HEIGHT));
                let named = if row == lobby.my_row { lobby.name.as_str() } else { lobby.names[row].as_str() };
                let who = if named.is_empty() { format!("{player} {}", row + 1) } else { named.to_owned() };
                let who = if row == lobby.my_row { format!("{who} (you)") } else { who };
                painter.outline(p, area, EDGE);
                painter.label(p, &who, TEXT, Rect::new(area.min.x + 2.0, area.min.y, area.max.x, area.max.y), Align::Left);
            } else if row == 0 {
                // The name is an edit box: click it and type. The player's name
                // can be edited here and is kept for next time.
                let area = Rect::from_corners(name.min, name.min + Vec2::new(name.width(), 20.0));
                let shown = if lobby.name.is_empty() { player.clone() } else { lobby.name.clone() };
                text_box(painter, p, area, &shown, crate::net::Focus::Name, lobby.focus);
            } else {
                let item = LEVELS.iter().position(|(value, _)| *value == level).unwrap_or(0);
                let is_open = matches!(open, Some((Combo::Level(r), _)) if r == row);
                let allowed = allowed && editable;
                let area = painter.combo(p, name.min, name.width(), &levels[item].0, None, allowed, is_open);
                if allowed {
                    p.spawn((place(area.max.x - 20.0, area.min.y, 20.0, area.height()), Button, OptionControl::Combo(Combo::Level(row))));
                    lists.push((Combo::Level(row), area));
                }
            }
            if active {
                let flag = if country < 0 { "rani.pcx" } else { COUNTRIES[country as usize % 9].1 };
                let pic = painter.shell.pcx(painter.images, flag);
                let slot = dialog_rect(143.0, y, 32.0, 12.0);
                p.spawn(picture(&pic, slot.min.x, slot.min.y + ((COMBO_HEIGHT - pic.size.y) / 2.0).round()));
            }
            let country_at = dialog_rect(180.0, y, 78.0, 12.0);
            let colour_at = dialog_rect(264.0, y, 35.0, 12.0);
            let country_item = if country < 0 { 0 } else { country as usize + 1 };
            let (country_text, swatch, colour_text) = if active {
                let swatch = usize::try_from(colour).ok().map(|c| Color::srgb_u8(SWATCHES[c % 8][0], SWATCHES[c % 8][1], SWATCHES[c % 8][2]));
                (countries[country_item.min(9)].0.as_str(), swatch, if colour < 0 { marks.as_str() } else { "" })
            } else {
                (random.as_str(), None, marks.as_str())
            };
            let country_open = matches!(open, Some((Combo::Country(r), _)) if r == row);
            let colour_open = matches!(open, Some((Combo::Colour(r), _)) if r == row);
            // A row that is somebody else's to choose for is shown, not offered.
            let mine = active && lobby.may_edit_row(row, human_row);
            let country_area = painter.combo(p, country_at.min, country_at.width(), country_text, None, mine, country_open);
            let colour_area = painter.combo(p, colour_at.min, colour_at.width(), colour_text, swatch, mine, colour_open);
            if mine {
                // Only the arrow's strip, the right 20
                // pixels, opens the list.
                for (area, combo) in [(country_area, Combo::Country(row)), (colour_area, Combo::Colour(row))] {
                    p.spawn((place(area.max.x - 20.0, area.min.y, 20.0, area.height()), Button, OptionControl::Combo(combo)));
                    lists.push((combo, area));
                }
            }
        }

        for (label, x, y, width, on, control) in [
            ("GUI:ShortGame", 35.0, 145.0, 100.0, options.short_game, Some(OptionControl::ShortGame)),
            ("GUI:MCVRepacks", 35.0, 162.0, 100.0, options.mcv_repacks, Some(OptionControl::McvRepacks)),
            ("GUI:CratesAppear", 35.0, 179.0, 100.0, options.crates, Some(OptionControl::Crates)),
            ("GUI:SuperWeaponsAllowed", 35.0, 197.0, 103.0, options.super_weapons, Some(OptionControl::SuperWeapons)),
            // DATA: disabled (WS_DISABLED) in dialog template 258.
            ("GUI:BuildOffAlly", 146.0, 196.0, 166.0, false, None),
        ] {
            // The 18x18 picture at the control's top
            // left, the text 26 to the right and centred on the control, and only
            // the picture takes the click.
            // The working check boxes are nudged a
            // pixel left.
            let control = control.filter(|_| editable);
            let mut area = dialog_rect(x, y, width, 10.0);
            if control.is_some() {
                area.min.x -= 1.0;
                area.max.x -= 1.0;
            }
            let pic = painter.shell.pcx(painter.images, if on { "cce_i.pcx" } else { "cue_i.pcx" });
            let mut image = ImageNode::new(pic.image.clone());
            if control.is_none() {
                image.color = Color::srgba(1.0 - 96.0 / 255.0, 1.0 - 96.0 / 255.0, 1.0 - 96.0 / 255.0, 1.0);
            }
            let mut picture = p.spawn((place(area.min.x, area.min.y, pic.size.x, pic.size.y), image));
            if let Some(control) = control {
                picture.insert((Button, control));
            }
            let text = painter.shell.string(label);
            let color = if control.is_some() { TEXT } else { TEXT_DISABLED };
            let text_area = Rect::new(area.min.x + 26.0, area.min.y, area.max.x.max(area.min.x + 26.0), area.min.y + pic.size.y);
            painter.label(p, &text, color, text_area, Align::Left);
        }

        for (label, y, slider) in [
            ("GUI:GameSpeed", 145.0, Slider::Speed),
            ("GUI:Credits", 162.0, Slider::Credits),
            ("GUI:UnitCount", 179.0, Slider::UnitCount),
        ] {
            let text = painter.shell.string(label);
            // The label's text is at the top
            // of its rectangle; the unit count slider is nudged a pixel up.
            let at = dialog_rect(146.0, y, 60.0, 10.0);
            let pic = painter.shell.text(painter.images, &text, TEXT);
            p.spawn(picture(&pic, at.min.x, at.min.y));
            let mut bar = dialog_rect(214.0, y, 85.0, 13.0);
            if matches!(slider, Slider::UnitCount) {
                bar.min.y -= 1.0;
                bar.max.y -= 1.0;
            }
            painter.value_slider(p, bar, slider, slider.value(&options));
        }

        // The open list goes on last, over everything.
        // Seven rows for countries, nine for colours.
        if let Some((combo, scroll)) = open
            && let Some((_, area)) = lists.iter().find(|(which, _)| *which == combo) {
                match combo {
                    Combo::Level(_) => painter.drop_list(p, combo, *area, &levels, 4, scroll),
                    Combo::Country(_) => painter.drop_list(p, combo, *area, &countries, 7, scroll),
                    Combo::Colour(row) => painter.drop_list(p, combo, *area, &colour_items(&colour_values(&options, row)), 9, scroll),
                    Combo::Resolution => {}
                }
            }
    });
}

/// The Options screen.
// Four
// sections (display, game, interface, audio) with their headings, labels,
// sliders, the resolution list and three check boxes; Keyboard and Network in
// the sidebar; Main Menu at the bottom.
// Not original behaviour: the Full Screen check box (the original always fills
// the screen), put under the resolution list.
fn spawn_options(
    mut commands: Commands,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
    prefs: Res<crate::prefs::Prefs>,
    video: Res<crate::video::Video>,
    open: Res<OpenCombo>,
) {
    let prefs = *prefs;
    let video = *video;
    let open = open.0;
    spawn_screen(&mut commands, Screen::Options, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:OptionsMenu",
            9.0,
            0,
            &[
                (0, "GUI:Keyboard", MenuAction::Mock("Keyboard")),
                (1, "GUI:Network", MenuAction::Mock("Network options")),
                (LAST_BUTTON_SLOT, "GUI:MainMenu", MenuAction::Goto(Screen::MainMenu)),
            ],
        );
        let mut text = |painter: &mut Painter, key: &str, rect: Rect, align: Align| {
            let words = painter.shell.string(key);
            painter.label(p, &words, TEXT, rect, align);
        };
        // Headings and labels; the right-aligned words say which way the
        // slider's right end goes.
        for (key, x, y, w, align) in [
            ("GUI:DisplayOptions", 10.0, 10.0, 140.0, Align::Left),
            ("GUI:VisualDetails", 25.0, 30.0, 60.0, Align::Left),
            ("GUI:HigherDetail", 85.0, 30.0, 60.0, Align::Right),
            ("GUI:SetResolution", 170.0, 30.0, 120.0, Align::Left),
            ("GUI:GameOptions", 10.0, 75.0, 298.0, Align::Left),
            ("GUI:Difficulty", 25.0, 95.0, 60.0, Align::Left),
            ("GUI:Harder", 85.0, 95.0, 60.0, Align::Right),
            ("GUI:UIOptions", 10.0, 140.0, 298.0, Align::Left),
            ("GUI:ScrollRate", 170.0, 160.0, 60.0, Align::Left),
            ("GUI:Faster", 230.0, 160.0, 60.0, Align::Right),
            ("GUI:AudioOptions", 10.0, 215.0, 298.0, Align::Left),
            ("GUI:MusicVolume", 25.0, 235.0, 85.0, Align::Center),
            ("GUI:SoundVolume", 122.0, 235.0, 85.0, Align::Center),
            ("GUI:VoiceVolume", 220.0, 235.0, 85.0, Align::Center),
        ] {
            text(painter, key, dialog_rect(x, y, w, 10.0), align);
        }
        for (x, y, w, slider, boxed) in [
            (25.0, 45.0, 120.0, Slider::Detail, false),
            (25.0, 110.0, 120.0, Slider::Difficulty, false),
            (170.0, 175.0, 120.0, Slider::ScrollRate, false),
            (25.0, 250.0, 85.0, Slider::Music, true),
            (122.0, 250.0, 85.0, Slider::Sound, true),
            (220.0, 250.0, 85.0, Slider::Voice, true),
        ] {
            let value = slider.pref_value(&prefs).unwrap_or(0);
            painter.slider(p, dialog_rect(x, y, w, 13.0), slider, value, boxed);
        }
        // Check boxes: the picture at the top left, the words 26 to the right.
        for (key, x, y, on, control) in [
            ("GUI:Tooltips", 25.0, 160.0, prefs.tooltips, OptionControl::Tooltips),
            ("GUI:TargetLines", 25.0, 175.0, prefs.target_lines, OptionControl::TargetLines),
            ("GUI:ShowHidden", 25.0, 190.0, prefs.show_hidden, OptionControl::ShowHidden),
            ("Full Screen", 170.0, 62.0, video.fullscreen, OptionControl::FullScreen),
        ] {
            let area = dialog_rect(x, y, 130.0, 10.0);
            let pic = painter.shell.pcx(painter.images, if on { "cce_i.pcx" } else { "cue_i.pcx" });
            p.spawn((place(area.min.x, area.min.y, pic.size.x, pic.size.y), ImageNode::new(pic.image.clone()), Button, control));
            let words = if key.starts_with("GUI:") { painter.shell.string(key) } else { key.to_owned() };
            painter.label(p, &words, TEXT, Rect::new(area.min.x + 26.0, area.min.y, area.max.x.max(area.min.x + 26.0), area.min.y + pic.size.y), Align::Left);
        }
        // The resolution list, last so that its open list is over the rest.
        let sizes: Vec<(String, Option<Color>)> = RESOLUTIONS.iter().map(|(w, h)| (format!("{w} x {h}"), None)).collect();
        let at = dialog_rect(170.0, 45.0, 120.0, 13.0);
        let is_open = matches!(open, Some((Combo::Resolution, _)));
        let chosen = resolution_item(&video);
        let area = painter.combo(p, at.min, at.width(), &sizes[chosen].0, None, true, is_open);
        p.spawn((place(area.max.x - 20.0, area.min.y, 20.0, area.height()), Button, OptionControl::Combo(Combo::Resolution)));
        if let Some((Combo::Resolution, scroll)) = open {
            painter.drop_list(p, Combo::Resolution, area, &sizes, RESOLUTION_ROWS, scroll);
        }
    });
}

/// A click on one of the skirmish screen's option controls.
// Opening and closing a list, each with its sound; a click outside an open list
// closes it and does nothing else.
fn on_option(
    activate: On<Activate>,
    controls: Query<&OptionControl>,
    rows: Query<&ComboRow>,
    mut setup: ResMut<GameSetup>,
    mut open: ResMut<OpenCombo>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut redraw: ResMut<Redraw>,
    mut cues: MessageWriter<crate::audio::Cue>,
    screen: Res<State<Screen>>,
    (mut prefs, mut video, mut window): (ResMut<crate::prefs::Prefs>, ResMut<crate::video::Video>, Single<&mut Window, With<PrimaryWindow>>),
    rules: Res<crate::files::Rules>,
) {
    let mut sound = |key: &str| {
        cues.write_batch(rules.sound(key).map(crate::audio::Cue::Sound));
    };
    let options = &mut setup.options;
    if let Ok(&ComboRow(combo, index)) = rows.get(activate.entity) {
        match combo {
            // A row set to none goes
            // back to random for both.
            Combo::Level(row) => {
                let slot = &mut options.opponents[row.saturating_sub(1).min(6)];
                slot.level = LEVELS[index.min(3)].0;
                if slot.level < 0 {
                    *slot = crate::game::Opponent::NONE;
                }
            }
            // The first item is Random (-2); the rest are the countries in order.
            Combo::Country(row) => *row_choice(options, row).0 = if index == 0 { -2 } else { index as i32 - 1 },
            Combo::Colour(row) => {
                let value = colour_values(options, row).get(index).copied().unwrap_or(-2);
                *row_choice(options, row).1 = value;
            }
            // Not original behaviour: the original asks to keep the new mode
            // (dialog 217, `GUI:ConfirmModeChange`) and goes back if not
            // answered; here the window takes the size at once.
            Combo::Resolution => {
                let size = RESOLUTIONS[index.min(RESOLUTIONS.len() - 1)];
                video.screen = Some(size);
                crate::video::apply(&video, &mut window);
                crate::net::Settings { screen: Some(size), ..crate::net::Settings::read() }.save();
            }
        }
        open.0 = None;
        sound("GUIComboCloseSound");
    } else if let Ok(control) = controls.get(activate.entity) {
        match *control {
            OptionControl::Combo(combo) => {
                open.0 = match open.0 {
                    Some((now, _)) if now == combo => None,
                    _ => {
                        // Start with the chosen item in view.
                        let (item, rows, items) = match combo {
                            Combo::Level(_) => (0, 4, 4),
                            Combo::Country(row) => {
                                let chosen = *row_choice(options, row).0;
                                (if chosen < 0 { 0 } else { chosen as usize + 1 }, 7, 10)
                            }
                            Combo::Colour(row) => {
                                let values = colour_values(options, row);
                                let chosen = *row_choice(options, row).1;
                                (values.iter().position(|&value| value == chosen).unwrap_or(0), 9, values.len())
                            }
                            Combo::Resolution => (resolution_item(&video), RESOLUTION_ROWS, RESOLUTIONS.len()),
                        };
                        Some((combo, item.saturating_sub(rows - 1).min(items - rows.min(items))))
                    }
                };
                sound("GUIComboOpenSound");
            }
            OptionControl::CloseCombo => {
                open.0 = None;
                sound("GUIComboCloseSound");
            }
            OptionControl::ScrollCombo(step) => {
                if let Some((combo, scroll)) = open.0 {
                    let most = match combo {
                        Combo::Country(_) => 3,
                        Combo::Resolution => (RESOLUTIONS.len() - RESOLUTION_ROWS) as i32,
                        _ => 0,
                    };
                    open.0 = Some((combo, (scroll as i32 + step).clamp(0, most) as usize));
                }
            }
            OptionControl::ShortGame => options.short_game = !options.short_game,
            OptionControl::McvRepacks => options.mcv_repacks = !options.mcv_repacks,
            OptionControl::Crates => options.crates = !options.crates,
            OptionControl::SuperWeapons => options.super_weapons = !options.super_weapons,
            OptionControl::Tooltips => prefs.tooltips = !prefs.tooltips,
            OptionControl::TargetLines => prefs.target_lines = !prefs.target_lines,
            OptionControl::ShowHidden => prefs.show_hidden = !prefs.show_hidden,
            OptionControl::FullScreen => {
                video.fullscreen = !video.fullscreen;
                crate::video::apply(&video, &mut window);
                crate::net::Settings { fullscreen: video.fullscreen, ..crate::net::Settings::read() }.save();
            }
        }
        if matches!(control, OptionControl::Tooltips | OptionControl::TargetLines | OptionControl::ShowHidden) {
            prefs.save();
        }
        if !matches!(control, OptionControl::Combo(_) | OptionControl::CloseCombo | OptionControl::ScrollCombo(_)) {
            sound("GUICheckboxSound");
        }
    } else {
        return;
    }
    // Draw the screen again with the new choice.
    redraw.0 = true;
    next_screen.set(*screen.get());
}

/// The sizes the Options screen offers.
// Not original behaviour: the original lists the display modes the graphics
// card reports; a window can be any size, so these are
// common ones.
const RESOLUTIONS: [(u32, u32); 12] = [
    (640, 480),
    (800, 600),
    (1024, 768),
    (1152, 864),
    (1280, 720),
    (1280, 800),
    (1280, 1024),
    (1366, 768),
    (1440, 900),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
];

/// Rows the resolution list shows at once.
const RESOLUTION_ROWS: usize = 4;

/// The list's item for the size asked for (the nearest smaller one if it is
/// not listed).
fn resolution_item(video: &crate::video::Video) -> usize {
    let (w, h) = video.screen.unwrap_or(crate::video::DEFAULT);
    RESOLUTIONS.iter().rposition(|&(rw, rh)| rw <= w && rh <= h).unwrap_or(0)
}

/// The row of an open list under the mouse is filled in the highlight colour.
// The highlight follows the mouse.
fn highlight_combo_rows(mut rows: Query<(&Hovered, &mut BackgroundColor), With<ComboRow>>) {
    for (hovered, mut fill) in &mut rows {
        let wanted = if hovered.get() { EDGE } else { Color::NONE };
        if fill.0 != wanted {
            fill.0 = wanted;
        }
    }
}

/// Dragging a slider's grip, or clicking in its run, sets its value.
// The position from the mouse: x less 6, kept between 1 and the run's width less
// 12, as a share of the travel, rounded down to the step; and `GenericClick` with
// each change.
fn drag_sliders(
    tracks: Query<(&SliderTrack, &bevy::ui::RelativeCursorPosition, &ComputedNode, Has<Pressed>)>,
    mut grips: Query<(&SliderGrip, &mut Node)>,
    mut setup: ResMut<GameSetup>,
    mut dragging: Local<Option<(Slider, i32)>>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut redraw: ResMut<Redraw>,
    mut cues: MessageWriter<crate::audio::Cue>,
    lobby: Res<crate::net::Lobby>,
    (screen, mut prefs): (Res<State<Screen>>, ResMut<crate::prefs::Prefs>),
    rules: Res<crate::files::Rules>,
) {
    // A player who joined a network game cannot change its options.
    if *screen.get() == Screen::Skirmish && !lobby.may_edit_game() {
        return;
    }
    let mut held = None;
    for (track, cursor, node, pressed) in &tracks {
        let (Some(at), true) = (cursor.normalized, pressed) else {
            continue;
        };
        let (low, range, step) = track.0.span();
        let width = node.size().x * node.inverse_scale_factor();
        let travel = (width - 13.0).max(1.0);
        let x = ((at.x + 0.5) * width - 6.0).clamp(1.0, width - 12.0);
        let position = (((x - 1.0) * (range + 1) as f32 / travel) as i32).min(range);
        let value = ((position + low) / step) * step;
        let left = 1.0 + (travel * (value - low) as f32 / range.max(1) as f32).floor();
        held = Some((track.0, value, left));
    }
    match (held, *dragging) {
        (Some((slider, value, left)), before) => {
            if before.is_none_or(|(_, was)| was != value) {
                cues.write_batch(rules.sound("GenericClick").map(crate::audio::Cue::Sound));
                // A volume takes effect as its
                // slider moves.
                if matches!(slider, Slider::Music | Slider::Sound | Slider::Voice) {
                    slider.set_pref(&mut prefs, value);
                }
            }
            for (grip, mut node) in &mut grips {
                if grip.0 == slider {
                    node.left = Val::Px(grip.1 + left);
                }
            }
            *dragging = Some((slider, value));
        }
        (None, Some((slider, value))) => {
            if slider.pref_value(&prefs).is_some() {
                slider.set_pref(&mut prefs, value);
                prefs.save();
            } else {
                slider.set(&mut setup.options, value);
            }
            *dragging = None;
            redraw.0 = true;
            next_screen.set(*screen.get());
        }
        (None, None) => {}
    }
}

fn spawn_choose_map(
    mut commands: Commands,
    mut shell: ResMut<Shell>,
    mut images: ResMut<Assets<Image>>,
    mut choice: ResMut<MapChoice>,
) {
    choice.highlighted = choice.selected;
    choice.scroll = 0;
    let highlighted = choice.highlighted;
    spawn_screen(&mut commands, Screen::ChooseMap, &mut shell, &mut images, |painter, p| {
        painter.sidebar(
            p,
            "GUI:ChooseMap",
            3.0,
            1,
            &[
                (0, "GUI:UseMap", MenuAction::UseMap),
                (1, "GUI:CreateRandomMap", MenuAction::Mock("Random Map")),
                (LAST_BUTTON_SLOT, "GUI:Cancel", MenuAction::Goto(Screen::Skirmish)),
            ],
        );
        painter.map_preview(p, highlighted);

        for (label, x, y, width, height) in [
            ("GUI:SelectEngagement", 23.0, 20.0, 257.0, 12.0),
            ("GUI:GameType", 20.0, 60.0, 130.0, 10.0),
            ("GUI:GameMap", 168.0, 60.0, 130.0, 10.0),
        ] {
            let text = painter.shell.string(label);
            painter.label_top(p, &text, TEXT, dialog_rect(x, y, width, height));
        }

        let modes = painter.shell.modes.clone();
        painter.list_box(p, dialog_rect(20.0, 78.0, 130.0, 160.0), &modes, false);
        let maps: Vec<String> = painter.shell.maps.iter().map(|m| m.name.clone()).collect();
        painter.list_box(p, dialog_rect(168.0, 78.0, 130.0, 160.0), &maps, true);
    });
}

/// Starts the buttons sliding in when a menu opens.
// With `GUIMoveInSound`.
fn start_slide(
    mut slide: ResMut<Slide>,
    mut redraw: ResMut<Redraw>,
    time: Res<Time<Real>>,
    mut cues: MessageWriter<crate::audio::Cue>,
    rules: Res<crate::files::Rules>,
) {
    if std::mem::take(&mut redraw.0) {
        // The same screen drawn again: the buttons are already in place.
        *slide = Slide { started: time.elapsed_secs_f64() - 60.0, leaving: None };
        return;
    }
    *slide = Slide { started: time.elapsed_secs_f64(), leaving: None };
    cues.write_batch(rules.sound("GUIMoveInSound").map(crate::audio::Cue::Sound));
}

/// Shows each sidebar slot's frame: sliding in, at rest, or sliding out.
// Slot i from
// the top starts a step after slot i − 1 and takes six steps; in, a button runs
// frames 10 down to 5 and an empty slot 16 down to 11; out, 5 up to 10 and 11 up to
// 16, ending on 10. A button at rest shows 2, or 4 while pressed, when its words
// move one right and two down. The words are hidden while the buttons slide. The
// click is heard as the button goes down.
// The mouse resting on a button
// does not light it; frame 3 belongs to a blink only the network lobby starts.
fn slot_frames(
    mut slots: Query<(Has<Pressed>, &Hovered, &mut Slot, &mut ImageNode, Option<&Children>), Without<SlotText>>,
    empty: Query<(&mut Slot, &mut ImageNode), (Without<Hovered>, Without<SlotText>)>,
    mut texts: Query<&mut Visibility, With<SlotText>>,
    mut words: Query<(&mut Node, &SlotTextHome)>,
    slide: Res<Slide>,
    time: Res<Time<Real>>,
    mut cues: MessageWriter<crate::audio::Cue>,
    rules: Res<crate::files::Rules>,
) {
    let now = time.elapsed_secs_f64();
    let (since, out) = match slide.leaving {
        Some((_, began)) => (began, true),
        None => (slide.started, false),
    };
    let step = ((now - since) / SLIDE_STEP) as i32;
    let sliding = |index: usize, button: bool| -> Option<usize> {
        let k = step - (index as i32 + 1);
        Some(match (out, button) {
            (false, _) if k < 0 => 10,
            (false, true) if k < 6 => 10 - k as usize,
            (false, false) if k < 6 => 16 - k as usize,
            (false, false) => 0,
            (false, true) => return None,
            (true, true) if k < 0 => 1,
            (true, false) if k < 0 => 0,
            (true, true) if k < 6 => 5 + k as usize,
            (true, false) if k < 6 => 11 + k as usize,
            (true, _) => 10,
        })
    };
    let settled = !out && step >= SLOT_COUNT as i32 + 6;
    for mut visibility in &mut texts {
        let wanted = if settled { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    for (pressed, hovered, mut slot, mut image, children) in &mut slots {
        let down = pressed && hovered.get();
        for child in children.into_iter().flatten() {
            if let Ok((mut node, home)) = words.get_mut(*child) {
                let (x, y) = if down { (home.0 + 1.0, home.1 + 2.0) } else { (home.0, home.1) };
                if node.left != Val::Px(x) || node.top != Val::Px(y) {
                    node.left = Val::Px(x);
                    node.top = Val::Px(y);
                }
            }
        }
        if pressed && !slot.was_pressed {
            cues.write_batch(rules.sound("GUIMainButtonSound").map(crate::audio::Cue::Sound));
        }
        slot.was_pressed = pressed;
        let frame = sliding(slot.index, true).unwrap_or(if down { 4 } else { 2 });
        if image.image != slot.frames[frame] {
            image.image = slot.frames[frame].clone();
        }
    }
    let mut empty = empty;
    for (slot, mut image) in &mut empty {
        let frame = sliding(slot.index, false).unwrap_or(0);
        if image.image != slot.frames[frame] {
            image.image = slot.frames[frame].clone();
        }
    }
}

/// Carries out the action a button was pressed for, once the buttons have slid out.
// The whole slide is the slots plus nine
// steps.
fn finish_leaving(
    mut slide: ResMut<Slide>,
    time: Res<Time<Real>>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut choice: ResMut<MapChoice>,
    shell: Res<Shell>,
    mut setup: ResMut<GameSetup>,
    mut exit: MessageWriter<AppExit>,
    mut lobby: ResMut<crate::net::Lobby>,
    mut redraw: ResMut<Redraw>,
    mut saves: ResMut<crate::saves::SaveLoad>,
) {
    let Some((action, began)) = slide.leaving else {
        return;
    };
    if (((time.elapsed_secs_f64() - began) / SLIDE_STEP) as usize) < SLOT_COUNT + 9 {
        return;
    }
    slide.leaving = None;
    slide.started = time.elapsed_secs_f64();
    match action {
        MenuAction::Goto(screen) => next_screen.set(screen),
        MenuAction::UseMap => {
            choice.selected = choice.highlighted;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::StartGame => {
            if let Some(map) = shell.maps.get(choice.selected) {
                setup.map = map.id.clone();
                setup.map_name = map.name.clone();
                setup.max_players = map.max_players;
                if let Some(rules) = shell.mode_rules.get(choice.mode) {
                    setup.mode_rules = rules.clone();
                }
                // Creating a network game, the game starts when the other
                // player is there (`net::arrange`); the screen stays.
                if lobby.mode == crate::net::LobbyMode::Hosting {
                    lobby.start = true;
                    redraw.0 = true;
                    next_screen.set(Screen::Skirmish);
                } else {
                    next_screen.set(Screen::LoadScreen);
                }
            }
        }
        MenuAction::HostGame => {
            lobby.mode = crate::net::LobbyMode::Hosting;
            lobby.via_server = false;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::JoinGame => {
            lobby.mode = crate::net::LobbyMode::Joining;
            lobby.via_server = false;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::LeaveLobby => {
            lobby.mode = crate::net::LobbyMode::Off;
            lobby.status.clear();
            next_screen.set(if lobby.via_server { Screen::Internet } else { Screen::Network });
        }
        MenuAction::Focus(focus) => lobby.focus = focus,
        MenuAction::LoadSaved => saves.load = saves.entries.get(saves.picked).map(|entry| entry.path.clone()),
        MenuAction::HostInternet => {
            lobby.mode = crate::net::LobbyMode::Hosting;
            lobby.via_server = true;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::JoinInternet => {
            lobby.mode = crate::net::LobbyMode::Joining;
            lobby.via_server = true;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::PickGame(id) => lobby.picked = Some(id),
        MenuAction::Refresh => {
            lobby.refresh = true;
            lobby.browse = true;
        }
        MenuAction::Exit => {
            exit.write(AppExit::Success);
        }
        MenuAction::Mock(_) | MenuAction::Tell(_) => {}
    }
}

fn on_menu_action(
    activate: On<Activate>,
    actions: Query<(&MenuAction, Has<Slot>)>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut choice: ResMut<MapChoice>,
    shell: Res<Shell>,
    mut setup: ResMut<GameSetup>,
    mut exit: MessageWriter<AppExit>,
    mut slide: ResMut<Slide>,
    time: Res<Time<Real>>,
    mut cues: MessageWriter<crate::audio::Cue>,
    mut lobby: ResMut<crate::net::Lobby>,
    mut redraw: ResMut<Redraw>,
    mut saves: ResMut<crate::saves::SaveLoad>,
    mut notices: MessageWriter<crate::notice::Notice>,
    rules: Res<crate::files::Rules>,
) {
    let Ok((action, slot)) = actions.get(activate.entity) else {
        return;
    };
    // Leaving a menu, its buttons slide out first,
    // with `GUIMoveOutSound`.
    if slot && !matches!(action, MenuAction::Mock(_) | MenuAction::Tell(_)) {
        if slide.leaving.is_none() {
            slide.leaving = Some((*action, time.elapsed_secs_f64()));
            cues.write_batch(rules.sound("GUIMoveOutSound").map(crate::audio::Cue::Sound));
        }
        return;
    }
    match *action {
        MenuAction::Goto(screen) => next_screen.set(screen),
        MenuAction::UseMap => {
            choice.selected = choice.highlighted;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::StartGame => {
            if let Some(map) = shell.maps.get(choice.selected) {
                setup.map = map.id.clone();
                setup.map_name = map.name.clone();
                setup.max_players = map.max_players;
                if let Some(rules) = shell.mode_rules.get(choice.mode) {
                    setup.mode_rules = rules.clone();
                }
                // Creating a network game, the game starts when the other
                // player is there (`net::arrange`); the screen stays.
                if lobby.mode == crate::net::LobbyMode::Hosting {
                    lobby.start = true;
                    redraw.0 = true;
                    next_screen.set(Screen::Skirmish);
                } else {
                    next_screen.set(Screen::LoadScreen);
                }
            }
        }
        MenuAction::HostGame => {
            lobby.mode = crate::net::LobbyMode::Hosting;
            lobby.via_server = false;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::JoinGame => {
            lobby.mode = crate::net::LobbyMode::Joining;
            lobby.via_server = false;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::LeaveLobby => {
            lobby.mode = crate::net::LobbyMode::Off;
            lobby.status.clear();
            next_screen.set(if lobby.via_server { Screen::Internet } else { Screen::Network });
        }
        MenuAction::Focus(focus) => lobby.focus = focus,
        MenuAction::LoadSaved => saves.load = saves.entries.get(saves.picked).map(|entry| entry.path.clone()),
        MenuAction::HostInternet => {
            lobby.mode = crate::net::LobbyMode::Hosting;
            lobby.via_server = true;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::JoinInternet => {
            lobby.mode = crate::net::LobbyMode::Joining;
            lobby.via_server = true;
            next_screen.set(Screen::Skirmish);
        }
        MenuAction::PickGame(id) => lobby.picked = Some(id),
        MenuAction::Refresh => {
            lobby.refresh = true;
            lobby.browse = true;
        }
        MenuAction::Exit => {
            exit.write(AppExit::Success);
        }
        MenuAction::Mock(name) => {
            notices.write(crate::notice::Notice::not_built(name));
        }
        MenuAction::Tell(words) => {
            notices.write(crate::notice::Notice(words.to_owned()));
        }
    }
}

fn on_check_box(activate: On<Activate>, mut boxes: Query<(&mut CheckBox, &mut ImageNode)>) {
    if let Ok((mut check, mut image)) = boxes.get_mut(activate.entity) {
        check.on = !check.on;
        image.image = if check.on {
            check.checked.clone()
        } else {
            check.unchecked.clone()
        };
    }
}

fn on_map_entry(
    activate: On<Activate>,
    entries: Query<&MapEntry>,
    mut choice: ResMut<MapChoice>,
) {
    if let Ok(entry) = entries.get(activate.entity) {
        choice.highlighted = entry.0;
    }
}

// The list holds the modes
// that allow computer players, with the session's mode selected.
fn on_mode_entry(activate: On<Activate>, entries: Query<&ModeEntry>, mut choice: ResMut<MapChoice>) {
    if let Ok(entry) = entries.get(activate.entity) {
        choice.mode = entry.0;
    }
}

fn mode_entry_colors(mut entries: Query<(&ModeEntry, &mut BackgroundColor)>, choice: Res<MapChoice>) {
    for (entry, mut background) in &mut entries {
        let color = if choice.mode == entry.0 { SELECTED } else { Color::NONE };
        if background.0 != color {
            background.0 = color;
        }
    }
}

fn map_entry_colors(
    mut entries: Query<(&MapEntry, &mut BackgroundColor)>,
    choice: Res<MapChoice>,
) {
    for (entry, mut background) in &mut entries {
        let color = if choice.highlighted == entry.0 {
            SELECTED
        } else {
            Color::NONE
        };
        if background.0 != color {
            background.0 = color;
        }
    }
}

fn map_list_scroll(
    mut wheel: MessageReader<MouseWheel>,
    mut lists: Query<(&MapListContent, &mut Node)>,
    mut choice: ResMut<MapChoice>,
) {
    let lines: f32 = wheel.read().map(|event| event.y).sum();
    let step = lines.signum() * lines.abs().ceil();
    for (list, mut node) in &mut lists {
        let max = list.rows.saturating_sub(list.visible_rows) as f32;
        let scroll = (choice.scroll as f32 - step).clamp(0.0, max) as usize;
        if scroll != choice.scroll {
            choice.scroll = scroll;
        }
        let top = Val::Px(-LIST_ROW_HEIGHT * choice.scroll as f32);
        if node.top != top {
            node.top = top;
        }
    }
}
