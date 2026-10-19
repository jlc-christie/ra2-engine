//! Playing against another machine.
//!
//! The lockstep rules (order delay, stalls, sync check, drops) are the
//! original's and live in the `ra2-net` crate; this file joins a session to
//! the game.
//!
//! Not original behaviour: how the machines find each other. One player
//! creates a game (the Network menu, or `RA2_HOST=port`) and sets it up on the
//! skirmish screen; another joins by address (`RA2_JOIN=address:port`) and
//! takes the first computer row's place. The original's lobby (dialog 187)
//! finds games on the local network by broadcast and lets every player choose
//! a country and colour.
//!

use std::net::TcpListener;

use bevy::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use ra2_net::directory::{FromServer, GameInfo, PeerLink, RoomLink, ServerConn, ToServer};
use ra2_net::{Config, Game, Link, MemoryLink, Msg, NetKind, Relay, Session, Step, TcpLink};
use ra2_sim::{Order, World};

use crate::game::{GameSetup, MatchSetup};
use crate::shell::Screen;
use crate::units::Sim;

/// The level a skirmish row carries when a person on another machine plays it.
pub const HUMAN_ROW: i32 = 3;

/// Set while a network game is being played: the simulation is then stepped by
/// the session, not by the clock.
#[derive(Resource)]
pub struct NetActive;

/// The house this machine's player sits at in a network game.
#[derive(Resource, Clone, Copy)]
pub struct NetSeat(pub u8);

/// What the menus know of a network game being arranged.
#[derive(Resource)]
pub struct Lobby {
    pub mode: LobbyMode,
    /// The address to join, or whose port to wait on.
    pub address: String,
    /// What to tell the player.
    pub status: String,
    /// This player's name, and whether typing on the Network screen goes to
    /// it (or to the address).
    pub name: String,
    /// Which box typing goes to on the Network and Internet screens.
    pub focus: Focus,
    /// Internet games: the meeting-place server's address, the password for
    /// the game made or joined, the games the server lists, the one picked,
    /// and whether the list is wanted (the Internet screen is up) or should
    /// be asked for again.
    pub server: String,
    pub password: String,
    pub games: Vec<GameInfo>,
    pub picked: Option<u32>,
    pub browse: bool,
    pub refresh: bool,
    /// The game being made or joined goes through the server.
    pub via_server: bool,
    /// The name of the player in each lobby row (row 0 is the host), and, once
    /// a game starts, of the player at each house.
    pub names: [String; 8],
    pub house_names: Vec<String>,
    /// Hosting: Start Game was pressed.
    pub start: bool,
    /// The game is starting: the menus must leave the screen alone.
    pub starting: bool,
    /// This machine's row on the lobby screen: 0 for the host, 1 to 7 for a
    /// player who joined.
    pub my_row: usize,
    /// The map and mode chosen, as the host has them (the menus keep these
    /// up to date on the host and follow them on the others).
    pub map: String,
    pub mode_rules: String,
    pub max_players: usize,
    /// The skirmish choices as they were before the lobby, to put back
    /// afterwards: the host's rows only, a joiner's every one.
    saved: Option<(crate::game::Options, bool)>,
}

/// A box that can be typed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Focus {
    #[default]
    Address,
    Name,
    Server,
    Password,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LobbyMode {
    Off,
    Hosting,
    Joining,
}

/// The port games are created on unless another is given.
pub const DEFAULT_PORT: u16 = 47000;
/// The meeting-place server's port unless another is given.
pub const SERVER_PORT: u16 = 47100;

impl Default for Lobby {
    fn default() -> Self {
        let mut kept = Settings::read();
        // RA2_HOST=port and RA2_JOIN=address:port do from the command line
        // what the Network menu does, the host starting as soon as
        // RA2_HOST_PLAYERS others (one, if not given) are there.
        // RA2_SERVER_HOST=1 and RA2_SERVER_JOIN=1 do the same through the
        // meeting-place server at RA2_SERVER (the joiner takes the first
        // game listed).
        let via_server = std::env::var_os("RA2_SERVER_HOST").is_some() || std::env::var_os("RA2_SERVER_JOIN").is_some();
        let (mode, address, start) = match (std::env::var("RA2_HOST"), std::env::var("RA2_JOIN")) {
            _ if std::env::var_os("RA2_SERVER_HOST").is_some() => (LobbyMode::Hosting, String::new(), true),
            _ if std::env::var_os("RA2_SERVER_JOIN").is_some() => (LobbyMode::Joining, String::new(), false),
            (Ok(port), _) => (LobbyMode::Hosting, format!("0.0.0.0:{}", port.trim()), true),
            (_, Ok(address)) => (LobbyMode::Joining, address.trim().to_owned(), false),
            // RA2_NET_ADDRESS=address:port is what the Network screen starts with.
            _ => (LobbyMode::Off, std::env::var("RA2_NET_ADDRESS").ok().or(kept.address.take()).unwrap_or_else(|| format!("127.0.0.1:{DEFAULT_PORT}")), false),
        };
        // RA2_NAME; else the name kept from last time; else the name the
        // machine knows its user by.
        let name = std::env::var("RA2_NAME").ok().or(kept.name.take()).or_else(|| std::env::var("USER").ok()).map(|name| clean_name(&name)).unwrap_or_default();
        Self { mode, address, status: String::new(), name, focus: Focus::default(), server: std::env::var("RA2_SERVER").ok().or(kept.server.take()).unwrap_or_else(|| format!("127.0.0.1:{SERVER_PORT}")), password: std::env::var("RA2_PASSWORD").unwrap_or_default(), games: Vec::new(), picked: None, browse: via_server, refresh: false, via_server, names: Default::default(), house_names: Vec::new(), start, starting: false, my_row: 0, map: String::new(), mode_rules: String::new(), max_players: 8, saved: None }
    }
}

impl Lobby {
    fn port(&self) -> u16 {
        self.address.rsplit(':').next().and_then(|port| port.trim().parse().ok()).unwrap_or(DEFAULT_PORT)
    }

    /// Whether this machine may change a row's country and colour: the host
    /// its own and the computer players', a joiner only their own.
    pub fn may_edit_row(&self, row: usize, human_row: bool) -> bool {
        match self.mode {
            LobbyMode::Off => true,
            LobbyMode::Hosting => row == 0 || !human_row,
            LobbyMode::Joining => row == self.my_row,
        }
    }

    /// Whether this machine may change the game's options, map and computer
    /// players: anyone but a joiner.
    pub fn may_edit_game(&self) -> bool {
        self.mode != LobbyMode::Joining
    }
}

/// What is kept between runs: the player's name and the addresses last typed.
// The original keeps the
// player's name in RA2.INI (`[MultiPlayer] Handle`). Not original behaviour:
// a file of our own (`settings.ini` in `home::data_dir`, or `RA2_SETTINGS`),
// since the game's folder may not be ours to write in.
#[derive(Default)]
pub struct Settings {
    pub name: Option<String>,
    pub address: Option<String>,
    pub server: Option<String>,
    /// The screen size asked for, and whether the game fills the screen.
    /// The original keeps the size as
    /// `[Video] ScreenWidth` and `ScreenHeight`. Not original behaviour: the
    /// original always fills the screen; `Fullscreen` is ours.
    pub screen: Option<(u32, u32)>,
    pub fullscreen: bool,
    /// The Options screen's choices.
    pub prefs: crate::prefs::Prefs,
    /// Where the player's copy of the game's files was last found
    /// (`[Files] GameFolder`; see `setup`).
    pub game_dir: Option<std::path::PathBuf>,
}

impl Settings {
    fn path() -> std::path::PathBuf {
        std::env::var_os("RA2_SETTINGS").map_or_else(|| crate::home::data_dir().join("settings.ini"), Into::into)
    }

    pub fn read() -> Settings {
        let text = std::fs::read_to_string(Self::path()).unwrap_or_default();
        let get = |key: &str| text.lines().filter_map(|line| line.split_once('=')).find(|(name, _)| name.trim().eq_ignore_ascii_case(key)).map(|(_, value)| value.trim().to_owned()).filter(|value| !value.is_empty());
        let number = |key: &str| get(key).and_then(|value| value.parse::<u32>().ok()).filter(|&n| n >= 200);
        Settings {
            name: get("Handle"),
            address: get("Address"),
            server: get("Server"),
            screen: number("ScreenWidth").zip(number("ScreenHeight")),
            fullscreen: get("Fullscreen").is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "1")),
            prefs: crate::prefs::Prefs::from_keys(&get),
            game_dir: get("GameFolder").map(Into::into),
        }
    }

    /// Writes what the lobby now has, keeping the rest.
    pub fn write(lobby: &Lobby) {
        let kept = Settings::read();
        Settings { name: Some(lobby.name.clone()), address: Some(lobby.address.clone()), server: Some(lobby.server.clone()), ..kept }.save();
    }

    /// Writes all of it.
    pub fn save(&self) {
        let field = |value: &Option<String>| value.clone().unwrap_or_default();
        let mut text = format!("[MultiPlayer]\nHandle={}\nAddress={}\nServer={}\n\n[Video]\n", field(&self.name), field(&self.address), field(&self.server));
        if let Some((width, height)) = self.screen {
            text += &format!("ScreenWidth={width}\nScreenHeight={height}\n");
        }
        text += &format!("Fullscreen={}\n", if self.fullscreen { "yes" } else { "no" });
        text += &self.prefs.to_keys();
        if let Some(dir) = &self.game_dir {
            text += &format!("\n[Files]\nGameFolder={}\n", dir.display());
        }
        let path = Self::path();
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(path, text) {
            warn!("could not keep the settings: {error}");
        }
    }
}

/// Whether this machine was told from the command line to create a game.
pub fn auto_host() -> bool {
    std::env::var_os("RA2_HOST").is_some() || std::env::var_os("RA2_SERVER_HOST").is_some()
}

/// A name as it may be shown: letters, digits, spaces and a few marks, at most
/// sixteen characters.
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')).take(16).collect::<String>().trim().to_owned()
}

/// What the host tells each player of the lobby.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
struct LobbyView {
    options: crate::game::Options,
    map: String,
    mode_rules: String,
    max_players: usize,
    /// The row of the player this is sent to.
    row: usize,
    /// Who is in each row.
    names: [String; 8],
}

/// What a joiner asks the host for: their own country and colour.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
struct Request {
    country: i32,
    colour: i32,
}

/// What the host sends to start: the match as settled, and the house the
/// player it is sent to sits at.
#[derive(serde::Serialize, serde::Deserialize)]
struct StartBlob {
    setup: MatchSetup,
    seat: u8,
    /// The players' names by house.
    house_names: Vec<String>,
}

/// A player who has joined the host's lobby.
struct Client {
    row: usize,
    /// Their number in the server's room, for a game made there.
    peer: Option<u32>,
    link: Box<dyn Link>,
    /// The last view sent, so that only changes are.
    sent: Option<LobbyView>,
}

/// A connection made before the game starts, waiting for it.
enum Waiting {
    /// The host's links to the players who joined, each with its house; and
    /// the server's room, for a game made there (the server then passes the
    /// game's packets round, not this machine).
    Host(Vec<(u8, Box<dyn Link>)>, Option<Rc<RefCell<ServerConn>>>),
    /// The joiner's link to the host (or the server's room), with what the
    /// host said to start with.
    Joiner { link: Box<dyn Link>, room: Option<Rc<RefCell<ServerConn>>>, internet: bool, game_id: u32, speed: u8, max_ahead: u16, frame_send_rate: u8 },
}

/// Where arranging a game has got to on this machine.
#[derive(Default)]
#[allow(clippy::large_enum_variant)]
enum Phase {
    #[default]
    Idle,
    /// Connected to the meeting-place server, looking at its list.
    Browsing(Rc<RefCell<ServerConn>>),
    /// Asked the server for a place in a game.
    Asking(Rc<RefCell<ServerConn>>),
    /// Hosting: where players arrive, and whoever has joined.
    Listening(Door, Vec<Client>),
    /// Joining: in the host's lobby, with the last view of it and the last
    /// request made.
    Joined(Box<dyn Link>, Option<LobbyView>, Option<Request>, Option<Rc<RefCell<ServerConn>>>),
    /// The game is starting: the session is made once the match is set up.
    Ready(Waiting),
}

/// Where a host's players arrive: a port of its own, or its room on the
/// server (with what the server's list last said of it).
enum Door {
    Port(TcpListener),
    Room(Rc<RefCell<ServerConn>>, (String, u8, u8)),
}

#[derive(Default)]
struct NetStart(Phase);

fn connect(address: &str, port: u16) -> Result<TcpLink, String> {
    use std::net::ToSocketAddrs;
    let address = if address.contains(':') { address.to_owned() } else { format!("{address}:{port}") };
    address
        .to_socket_addrs()
        .ok()
        .and_then(|mut found| found.next())
        .ok_or_else(|| "no such address".to_owned())
        .and_then(|to| std::net::TcpStream::connect_timeout(&to, std::time::Duration::from_secs(3)).map_err(|error| error.to_string()))
        .and_then(|stream| TcpLink::new(stream).map_err(|error| error.to_string()))
}

/// The houses the rows of a lobby come to: the first player's is 1, and each
/// row in use, in order, takes the next from 2.
fn seat_of_row(options: &crate::game::Options, max_players: usize, row: usize) -> Option<u8> {
    if row == 0 {
        return Some(crate::terrain::PLAYER);
    }
    let room = max_players.saturating_sub(1);
    let rows = &options.opponents;
    (row - 1 < room && rows[row - 1].level >= 0).then(|| 2 + rows[..row - 1].iter().filter(|row| row.level >= 0).count() as u8)
}

/// Every person's house in a settled match (its rows are the ones in use,
/// closed up).
fn human_houses(settled: &MatchSetup) -> Vec<u8> {
    std::iter::once(crate::terrain::PLAYER).chain(settled.options.opponents.iter().enumerate().filter(|(_, row)| row.level == HUMAN_ROW).map(|(n, _)| 2 + n as u8)).collect()
}

struct NetGame {
    session: Session,
    /// On the host: the forwarder every machine's packets go through.
    relay: Option<Relay>,
}

/// The simulation as the session sees it.
struct Lockstep<'a>(&'a mut World, &'a mut Vec<(u64, Order)>);

impl Game for Lockstep<'_> {
    fn frame(&self) -> u32 {
        self.0.current_tick() as u32
    }

    fn apply(&mut self, house: u8, order: &[u8]) {
        // An order the house may not give, or that cannot be read, is dropped
        // on every machine alike.
        if let Ok(order) = bincode::deserialize::<Order>(order)
            && self.0.order_as(house, order) {
                self.1.push((self.0.current_tick(), order));
            }
    }

    fn tick(&mut self) {
        self.0.tick();
    }

    // Not original behaviour: the whole state is hashed, where the original
    // folds in positions and facings only.
    fn check_value(&self) -> u32 {
        let hash = self.0.state_hash();
        (hash ^ (hash >> 32)) as u32
    }

    // A player who quits or
    // is dropped has their house destroyed.
    fn destroy_house(&mut self, house: u8) {
        self.0.destroy_house_for_tests(house);
    }
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send(NetStart::default())
            .init_resource::<Lobby>()
            .add_systems(Update, arrange.run_if(not(in_state(Screen::InGame))))
            .add_systems(Update, (begin, step).chain().run_if(in_state(Screen::InGame)).run_if(resource_exists::<Sim>))
            .add_systems(OnExit(Screen::InGame), |world: &mut bevy::ecs::world::World| {
                world.remove_non_send::<NetGame>();
                world.remove_resource::<NetActive>();
                world.remove_resource::<NetSeat>();
                world.non_send_mut::<NetStart>().0 = Phase::Idle;
                let mut lobby = world.resource_mut::<Lobby>();
                lobby.mode = LobbyMode::Off;
                lobby.start = false;
                lobby.starting = false;
                lobby.status.clear();
                lobby.my_row = 0;
                lobby.names = Default::default();
                lobby.house_names.clear();
                lobby.via_server = false;
                lobby.browse = false;
                // The skirmish rows go back to what they were.
                let saved = lobby.saved.take();
                let mut setup = world.resource_mut::<GameSetup>();
                match saved {
                    Some((options, true)) => setup.options = options,
                    Some((options, false)) => setup.options.opponents = options.opponents,
                    None => {}
                }
                for row in &mut setup.options.opponents {
                    if row.level == HUMAN_ROW {
                        *row = crate::game::Opponent::NONE;
                    }
                }
            });
    }
}

/// Arranges a game from the menus: the host's lobby, or a joiner's place in it.
fn arrange(mut commands: Commands, mut start: NonSendMut<NetStart>, mut lobby: ResMut<Lobby>, mut setup: ResMut<GameSetup>, mut next: ResMut<NextState<Screen>>) {
    let lobby = &mut *lobby;
    let say = |status: &mut String, text: String| {
        if *status != text {
            info!("network: {text}");
            *status = text;
        }
    };
    // The game is on its way in: nothing more to arrange.
    if lobby.starting {
        next.set(Screen::InGame);
        return;
    }
    match (lobby.mode, std::mem::take(&mut start.0)) {
        // Called off: whatever was open is let go, and the rows put back.
        // The Internet screen: the server's list of games.
        (_, Phase::Idle) if lobby.browse => match connect(&lobby.server, SERVER_PORT) {
            Ok(link) => {
                let conn = ServerConn::new(Box::new(link));
                conn.borrow_mut().say(&ToServer::List);
                say(&mut lobby.status, format!("Connected to the server at {}", lobby.server));
                start.0 = Phase::Browsing(conn);
            }
            Err(error) => {
                say(&mut lobby.status, format!("Cannot reach the server at {}: {error}", lobby.server));
                lobby.browse = false;
                lobby.mode = LobbyMode::Off;
            }
        },
        (LobbyMode::Off, Phase::Browsing(conn)) if lobby.browse => {
            let gone = {
                let mut server = conn.borrow_mut();
                if std::mem::take(&mut lobby.refresh) {
                    server.say(&ToServer::List);
                }
                server.pump();
                while let Some(news) = server.news.pop_front() {
                    if let FromServer::Games(games) = news {
                        if !lobby.picked.is_some_and(|id| games.iter().any(|game| game.id == id)) {
                            lobby.picked = games.first().map(|game| game.id);
                        }
                        lobby.games = games;
                    }
                }
                server.closed()
            };
            if gone {
                say(&mut lobby.status, "The server went away".to_owned());
                lobby.browse = false;
                lobby.games.clear();
            } else {
                start.0 = Phase::Browsing(conn);
            }
        }
        // Creating a game on the server: it goes on the list, and players
        // come in through the room.
        (LobbyMode::Hosting, Phase::Browsing(conn)) => {
            conn.borrow_mut().say(&ToServer::Create { name: format!("{}'s game", if lobby.name.is_empty() { "A player" } else { &lobby.name }), map: lobby.map.clone(), max_players: lobby.max_players.clamp(2, 8) as u8, password: lobby.password.clone() });
            say(&mut lobby.status, "The game is on the server's list. Waiting for players.".to_owned());
            lobby.saved = Some((setup.options, false));
            lobby.my_row = 0;
            lobby.names = Default::default();
            start.0 = Phase::Listening(Door::Room(conn, (String::new(), 0, 0)), Vec::new());
        }
        // From the command line: wait for a game to be listed, and take it.
        (LobbyMode::Joining, Phase::Browsing(conn)) if lobby.picked.is_none() && std::env::var_os("RA2_SERVER_JOIN").is_some() => {
            {
                let mut server = conn.borrow_mut();
                server.pump();
                let mut asked = true;
                while let Some(news) = server.news.pop_front() {
                    if let FromServer::Games(games) = news {
                        lobby.picked = games.first().map(|game| game.id);
                        asked = false;
                    }
                }
                if !asked && lobby.picked.is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    server.say(&ToServer::List);
                }
            }
            start.0 = Phase::Browsing(conn);
        }
        (LobbyMode::Joining, Phase::Browsing(conn)) => match lobby.picked {
            Some(id) => {
                conn.borrow_mut().say(&ToServer::Join { id, password: lobby.password.clone() });
                say(&mut lobby.status, "Asking to join...".to_owned());
                start.0 = Phase::Asking(conn);
            }
            None => {
                say(&mut lobby.status, "There is no game to join".to_owned());
                lobby.mode = LobbyMode::Off;
                start.0 = Phase::Browsing(conn);
            }
        },
        (LobbyMode::Joining, Phase::Asking(conn)) => {
            let answer = {
                let mut server = conn.borrow_mut();
                server.pump();
                let mut answer = None;
                while let Some(news) = server.news.pop_front() {
                    match news {
                        FromServer::Joined { .. } => answer = Some(Ok(())),
                        FromServer::Refused { text } => answer = Some(Err(text)),
                        _ => {}
                    }
                }
                if server.closed() { Some(Err("The server went away".to_owned())) } else { answer }
            };
            match answer {
                Some(Ok(())) => {
                    // The game's creator is number 0 in its room.
                    let mut link: Box<dyn Link> = Box::new(PeerLink::new(&conn, 0));
                    link.send(&Msg::Hello { protocol: ra2_net::PROTOCOL, build: env!("CARGO_PKG_VERSION").to_owned(), name: lobby.name.clone() }.encode());
                    say(&mut lobby.status, "In the game. Its creator starts it.".to_owned());
                    lobby.saved = Some((setup.options, true));
                    start.0 = Phase::Joined(link, None, None, Some(conn));
                }
                Some(Err(text)) => {
                    say(&mut lobby.status, format!("Could not join: {text}"));
                    lobby.mode = LobbyMode::Off;
                    start.0 = Phase::Browsing(conn);
                }
                None => start.0 = Phase::Asking(conn),
            }
        }
        (LobbyMode::Off, _) => {
            if let Some((options, whole)) = lobby.saved.take() {
                if whole {
                    setup.options = options;
                } else {
                    setup.options.opponents = options.opponents;
                }
            }
            lobby.my_row = 0;
        }
        (LobbyMode::Hosting, Phase::Idle) if lobby.via_server => {
            say(&mut lobby.status, "Not connected to the server".to_owned());
            lobby.mode = LobbyMode::Off;
        }
        (LobbyMode::Hosting, Phase::Idle) => {
            let port = lobby.port();
            match TcpListener::bind(("0.0.0.0", port)).and_then(|listener| listener.set_nonblocking(true).map(|_| listener)) {
                Ok(listener) => {
                    say(&mut lobby.status, format!("Waiting for players to join on port {port}"));
                    lobby.saved = Some((setup.options, false));
                    lobby.my_row = 0;
                    lobby.names = Default::default();
                    start.0 = Phase::Listening(Door::Port(listener), Vec::new());
                }
                Err(error) => {
                    say(&mut lobby.status, format!("Cannot create a game on port {port}: {error}"));
                    lobby.mode = LobbyMode::Off;
                }
            }
        }
        (LobbyMode::Hosting, Phase::Listening(mut door, mut clients)) => {
            // From the command line there is no map screen: the map is the one
            // the game was started with.
            if auto_host() {
                lobby.max_players = setup.max_players;
                lobby.map = setup.map.clone();
                lobby.mode_rules = setup.mode_rules.clone();
            }
            let room = lobby.max_players.saturating_sub(1).min(7);
            // Somebody new: they take the first row nobody is using, or else
            // the first computer player's.
            let arrived: Option<(Box<dyn Link>, String, Option<u32>)> = match &mut door {
                Door::Port(listener) => listener.accept().ok().and_then(|(stream, from)| {
                    let _ = stream.set_nonblocking(false);
                    Some((Box::new(TcpLink::new(stream).ok()?) as Box<dyn Link>, from.ip().to_string(), None))
                }),
                Door::Room(conn, listed) => {
                    let mut server = conn.borrow_mut();
                    server.pump();
                    let mut arrived = None;
                    while let Some(news) = server.news.pop_front() {
                        if let (FromServer::PeerJoined { peer }, None) = (news, &arrived) {
                            arrived = Some(peer);
                        }
                    }
                    // The server's list is kept to what this lobby shows.
                    let now = (lobby.map.clone(), clients.len() as u8 + 1 + arrived.is_some() as u8, lobby.max_players.clamp(2, 8) as u8);
                    if *listed != now {
                        server.say(&ToServer::Update { map: now.0.clone(), players: now.1, max_players: now.2 });
                        *listed = now;
                    }
                    drop(server);
                    arrived.map(|peer| (Box::new(PeerLink::new(conn, peer)) as Box<dyn Link>, "the Internet".to_owned(), Some(peer)))
                }
            };
            if let Some((mut link, from, peer)) = arrived {
                {
                    let rows = &mut setup.options.opponents;
                    let free = (0..room).find(|&n| rows[n].level < 0).or_else(|| (0..room).find(|&n| rows[n].level != HUMAN_ROW));
                    match free {
                        Some(n) => {
                            rows[n] = crate::game::Opponent { level: HUMAN_ROW, country: -2, colour: -2 };
                            clients.push(Client { row: n + 1, peer, link, sent: None });
                            say(&mut lobby.status, format!("A player has joined from {from} ({} in the game)", clients.len() + 1));
                        }
                        None => link.send(&Msg::Refuse { code: 1, text: "The game is full".into() }.encode()),
                    }
                }
            }
            // What the players ask for, and those who have gone.
            let mut gone = Vec::new();
            for (at, client) in clients.iter_mut().enumerate() {
                while let Some(bytes) = client.link.poll() {
                    let blob = match Msg::decode(&bytes) {
                        Ok(Msg::Lobby { setup: blob }) => blob,
                        // Who they are.
                        Ok(Msg::Hello { name, .. }) => {
                            lobby.names[client.row] = clean_name(&name);
                            client.sent = None;
                            continue;
                        }
                        _ => continue,
                    };
                    let Ok(asked) = bincode::deserialize::<Request>(&blob) else {
                        continue;
                    };
                    // No two
                    // rows may have the same colour. A request for one taken
                    // is passed over; the view sent back says what stands.
                    let options = &mut setup.options;
                    let taken = |colour: i32| colour >= 0 && (options.colour == colour || options.opponents.iter().enumerate().any(|(n, row)| n + 1 != client.row && row.level >= 0 && row.colour == colour));
                    let colour = if (-2..8).contains(&asked.colour) && asked.colour != -1 && !taken(asked.colour) { asked.colour } else { options.opponents[client.row - 1].colour };
                    let country = if asked.country == -2 || (0..9).contains(&asked.country) { asked.country } else { options.opponents[client.row - 1].country };
                    options.opponents[client.row - 1] = crate::game::Opponent { level: HUMAN_ROW, country, colour };
                    client.sent = None;
                }
                if client.link.closed() {
                    gone.push(at);
                }
            }
            for at in gone.into_iter().rev() {
                let client = clients.remove(at);
                setup.options.opponents[client.row - 1] = crate::game::Opponent::NONE;
                lobby.names[client.row].clear();
                say(&mut lobby.status, format!("A player has left ({} in the game)", clients.len() + 1));
            }
            // A map with less room: the last to join make way.
            while clients.iter().any(|client| client.row > room) {
                let at = clients.iter().position(|client| client.row > room).unwrap_or(0);
                let mut client = clients.remove(at);
                client.link.send(&Msg::Refuse { code: 2, text: "The map has no room for you".into() }.encode());
                setup.options.opponents[client.row - 1] = crate::game::Opponent::NONE;
                lobby.names[client.row].clear();
            }
            // Everyone is shown the lobby as it stands.
            if lobby.names[0] != lobby.name {
                lobby.names[0] = lobby.name.clone();
            }
            for client in &mut clients {
                let view = LobbyView { options: setup.options, map: lobby.map.clone(), mode_rules: lobby.mode_rules.clone(), max_players: lobby.max_players, row: client.row, names: lobby.names.clone() };
                if client.sent.as_ref() != Some(&view) {
                    client.link.send(&Msg::Lobby { setup: bincode::serialize(&view).unwrap_or_default() }.encode());
                    client.sent = Some(view);
                }
            }
            let wanted = std::env::var("RA2_HOST_PLAYERS").ok().and_then(|n| n.trim().parse().ok()).unwrap_or(1usize);
            let from_command_line = auto_host();
            let pressed = std::mem::take(&mut lobby.start);
            if pressed && !clients.is_empty() && (!from_command_line || clients.len() >= wanted) {
                // The names go with the houses the rows come to.
                let mut house_names = vec![String::new(), lobby.name.clone()];
                for client in &clients {
                    if let Some(seat) = seat_of_row(&setup.options, lobby.max_players, client.row) {
                        house_names.resize(house_names.len().max(seat as usize + 1), String::new());
                        house_names[seat as usize] = lobby.names[client.row].clone();
                    }
                }
                lobby.house_names = house_names;
                // A game on the server's list leaves it, and the server is told
                // which house each player has: from here it passes the game's
                // packets round itself.
                let room = match &door {
                    Door::Room(conn, _) => {
                        let houses = std::iter::once((0, crate::terrain::PLAYER)).chain(clients.iter().filter_map(|client| Some((client.peer?, seat_of_row(&setup.options, lobby.max_players, client.row)?)))).collect();
                        conn.borrow_mut().say(&ToServer::Started { houses });
                        Some(conn.clone())
                    }
                    Door::Port(_) => None,
                };
                let seats = clients.into_iter().filter_map(|client| Some((seat_of_row(&setup.options, lobby.max_players, client.row)?, client.link))).collect();
                commands.insert_resource(NetSeat(crate::terrain::PLAYER));
                start.0 = Phase::Ready(Waiting::Host(seats, room));
                lobby.starting = true;
                next.set(Screen::InGame);
            } else {
                if pressed && !from_command_line {
                    say(&mut lobby.status, "Nobody has joined yet".to_owned());
                }
                lobby.start = pressed && from_command_line;
                start.0 = Phase::Listening(door, clients);
            }
        }
        (LobbyMode::Joining, Phase::Idle) => {
            let address = if lobby.address.contains(':') { lobby.address.clone() } else { format!("{}:{DEFAULT_PORT}", lobby.address) };
            let stream = connect(&address, DEFAULT_PORT).map(|link| Box::new(link) as Box<dyn Link>);
            match stream {
                Ok(mut link) => {
                    link.send(&Msg::Hello { protocol: ra2_net::PROTOCOL, build: env!("CARGO_PKG_VERSION").to_owned(), name: lobby.name.clone() }.encode());
                    say(&mut lobby.status, format!("Connected to {address}. The host starts the game."));
                    lobby.saved = Some((setup.options, true));
                    start.0 = Phase::Joined(link, None, None, None);
                }
                Err(error) => {
                    say(&mut lobby.status, format!("Cannot join {address}: {error}"));
                    lobby.mode = LobbyMode::Off;
                }
            }
        }
        (LobbyMode::Joining, Phase::Joined(mut link, mut view, mut asked, room)) => {
            let mut over = false;
            while let Some(bytes) = link.poll() {
                match Msg::decode(&bytes) {
                    // The lobby as the host has it: this machine shows the same.
                    Ok(Msg::Lobby { setup: blob }) => {
                        if let Ok(new) = bincode::deserialize::<LobbyView>(&blob) {
                            setup.options = new.options;
                            lobby.my_row = new.row;
                            lobby.map = new.map.clone();
                            lobby.mode_rules = new.mode_rules.clone();
                            lobby.max_players = new.max_players;
                            lobby.names = new.names.clone();
                            let mine = new.options.opponents[new.row.clamp(1, 7) - 1];
                            asked = Some(Request { country: mine.country, colour: mine.colour });
                            view = Some(new);
                        }
                    }
                    // The host has settled the match and says where we sit.
                    Ok(Msg::Start { setup: blob, game_id, internet, speed, max_ahead, frame_send_rate }) => {
                        match bincode::deserialize::<StartBlob>(&blob) {
                            Ok(blob) => {
                                lobby.house_names = blob.house_names;
                                *setup = blob.setup.as_game_setup();
                                commands.insert_resource(NetSeat(blob.seat));
                                start.0 = Phase::Ready(Waiting::Joiner { link, room, internet, game_id, speed, max_ahead, frame_send_rate });
                                lobby.starting = true;
                                next.set(Screen::InGame);
                                return;
                            }
                            Err(error) => {
                                say(&mut lobby.status, format!("The host's game could not be read: {error}"));
                                over = true;
                            }
                        }
                    }
                    Ok(Msg::Refuse { text, .. }) => {
                        say(&mut lobby.status, format!("The host refused: {text}"));
                        over = true;
                    }
                    _ => {}
                }
            }
            if link.closed() && !over {
                say(&mut lobby.status, "The host went away".to_owned());
                over = true;
            }
            if over {
                lobby.mode = LobbyMode::Off;
                return;
            }
            // A change the player made to their own row on this screen is
            // asked of the host, whose answer is what then shows.
            if let Some(view) = &view {
                let mine = setup.options.opponents[view.row.clamp(1, 7) - 1];
                let want = Request { country: mine.country, colour: mine.colour };
                if asked != Some(want) {
                    link.send(&Msg::Lobby { setup: bincode::serialize(&want).unwrap_or_default() }.encode());
                    asked = Some(want);
                }
            }
            start.0 = Phase::Joined(link, view, asked, room);
        }
        (_, phase) => start.0 = phase,
    }
}

/// Once the match is set up on this machine, starts the session.
fn begin(world: &mut bevy::ecs::world::World) {
    let waiting = match std::mem::take(&mut world.non_send_mut::<NetStart>().0) {
        Phase::Ready(waiting) => waiting,
        other => {
            world.non_send_mut::<NetStart>().0 = other;
            return;
        }
    };
    let Some(settled) = world.get_resource::<MatchSetup>().cloned() else {
        world.non_send_mut::<NetStart>().0 = Phase::Ready(waiting);
        return;
    };
    let now = world.resource::<Time<Real>>().elapsed().as_millis() as u64;
    let players = human_houses(&settled);
    let house_names = world.resource::<Lobby>().house_names.clone();
    let game = match waiting {
        Waiting::Host(clients, room) => {
            let speed = settled.options.speed.clamp(0, 6) as u8;
            let game_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |since| since.subsec_nanos());
            let config = Config::new(crate::terrain::PLAYER, &players, if room.is_some() { NetKind::Internet } else { NetKind::Lan }, speed, game_id);
            let mut links = Vec::new();
            for (seat, mut link) in clients {
                let blob = bincode::serialize(&StartBlob { setup: settled.clone(), seat, house_names: house_names.clone() }).unwrap_or_default();
                link.send(&Msg::Start { setup: blob, game_id, internet: room.is_some(), speed, max_ahead: config.max_ahead as u16, frame_send_rate: config.frame_send_rate as u8 }.encode());
                links.push((seat, link));
            }
            match room {
                // Through the server: it passes the packets round, and this
                // machine is one player among the others.
                Some(room) => NetGame { session: Session::new(config, Box::new(RoomLink::new(&room)), now), relay: None },
                // On the local network this machine passes them round.
                None => {
                    let mut relay = Relay::new();
                    for (seat, link) in links {
                        relay.add(seat, link);
                    }
                    let (ours, theirs) = MemoryLink::pair();
                    relay.add(crate::terrain::PLAYER, Box::new(theirs));
                    NetGame { session: Session::new(config, Box::new(ours), now), relay: Some(relay) }
                }
            }
        }
        Waiting::Joiner { link, room, internet, game_id, speed, max_ahead, frame_send_rate } => {
            let seat = world.get_resource::<NetSeat>().map_or(crate::terrain::PLAYER + 1, |seat| seat.0);
            // The Internet's longer time-outs, if the host says so.
            let mut config = Config::new(seat, &players, if internet { NetKind::Internet } else { NetKind::Lan }, speed, game_id);
            config.max_ahead = max_ahead as u32;
            config.frame_send_rate = frame_send_rate as u32;
            let link: Box<dyn Link> = match room {
                Some(room) => Box::new(RoomLink::new(&room)),
                None => link,
            };
            NetGame { session: Session::new(config, link, now), relay: None }
        }
    };
    world.insert_non_send(game);
    world.insert_resource(NetActive);
    // For trying things by hand: which units each player starts with.
    let sim = &world.resource::<Sim>().0;
    for house in players {
        let ids: Vec<u32> = sim.units().iter().filter(|unit| unit.owner == house).map(|unit| unit.id).collect();
        info!("network game started: house {house} has units {ids:?}");
    }
}

/// Hands the player's orders to the session and lets it run the frames due.
fn step(
    net: Option<NonSendMut<NetGame>>,
    mut sim: ResMut<Sim>,
    time: Res<Time<Real>>,
    mut status: MessageWriter<crate::bar::Status>,
    mut said: Local<Option<String>>,
    lobby: Res<Lobby>,
) {
    let who = |house: u8| lobby.house_names.get(house as usize).filter(|name| !name.is_empty()).cloned().unwrap_or_else(|| format!("player {house}"));
    let Some(mut net) = net else {
        return;
    };
    let net = &mut *net;
    if let Some(relay) = net.relay.as_mut() {
        relay.pump();
    }
    for order in std::mem::take(&mut sim.1) {
        if let Ok(bytes) = bincode::serialize(&order) {
            net.session.issue(bytes);
        }
    }
    let now = time.elapsed().as_millis() as u64;
    let tell = |text: String, said: &mut Option<String>, status: &mut MessageWriter<crate::bar::Status>| {
        if said.as_ref() != Some(&text) {
            info!("network: {text}");
            status.write(crate::bar::Status::Text(text.clone()));
            *said = Some(text);
        }
    };
    let Sim(world, _, log) = &mut *sim;
    let result = net.session.step(now, &mut Lockstep(world, log));
    if matches!(result, Step::Run) {
        crate::units::debug_orders(&mut sim);
    }
    let world = &sim.0;
    match result {
        Step::Idle | Step::Run => {}
        Step::Wait { house, dialog, seconds_left } => {
            if dialog {
                tell(format!("Waiting for {} ({seconds_left} s)", who(house)), &mut said, &mut status);
            }
        }
        // "connection lost", and play goes on.
        Step::Dropped(house) => tell(format!("Connection to {} lost!", who(house)), &mut said, &mut status),
        // A dump is written,
        // the player is told, and the game is over on this machine.
        Step::OutOfSync { frame } => {
            let path = format!("desync-{frame}-house{}.txt", crate::terrain::local());
            let lines: Vec<String> = world.state_hashes().into_iter().map(|(name, hash)| format!("{name}: {hash:016x}")).collect();
            let _ = std::fs::write(&path, format!("frame {frame}, tick {}\n{}\n", world.current_tick(), lines.join("\n")));
            tell(format!("Out of sync at frame {frame} (see {path})"), &mut said, &mut status);
        }
        Step::Ended(reason) => tell(format!("The network game has ended ({reason:?})"), &mut said, &mut status),
    }
    if let Some(relay) = net.relay.as_mut() {
        relay.pump();
    }
}
