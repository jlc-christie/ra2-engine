//! The in-game sidebar and bottom bar, drawn with the original art.
//!
//! The layout follows the original's.
//!

use bevy::asset::RenderAssetUsages;
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use ra2_formats::csf::Csf;
use ra2_formats::fnt::Font;
use ra2_formats::ini::Ini;
use ra2_formats::pal::Palette;
use ra2_formats::shp::Shp;
use ra2_sim::Order;

use crate::audio::{Cue, rules_sound};
use crate::files::GameFiles;
use crate::shell::Screen;
use crate::terrain::local;
use crate::units::{BuildCatalog, Placing, Sim};

/// DATA: every sidebar piece is this wide.
pub const SIDEBAR_WIDTH: f32 = 168.0;
/// The bar along the bottom of the map view.
pub const BOTTOM_BAR_HEIGHT: f32 = 32.0;

// DATA: the heights of credits.shp, top.shp, radar.shp, side1.shp, side2.shp and
// side3.shp. They stack in that order from the top.
pub(crate) const CREDITS_HEIGHT: f32 = 16.0;
const TOP_HEIGHT: f32 = 32.0;
const RADAR_HEIGHT: f32 = 110.0;
const CONTROLS_HEIGHT: f32 = 69.0;
pub(crate) const ROW_HEIGHT: f32 = 50.0;
const BOTTOM_HEIGHT: f32 = 26.0;
/// Where side1.shp starts.
pub(crate) const CONTROLS_Y: f32 = CREDITS_HEIGHT + TOP_HEIGHT + RADAR_HEIGHT;
/// Where the cameo rows start.
const STRIP_Y: f32 = 227.0;

// Positions within the sidebar. The code has
// two sets of numbers; these are the set whose spacing fits this art (28-pixel
// tabs at a pitch of 29).
const REPAIR_AT: (f32, f32) = (20.0, 166.0);
const SELL_AT: (f32, f32) = (84.0, 166.0);
const MODE_BUTTON_SIZE: Vec2 = Vec2::new(64.0, 31.0);
const TAB_X: f32 = 26.0;
const TAB_PITCH: f32 = 29.0;
const TAB_Y: f32 = 197.0;
const CAMEO_X: [f32; 2] = [22.0, 85.0];
const CAMEO_SIZE: Vec2 = Vec2::new(60.0, 48.0);
const SCROLL_DOWN_X: f32 = 39.0;
const SCROLL_UP_X: f32 = 85.0;
const POWER_X: f32 = 5.0;
// The two buttons on top.shp.
const DIPLOMACY_AT: (f32, f32) = (11.0, 4.0);
const OPTIONS_AT: (f32, f32) = (83.0, 4.0);

pub(crate) fn top_button_at(window: &Window, at: Vec2) -> Option<crate::pause::Menu> {
    let at = at - Vec2::new(window.width() - SIDEBAR_WIDTH, CREDITS_HEIGHT);
    [(DIPLOMACY_AT, crate::pause::Menu::Players), (OPTIONS_AT, crate::pause::Menu::Options)]
        .into_iter()
        .find(|(p, _)| contains(Rect::from_corners(Vec2::from(*p), Vec2::from(*p) + Vec2::new(72.0, 18.0)), at))
        .map(|(_, menu)| menu)
}

/// Adjacent controls share an edge, never the pixels that receive a click.
fn contains(rect: Rect, at: Vec2) -> bool {
    at.x >= rect.min.x && at.y >= rect.min.y && at.x < rect.max.x && at.y < rect.max.y
}

fn mode_button_at(at: Vec2) -> Option<crate::control::Mode> {
    [(REPAIR_AT, crate::control::Mode::Repair), (SELL_AT, crate::control::Mode::Sell)]
        .into_iter()
        .find(|(corner, _)| contains(Rect::from_corners(Vec2::from(*corner), Vec2::from(*corner) + MODE_BUTTON_SIZE), at))
        .map(|(_, mode)| mode)
}

pub(crate) const PALETTE: &str = "sidebar.pal";
const CAMEO_PALETTE: &str = "cameo.pal";

/// Sidebar text colour for the first side.
// The other side uses (255, 255, 0).
pub(crate) const TEXT: [u8; 3] = [164, 210, 255];

/// Tooltip borders follow the player's faction, independently of text language
/// and owner remap colour. ThirdSide is a UI theme here, not a simulation mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SidebarTheme {
    #[default]
    Allied,
    Soviet,
    Yuri,
}

impl SidebarTheme {
    fn from_side(side: &str) -> Option<Self> {
        match side.trim().to_ascii_lowercase().as_str() {
            "gdi" | "allied" | "allies" => Some(Self::Allied),
            "nod" | "soviet" | "soviets" => Some(Self::Soviet),
            "thirdside" | "yuri" => Some(Self::Yuri),
            _ => None,
        }
    }

    fn for_house(countries: &[Option<Self>], house: &ra2_sim::House) -> Self {
        countries.get(house.country as usize).copied().flatten().unwrap_or(match house.side {
            1 => Self::Soviet,
            2 => Self::Yuri,
            _ => Self::Allied,
        })
    }

    fn border(self) -> Color {
        let rgb = match self {
            Self::Allied => TEXT,
            Self::Soviet => [255, 96, 48],
            Self::Yuri => [196, 112, 255],
        };
        Color::srgb_u8(rgb[0], rgb[1], rgb[2])
    }
}

fn country_themes(rules: &Ini) -> Vec<Option<SidebarTheme>> {
    rules.section("Countries").iter().map(|(_, country)| rules.get(country, "Side").and_then(SidebarTheme::from_side)).collect()
}

/// Yuri adds only these backgrounds and radar frames to the Soviet sidebar;
/// shared buttons and strips continue to use the original Soviet resources.
pub(crate) fn sidebar_background<'a>(files: &GameFiles, name: &'a str, side: i32) -> (&'a str, &'static str) {
    let palette = if name.starts_with("bkgd") { "uibkgd.pal" } else { PALETTE };
    let yuri = match name {
        "bkgdlg.shp" => Some("bkgdlgy.shp"),
        "bkgdmd.shp" => Some("bkgdmdy.shp"),
        "bkgdsm.shp" => Some("bkgdsmy.shp"),
        _ => None,
    };
    if files.edition() == crate::files::Edition::YuriAssets && side == 2 {
        if let Some(yuri) = yuri.filter(|name| files.get(name).is_some() && files.get("uibkgdy.pal").is_some()) {
            return (yuri, "uibkgdy.pal");
        }
    }
    (name, palette)
}

pub(crate) fn sidebar_radar(files: &GameFiles, side: i32) -> (&'static str, &'static str) {
    if files.edition() == crate::files::Edition::YuriAssets && side == 2 && files.get("radary.shp").is_some() && files.get("radaryuri.pal").is_some() {
        ("radary.shp", "radaryuri.pal")
    } else { ("radar.shp", PALETTE) }
}

/// A tooltip appears after the mouse has been still
/// this long, in seconds.
const TIP_DELAY: f32 = 1.0;

/// Sidebar items from this number up are super weapons, by type.
const SUPER: u16 = 0xf000;

#[derive(Resource)]
struct Hud {
    root: Entity,
    /// The window size it was laid out for.
    size: Vec2,
    font: Option<Font>,
    strings: Option<Csf>,
    theme: SidebarTheme,
    country_themes: Vec<Option<SidebarTheme>>,
    /// How many strip rows fit.
    rows: usize,
    /// The tab on show, and which tabs had anything in them last time.
    tab: usize,
    enabled: [bool; 4],
    /// `GUITabSound`.
    tab_sound: Option<String>,
    /// `GUIBuildSound`, and the two `CreditTicks` sounds (up, down).
    build_sound: Option<String>,
    /// `ScoldSound`: a request the factory turns away.
    scold_sound: Option<String>,
    credit_sounds: Option<(String, String)>,
    /// The number of credits on show, which runs towards the real one.
    credits: Option<i32>,
    credits_tick: u64,
    /// Power pips on show (surplus, reserve, drain), which step towards the real
    /// ones.
    pips: [i32; 3],
    pips_stepped: f32,
    /// Where the mouse was last, and when it last moved.
    mouse: Vec2,
    mouse_still_since: f32,
    shown: Shown,
}

/// What each changing part of the sidebar was last drawn from.
#[derive(Default)]
struct Shown {
    credits: Option<i32>,
    power: Option<[i32; 3]>,
    strip: Option<StripState>,
    tip: Option<Option<u16>>,
}

#[derive(Clone, PartialEq, Eq)]
struct StripState {
    /// The tab on show: 0 buildings, 1 defences, 2 infantry, 3 vehicles.
    tab: usize,
    /// Which tabs have anything in them, and which of the first two are lit.
    enabled: [bool; 4],
    flashing: [bool; 2],
    buildable: Vec<u16>,
    /// For unit tabs, how many of each cameo's type are in hand.
    counts: Vec<usize>,
    /// Which cameos are at their build limit, and which show the picture of a
    /// type that will be built a veteran.
    limited: Vec<bool>,
    veteran: Vec<bool>,
    /// The building in production, its clock stage, whether it is ready, and
    /// whether it is on hold.
    production: Option<(u16, usize, bool, bool)>,
    /// On the vehicle tab: the same for the ship and the aircraft in production.
    others: Vec<(u16, usize, bool, bool)>,
    /// The same for each super weapon the player has.
    supers: Vec<(u16, usize, bool, bool)>,
    hovered: Option<usize>,
}

#[derive(Component)]
struct CreditsPart;
#[derive(Component)]
struct PowerPart;
#[derive(Component)]
struct StripPart;
#[derive(Component)]
struct TipPart;
#[derive(Component)]
struct ButtonTipPart;
#[derive(Component)]
struct TooltipFrame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarButton {
    Diplomacy,
    Options,
    Repair,
    Sell,
}

/// Only fallback text needs a language choice: an existing CSF label always
/// wins. Csf does not retain its header's language code, so consult UI labels
/// from the imported game, never the desktop language or unit/map names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TooltipLanguage {
    English,
    TraditionalChinese,
    SimplifiedChinese,
}

impl TooltipLanguage {
    fn of(strings: Option<&Csf>) -> Self {
        for key in ["TIP:DiplomacyButton", "TIP:OptionsButton", "TIP:Repair", "GUI:Options", "GUI:LoadGame", "GUI:SaveGame", "GUI:Tooltips"] {
            let Some(text) = strings.and_then(|strings| strings.get(key)) else { continue };
            if text.chars().any(|ch| "鈕選項復遊戲儲載設圖體".contains(ch)) { return Self::TraditionalChinese }
            if text.chars().any(|ch| "钮选项复游戏储载设图体".contains(ch)) { return Self::SimplifiedChinese }
        }
        Self::English
    }
}

impl SidebarButton {
    fn text(self, strings: Option<&Csf>) -> &str {
        let (key, english, traditional, simplified) = match self {
            Self::Diplomacy => ("TIP:DiplomacyButton", "Diplomacy Button", "外交按鈕", "外交按钮"),
            Self::Options => ("TIP:OptionsButton", "Options Button", "選項按鈕", "选项按钮"),
            Self::Repair => ("TIP:Repair", "Repair", "修復", "修复"),
            Self::Sell => ("TIP:Sell", "Sell", "出售", "出售"),
        };
        strings.and_then(|strings| strings.get(key)).unwrap_or_else(|| match TooltipLanguage::of(strings) {
            TooltipLanguage::English => english,
            TooltipLanguage::TraditionalChinese => traditional,
            TooltipLanguage::SimplifiedChinese => simplified,
        })
    }
}

/// All rectangles here are local to the sidebar, in logical pixels.
fn button_tip_at(window: &Window, menu: crate::pause::Menu, at: Vec2) -> Option<(SidebarButton, Rect)> {
    if let Some(menu) = top_button_at(window, at) {
        let (button, corner) = match menu {
            crate::pause::Menu::Players => (SidebarButton::Diplomacy, DIPLOMACY_AT),
            crate::pause::Menu::Options => (SidebarButton::Options, OPTIONS_AT),
            _ => return None,
        };
        let corner = Vec2::from(corner) + Vec2::new(0.0, CREDITS_HEIGHT);
        return Some((button, Rect::from_corners(corner, corner + Vec2::new(72.0, 18.0))));
    }
    // Menus blank the repair and sell controls; only describe visible buttons.
    if menu != crate::pause::Menu::Closed { return None }
    let at = at - Vec2::new(window.width() - SIDEBAR_WIDTH, 0.0);
    let (button, corner) = match mode_button_at(at)? {
        crate::control::Mode::Repair => (SidebarButton::Repair, REPAIR_AT),
        crate::control::Mode::Sell => (SidebarButton::Sell, SELL_AT),
        _ => return None,
    };
    let corner = Vec2::from(corner);
    Some((button, Rect::from_corners(corner, corner + MODE_BUTTON_SIZE)))
}

/// The same diagonal placement as a cameo tooltip, clamped inside the sidebar.
fn tooltip_rect(anchor: Rect, size: Vec2, bounds: Vec2) -> Rect {
    let x = if anchor.min.x < bounds.x / 2.0 { anchor.max.x - 5.0 } else { anchor.min.x - size.x + 5.0 };
    let y = if anchor.min.y < bounds.y / 2.0 { anchor.max.y - 5.0 } else { anchor.min.y - size.y + 5.0 };
    let corner = Vec2::new(x, y).clamp(Vec2::ZERO, (bounds - size).max(Vec2::ZERO));
    Rect::from_corners(corner, corner + size)
}

/// Button and cameo tooltips share their frame, padding and text. They never
/// participate in picking, including when displayed over a menu.
fn spawn_tooltip(
    parent: &mut ChildSpawnerCommands,
    marker: impl Component,
    image: Handle<Image>,
    size: Vec2,
    corner: Vec2,
    z: i32,
    theme: SidebarTheme,
) {
    parent.spawn((
        marker,
        TooltipFrame,
        Node { border: UiRect::all(Val::Px(1.0)), ..absolute(corner.x, corner.y, size + Vec2::new(8.0, 4.0)) },
        BackgroundColor(Color::BLACK),
        BorderColor::all(theme.border()),
        GlobalZIndex(z),
        Pickable::IGNORE,
    )).with_children(|tip| {
        tip.spawn((absolute(3.0, 1.0, size), ImageNode::new(image), Pickable::IGNORE));
    });
}

#[derive(Default)]
struct ButtonTooltip {
    pointer: Option<Vec2>,
    hovered: Option<SidebarButton>,
    menu: crate::pause::Menu,
    since: f32,
    root: Option<Entity>,
    shown: Option<(SidebarButton, Entity, Vec2)>,
}

fn button_tooltips(
    mut commands: Commands,
    hud: Res<Hud>,
    prefs: Res<crate::prefs::Prefs>,
    menu: Res<crate::pause::Menu>,
    time: Res<Time<Real>>,
    pointer: Res<crate::cursor::Pointer>,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut state: Local<ButtonTooltip>,
    mut images: ResMut<Assets<Image>>,
    parts: Query<Entity, With<ButtonTipPart>>,
) {
    let hovered = pointer.0.and_then(|at| button_tip_at(&window, *menu, at));
    let target = hovered.map(|(button, _)| button);
    let now = time.elapsed_secs();
    let pressed = buttons.pressed(MouseButton::Left) || buttons.pressed(MouseButton::Right);
    if state.pointer != pointer.0 || state.hovered != target || state.menu != *menu || state.root != Some(hud.root) || pressed {
        state.pointer = pointer.0;
        state.hovered = target;
        state.menu = *menu;
        state.root = Some(hud.root);
        state.since = now;
    }
    let shown = target.filter(|_| prefs.tooltips && !pressed && now - state.since >= TIP_DELAY)
        .map(|button| (button, hud.root, window.size()));
    if shown == state.shown { return }
    for entity in &parts { commands.entity(entity).despawn(); }
    state.shown = shown;
    if let (Some((button, _, _)), Some((_, anchor)), Some(font)) = (shown, hovered, &hud.font) {
        let (image, size) = text_image(font, &mut images, button.text(hud.strings.as_ref()));
        let area = tooltip_rect(anchor, size + Vec2::new(8.0, 4.0), Vec2::new(SIDEBAR_WIDTH, window.height()));
        commands.entity(hud.root).with_children(|parent| {
            // The menu uses z=100; the still-visible top buttons retain tips.
            spawn_tooltip(parent, ButtonTipPart, image, size, area.min, 110, hud.theme);
        });
    }
}

/// Update existing frames as well as future tooltips, so changing the local
/// house or its country cannot leave a visible tooltip in the previous theme.
fn sync_tooltip_theme(
    mut hud: ResMut<Hud>,
    sim: Res<Sim>,
    mut frames: Query<&mut BorderColor, With<TooltipFrame>>,
) {
    let theme = sim.0.houses.get(local() as usize).map_or(SidebarTheme::Allied, |house| SidebarTheme::for_house(&hud.country_themes, house));
    if hud.theme == theme { return }
    hud.theme = theme;
    for mut border in &mut frames { *border = BorderColor::all(theme.border()); }
}

pub struct HudPlugin;

/// The pressed picture of the repair or sell button.
#[derive(Component)]
struct ModeLight(crate::control::Mode);

#[derive(Component)]
struct TopButtonArt {
    menu: crate::pause::Menu,
    up: Handle<Image>,
    down: Handle<Image>,
}

fn top_button_lights(
    press: Res<crate::pause::ButtonPress>,
    mouse: Res<ButtonInput<MouseButton>>,
    pointer: Res<crate::cursor::Pointer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut buttons: Query<(&TopButtonArt, &mut ImageNode)>,
) {
    let over = pointer.0.and_then(|at| top_button_at(&window, at));
    for (button, mut image) in &mut buttons {
        let down = mouse.pressed(MouseButton::Left) && press.top() == Some(button.menu) && over == Some(button.menu);
        let wanted = if down { &button.down } else { &button.up };
        if image.image != *wanted { image.image = wanted.clone() }
    }
}

fn mode_lights(mode: Res<crate::control::Mode>, mut lights: Query<(&ModeLight, &mut Visibility)>) {
    for (light, mut visibility) in &mut lights {
        let wanted = if light.0 == *mode { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// The one-shot form of `spawn_sidebar`, to lay the sidebar out again.
#[derive(Resource)]
struct Respawn(bevy::ecs::system::SystemId);

/// A window that changes size mid-game gets its sidebar laid out again for the
/// new height, keeping the tab and the credits shown.
fn relayout(mut commands: Commands, hud: Res<Hud>, respawn: Res<Respawn>, window: Single<&Window, With<PrimaryWindow>>) {
    if Vec2::new(window.width(), window.height()) != hud.size {
        commands.entity(hud.root).despawn();
        commands.run_system(respawn.0);
    }
}

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        let respawn = app.register_system(spawn_sidebar);
        app.insert_resource(Respawn(respawn))
            .add_systems(Update, relayout.run_if(in_state(Screen::InGame)).run_if(resource_exists::<Hud>));
        // The sidebar's art depends on the player's side, which the map's set-up
        // settles.
        app.add_systems(OnEnter(Screen::InGame), spawn_sidebar.after(crate::game::load_map)).add_systems(
            Update,
            update_sidebar
                .run_if(in_state(Screen::InGame))
                .run_if(crate::pause::playing)
                .run_if(resource_exists::<Hud>)
                .run_if(resource_exists::<Sim>),
        )
        .add_systems(Update, mode_lights.run_if(in_state(Screen::InGame)).run_if(resource_exists::<crate::control::Mode>))
        .add_systems(Update, top_button_lights.run_if(in_state(Screen::InGame)))
        .add_systems(Update, button_tooltips.after(update_sidebar).after(relayout).run_if(in_state(Screen::InGame)).run_if(resource_exists::<Hud>))
        .add_systems(Update, sync_tooltip_theme.after(relayout).before(update_sidebar).before(button_tooltips).run_if(in_state(Screen::InGame)).run_if(resource_exists::<Hud>).run_if(resource_exists::<Sim>));
    }
}

pub(crate) fn rgba_image(width: u32, height: u32, data: Vec<u8>) -> Image {
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

/// One frame of a shape as an image the size of the whole shape.
pub(crate) fn load(
    files: &GameFiles,
    images: &mut Assets<Image>,
    name: &str,
    frame: usize,
    palette: &str,
) -> Option<(Handle<Image>, Vec2)> {
    let shp = Shp::parse(files.get(name)?)?;
    let palette = Palette::parse(files.get(palette)?)?;
    let (w, h) = (shp.width as usize, shp.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    blit(&mut rgba, w, &shp, frame, &palette, 0, 0);
    Some((images.add(rgba_image(w as u32, h as u32, rgba)), Vec2::new(w as f32, h as f32)))
}

/// Copies a frame into an RGBA buffer `stride` pixels wide, with the shape's
/// corner at (x, y). Index 0 is transparent.
fn blit(rgba: &mut [u8], stride: usize, shp: &Shp, frame: usize, palette: &Palette, x: usize, y: usize) {
    let Some(frame) = shp.frames.get(frame) else {
        return;
    };
    for row in 0..frame.height as usize {
        for col in 0..frame.width as usize {
            let index = frame.pixels[row * frame.width as usize + col];
            let at = ((y + frame.y as usize + row) * stride + x + frame.x as usize + col) * 4;
            if index != 0 && at + 3 < rgba.len() {
                let [r, g, b] = palette.0[index as usize];
                rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
}

/// Lines of text in the game font, in the sidebar's colour, centred on each other.
pub(crate) fn text_image(font: &Font, images: &mut Assets<Image>, text: &str) -> (Handle<Image>, Vec2) {
    text_image_coloured(font, images, text, TEXT)
}

pub(crate) fn text_image_coloured(font: &Font, images: &mut Assets<Image>, text: &str, colour: [u8; 3]) -> (Handle<Image>, Vec2) {
    let lines: Vec<(usize, Vec<u8>)> = text.lines().map(|line| font.rasterize(line)).collect();
    let width = lines.iter().map(|(width, _)| *width).max().unwrap_or(1).max(1);
    let height = font.height * lines.len().max(1);
    let mut rgba = vec![0u8; width * height * 4];
    for (index, (line_width, mask)) in lines.iter().enumerate() {
        for row in 0..font.height {
            for col in 0..*line_width {
                if mask[row * line_width + col] != 0 {
                    let at = ((index * font.height + row) * width + col) * 4;
                    rgba[at..at + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
                }
            }
        }
    }
    (images.add(rgba_image(width as u32, height as u32, rgba)), Vec2::new(width as f32, height as f32))
}

pub(crate) fn absolute(x: f32, y: f32, size: Vec2) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x),
        top: Val::Px(y),
        width: Val::Px(size.x),
        height: Val::Px(size.y),
        ..default()
    }
}

fn text_color() -> Color {
    Color::srgb_u8(TEXT[0], TEXT[1], TEXT[2])
}

/// How many cameo rows fit a screen height.
// What is left after the fixed pieces, in whole rows.
pub(crate) fn strip_rows(height: f32) -> usize {
    let fixed = CREDITS_HEIGHT + TOP_HEIGHT + RADAR_HEIGHT + CONTROLS_HEIGHT + BOTTOM_HEIGHT;
    ((height - fixed) / ROW_HEIGHT).floor().max(1.0) as usize
}

fn spawn_sidebar(
    mut commands: Commands,
    files: Res<GameFiles>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<PrimaryWindow>>,
    old: Option<Res<Hud>>,
) {
    let lit: Vec<_> = [("repair.shp", REPAIR_AT, crate::control::Mode::Repair), ("sell.shp", SELL_AT, crate::control::Mode::Sell)]
        .into_iter()
        .filter_map(|(name, at, mode)| load(&files, &mut images, name, 1, PALETTE).map(|(image, size)| (image, size, at, mode)))
        .collect();
    let top_buttons: Vec<_> = [("diplobtn.shp", DIPLOMACY_AT, crate::pause::Menu::Players), ("optbtn.shp", OPTIONS_AT, crate::pause::Menu::Options)]
        .into_iter()
        .filter_map(|(name, at, menu)| {
            let (up, size) = load(&files, &mut images, name, 0, PALETTE)?;
            let (down, _) = load(&files, &mut images, name, 1, PALETTE)?;
            Some((TopButtonArt { menu, up, down }, at, size))
        }).collect();
    let mut piece = |parent: &mut ChildSpawnerCommands, name: &str, frame: usize, x: f32, y: f32| {
        let Some((image, size)) = load(&files, &mut images, name, frame, PALETTE) else {
            warn!("sidebar piece {name} not found");
            return;
        };
        parent.spawn((absolute(x, y, size), ImageNode::new(image)));
    };

    let rows = strip_rows(window.height());
    let root = commands
        .spawn((
            DespawnOnExit(Screen::InGame),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(SIDEBAR_WIDTH),
                height: Val::Percent(100.0),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::BLACK),
        ))
        .with_children(|parent| {
            // The pieces and
            // where they go.
            piece(parent, "credits.shp", 0, 0.0, 0.0);
            let top = CREDITS_HEIGHT;
            piece(parent, "top.shp", 0, 0.0, top);
            for (button, at, size) in top_buttons {
                parent.spawn((absolute(at.0, top + at.1, size), ImageNode::new(button.up.clone()), button));
            }
            // Frame 0 is the radar's closed state.
            // The radar: its art, which `radar` animates, and the map picture.
            parent.spawn((
                absolute(0.0, top + TOP_HEIGHT, Vec2::new(SIDEBAR_WIDTH, RADAR_HEIGHT)),
                ImageNode::default(),
                crate::radar::RadarArt,
            ));
            parent.spawn((absolute(0.0, 0.0, Vec2::ONE), ImageNode::default(), Visibility::Hidden, crate::radar::RadarMap));

            piece(parent, "side1.shp", 0, 0.0, CONTROLS_Y);
            piece(parent, "repair.shp", 0, REPAIR_AT.0, REPAIR_AT.1);
            piece(parent, "sell.shp", 0, SELL_AT.0, SELL_AT.1);
            // Frame 1 while the mode is on.
            for (image, size, at, mode) in lit {
                parent.spawn((absolute(at.0, at.1, size), ImageNode::new(image), Visibility::Hidden, ModeLight(mode)));
            }
            for index in 0..4 {
                // Frame 1 is the chosen tab, 0 the rest.
                let frame = if index == 0 { 1 } else { 0 };
                piece(parent, &format!("tab0{index}.shp"), frame, TAB_X + TAB_PITCH * index as f32, TAB_Y);
            }

            let mut y = STRIP_Y;
            for _ in 0..rows {
                piece(parent, "side2.shp", 0, 0.0, y);
                y += ROW_HEIGHT;
            }
            piece(parent, "side3.shp", 0, 0.0, y);
            // The scroll buttons sit 7 pixels below the
            // last row, down on the left. Frame 2 is the disabled look.
            piece(parent, "r-dn.shp", 2, SCROLL_DOWN_X, y + 7.0);
            piece(parent, "r-up.shp", 2, SCROLL_UP_X, y + 7.0);
            piece(parent, "addon.shp", 0, 0.0, y + BOTTOM_HEIGHT);
        })
        .id();

    // The bar under the map view is the long spacer
    // from the left, with the two end caps side by side against the sidebar.
    // DATA: both caps are 28 pixels wide.
    commands
        .spawn((
            DespawnOnExit(Screen::InGame),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(SIDEBAR_WIDTH),
                bottom: Val::Px(0.0),
                height: Val::Px(BOTTOM_BAR_HEIGHT),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::BLACK),
        ))
        .with_children(|parent| {
            piece(parent, "lspacer.shp", 0, 0.0, 0.0);
            let width = window.width() - SIDEBAR_WIDTH;
            piece(parent, "lendcap.shp", 0, width - 56.0, 0.0);
            piece(parent, "rendcap.shp", 0, width - 28.0, 0.0);
        });

    // DATA: rules.ini `CreditTicks` names the counting-up and counting-down sounds.
    let rules = files.get("rules.ini").map(ra2_formats::ini::Ini::parse);
    let credit_sounds = rules
        .as_ref()
        .and_then(|rules| rules.get("AudioVisual", "CreditTicks").or_else(|| rules.get("General", "CreditTicks")))
        .and_then(|list| {
            let (up, down) = list.split_once(',')?;
            Some((up.trim().to_owned(), down.trim().to_owned()))
        });
    let (tab, enabled, credits, credits_tick) = old.as_ref().map_or((0, [false; 4], None, 0), |old| (old.tab, old.enabled, old.credits, old.credits_tick));
    commands.insert_resource(Hud {
        root,
        size: Vec2::new(window.width(), window.height()),
        font: files.get("game.fnt").and_then(Font::parse),
        strings: files.get("ra2.csf").and_then(Csf::parse),
        theme: SidebarTheme::Allied,
        country_themes: rules.as_ref().map(country_themes).unwrap_or_default(),
        rows,
        tab,
        enabled,
        tab_sound: rules_sound(&files, "GUITabSound"),
        build_sound: rules_sound(&files, "GUIBuildSound"),
        scold_sound: rules_sound(&files, "ScoldSound"),
        credit_sounds,
        credits,
        credits_tick,
        pips: [0; 3],
        pips_stepped: 0.0,
        mouse: Vec2::ZERO,
        mouse_still_since: 0.0,
        shown: Shown::default(),
    });
}

/// The power bar's pips for an output and drain: how many of the `slots` show
/// surplus beyond the first hundred, the first hundred of surplus, and drain.
fn power_pips(slots: i32, output: i32, drain: i32) -> [i32; 3] {
    if slots <= 0 || output + drain <= 0 {
        return [0; 3];
    }
    let empty = (slots as i64 * 400 / (output + drain + 400) as i64).clamp(0, slots as i64 - 1) as i32;
    let used = slots - empty;
    if output < drain {
        return [0, 0, used];
    }
    let reserve = (output - drain).min(100);
    let surplus = output - drain - reserve;
    let total = (drain + reserve + surplus) as f64;
    let share = |part: i32| used as f64 * part as f64 / total;
    let (s, r, d) = (share(surplus), share(reserve), share(drain));
    let spare = (s.fract() + r.fract() + d.fract() + 0.01).trunc();
    [s.trunc() as i32, r.trunc() as i32, (d.trunc() + spare) as i32]
}

/// The rectangle of a cameo slot within the sidebar.
fn slot_rect(slot: usize) -> Rect {
    let corner = Vec2::new(CAMEO_X[slot % 2], STRIP_Y + ROW_HEIGHT * (slot / 2) as f32 + 1.0);
    Rect::from_corners(corner, corner + CAMEO_SIZE)
}

/// Runs the sidebar: mouse clicks on cameos, and redrawing whichever of the
/// credits, power bar, cameo strip and tooltip has changed.
#[allow(clippy::too_many_arguments)]
fn update_sidebar(
    mut commands: Commands,
    mut hud: ResMut<Hud>,
    mut sim: ResMut<Sim>,
    mut placing: ResMut<Placing>,
    mut modes: (ResMut<crate::control::Mode>, ResMut<crate::control::Selection>),
    catalog: Res<BuildCatalog>,
    catalogs: (Res<crate::units::UnitCatalog>, Res<crate::units::SuperCatalog>, Res<crate::prefs::Prefs>),
    files: Res<GameFiles>,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    pointer: Res<crate::cursor::Pointer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut cues: MessageWriter<Cue>,
    parts: (
        Query<Entity, With<CreditsPart>>,
        Query<Entity, With<PowerPart>>,
        Query<Entity, With<StripPart>>,
        Query<Entity, With<TipPart>>,
    ),
) {
    let hud = &mut *hud;
    let (unit_catalog, super_catalog, prefs) = catalogs;
    let now = time.elapsed_secs();
    let Some(house) = sim.0.houses.get(local() as usize).cloned() else {
        return;
    };
    let slots = hud.rows * 2;

    // The mouse, in the sidebar's own coordinates.
    let mouse = pointer.0.map(|at| Vec2::new(at.x - (window.width() - SIDEBAR_WIDTH), at.y));
    // What each tab holds, and what is in hand in it.
    // Buildings other than defences, defences,
    // infantry, then vehicles with aircraft and ships.
    let everything = sim.0.buildable(local());
    let units = sim.0.buildable_units(local());
    // Super weapons
    // go on the defences tab before the buildings, the quicker to charge first
    // and ties by name.
    let mut supers: Vec<u16> = (0..house.supers.len() as u16).filter(|&index| house.supers[index as usize].granted).collect();
    supers.sort_by_key(|&index| (sim.0.super_types.get(index as usize).map_or(0, |kind| kind.recharge), super_catalog.0.get(index as usize).map(|entry| entry.0.clone())));
    let in_tab = |tab: usize| -> Vec<u16> {
        match tab {
            1 => supers.iter().map(|index| SUPER + index).chain(everything.iter().copied().filter(|&building| sim.0.is_defence(building))).collect(),
            0 | 1 => everything.iter().copied().filter(|&building| sim.0.is_defence(building) == (tab == 1)).collect(),
            _ => units.iter().copied().filter(|&unit| sim.0.unit_tab(unit) == tab).collect(),
        }
    };
    let enabled = [0, 1, 2, 3].map(|tab| !in_tab(tab).is_empty());
    // When the first cameo arrives in an
    // empty sidebar, or any tab loses its last cameo, the first tab with something
    // in it is shown.
    let was = hud.enabled;
    if (was.iter().all(|on| !on) || (0..4).any(|tab| was[tab] && !enabled[tab])) && enabled.contains(&true) {
        hud.tab = enabled.iter().position(|on| *on).unwrap_or(0);
    }
    hud.enabled = enabled;
    // RA2_TAB=0..3 keeps that tab showing whenever it has something, for checking it.
    if let Some(forced) = std::env::var("RA2_TAB").ok().and_then(|n| n.trim().parse::<usize>().ok()) {
        if forced < 4 && enabled[forced] {
            hud.tab = forced;
        }
    }
    // A tab with nothing in it is disabled and takes no clicks; a click on any
    // other shows it and plays `GUITabSound`.
    if buttons.just_pressed(MouseButton::Left) {
        let tab = (0..4).find(|&tab| {
            let corner = Vec2::new(TAB_X + TAB_PITCH * tab as f32, TAB_Y);
            mouse.is_some_and(|at| contains(Rect::from_corners(corner, corner + Vec2::new(TAB_PITCH, 26.0)), at))
        });
        if let Some(tab) = tab.filter(|&tab| enabled[tab]) {
            hud.tab = tab;
            cues.write_batch(hud.tab_sound.clone().map(Cue::Sound));
        }
    }
    // The repair and sell buttons turn
    // their modes on and off.
    // With `GUIMainButtonSound`, whether or
    // not the mode can be turned on.
    if buttons.just_pressed(MouseButton::Left) {
        if let Some(which) = mouse.and_then(mode_button_at) {
            let (mode, selection) = &mut modes;
            mode.toggle(which, &sim.0, selection, &placing);
            cues.write_batch(rules_sound(&files, "GUIMainButtonSound").map(Cue::Sound));
        }
    }

    // DATA: the game's default keys (keyboard.ini): Q structures, W defences, E
    // infantry, R vehicles.
    // The keys are silent, do nothing for an
    // empty tab, and the first two also pick up a finished building to place.
    for (key, tab) in [(KeyCode::KeyQ, 0), (KeyCode::KeyW, 1), (KeyCode::KeyE, 2), (KeyCode::KeyR, 3)] {
        if keys.just_pressed(key) && enabled[tab] {
            hud.tab = tab;
            if let Some(ready) = house.slot(tab == 1).filter(|p| tab < 2 && p.is_ready()) {
                placing.0.get_or_insert(ready.building);
            }
        }
    }
    let tab = hud.tab;
    let mut buildable = in_tab(tab);
    let in_production = house.tab_slot(tab);
    // Production is abandoned only when the last factory for it goes; losing
    // what a type needs to be offered does not stop one already begun.
    if tab < 2 {
        if let Some(production) = in_production.filter(|production| !buildable.contains(&production.building)) {
            buildable.push(production.building);
        }
    }

    if mouse != Some(hud.mouse) {
        hud.mouse = mouse.unwrap_or(Vec2::NEG_ONE);
        hud.mouse_still_since = now;
    }
    let hovered = mouse.and_then(|at| (0..buildable.len().min(slots)).find(|&slot| slot_rect(slot).contains(at)));

    // Clicks on a cameo.
    if let Some(index) = hovered.map(|slot| buildable[slot]).filter(|&item| item >= SUPER).map(|item| item - SUPER) {
        // A left click plays
        // the click; on one that can fire it clears the other modes and the
        // selection, starts the choosing of a target and says so. A right
        // click ends the choosing.
        let (mode, selection) = &mut modes;
        if buttons.just_pressed(MouseButton::Left) {
            cues.write_batch(rules_sound(&files, "GUIMainButtonSound").map(Cue::Sound));
            if house.supers.get(index as usize).is_some_and(|state| state.can_fire()) {
                placing.0 = None;
                selection.0.clear();
                **mode = crate::control::Mode::Super(index);
                cues.write(Cue::Eva("EVA_SelectTarget"));
            }
        } else if buttons.just_pressed(MouseButton::Right) && matches!(**mode, crate::control::Mode::Super(_)) {
            **mode = crate::control::Mode::None;
        }
    } else if let Some(unit) = hovered.map(|slot| buildable[slot]).filter(|_| tab >= 2) {
        // Units: a busy factory queues the request without a word; a right click
        // takes a queued one away before touching the one in hand.
        // Vehicles, ships and aircraft share
        // the tab but each kind has its own factory.
        let in_production = house.tab_slot(sim.0.unit_slot(unit));
        let mine = in_production.filter(|production| production.building == unit);
        let build_sound = hud.build_sound.clone().map(Cue::Sound);
        let voice = if tab == 2 { "EVA_Training" } else { "EVA_Building" };
        if buttons.just_pressed(MouseButton::Left) {
            match mine {
                Some(production) if production.on_hold() => {
                    cues.write_batch(build_sound);
                    cues.write(Cue::Eva(voice));
                    sim.give(Order::BuildUnit { house: local(), unit });
                }
                // Finished and unable to leave: it keeps trying by itself.
                Some(production) if production.is_ready() => {}
                _ if sim.0.at_limit(local(), ra2_sim::Item::Unit(unit)) => {}
                _ => {
                    if in_production.is_none() {
                        cues.write(Cue::Eva(voice));
                    }
                    cues.write_batch(build_sound);
                    // With the queue
                    // full the factory turns the request away with the
                    // scolding sound, after the click.
                    if sim.0.queue_full(local(), unit) {
                        cues.write_batch(hud.scold_sound.clone().map(Cue::Sound));
                    }
                    sim.give(Order::BuildUnit { house: local(), unit });
                }
            }
        } else if buttons.just_pressed(MouseButton::Right) {
            match mine {
                Some(production) if !production.suspended => {
                    cues.write_batch(build_sound);
                    cues.write(Cue::Eva("EVA_OnHold"));
                    sim.give(Order::SuspendUnit { house: local(), unit });
                }
                Some(_) => {
                    cues.write_batch(build_sound);
                    cues.write(Cue::Eva("EVA_Canceled"));
                    sim.give(Order::CancelUnit { house: local(), unit });
                }
                None if sim.0.units_in_hand(local(), unit) > 0 => {
                    cues.write_batch(build_sound);
                    sim.give(Order::CancelUnit { house: local(), unit });
                }
                None => {}
            }
        }
    } else if let Some(building) = hovered.map(|slot| buildable[slot]) {
        let mine = in_production.filter(|production| production.building == building);
        let build_sound = hud.build_sound.clone().map(Cue::Sound);
        if buttons.just_pressed(MouseButton::Left) {
            match mine {
                // Finished: pick it up to place.
                Some(production) if production.is_ready() => {
                    cues.write_batch(build_sound);
                    placing.0.get_or_insert(building);
                }
                // On hold: carry on.
                Some(production) if production.on_hold() => {
                    cues.write_batch(build_sound);
                    cues.write(Cue::Eva("EVA_Building"));
                    sim.give(Order::Build { house: local(), building });
                }
                _ if sim.0.at_build_limit(local(), building) => {}
                // Buildings do not queue: with anything in hand, a refusal.
                _ if in_production.is_some() => {
                    cues.write(Cue::Eva("EVA_UnableToComply"));
                }
                _ => {
                    cues.write(Cue::Eva("EVA_Building"));
                    cues.write_batch(build_sound);
                    sim.give(Order::Build { house: local(), building });
                }
            }
        } else if buttons.just_pressed(MouseButton::Right) {
            if let Some(production) = mine {
                placing.0 = None;
                cues.write_batch(build_sound);
                if production.suspended {
                    // On hold, or finished and not placed: abandon it.
                    cues.write(Cue::Eva("EVA_Canceled"));
                    sim.give(Order::CancelBuild { house: local(), building });
                } else {
                    cues.write(Cue::Eva("EVA_OnHold"));
                    sim.give(Order::Suspend { house: local(), building });
                }
            }
        }
    }

    // Credits run towards the real figure an eighth of the gap at a time.
    // The step and its limits, one step each logic
    // frame.
    let target = house.credits.max(0);
    let tick = sim.0.current_tick();
    match hud.credits {
        None => hud.credits = Some(target),
        Some(shown) if shown != target && tick != hud.credits_tick => {
            let step = ((target - shown).abs() >> 3).clamp(1, 143);
            hud.credits = Some(shown + step * (target - shown).signum());
            if let Some((up, down)) = &hud.credit_sounds {
                // At half the sound's own volume.
                cues.write(Cue::SoundAt(if target > shown { up.clone() } else { down.clone() }, 0.5));
            }
        }
        _ => {}
    }
    hud.credits_tick = tick;

    // Power pips step one at a time towards their targets.
    let pip_slots = (hud.rows as i32 * ROW_HEIGHT as i32 + 3) / 3;
    let (output, drain) = sim.0.power(local());
    let pip_target = power_pips(pip_slots, output, drain);
    if hud.pips != pip_target && now - hud.pips_stepped >= 0.08 {
        hud.pips_stepped = now;
        for (shown, target) in hud.pips.iter_mut().zip(pip_target) {
            *shown += (target - *shown).signum();
        }
    }

    let (credits_parts, power_parts, strip_parts, tip_parts) = parts;
    let root = hud.root;

    if hud.shown.credits != hud.credits {
        for entity in &credits_parts {
            commands.entity(entity).despawn();
        }
        if let (Some(font), Some(credits)) = (&hud.font, hud.credits) {
            // Centred on the bar, two pixels down.
            let (image, size) = text_image(font, &mut images, &credits.to_string());
            let x = (SIDEBAR_WIDTH / 2.0 - size.x / 2.0).floor();
            commands.entity(root).with_children(|parent| {
                parent.spawn((CreditsPart, absolute(x, 2.0, size), ImageNode::new(image)));
            });
        }
        hud.shown.credits = hud.credits;
    }

    if hud.shown.power != Some(hud.pips) {
        for entity in &power_parts {
            commands.entity(entity).despawn();
        }
        // A pip every three pixels from the top of the
        // strip: unlit ones (frame 0), then surplus (1), reserve (2) and drain (3).
        let art = files.get("powerp.shp").and_then(Shp::parse);
        let palette = files.get(PALETTE).and_then(Palette::parse);
        if let (Some(shp), Some(palette)) = (art, palette) {
            let (w, h) = (shp.width as usize, pip_slots as usize * 3);
            let mut rgba = vec![0u8; w * h * 4];
            let [surplus, reserve, drain] = hud.pips;
            let unlit = pip_slots - surplus - reserve - drain;
            let frames = [(0, unlit), (1, surplus), (2, reserve), (3, drain)];
            let mut slot = 0;
            for (frame, count) in frames {
                for _ in 0..count.max(0) {
                    blit(&mut rgba, w, &shp, frame, &palette, 0, slot * 3);
                    slot += 1;
                }
            }
            let image = images.add(rgba_image(w as u32, h as u32, rgba));
            commands.entity(root).with_children(|parent| {
                parent.spawn((
                    PowerPart,
                    absolute(POWER_X, STRIP_Y, Vec2::new(w as f32, h as f32)),
                    ImageNode::new(image),
                ));
            });
        }
        hud.shown.power = Some(hud.pips);
    }

    // The structure and defence tabs flash
    // while one of their cameos holds a finished building: ten frames lit, ten not,
    // in step with the frame number.
    let lit = (tick / 10) % 2 == 1;
    let mut flashing = [house.production, house.defence].map(|slot| slot.is_some_and(|p| p.is_ready()) && lit);
    // And the defences tab while a super weapon
    // can be fired.
    flashing[1] |= lit && house.supers.iter().any(|state| state.can_fire());
    let strip = StripState {
        tab,
        enabled,
        flashing,
        buildable: buildable.clone(),
        counts: buildable.iter().map(|&item| if tab >= 2 { sim.0.units_in_hand(local(), item) } else { 0 }).collect(),
        limited: buildable
            .iter()
            .map(|&item| item < SUPER && sim.0.at_limit(local(), if tab >= 2 { ra2_sim::Item::Unit(item) } else { ra2_sim::Item::Building(item) }))
            .collect(),
        // The cameo is the type's `AltCameo` when
        // the player's house builds veterans of its kind (a spy has been in
        // an enemy barracks, for soldiers; a vehicle factory, for vehicles,
        // ships too though they are not built veterans) and it is `Trainable`.
        veteran: buildable
            .iter()
            .map(|&item| {
                tab >= 2
                    && sim.0.unit_types.get(item as usize).is_some_and(|kind| {
                        kind.stats.trainable && if kind.stats.infantry { house.veterans[0] } else { house.veterans[1] && kind.stats.plane.is_none() }
                    })
            })
            .collect(),
        production: in_production.map(|p| (p.building, p.stage.clamp(0, 53) as usize, p.is_ready(), p.on_hold())),
        others: if tab == 3 {
            [house.ships, house.aircraft].into_iter().flatten().map(|p| (p.building, p.stage.clamp(0, 53) as usize, p.is_ready(), p.on_hold())).collect()
        } else {
            Vec::new()
        },
        supers: supers.iter().map(|&index| (SUPER + index, (sim.0.super_stage(local(), index as usize) as usize).min(53), house.supers[index as usize].ready, house.supers[index as usize].hold)).collect(),
        hovered,
    };
    if hud.shown.strip.as_ref() != Some(&strip) {
        for entity in &strip_parts {
            commands.entity(entity).despawn();
        }
        commands.entity(root).with_children(|parent| {
            // Frame 2 for a tab with nothing in it, 0
            // otherwise, 1 for the one showing; 3 and 4 are 0 and 1 lit up.
            for index in 0..4 {
                let frame = if !strip.enabled[index] {
                    2
                } else {
                    usize::from(index == strip.tab) + if index < 2 && strip.flashing[index] { 3 } else { 0 }
                };
                if let Some((image, size)) = load(&files, &mut images, &format!("tab0{index}.shp"), frame, PALETTE) {
                    parent.spawn((
                        StripPart,
                        absolute(TAB_X + TAB_PITCH * index as f32, TAB_Y, size),
                        ImageNode::new(image),
                    ));
                }
            }
            for (slot, &item) in strip.buildable.iter().take(slots).enumerate() {
                let cameo = if item >= SUPER {
                    super_catalog.0.get((item - SUPER) as usize).map(|entry| entry.2.as_str())
                } else if strip.tab >= 2 {
                    unit_catalog.0.get(item as usize).map(|entry| match (&entry.alt_cameo, strip.veteran.get(slot)) {
                        (Some(alt), Some(true)) if files.get(&format!("{alt}.shp")).is_some() => alt.as_str(),
                        _ => entry.cameo.as_str(),
                    })
                } else {
                    catalog.0.get(item as usize).map(|entry| entry.cameo.as_str())
                };
                if let Some(cameo) = cameo {
                    draw_cameo(parent, hud, &files, cameo, &mut images, &strip, slot, item);
                }
            }
        });
        hud.shown.strip = Some(strip);
    }

    // The tooltip for the cameo under a mouse that has stopped moving.
    // The Options screen's Tooltips turns them off.
    let tip = hovered.filter(|_| prefs.tooltips && now - hud.mouse_still_since >= TIP_DELAY).map(|slot| buildable[slot]);
    if hud.shown.tip != Some(tip) {
        for entity in &tip_parts {
            commands.entity(entity).despawn();
        }
        if let (Some(building), Some(slot), Some(font)) = (tip, hovered, &hud.font) {
            // The name and price, with every space
            // turned into a line break.
            // DATA: the format is the string `TXT_MONEY_FORMAT_2`, "%s $%d".
            let (key, cost) = if tab >= 2 {
                unit_catalog.0.get(building as usize).map_or(("", 0), |entry| (entry.ui_name.as_str(), entry.cost))
            } else {
                (
                    catalog.0.get(building as usize).map_or("", |entry| entry.ui_name.as_str()),
                    sim.0.building_types.get(building as usize).map_or(0, |kind| kind.cost),
                )
            };
            let key = if building >= SUPER { super_catalog.0.get((building - SUPER) as usize).map_or("", |entry| entry.1.as_str()) } else { key };
            let name = hud.strings.as_ref().and_then(|strings| strings.get(key)).unwrap_or("");
            let text = if building >= SUPER { name.replace(' ', "\n") } else { format!("{name} ${cost}").replace(' ', "\n") };
            let (image, size) = text_image(font, &mut images, &text);
            // A black box with a one-pixel
            // outline, text inset by (4, 2), placed diagonally off the cameo
            // towards the middle of the sidebar, overlapping it by 5 pixels.
            let box_size = size + Vec2::new(8.0, 4.0);
            let area = tooltip_rect(slot_rect(slot), box_size, Vec2::new(SIDEBAR_WIDTH, window.height()));
            commands.entity(root).with_children(|parent| {
                spawn_tooltip(parent, TipPart, image, size, area.min, 10, hud.theme);
            });
        }
        hud.shown.tip = Some(tip);
    }
}

/// Draws one cameo and what lies over it.
// The layers and their order.
#[allow(clippy::too_many_arguments)]
fn draw_cameo(
    parent: &mut ChildSpawnerCommands,
    hud: &Hud,
    files: &GameFiles,
    cameo_art: &str,
    images: &mut Assets<Image>,
    strip: &StripState,
    slot: usize,
    building: u16,
) {
    let Some((image, size)) = load(files, images, &format!("{cameo_art}.shp"), 0, CAMEO_PALETTE) else {
        return;
    };
    let rect = slot_rect(slot);
    // A super weapon's cameo carries its own
    // clock, "Ready" and "On Hold".
    let mine = strip.supers.iter().copied().find(|(b, ..)| *b == building).or(strip.production.filter(|(b, ..)| *b == building)).or_else(|| strip.others.iter().copied().find(|(b, ..)| strip.tab == 3 && *b == building));
    parent.spawn((StripPart, absolute(rect.min.x, rect.min.y, size), ImageNode::new(image))).with_children(
        |cameo| {
            let whole = absolute(0.0, 0.0, CAMEO_SIZE);
            if strip.hovered == Some(slot) {
                cameo.spawn((
                    Node {
                        border: UiRect::all(Val::Px(1.0)),
                        ..whole.clone()
                    },
                    BorderColor::all(text_color()),
                ));
            }
            // A building's cameo is darkened while any cameo of its tab is in
            // production, on hold or waiting to be placed. Nothing in
            // ordinary production flashes a cameo. And any idle cameo at its
            // build limit (aircraft: as many as the house has pads).
            let shade = BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5));
            if (strip.tab < 2 && strip.production.is_some() && mine.is_none()) || (mine.is_none() && strip.limited.get(slot) == Some(&true)) {
                cameo.spawn((whole.clone(), shade));
            }
            // How many of the type are in hand, when
            // more than one or when another type is the one being made: at the
            // top right, on a darkened box.
            let count = strip.counts.get(slot).copied().unwrap_or(0);
            let counted = count > 1 || (count == 1 && mine.is_none());
            if counted {
                if let Some(font) = &hud.font {
                    let (image, text_size) = text_image(font, images, &count.to_string());
                    let x = 60.0 - text_size.x - 2.0;
                    cameo.spawn((
                        absolute(x - 2.0, 0.0, text_size + Vec2::new(4.0, 2.0)),
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 175.0 / 255.0)),
                    ));
                    cameo.spawn((absolute(x, 1.0, text_size), ImageNode::new(image)));
                }
            }
            let Some((_, stage, ready, on_hold)) = mine else {
                return;
            };
            // The status text, centred one pixel down, on a box blended towards
            // black (alpha 175 of 255) and padded by (2, 1).
            let status = |cameo: &mut ChildSpawnerCommands, images: &mut Assets<Image>, key: &str, fallback: &str| {
                if let (Some(font), Some(strings)) = (&hud.font, &hud.strings) {
                    let text = strings.get(key).unwrap_or(fallback);
                    let (image, text_size) = text_image(font, images, text);
                    // Centred, or at the left when the count has the right.
                    let x = if counted { 2.0 } else { (30.0 - text_size.x / 2.0).floor() };
                    cameo.spawn((
                        absolute(x - 2.0, 0.0, text_size + Vec2::new(4.0, 2.0)),
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 175.0 / 255.0)),
                    ));
                    cameo.spawn((absolute(x, 1.0, text_size), ImageNode::new(image)));
                }
            };
            if ready {
                // Finished: the word alone, no clock.
                status(cameo, images, "TXT_READY", "Ready");
                return;
            }
            // The clock, frame = stage + 1, with "On Hold" over it when stopped.
            if let Some((image, clock_size)) = load(files, images, "gclock2.shp", stage + 1, PALETTE) {
                let clock = ImageNode::new(image).with_color(Color::srgba(1.0, 1.0, 1.0, 0.5));
                cameo.spawn((absolute(0.0, 0.0, clock_size), clock));
            }
            if on_hold {
                status(cameo, images, "TXT_HOLD", "On Hold");
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small real-format tables exercise CSF parsing and case-insensitive labels
    /// without requiring a particular imported edition of the game.
    fn csf(entries: &[(&str, &str)]) -> Csf {
        let mut data = b" FSC".to_vec();
        for value in [3, entries.len() as u32, entries.len() as u32, 0, 0] { data.extend(value.to_le_bytes()); }
        for &(key, value) in entries {
            data.extend(b" LBL");
            data.extend(1u32.to_le_bytes());
            data.extend((key.len() as u32).to_le_bytes());
            data.extend(key.as_bytes());
            data.extend(b" RTS");
            let units: Vec<_> = value.encode_utf16().collect();
            data.extend((units.len() as u32).to_le_bytes());
            for unit in units { data.extend((!unit).to_le_bytes()); }
        }
        Csf::parse(&data).expect("valid CSF fixture")
    }

    fn faction_rules() -> Ini {
        Ini::parse(b"[Countries]\n0=Americans\n1=Russians\n2=YuriCountry\n3=YuriAlias\n[Americans]\nSide=GDI\n[Russians]\nSide=Nod\n[YuriCountry]\nSide=ThirdSide\n[YuriAlias]\nSide=Yuri\n")
    }

    #[test]
    fn yuri_sidebar_uses_native_background_and_radar_over_the_soviet_base() {
        let dir = crate::files::dev_dir();
        let Ok(files) = GameFiles::open_for_edition(&dir, crate::files::Edition::YuriAssets) else { return };
        files.set_side(2);
        for (regular, yuri) in [("bkgdsm.shp", "bkgdsmy.shp"), ("bkgdmd.shp", "bkgdmdy.shp"), ("bkgdlg.shp", "bkgdlgy.shp")] {
            assert_eq!(sidebar_background(&files, regular, 2), (yuri, "uibkgdy.pal"));
            assert!(Shp::parse(files.get(yuri).unwrap()).is_some());
            assert_eq!(sidebar_background(&files, regular, 1), (regular, "uibkgd.pal"));
        }
        assert_eq!(sidebar_radar(&files, 2), ("radary.shp", "radaryuri.pal"));
        assert!(Shp::parse(files.get("radary.shp").unwrap()).is_some());
        assert!(Palette::parse(files.get("radaryuri.pal").unwrap()).is_some());
        assert!(Palette::parse(files.get("uibkgdy.pal").unwrap()).is_some());
        // Shared strip/button art is supplied by GameFiles' Soviet fallback.
        assert_eq!(sidebar_background(&files, "side1.shp", 2), ("side1.shp", PALETTE));
        assert!(Shp::parse(files.get("side1.shp").unwrap()).is_some());
    }

    #[test]
    fn tooltip_border_switches_between_all_three_factions_from_country_side() {
        let countries = country_themes(&faction_rules());
        // Country metadata also identifies the theme in old snapshots whose
        // saved side value predates the third-side simulation enum.
        let mut house = ra2_sim::House { side: 0, ..default() };
        for (country, expected) in [(0, SidebarTheme::Allied), (1, SidebarTheme::Soviet), (2, SidebarTheme::Yuri), (1, SidebarTheme::Soviet), (0, SidebarTheme::Allied), (3, SidebarTheme::Yuri)] {
            house.country = country;
            assert_eq!(SidebarTheme::for_house(&countries, &house), expected);
        }
        assert_eq!(SidebarTheme::Allied.border(), Color::srgb_u8(164, 210, 255));
        assert_eq!(SidebarTheme::Soviet.border(), Color::srgb_u8(255, 96, 48));
        assert_eq!(SidebarTheme::Yuri.border(), Color::srgb_u8(196, 112, 255));
        for (side, expected) in [(0, SidebarTheme::Allied), (1, SidebarTheme::Soviet), (2, SidebarTheme::Yuri)] {
            house.side = side;
            assert_eq!(SidebarTheme::for_house(&[], &house), expected);
        }
    }

    #[test]
    fn switching_factions_recolours_already_visible_button_and_cameo_tooltips() {
        let mut world = ra2_sim::World::new(ra2_sim::Map::new(4, 4), None);
        // Do not mutate the process-wide local player used by other tests.
        world.houses = vec![ra2_sim::House::default(); 256];
        let mut app = App::new();
        app.insert_resource(Sim(world, Vec::new(), Vec::new()))
            .insert_resource(Hud {
                root: Entity::PLACEHOLDER, size: Vec2::ZERO, font: None, strings: None,
                theme: SidebarTheme::Allied, country_themes: country_themes(&faction_rules()),
                rows: 0, tab: 0, enabled: [false; 4], tab_sound: None, build_sound: None,
                scold_sound: None, credit_sounds: None, credits: None, credits_tick: 0,
                pips: [0; 3], pips_stepped: 0.0, mouse: Vec2::ZERO,
                mouse_still_since: 0.0, shown: Shown::default(),
            })
            .add_systems(Update, sync_tooltip_theme);
        let button = app.world_mut().spawn((Node::default(), Visibility::Visible, ButtonTipPart, TooltipFrame, BorderColor::all(SidebarTheme::Allied.border()))).id();
        let cameo = app.world_mut().spawn((Node::default(), Visibility::Visible, TipPart, TooltipFrame, BorderColor::all(SidebarTheme::Allied.border()))).id();
        for (country, expected) in [(1, SidebarTheme::Soviet), (2, SidebarTheme::Yuri), (0, SidebarTheme::Allied), (2, SidebarTheme::Yuri)] {
            for house in &mut app.world_mut().resource_mut::<Sim>().0.houses { house.country = country; }
            app.update();
            assert_eq!(app.world().resource::<Hud>().theme, expected);
            for entity in [button, cameo] {
                assert_eq!(app.world().get::<BorderColor>(entity).unwrap(), &BorderColor::all(expected.border()));
                assert_eq!(app.world().get::<Visibility>(entity), Some(&Visibility::Visible));
            }
        }
    }

    #[test]
    fn tooltip_theme_does_not_depend_on_language_or_desktop_dpi() {
        use bevy::window::WindowResolution;
        let countries = country_themes(&faction_rules());
        for words in ["Options Button", "選項按鈕", "选项按钮"] {
            let strings = csf(&[("TIP:OptionsButton", words)]);
            for dpi in [1.0, 1.5, 2.0] {
                let mut window = Window { resolution: WindowResolution::new(1600, 1200), ..default() };
                window.resolution.set_scale_factor(dpi);
                let at = Vec2::new(window.width() - SIDEBAR_WIDTH + 119.0, 29.0);
                let button = button_tip_at(&window, crate::pause::Menu::Closed, at).unwrap().0;
                assert_eq!(button.text(Some(&strings)), words);
                for (country, expected) in [(0, SidebarTheme::Allied), (1, SidebarTheme::Soviet), (2, SidebarTheme::Yuri)] {
                    let house = ra2_sim::House { country, side: 0, ..default() };
                    assert_eq!(SidebarTheme::for_house(&countries, &house).border(), expected.border());
                }
            }
        }
    }

    #[test]
    fn tooltips_use_the_imported_ui_language_for_missing_labels() {
        let english = csf(&[("tip:diplomacybutton", "Diplomacy Button"), ("TIP:OptionsButton", "Options Button"), ("TIP:Repair", "Repair"), ("GUI:Options", "Options"), ("Name:ChineseUnit", "中國單位")]);
        let traditional = csf(&[("TIP:DiplomacyButton", "外交按鈕"), ("TIP:OptionsButton", "選項按鈕"), ("TIP:Repair", "修復"), ("GUI:Options", "選項")]);
        let simplified = csf(&[("TIP:DiplomacyButton", "外交按钮"), ("TIP:OptionsButton", "选项按钮"), ("TIP:Repair", "修复"), ("GUI:Options", "选项")]);
        for (strings, language, labels) in [
            (&english, TooltipLanguage::English, ["Diplomacy Button", "Options Button", "Repair", "Sell"]),
            (&traditional, TooltipLanguage::TraditionalChinese, ["外交按鈕", "選項按鈕", "修復", "出售"]),
            (&simplified, TooltipLanguage::SimplifiedChinese, ["外交按钮", "选项按钮", "修复", "出售"]),
        ] {
            assert_eq!(TooltipLanguage::of(Some(strings)), language);
            for (button, expected) in [SidebarButton::Diplomacy, SidebarButton::Options, SidebarButton::Repair, SidebarButton::Sell].into_iter().zip(labels) {
                assert_eq!(button.text(Some(strings)), expected);
            }
        }
        assert_eq!(SidebarButton::Sell.text(None), "Sell");
    }

    #[test]
    fn native_tooltip_labels_override_language_fallbacks() {
        let strings = csf(&[("GUI:Options", "選項"), ("TIP:Sell", "Native sell description")]);
        assert_eq!(SidebarButton::Sell.text(Some(&strings)), "Native sell description");
        assert_eq!(SidebarButton::Diplomacy.text(Some(&strings)), "外交按鈕");
        assert_eq!(SidebarButton::Options.text(Some(&strings)), "選項按鈕");
        assert_eq!(SidebarButton::Repair.text(Some(&strings)), "修復");
        let strings = csf(&[("GUI:Options", "选项")]);
        assert_eq!(SidebarButton::Options.text(Some(&strings)), "选项按钮");
        assert_eq!(SidebarButton::Repair.text(Some(&strings)), "修复");
    }

    #[test]
    fn button_tooltips_follow_logical_positions_at_desktop_dpi() {
        use crate::pause::Menu;
        use bevy::window::WindowResolution;
        let mut window = Window { resolution: WindowResolution::new(1600, 1200), ..default() };
        window.resolution.set_scale_factor(2.0);
        assert_eq!(window.size(), Vec2::new(800.0, 600.0));
        let side = window.width() - SIDEBAR_WIDTH;
        for (at, expected) in [
            (Vec2::new(side + 47.0, 29.0), SidebarButton::Diplomacy),
            (Vec2::new(side + 119.0, 29.0), SidebarButton::Options),
            (Vec2::new(side + 52.0, 180.0), SidebarButton::Repair),
            (Vec2::new(side + 116.0, 180.0), SidebarButton::Sell),
        ] {
            assert_eq!(button_tip_at(&window, Menu::Closed, at).map(|hit| hit.0), Some(expected));
        }
        // The top buttons remain visible over paused menus, the mode buttons do not.
        assert_eq!(button_tip_at(&window, Menu::Options, Vec2::new(side + 47.0, 29.0)).map(|hit| hit.0), Some(SidebarButton::Diplomacy));
        assert!(button_tip_at(&window, Menu::Options, Vec2::new(side + 52.0, 180.0)).is_none());
        assert!(button_tip_at(&window, Menu::Closed, Vec2::new(side + 52.0, 197.0)).is_none());
    }

    #[test]
    fn tooltip_frames_stay_inside_the_sidebar_near_all_edges() {
        let bounds = Vec2::new(SIDEBAR_WIDTH, 600.0);
        for anchor in [Rect::new(11.0, 20.0, 83.0, 38.0), Rect::new(83.0, 20.0, 155.0, 38.0), Rect::new(85.0, 570.0, 145.0, 600.0)] {
            let area = tooltip_rect(anchor, Vec2::new(110.0, 22.0), bounds);
            assert!(area.min.cmpge(Vec2::ZERO).all());
            assert!(area.max.cmple(bounds).all());
        }
    }

    #[test]
    fn mode_buttons_do_not_share_clicks_with_each_other_or_the_tabs() {
        assert!(mode_button_at(Vec2::new(83.99, 180.0)) == Some(crate::control::Mode::Repair));
        assert!(mode_button_at(Vec2::new(84.0, 180.0)) == Some(crate::control::Mode::Sell));
        assert!(mode_button_at(Vec2::new(147.99, 180.0)) == Some(crate::control::Mode::Sell));
        assert!(mode_button_at(Vec2::new(148.0, 180.0)).is_none());
        assert!(mode_button_at(Vec2::new(52.0, 196.99)) == Some(crate::control::Mode::Repair));
        assert!(mode_button_at(Vec2::new(52.0, 197.0)).is_none());
    }
}
