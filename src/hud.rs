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
use ra2_formats::pal::Palette;
use ra2_formats::shp::Shp;
use ra2_sim::Order;

use crate::audio::Cue;
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

pub(crate) const PALETTE: &str = "sidebar.pal";
const CAMEO_PALETTE: &str = "cameo.pal";

/// Sidebar text colour for the first side.
// The other side uses (255, 255, 0).
pub(crate) const TEXT: [u8; 3] = [164, 210, 255];

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

pub struct HudPlugin;

/// The pressed picture of the repair or sell button.
#[derive(Component)]
struct ModeLight(crate::control::Mode);

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
            .add_systems(Update, relayout.run_if(in_state(Screen::InGame)).run_if(resource_exists::<Hud>).in_set(crate::schedule::GameSet::Hud));
        // The sidebar's art depends on the player's side, which the map's set-up
        // settles.
        app.add_systems(OnEnter(Screen::InGame), spawn_sidebar.after(crate::game::load_map)).add_systems(
            Update,
            update_sidebar
                .run_if(in_state(Screen::InGame))
                .run_if(crate::pause::playing)
                .run_if(resource_exists::<Hud>)
                .run_if(resource_exists::<Sim>)
                .in_set(crate::schedule::GameSet::Input),
        )
        .add_systems(Update, mode_lights.run_if(in_state(Screen::InGame)).run_if(resource_exists::<crate::control::Mode>).in_set(crate::schedule::GameSet::Hud));
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
    let lines: Vec<(usize, Vec<u8>)> = text.lines().map(|line| font.rasterize(line)).collect();
    let width = lines.iter().map(|(width, _)| *width).max().unwrap_or(1);
    let height = font.height * lines.len().max(1);
    let mut rgba = vec![0u8; width * height * 4];
    for (index, (line_width, mask)) in lines.iter().enumerate() {
        for row in 0..font.height {
            for col in 0..*line_width {
                if mask[row * line_width + col] != 0 {
                    let at = ((index * font.height + row) * width + col) * 4;
                    rgba[at..at + 4].copy_from_slice(&[TEXT[0], TEXT[1], TEXT[2], 255]);
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
    rules: Res<crate::files::Rules>,
) {
    let lit: Vec<_> = [("repair.shp", REPAIR_AT, crate::control::Mode::Repair), ("sell.shp", SELL_AT, crate::control::Mode::Sell)]
        .into_iter()
        .filter_map(|(name, at, mode)| load(&files, &mut images, name, 1, PALETTE).map(|(image, size)| (image, size, at, mode)))
        .collect();
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
            piece(parent, "diplobtn.shp", 0, DIPLOMACY_AT.0, top + DIPLOMACY_AT.1);
            piece(parent, "optbtn.shp", 0, OPTIONS_AT.0, top + OPTIONS_AT.1);
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
    let credit_sounds = rules
        .visual("CreditTicks")
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
        rows,
        tab,
        enabled,
        tab_sound: rules.sound("GUITabSound"),
        build_sound: rules.sound("GUIBuildSound"),
        scold_sound: rules.sound("ScoldSound"),
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
fn update_sidebar(
    mut commands: Commands,
    mut hud: ResMut<Hud>,
    mut sim: ResMut<Sim>,
    mut placing: ResMut<Placing>,
    mut modes: (ResMut<crate::control::Mode>, ResMut<crate::control::Selection>),
    catalog: Res<BuildCatalog>,
    catalogs: (Res<crate::units::UnitCatalog>, Res<crate::units::SuperCatalog>, Res<crate::prefs::Prefs>),
    (files, rules): (Res<GameFiles>, Res<crate::files::Rules>),
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
            0 => everything.iter().copied().filter(|&building| !sim.0.is_defence(building)).collect(),
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
    if let Some(forced) = std::env::var("RA2_TAB").ok().and_then(|n| n.trim().parse::<usize>().ok())
        && forced < 4 && enabled[forced] {
            hud.tab = forced;
        }
    // A tab with nothing in it is disabled and takes no clicks; a click on any
    // other shows it and plays `GUITabSound`.
    if buttons.just_pressed(MouseButton::Left) {
        let tab = (0..4).find(|&tab| {
            let corner = Vec2::new(TAB_X + TAB_PITCH * tab as f32, TAB_Y);
            mouse.is_some_and(|at| Rect::from_corners(corner, corner + Vec2::new(TAB_PITCH, 26.0)).contains(at))
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
        for (name, corner, which) in [("repair.shp", REPAIR_AT, crate::control::Mode::Repair), ("sell.shp", SELL_AT, crate::control::Mode::Sell)] {
            let size = files.get(name).and_then(Shp::parse).map_or(Vec2::ZERO, |shp| Vec2::new(shp.width as f32, shp.height as f32));
            let corner = Vec2::new(corner.0, corner.1);
            if mouse.is_some_and(|at| Rect::from_corners(corner, corner + size).contains(at)) {
                let (mode, selection) = &mut modes;
                mode.toggle(which, &sim.0, selection, &placing);
                cues.write_batch(rules.sound("GUIMainButtonSound").map(Cue::Sound));
            }
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
    if tab < 2
        && let Some(production) = in_production.filter(|production| !buildable.contains(&production.building)) {
            buildable.push(production.building);
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
            cues.write_batch(rules.sound("GUIMainButtonSound").map(Cue::Sound));
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
        } else if buttons.just_pressed(MouseButton::Right)
            && let Some(production) = mine {
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
            let cameo = slot_rect(slot);
            let x = if cameo.min.x < SIDEBAR_WIDTH / 2.0 {
                cameo.max.x - 5.0
            } else {
                cameo.min.x - box_size.x + 5.0
            };
            let y = if cameo.min.y < window.height() / 2.0 {
                cameo.max.y - 5.0
            } else {
                cameo.min.y - box_size.y + 5.0
            };
            let x = x.min(SIDEBAR_WIDTH - box_size.x).max(0.0);
            commands.entity(root).with_children(|parent| {
                parent
                    .spawn((
                        TipPart,
                        Node {
                            border: UiRect::all(Val::Px(1.0)),
                            ..absolute(x, y.max(0.0), box_size)
                        },
                        BackgroundColor(Color::BLACK),
                        BorderColor::all(text_color()),
                        GlobalZIndex(10),
                    ))
                    .with_children(|tip| {
                        tip.spawn((absolute(3.0, 1.0, size), ImageNode::new(image)));
                    });
            });
        }
        hud.shown.tip = Some(tip);
    }
}

/// Draws one cameo and what lies over it.
// The layers and their order.
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
            if counted
                && let Some(font) = &hud.font {
                    let (image, text_size) = text_image(font, images, &count.to_string());
                    let x = 60.0 - text_size.x - 2.0;
                    cameo.spawn((
                        absolute(x - 2.0, 0.0, text_size + Vec2::new(4.0, 2.0)),
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 175.0 / 255.0)),
                    ));
                    cameo.spawn((absolute(x, 1.0, text_size), ImageNode::new(image)));
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
