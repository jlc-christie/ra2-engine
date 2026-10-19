//! Not original behaviour: a meeting place on the Internet. A small server
//! keeps the list of games waiting for players and passes messages between
//! the players of each, so nobody needs a reachable address of their own. It
//! knows nothing of the game: what the players say to each other is bytes to
//! it. (The original used Westwood Online, which is gone.)
//!
//! A connection first lists, creates or joins. Once in a game's room, each
//! player has a number there (the one who created it is 0) and sends bytes to
//! another by that number.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::link::Link;

/// A game waiting for players, as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInfo {
    pub id: u32,
    pub name: String,
    pub map: String,
    pub players: u8,
    pub max_players: u8,
    pub locked: bool,
}

/// What a player's program says to the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToServer {
    List,
    Create { name: String, map: String, max_players: u8, password: String },
    /// The game's creator: what the list should now say.
    Update { map: String, players: u8, max_players: u8 },
    /// The game's creator: it has begun, and leaves the list. From now on the
    /// server passes the game's packets round itself (`All`), putting in front
    /// of each the house of whoever sent it, as given here by room number.
    Started { houses: Vec<(u32, u8)> },
    Join { id: u32, password: String },
    /// Bytes for another player of the room.
    To { peer: u32, bytes: Vec<u8> },
    /// A started game's packet, for every other player.
    All { bytes: Vec<u8> },
}

/// What the server says back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromServer {
    Games(Vec<GameInfo>),
    Created { id: u32 },
    /// In the room, with this number.
    Joined { peer: u32 },
    Refused { text: String },
    /// To the creator: a player has come into the room, or gone.
    PeerJoined { peer: u32 },
    PeerLeft { peer: u32 },
    /// The creator has gone and the room with them.
    Closed,
    From { peer: u32, bytes: Vec<u8> },
    /// A started game's packet from another player: one byte, the house that
    /// sent it, then the packet (the framing of `Relay`).
    Game { bytes: Vec<u8> },
}

struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    fn text(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
}

struct R<'a>(&'a [u8]);
impl R<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (first, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(*first)
    }
    fn u32(&mut self) -> Option<u32> {
        let (head, rest) = self.0.split_at_checked(4)?;
        self.0 = rest;
        Some(u32::from_le_bytes(head.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let length = self.u32()? as usize;
        let (head, rest) = self.0.split_at_checked(length)?;
        self.0 = rest;
        Some(head.to_vec())
    }
    fn text(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }
}

impl ToServer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            ToServer::List => w.u8(0),
            ToServer::Create { name, map, max_players, password } => {
                w.u8(1);
                w.text(name);
                w.text(map);
                w.u8(*max_players);
                w.text(password);
            }
            ToServer::Update { map, players, max_players } => {
                w.u8(2);
                w.text(map);
                w.u8(*players);
                w.u8(*max_players);
            }
            ToServer::Started { houses } => {
                w.u8(3);
                w.u32(houses.len() as u32);
                for (peer, house) in houses {
                    w.u32(*peer);
                    w.u8(*house);
                }
            }
            ToServer::Join { id, password } => {
                w.u8(4);
                w.u32(*id);
                w.text(password);
            }
            ToServer::To { peer, bytes } => {
                w.u8(5);
                w.u32(*peer);
                w.bytes(bytes);
            }
            ToServer::All { bytes } => {
                w.u8(6);
                w.bytes(bytes);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Option<ToServer> {
        let mut r = R(bytes);
        Some(match r.u8()? {
            0 => ToServer::List,
            1 => ToServer::Create { name: r.text()?, map: r.text()?, max_players: r.u8()?, password: r.text()? },
            2 => ToServer::Update { map: r.text()?, players: r.u8()?, max_players: r.u8()? },
            3 => {
                let count = r.u32()?.min(64);
                let mut houses = Vec::new();
                for _ in 0..count {
                    houses.push((r.u32()?, r.u8()?));
                }
                ToServer::Started { houses }
            }
            4 => ToServer::Join { id: r.u32()?, password: r.text()? },
            5 => ToServer::To { peer: r.u32()?, bytes: r.bytes()? },
            6 => ToServer::All { bytes: r.bytes()? },
            _ => return None,
        })
    }
}

impl FromServer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            FromServer::Games(games) => {
                w.u8(0);
                w.u32(games.len() as u32);
                for game in games {
                    w.u32(game.id);
                    w.text(&game.name);
                    w.text(&game.map);
                    w.u8(game.players);
                    w.u8(game.max_players);
                    w.u8(game.locked as u8);
                }
            }
            FromServer::Created { id } => {
                w.u8(1);
                w.u32(*id);
            }
            FromServer::Joined { peer } => {
                w.u8(2);
                w.u32(*peer);
            }
            FromServer::Refused { text } => {
                w.u8(3);
                w.text(text);
            }
            FromServer::PeerJoined { peer } => {
                w.u8(4);
                w.u32(*peer);
            }
            FromServer::PeerLeft { peer } => {
                w.u8(5);
                w.u32(*peer);
            }
            FromServer::Closed => w.u8(6),
            FromServer::From { peer, bytes } => {
                w.u8(7);
                w.u32(*peer);
                w.bytes(bytes);
            }
            FromServer::Game { bytes } => {
                w.u8(8);
                w.bytes(bytes);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Option<FromServer> {
        let mut r = R(bytes);
        Some(match r.u8()? {
            0 => {
                let count = r.u32()?.min(10_000);
                let mut games = Vec::new();
                for _ in 0..count {
                    games.push(GameInfo { id: r.u32()?, name: r.text()?, map: r.text()?, players: r.u8()?, max_players: r.u8()?, locked: r.u8()? != 0 });
                }
                FromServer::Games(games)
            }
            1 => FromServer::Created { id: r.u32()? },
            2 => FromServer::Joined { peer: r.u32()? },
            3 => FromServer::Refused { text: r.text()? },
            4 => FromServer::PeerJoined { peer: r.u32()? },
            5 => FromServer::PeerLeft { peer: r.u32()? },
            6 => FromServer::Closed,
            7 => FromServer::From { peer: r.u32()?, bytes: r.bytes()? },
            8 => FromServer::Game { bytes: r.bytes()? },
            _ => return None,
        })
    }
}

// ------------------------------------------------------------------ the server

struct Room {
    info: GameInfo,
    password: String,
    /// Connection ids by their number in the room; the creator is 0.
    members: Vec<Option<usize>>,
    started: bool,
    /// Once started: each member's house, by room number.
    houses: HashMap<u32, u8>,
}

struct Conn {
    link: Box<dyn Link>,
    /// The room it is in, and its number there.
    room: Option<(u32, u32)>,
}

/// The server's whole state. `add` each new connection and call `pump` often.
#[derive(Default)]
pub struct Directory {
    conns: HashMap<usize, Conn>,
    rooms: HashMap<u32, Room>,
    next_conn: usize,
    next_room: u32,
}

/// Names and such are cut to this many bytes.
const MAX_TEXT: usize = 48;

fn tidy(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect()
}

impl Directory {
    pub fn new() -> Directory {
        Directory::default()
    }

    pub fn add(&mut self, link: Box<dyn Link>) {
        self.conns.insert(self.next_conn, Conn { link, room: None });
        self.next_conn += 1;
    }

    /// How many connections and how many games are open.
    pub fn counts(&self) -> (usize, usize) {
        (self.conns.len(), self.rooms.len())
    }

    fn list(&self) -> Vec<GameInfo> {
        let mut games: Vec<GameInfo> = self.rooms.values().filter(|room| !room.started).map(|room| room.info.clone()).collect();
        games.sort_by_key(|game| game.id);
        games
    }

    fn send(&mut self, conn: usize, message: FromServer) {
        if let Some(conn) = self.conns.get_mut(&conn) {
            conn.link.send(&message.encode());
        }
    }

    /// Deals with everything that has arrived. Returns how many messages.
    pub fn pump(&mut self) -> usize {
        let mut handled = 0;
        let mut ids: Vec<usize> = self.conns.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            while let Some(bytes) = self.conns.get_mut(&id).and_then(|conn| conn.link.poll()) {
                handled += 1;
                if let Some(message) = ToServer::decode(&bytes) {
                    self.handle(id, message);
                }
            }
            if self.conns.get(&id).is_some_and(|conn| conn.link.closed()) {
                self.leave(id);
                self.conns.remove(&id);
            }
        }
        handled
    }

    fn handle(&mut self, id: usize, message: ToServer) {
        let room_of = self.conns.get(&id).and_then(|conn| conn.room);
        match (message, room_of) {
            (ToServer::List, _) => {
                let games = self.list();
                self.send(id, FromServer::Games(games));
            }
            (ToServer::Create { name, map, max_players, password }, None) => {
                self.next_room += 1;
                let room = self.next_room;
                let info = GameInfo { id: room, name: tidy(&name), map: tidy(&map), players: 1, max_players: max_players.clamp(2, 8), locked: !password.is_empty() };
                self.rooms.insert(room, Room { info, password, members: vec![Some(id)], started: false, houses: HashMap::new() });
                if let Some(conn) = self.conns.get_mut(&id) {
                    conn.room = Some((room, 0));
                }
                self.send(id, FromServer::Created { id: room });
            }
            (ToServer::Update { map, players, max_players }, Some((room, 0))) => {
                if let Some(room) = self.rooms.get_mut(&room) {
                    room.info.map = tidy(&map);
                    room.info.players = players;
                    room.info.max_players = max_players.clamp(2, 8);
                }
            }
            (ToServer::Started { houses }, Some((room, 0))) => {
                if let Some(room) = self.rooms.get_mut(&room) {
                    room.started = true;
                    room.houses = houses.into_iter().collect();
                }
            }
            // A started game's packet goes to every other player, with the
            // sender's house in front. The server puts it there, from what
            // the room was started with: a player cannot speak as another.
            (ToServer::All { bytes }, Some((room, from))) => {
                let Some((house, targets)) = self.rooms.get(&room).filter(|room| room.started).and_then(|room| {
                    let house = *room.houses.get(&from)?;
                    let targets: Vec<usize> = room.members.iter().enumerate().filter(|(peer, _)| *peer as u32 != from).filter_map(|(_, member)| *member).collect();
                    Some((house, targets))
                }) else {
                    return;
                };
                let mut framed = Vec::with_capacity(bytes.len() + 1);
                framed.push(house);
                framed.extend_from_slice(&bytes);
                for target in targets {
                    self.send(target, FromServer::Game { bytes: framed.clone() });
                }
            }
            (ToServer::Join { id: wanted, password }, None) => {
                let answer = match self.rooms.get_mut(&wanted) {
                    None => Err("That game is no longer there"),
                    Some(room) if room.started => Err("That game has started"),
                    Some(room) if room.password != password => Err("Wrong password"),
                    Some(room) if room.members.iter().flatten().count() >= room.info.max_players as usize => Err("That game is full"),
                    Some(room) => {
                        room.members.push(Some(id));
                        Ok((room.members.len() as u32 - 1, room.members[0]))
                    }
                };
                match answer {
                    Ok((peer, creator)) => {
                        if let Some(conn) = self.conns.get_mut(&id) {
                            conn.room = Some((wanted, peer));
                        }
                        self.send(id, FromServer::Joined { peer });
                        if let Some(creator) = creator {
                            self.send(creator, FromServer::PeerJoined { peer });
                        }
                    }
                    Err(text) => self.send(id, FromServer::Refused { text: text.to_owned() }),
                }
            }
            (ToServer::To { peer, bytes }, Some((room, from))) => {
                let target = self.rooms.get(&room).and_then(|room| room.members.get(peer as usize).copied().flatten());
                if let Some(target) = target {
                    self.send(target, FromServer::From { peer: from, bytes });
                }
            }
            // Anything out of place is passed over.
            _ => {}
        }
    }

    /// A connection has gone: its room is told, or closed if it made it.
    fn leave(&mut self, id: usize) {
        let Some((room_id, peer)) = self.conns.get(&id).and_then(|conn| conn.room) else {
            return;
        };
        // A game that has started no longer depends on whoever made it: they
        // leave like anyone, and the room goes when the last player does.
        let started = self.rooms.get(&room_id).is_some_and(|room| room.started);
        if peer == 0 && !started {
            let members = self.rooms.remove(&room_id).map(|room| room.members).unwrap_or_default();
            for member in members.into_iter().flatten().filter(|member| *member != id) {
                self.send(member, FromServer::Closed);
                if let Some(conn) = self.conns.get_mut(&member) {
                    conn.room = None;
                }
            }
        } else if let Some(room) = self.rooms.get_mut(&room_id) {
            room.members[peer as usize] = None;
            // Every other player hears of it: in play there is nobody else
            // to say so.
            let others: Vec<usize> = room.members.iter().flatten().copied().collect();
            if others.is_empty() {
                self.rooms.remove(&room_id);
            }
            for member in others {
                self.send(member, FromServer::PeerLeft { peer });
            }
        }
    }
}

// ------------------------------------------------------- a player's connection

/// A player's one connection to the server, shared by the links to each other
/// player of the room.
pub struct ServerConn {
    link: Box<dyn Link>,
    inbox: HashMap<u32, VecDeque<Vec<u8>>>,
    /// A started game's packets, each with its sender's house in front.
    game: VecDeque<Vec<u8>>,
    /// What the server said that was not from another player.
    pub news: VecDeque<FromServer>,
    gone: Vec<u32>,
    closed: bool,
}

impl ServerConn {
    pub fn new(link: Box<dyn Link>) -> Rc<RefCell<ServerConn>> {
        Rc::new(RefCell::new(ServerConn { link, inbox: HashMap::new(), game: VecDeque::new(), news: VecDeque::new(), gone: Vec::new(), closed: false }))
    }

    pub fn say(&mut self, message: &ToServer) {
        self.link.send(&message.encode());
    }

    /// Sorts what has arrived: other players' bytes by who sent them, the rest
    /// into `news`.
    pub fn pump(&mut self) {
        while let Some(bytes) = self.link.poll() {
            match FromServer::decode(&bytes) {
                Some(FromServer::From { peer, bytes }) => self.inbox.entry(peer).or_default().push_back(bytes),
                Some(FromServer::Game { bytes }) => self.game.push_back(bytes),
                Some(other) => {
                    match &other {
                        FromServer::PeerLeft { peer } => self.gone.push(*peer),
                        FromServer::Closed => self.closed = true,
                        _ => {}
                    }
                    self.news.push_back(other);
                }
                None => {}
            }
        }
        if self.link.closed() {
            self.closed = true;
        }
    }

    pub fn closed(&self) -> bool {
        self.closed
    }
}

/// The link to one other player of the room, through the server.
pub struct PeerLink {
    conn: Rc<RefCell<ServerConn>>,
    peer: u32,
}

impl PeerLink {
    pub fn new(conn: &Rc<RefCell<ServerConn>>, peer: u32) -> PeerLink {
        PeerLink { conn: conn.clone(), peer }
    }
}

impl Link for PeerLink {
    fn send(&mut self, bytes: &[u8]) {
        self.conn.borrow_mut().say(&ToServer::To { peer: self.peer, bytes: bytes.to_vec() });
    }

    fn poll(&mut self) -> Option<Vec<u8>> {
        let mut conn = self.conn.borrow_mut();
        conn.pump();
        conn.inbox.get_mut(&self.peer).and_then(VecDeque::pop_front)
    }

    fn closed(&self) -> bool {
        let conn = self.conn.borrow();
        conn.closed || conn.gone.contains(&self.peer)
    }
}

/// A player's link to a started game: what they send goes to every other
/// player, and what arrives has its sender's house in front, exactly as with
/// a `Relay`. A `Session` runs over it unchanged.
pub struct RoomLink {
    conn: Rc<RefCell<ServerConn>>,
}

impl RoomLink {
    pub fn new(conn: &Rc<RefCell<ServerConn>>) -> RoomLink {
        RoomLink { conn: conn.clone() }
    }
}

impl Link for RoomLink {
    fn send(&mut self, bytes: &[u8]) {
        self.conn.borrow_mut().say(&ToServer::All { bytes: bytes.to_vec() });
    }

    fn poll(&mut self) -> Option<Vec<u8>> {
        let mut conn = self.conn.borrow_mut();
        conn.pump();
        conn.game.pop_front()
    }

    // Another player going is not this link closing: the game's own rules
    // deal with a player who stops being heard from.
    fn closed(&self) -> bool {
        self.conn.borrow().link.closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::MemoryLink;

    fn join(directory: &mut Directory) -> Rc<RefCell<ServerConn>> {
        let (ours, theirs) = MemoryLink::pair();
        directory.add(Box::new(theirs));
        ServerConn::new(Box::new(ours))
    }

    fn news(directory: &mut Directory, conn: &Rc<RefCell<ServerConn>>) -> Vec<FromServer> {
        directory.pump();
        let mut conn = conn.borrow_mut();
        conn.pump();
        conn.news.drain(..).collect()
    }

    #[test]
    fn messages_round_trip() {
        let to = [ToServer::List, ToServer::Create { name: "a".into(), map: "m".into(), max_players: 4, password: "p".into() }, ToServer::Update { map: "m2".into(), players: 2, max_players: 4 }, ToServer::Started { houses: vec![(0, 1), (2, 3)] }, ToServer::Join { id: 7, password: String::new() }, ToServer::To { peer: 3, bytes: vec![1, 2, 3] }, ToServer::All { bytes: vec![4, 5] }];
        for message in to {
            assert_eq!(ToServer::decode(&message.encode()), Some(message));
        }
        let from = [FromServer::Games(vec![GameInfo { id: 1, name: "g".into(), map: "m".into(), players: 1, max_players: 4, locked: true }]), FromServer::Created { id: 9 }, FromServer::Joined { peer: 2 }, FromServer::Refused { text: "no".into() }, FromServer::PeerJoined { peer: 1 }, FromServer::PeerLeft { peer: 1 }, FromServer::Closed, FromServer::From { peer: 0, bytes: vec![9] }, FromServer::Game { bytes: vec![1, 9] }];
        for message in from {
            assert_eq!(FromServer::decode(&message.encode()), Some(message));
        }
        assert_eq!(ToServer::decode(&[5, 1]), None);
    }

    #[test]
    fn games_are_listed_joined_with_a_password_and_talk_through_the_server() {
        let mut directory = Directory::new();
        let (host, guest, other) = (join(&mut directory), join(&mut directory), join(&mut directory));
        host.borrow_mut().say(&ToServer::Create { name: "James's game".into(), map: "Anytown".into(), max_players: 2, password: "secret".into() });
        let id = match news(&mut directory, &host)[..] {
            [FromServer::Created { id }] => id,
            ref other => panic!("{other:?}"),
        };
        guest.borrow_mut().say(&ToServer::List);
        assert_eq!(news(&mut directory, &guest), [FromServer::Games(vec![GameInfo { id, name: "James's game".into(), map: "Anytown".into(), players: 1, max_players: 2, locked: true }])]);
        // The wrong password is refused; the right one lets them in.
        guest.borrow_mut().say(&ToServer::Join { id, password: "guess".into() });
        assert_eq!(news(&mut directory, &guest), [FromServer::Refused { text: "Wrong password".into() }]);
        guest.borrow_mut().say(&ToServer::Join { id, password: "secret".into() });
        assert_eq!(news(&mut directory, &guest), [FromServer::Joined { peer: 1 }]);
        assert_eq!(news(&mut directory, &host), [FromServer::PeerJoined { peer: 1 }]);
        // A full game takes nobody else.
        other.borrow_mut().say(&ToServer::Join { id, password: "secret".into() });
        assert_eq!(news(&mut directory, &other), [FromServer::Refused { text: "That game is full".into() }]);
        // The two talk through the server, each by the other's number.
        let (mut to_guest, mut to_host) = (PeerLink::new(&host, 1), PeerLink::new(&guest, 0));
        to_guest.send(b"hello");
        to_host.send(b"hi");
        directory.pump();
        assert_eq!((to_host.poll(), to_guest.poll()), (Some(b"hello".to_vec()), Some(b"hi".to_vec())));
        // Once started it leaves the list, and the server passes the game's
        // packets round with the sender's house in front.
        host.borrow_mut().say(&ToServer::Started { houses: vec![(0, 1), (1, 3)] });
        directory.pump();
        other.borrow_mut().say(&ToServer::List);
        assert_eq!(news(&mut directory, &other), [FromServer::Games(Vec::new())]);
        let (mut host_game, mut guest_game) = (RoomLink::new(&host), RoomLink::new(&guest));
        host_game.send(b"from the host");
        guest_game.send(b"from the guest");
        directory.pump();
        assert_eq!(guest_game.poll(), Some([&[1u8][..], b"from the host"].concat()));
        assert_eq!(host_game.poll(), Some([&[3u8][..], b"from the guest"].concat()));
        // The game does not end with its creator's connection: the room
        // stays for whoever is left, and goes with the last of them.
        drop(to_guest);
        drop(host_game);
        drop(host);
        assert_eq!(news(&mut directory, &guest), [FromServer::PeerLeft { peer: 0 }]);
        assert!(!guest_game.closed());
        assert_eq!(directory.counts().1, 1);
        drop(to_host);
        drop(guest_game);
        drop(guest);
        directory.pump();
        assert_eq!(directory.counts().1, 0);
    }
}
