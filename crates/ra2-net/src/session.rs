//! The lockstep state machine, one per machine. It is the original's network queue routine cut into a non-blocking `step` that the game loop calls once
//! per rendered frame: check value, PROCESS_TIME / TIMING generation, send, wait for the
//! other players, execute the orders due, run the frame.

use std::collections::{BTreeMap, VecDeque};

use crate::Game;
use crate::link::Link;
use crate::msg::{Body, Msg, Order};
use crate::rules::{self, NetKind, TimingInput};

/// How a session starts. Every machine must be given the same values (they travel in
/// `Msg::Start`), except `local`.
#[derive(Clone, Debug)]
pub struct Config {
    /// The house played on this machine.
    pub local: u8,
    /// Every human house in the game, the local one included.
    /// Not original behaviour: a player is identified by its house number alone, and the
    /// relay labels each message with it.
    pub players: Vec<u8>,
    pub kind: NetKind,
    /// Carried in FRAMESYNC; a different
    /// value ends the game.
    pub game_id: u32,
    /// Game speed, 0 (fastest) .. 6.
    pub speed: u8,
    pub max_ahead: u32,
    pub frame_send_rate: u32,
}

impl Config {
    /// The original's starting MaxAhead and FrameSendRate for the speed
    /// (`rules::start_values`).
    pub fn new(local: u8, players: &[u8], kind: NetKind, speed: u8, game_id: u32) -> Config {
        let start = rules::start_values(speed, kind);
        Config {
            local,
            players: players.to_vec(),
            kind,
            game_id,
            speed,
            max_ahead: start.max_ahead,
            frame_send_rate: start.frame_send_rate,
        }
    }
}

/// What one call of [`Session::step`] did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// Not original behaviour (a blocking loop needs no such state): nothing to do this
    /// call, the next frame is not due yet at the published frame rate.
    Idle,
    /// One frame was run now: the orders due were applied and `Game::tick` was called.
    Run,
    /// Stalled on `house`. Keep drawing and taking input. `dialog` is set once the stall has
    /// lasted long enough for the original's "waiting / reconnecting" box, and
    /// `seconds_left` counts down to the drop.
    Wait {
        house: u8,
        dialog: bool,
        seconds_left: u32,
    },
    /// The connection to `house` was given up ("connection lost"). The game goes on; the
    /// house is destroyed MaxAhead frames later on every machine.
    Dropped(u8),
    /// A peer's check value for `frame` differs from ours. The session is over; the caller
    /// writes the dump and tells the player. Returned again on every later call.
    OutOfSync { frame: u32 },
    /// The session is over for another reason. Returned again on every later call.
    Ended(End),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    /// The local player's EXIT ran, or they left from the waiting box.
    Left,
    /// Error 7, a time-out on frame 0.
    TimedOut,
    /// Error 2, a FRAMESYNC from another
    /// game.
    GameIdMismatch,
}

/// Things the user interface will want to show. Drained with [`Session::take_notices`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Notice {
    /// An order arrived after its frame and was skipped ("received too late").
    Late { house: u8, frame: u32 },
    /// A TIMING order ran on `frame`.
    Timing {
        frame: u32,
        fps: u32,
        max_ahead: u32,
        frame_send_rate: u32,
    },
    /// TXT_PLAYER_CHANGED_LATENCY
    LatencyChanged { house: u8, level: u8 },
    /// TXT_PLAYER_CHANGED_SPEED
    SpeedChanged { house: u8, speed: u8 },
    /// TXT_LEFT_GAME: the house's EXIT ran.
    Left { house: u8 },
    /// TXT_CONNECTION_LOST: the house was dropped here.
    ConnectionLost { house: u8 },
    /// `Game::destroy_house` was called on `frame`.
    Destroyed { house: u8, frame: u32 },
}

/// The per-peer entry.
struct Peer {
    house: u8,
    /// Last frame the peer reported (header frame minus its delay). Starts at -1:
    frame: i64,
    /// Orders the peer says it has sent.
    sent: u32,
    /// Orders received from it.
    received: u32,
    /// Not original behaviour: last measured round trip (see `Msg::Ping`).
    round_trip: Option<u64>,
}

/// An entry of the do list.
struct Record {
    frame: u32,
    house: u8,
    body: Body,
}

/// A peer's FRAMEINFO header waiting for its frame to come up.
struct Check {
    frame: u32,
    house: u8,
    value: u32,
    delay: u8,
}

/// Not original behaviour: how often the response time is measured.
const PING_INTERVAL_MS: u64 = 1000;

pub struct Session {
    link: Box<dyn Link>,
    local: u8,
    kind: NetKind,
    game_id: u32,
    /// Connected peers, in house order.
    peers: Vec<Peer>,

    speed: u8,
    max_ahead: u32,
    frame_send_rate: u32,
    fps: u32,
    level: u8,

    /// The out list: orders issued here and not yet sent. The second field forces an
    /// execution frame (tests only).
    out: VecDeque<(Body, Option<u32>)>,
    /// The do list, in arrival order.
    todo: Vec<Record>,
    checks: Vec<Check>,
    history: [u32; rules::HISTORY],
    /// First frame of this session; no compare until `SYNC_GRACE_FRAMES` after it.
    first_frame: Option<u32>,
    /// Running count of orders sent.
    sent: u32,
    /// After a TIMING that
    /// raises MaxAhead or FrameSendRate, orders strictly between these frames are moved to
    /// the second.
    window: Option<(u32, u32)>,

    /// Last PROCESS_TIME of each player (the original keeps it in the player node).
    process: BTreeMap<u8, u16>,
    process_ms: u64,
    process_frames: u32,

    /// This frame's check value is stored and its packet sent; it has not run yet.
    prepared: bool,
    next_due: u64,
    stall_since: Option<u64>,
    last_sync: u64,
    dialog_shown: bool,
    /// (voter, target)
    votes: Vec<(u8, u8)>,
    cooldown_until: u64,
    last_ping: Option<u64>,

    destroyed: Vec<u8>,
    notices: Vec<Notice>,
    pending: VecDeque<Step>,
    over: Option<Step>,
}

impl Session {
    /// `link` leads to the relay (see `relay.rs` for the framing). `now` is the caller's
    /// clock in milliseconds; any origin, but the same one on every later call.
    pub fn new(config: Config, link: Box<dyn Link>, now: u64) -> Session {
        let mut houses: Vec<u8> = config
            .players
            .iter()
            .copied()
            .filter(|h| *h != config.local)
            .collect();
        houses.sort_unstable();
        houses.dedup();
        let peers = houses
            .into_iter()
            .map(|house| Peer {
                house,
                frame: -1,
                sent: 0,
                received: 0,
                round_trip: None,
            })
            .collect();
        let frame_send_rate = config.frame_send_rate.clamp(1, 255);
        // The delay must fit the header's byte.
        let max_ahead = rules::roundup(config.max_ahead.max(1), frame_send_rate)
            .min(255 / frame_send_rate * frame_send_rate);
        Session {
            link,
            local: config.local,
            kind: config.kind,
            game_id: config.game_id,
            peers,
            speed: config.speed.min(6),
            max_ahead,
            frame_send_rate,
            // The original has the host publish these "precalculated" values in a first TIMING at once; here they travel in
            // `Msg::Start` instead.
            fps: rules::start_values(config.speed.min(6), config.kind).fps,
            level: rules::START_LATENCY_LEVEL,
            out: VecDeque::new(),
            todo: Vec::new(),
            checks: Vec::new(),
            history: [0; rules::HISTORY],
            first_frame: None,
            sent: 0,
            window: None,
            process: BTreeMap::new(),
            process_ms: 0,
            process_frames: 0,
            prepared: false,
            next_due: now,
            stall_since: None,
            last_sync: now,
            dialog_shown: false,
            votes: Vec::new(),
            cooldown_until: 0,
            last_ping: None,
            destroyed: Vec::new(),
            notices: Vec::new(),
            pending: VecDeque::new(),
            over: None,
        }
    }

    // ---- what the player does ---------------------------------------------------------

    /// Queue a game order from the local player. It runs on every machine on
    /// roundup(F + MaxAhead, FrameSendRate), F being the frame it is sent on.
    /// Returns false, and the order is
    /// lost, when the out list is full.
    pub fn issue(&mut self, order: Vec<u8>) -> bool {
        self.push(Body::Game(order))
    }

    /// Change the latency level (0..3) for everyone.
    /// Any player may, at most
    /// once per 900 ticks; the timer is shared with `set_game_speed_cooled`. It takes effect
    /// at the host's next TIMING. Returns false if refused.
    pub fn set_latency_level(&mut self, now: u64, level: u8) -> bool {
        self.cooled(now) && self.push(Body::LatencyFudge(level.min(rules::MAX_LATENCY_LEVEL)))
    }

    /// Change the game speed for everyone, as from the options dialog.
    /// Any player, no cool-down.
    pub fn set_game_speed(&mut self, speed: u8) -> bool {
        self.push(Body::GameSpeed(speed.min(6)))
    }

    /// The same from the multiplayer options dialog.
    /// Shares the 900-tick cool-down with
    /// the latency slider.
    pub fn set_game_speed_cooled(&mut self, now: u64, speed: u8) -> bool {
        self.cooled(now) && self.push(Body::GameSpeed(speed.min(6)))
    }

    /// Quit: DESTRUCT then EXIT. The local
    /// house is destroyed and `step` returns `Ended(Left)` when the EXIT runs, MaxAhead
    /// frames from now. Keep calling `step` until then.
    pub fn quit(&mut self) {
        self.out.push_back((Body::Destruct, None));
        self.out.push_back((Body::Exit, None));
    }

    /// Resign but stay: DESTRUCT only.
    pub fn resign(&mut self) {
        self.out.push_back((Body::Destruct, None));
    }

    /// Leave at once without telling anyone, as from the waiting box.
    /// Returns 8 / 9, the game ends
    /// locally; the others find out by timing out.
    pub fn leave_now(&mut self) {
        if self.over.is_none() {
            self.over = Some(Step::Ended(End::Left));
        }
    }

    /// Vote to drop `house` from the waiting box. Only with three or more players; a player
    /// with votes from all but one is dropped at once.
    pub fn vote_drop(&mut self, house: u8) -> bool {
        let known = self.peers.iter().any(|p| p.house == house);
        if !known || !self.dialog_shown || self.stall_since.is_none() || self.peers.len() < 2 {
            return false;
        }
        if !self.votes.contains(&(self.local, house)) {
            self.votes.push((self.local, house));
            self.send(&Msg::Vote { target: house });
        }
        true
    }

    /// Tell the session how long the last frame took to process (simulation and whatever
    /// else the caller counts), in milliseconds. Averaged into PROCESS_TIME. Optional: with
    /// no reports the host assumes everyone manages the speed's full frame rate.
    pub fn note_process_ms(&mut self, ms: u32) {
        self.process_ms += u64::from(ms);
    }

    // ---- state the caller may read ----------------------------------------------------

    pub fn max_ahead(&self) -> u32 {
        self.max_ahead
    }
    pub fn frame_send_rate(&self) -> u32 {
        self.frame_send_rate
    }
    /// The frame rate everybody runs at: the host-published one, not the local speed slider.
    pub fn fps(&self) -> u32 {
        self.fps
    }
    pub fn speed(&self) -> u8 {
        self.speed
    }
    pub fn latency_level(&self) -> u8 {
        self.level
    }
    /// Houses of the peers still connected, in order.
    pub fn peers(&self) -> Vec<u8> {
        self.peers.iter().map(|p| p.house).collect()
    }
    /// The host's only privilege in play
    /// is generating TIMING.
    /// Not original behaviour: the host is the lowest-numbered connected player, so it
    /// passes on by itself when that player leaves.
    pub fn is_host(&self) -> bool {
        self.peers.iter().all(|p| p.house > self.local)
    }
    /// The current frame's packet has gone out but the frame has not run yet (stalled).
    /// An order issued now goes out with the next packet, not this one.
    pub fn waiting(&self) -> bool {
        self.prepared
    }
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// Test hook: queue a game order stamped with a given execution frame instead of the
    /// rule's, to stand in for one that "arrives too late".
    #[doc(hidden)]
    pub fn issue_stamped(&mut self, order: Vec<u8>, frame: u32) {
        self.out.push_back((Body::Game(order), Some(frame)));
    }

    // ---- the frame --------------------------------------------------------------------

    /// Drive the session. Call once per rendered frame with the current time; at most one
    /// frame of the game runs per call.
    pub fn step(&mut self, now: u64, game: &mut dyn Game) -> Step {
        if let Some(step) = self.pending.pop_front() {
            return step;
        }
        if let Some(over) = &self.over {
            return over.clone();
        }
        self.pump(now);
        if let Some(over) = &self.over {
            return over.clone();
        }
        if self.last_ping.is_none_or(|at| now - at >= PING_INTERVAL_MS) && !self.peers.is_empty() {
            self.last_ping = Some(now);
            self.send(&Msg::Ping { stamp: now as u32 });
        }

        let frame = game.frame();
        self.first_frame.get_or_insert(frame);
        if !self.prepared {
            // Frames are paced at the
            // published frame rate.
            if now < self.next_due {
                return Step::Idle;
            }
            self.prepare(game);
            self.prepared = true;
        }

        if let Some(house) = self.blocker(frame) {
            return self.stalled(now, house, game);
        }
        let stalled = self.stall_since.take().is_some();
        if std::mem::take(&mut self.dialog_shown) {
            self.votes.clear();
        }

        if let Some(out_of_sync) = self.execute(now, game) {
            self.over = Some(out_of_sync.clone());
            return out_of_sync;
        }
        if let Some(over) = &self.over {
            // The local EXIT ran: the
            // game is aborted here.
            return over.clone();
        }
        game.tick();
        self.process_frames += 1;
        self.prepared = false;
        // Not original behaviour: the exact pacing arithmetic. A steady cadence, at most
        // one frame per call, and no catching up after a stall.
        let interval = 1000 / u64::from(self.fps.max(1));
        self.next_due = if stalled {
            now + interval
        } else {
            (self.next_due + interval).max(now)
        };
        Step::Run
    }

    /// The top of the original's network queue routine for one frame: check value,
    /// PROCESS_TIME, TIMING, send.
    fn prepare(&mut self, game: &mut dyn Game) {
        let frame = game.frame();
        // Computed once per frame
        // at the top, kept in a ring of 256 indexed by Frame & 0xff.
        let check = game.check_value();
        self.history[frame as usize % rules::HISTORY] = check;

        // Every 128 frames each player queues its average processing
        // time per frame over the window.
        if frame.is_multiple_of(rules::PROCESS_TIME_PERIOD) && self.process_frames > 0 {
            let average = self.process_ms / u64::from(self.process_frames);
            self.process_ms = 0;
            self.process_frames = 0;
            self.out.push_back((
                Body::ProcessTime(average.min(u64::from(u16::MAX)) as u16),
                None,
            ));
        }

        // Only the host, when
        // the low byte of Frame is 0, once more than 4 commands have been sent, and only if
        // every player's processing time is known.
        if self.is_host()
            && frame.is_multiple_of(rules::TIMING_PERIOD)
            && self.sent > rules::TIMING_MIN_SENT
            && let Some(worst_process) = self.worst_process()
        {
            let t = rules::timing(TimingInput {
                worst_process,
                speed: self.speed,
                response: self.response_ticks() as u32,
                level: self.level,
                max_ahead: self.max_ahead,
                frame_send_rate: self.frame_send_rate,
                kind: self.kind,
            });
            self.out.push_back((
                Body::Timing {
                    fps: t.fps,
                    max_ahead: t.max_ahead,
                    frame_send_rate: t.frame_send_rate,
                },
                None,
            ));
        }

        // Packets go out only on
        // multiples of FrameSendRate. Every drained order is stamped
        // roundup(Frame + MaxAhead, FrameSendRate), appended to the local do list and sent
        // under a header with that frame, this frame's check value, the running count and
        // the delay.
        if frame.is_multiple_of(self.frame_send_rate) {
            let stamp = rules::stamp(frame, self.max_ahead, self.frame_send_rate);
            let mut orders = Vec::with_capacity(self.out.len());
            for (body, forced) in self.out.drain(..) {
                let frame = forced.unwrap_or(stamp);
                self.todo.push(Record {
                    frame,
                    house: self.local,
                    body: body.clone(),
                });
                orders.push(Order {
                    frame,
                    house: self.local,
                    body,
                });
            }
            self.sent = self.sent.wrapping_add(orders.len() as u32);
            // The notes give it as MaxAhead, which is the same thing whenever MaxAhead is
            // a multiple of FrameSendRate; receivers subtract it to get the sender's
            // frame.
            let delay = (stamp - frame).min(255) as u8;
            let packet = Msg::Packet {
                frame: stamp,
                check,
                sent: self.sent,
                delay,
                orders,
            };
            self.send(&packet);
        }
    }

    /// The stall rule. `frame` may run
    /// only if every peer's announced orders have arrived and
    /// Frame < lowest peer frame + MaxAhead. Returns the house being waited for.
    fn blocker(&self, frame: u32) -> Option<u8> {
        if let Some(peer) = self.peers.iter().find(|p| p.received < p.sent) {
            return Some(peer.house);
        }
        let laggard = self.laggard()?;
        // Nothing runs until every peer
        // has been heard from (its frame starts at -1).
        if laggard.frame < 0 || i64::from(frame) >= laggard.frame + i64::from(self.max_ahead) {
            return Some(laggard.house);
        }
        None
    }

    /// The peer with the lowest reported frame.
    fn laggard(&self) -> Option<&Peer> {
        self.peers.iter().min_by_key(|p| (p.frame, p.house))
    }

    /// One pass of the original's wait loop, entered while
    /// `blocker` holds.
    fn stalled(&mut self, now: u64, house: u8, game: &mut dyn Game) -> Step {
        let since = match self.stall_since {
            Some(since) => since,
            None => {
                self.stall_since = Some(now);
                self.last_sync = now;
                now
            }
        };
        let elapsed = now - since;
        let frame = game.frame();
        // Resend interval and dialog delay from the table at the start of the game; in
        // game 180 ticks and max(table value, 3 x response time).
        let table = rules::timeouts(self.kind);
        let at_start = frame == 0;
        let (resend, dialog) = if at_start {
            (table.resend, table.start_dialog)
        } else {
            (
                rules::GAME_RESEND_TICKS,
                table.game_dialog.max(3 * self.response_ticks()),
            )
        };
        let hard = rules::ticks_to_ms(table.hard);

        // FRAMESYNC on a timer.
        if now - self.last_sync >= rules::ticks_to_ms(resend) {
            self.last_sync = now;
            self.send(&Msg::FrameSync {
                frame,
                sent: self.sent,
                game_id: self.game_id,
            });
        }

        let dialog = elapsed >= rules::ticks_to_ms(dialog);
        self.dialog_shown |= dialog;

        // A player with votes from all
        // but one is kicked at once; if that is the local player it drops every other
        // connection instead.
        if let Some(target) = self.outvoted() {
            self.stall_since = Some(now);
            if target != self.local {
                self.kick(target, game);
                return Step::Dropped(target);
            }
            for house in self.peers() {
                self.kick(house, game);
                self.pending.push_back(Step::Dropped(house));
            }
            return self.pending.pop_front().unwrap_or(Step::Idle);
        }

        if elapsed >= hard {
            // At frame 0 a time-out
            // always returns error 7 and the game ends.
            if at_start {
                self.over = Some(Step::Ended(End::TimedOut));
                return Step::Ended(End::TimedOut);
            }
            // LAN / Internet: kick the
            // peer with the lowest frame, restart the timers, go on.
            let lowest = self.laggard().map_or(house, |p| p.house);
            self.kick(lowest, game);
            self.stall_since = Some(now);
            return Step::Dropped(lowest);
        }
        Step::Wait {
            house,
            dialog,
            seconds_left: (hard - elapsed).div_ceil(1000) as u32,
        }
    }

    fn outvoted(&self) -> Option<u8> {
        let players = self.peers.len() + 1;
        if players < 3 {
            return None;
        }
        let here = |house: u8| house == self.local || self.peers.iter().any(|p| p.house == house);
        let mut targets = self.peers();
        targets.push(self.local);
        targets.into_iter().find(|target| {
            let votes = self
                .votes
                .iter()
                .filter(|(voter, voted)| voted == target && voter != target && here(*voter))
                .count();
            votes >= players - 1
        })
    }

    /// `Kick_Player_Now`.
    /// "Connection lost", the connection goes, and unless this is frame 0 the local machine
    /// queues REMOVEPLAYER for the house. Every remaining machine does the same, so the
    /// house is destroyed MaxAhead frames later everywhere.
    fn kick(&mut self, house: u8, game: &mut dyn Game) {
        let Some(index) = self.peers.iter().position(|p| p.house == house) else {
            return;
        };
        self.peers.remove(index);
        // Orders of its that are already in the do list still run. Its check values are not
        // compared any more.
        self.checks.retain(|c| c.house != house);
        self.process.remove(&house);
        self.votes.clear();
        self.notices.push(Notice::ConnectionLost { house });
        if game.frame() == 0 {
            self.destroy(house, game);
        } else {
            self.out.push_back((Body::RemovePlayer(house), None));
        }
    }

    /// For the
    /// current frame. Returns `OutOfSync` on a check value mismatch.
    fn execute(&mut self, now: u64, game: &mut dyn Game) -> Option<Step> {
        let frame = game.frame();

        // Orders in
        // flight across a TIMING that raised MaxAhead are moved to the new boundary.
        if let Some((from, to)) = self.window {
            for record in &mut self.todo {
                if record.frame > from && record.frame < to {
                    record.frame = to;
                }
            }
            if frame >= to {
                self.window = None;
            }
        }

        // A peer's header is
        // compared when its frame comes up: our history entry for (frame - delay) must equal
        // its value. Nothing is compared in the first 256 frames.
        let first = self.first_frame.unwrap_or(0);
        let mut differs = None;
        if frame >= first.saturating_add(rules::SYNC_GRACE_FRAMES) {
            for check in self.checks.iter().filter(|c| c.frame == frame) {
                let of = check.frame.wrapping_sub(u32::from(check.delay));
                if self.history[of as usize % rules::HISTORY] != check.value {
                    differs = Some(of);
                    break;
                }
            }
        }
        self.checks.retain(|c| c.frame > frame);
        if let Some(frame) = differs {
            // The game ends on this
            // machine; there is no resync. The dump and the message are the caller's.
            return Some(Step::OutOfSync { frame });
        }

        // House order first, then
        // arrival (= issue) order; executed and old records leave the list.
        let (mut due, rest): (Vec<Record>, Vec<Record>) = std::mem::take(&mut self.todo)
            .into_iter()
            .partition(|r| r.frame <= frame);
        self.todo = rest;
        due.sort_by_key(|r| r.house); // stable: keeps arrival order within a house
        for record in due {
            // "received too late":
            // logged and skipped, never run late.
            if record.frame < frame {
                self.notices.push(Notice::Late {
                    house: record.house,
                    frame: record.frame,
                });
                continue;
            }
            self.handle(now, frame, record, game);
        }
        None
    }

    /// The event handlers the session owns.
    fn handle(&mut self, now: u64, frame: u32, record: Record, game: &mut dyn Game) {
        let house = record.house;
        match record.body {
            Body::Game(order) => game.apply(house, &order),
            // Stored in the sender's
            // player node.
            Body::ProcessTime(value) => {
                if house == self.local || self.peers.iter().any(|p| p.house == house) {
                    self.process.insert(house, value);
                }
            }
            // Sets the frame rate,
            // MaxAhead and FrameSendRate for everybody on the event's frame; a raise opens
            // the move window of 1.3, a shrink is immediate.
            Body::Timing {
                fps,
                max_ahead,
                frame_send_rate,
            } => {
                let rate = u32::from(frame_send_rate).max(1);
                let ahead = u32::from(max_ahead).clamp(1, 255);
                if ahead > self.max_ahead || rate > self.frame_send_rate {
                    self.window = Some((frame, rules::roundup(frame + ahead, rate)));
                }
                self.fps = u32::from(fps).max(1);
                self.max_ahead = ahead;
                self.frame_send_rate = rate;
                self.notices.push(Notice::Timing {
                    frame,
                    fps: self.fps,
                    max_ahead: ahead,
                    frame_send_rate: rate,
                });
            }
            // Sets the level for
            // everyone; the others are told who. Used by the host's next TIMING.
            Body::LatencyFudge(level) => {
                self.level = level.min(rules::MAX_LATENCY_LEVEL);
                if house != self.local {
                    let until = now + rules::ticks_to_ms(rules::CHANGE_COOLDOWN_TICKS);
                    self.cooldown_until = self.cooldown_until.max(until);
                }
                self.notices.push(Notice::LatencyChanged {
                    house,
                    level: self.level,
                });
            }
            // Sets the speed for everyone;
            // the real pace follows the host's next TIMING through the cap.
            Body::GameSpeed(speed) => {
                self.speed = speed.min(6);
                self.notices.push(Notice::SpeedChanged {
                    house,
                    speed: self.speed,
                });
            }
            // FlagToDie on the sender.
            Body::Destruct => self.destroy(house, game),
            // On the sender's
            // machine the game is aborted; elsewhere "has left the game" and the connection
            // goes. The house is not touched by EXIT itself.
            Body::Exit => {
                if house == self.local {
                    self.over = Some(Step::Ended(End::Left));
                } else if let Some(index) = self.peers.iter().position(|p| p.house == house) {
                    self.peers.remove(index);
                    // Not original behaviour: whatever the leaver stamped for frames after
                    // its EXIT is thrown away here, and anything still on the way is ignored
                    // on arrival (it is no longer a peer), so every machine cuts its stream
                    // at the same frame.
                    self.todo.retain(|r| r.house != house);
                    self.checks.retain(|c| c.house != house);
                    self.process.remove(&house);
                    self.votes
                        .retain(|(voter, target)| *voter != house && *target != house);
                    self.notices.push(Notice::Left { house });
                }
            }
            // FlagToDie on the house in
            // the payload.
            Body::RemovePlayer(target) => self.destroy(target, game),
        }
    }

    /// Every remaining machine queues its own REMOVEPLAYER, so the same house comes up
    /// several times on one frame; the game is told once.
    fn destroy(&mut self, house: u8, game: &mut dyn Game) {
        if !self.destroyed.contains(&house) {
            self.destroyed.push(house);
            self.notices.push(Notice::Destroyed {
                house,
                frame: game.frame(),
            });
            game.destroy_house(house);
        }
    }

    // ---- plumbing ---------------------------------------------------------------------

    /// Take in everything that has arrived.
    fn pump(&mut self, now: u64) {
        while let Some(bytes) = self.link.poll() {
            // Relay framing: the sending house, then the message. Anything malformed, or
            // from a house that is not (or no longer) a peer, is ignored.
            let Some((&from, rest)) = bytes.split_first() else {
                continue;
            };
            let Ok(msg) = Msg::decode(rest) else { continue };
            let Some(index) = self.peers.iter().position(|p| p.house == from) else {
                continue;
            };
            match msg {
                Msg::Packet {
                    frame,
                    check,
                    sent,
                    delay,
                    orders,
                } => {
                    // Peer
                    // frame = header frame minus its delay; note what it says it has sent
                    // and count what arrives.
                    let peer = &mut self.peers[index];
                    peer.frame = peer.frame.max(i64::from(frame) - i64::from(delay));
                    peer.sent = sent;
                    peer.received = peer.received.wrapping_add(orders.len() as u32);
                    self.checks.push(Check {
                        frame,
                        house: from,
                        value: check,
                        delay,
                    });
                    for order in orders {
                        // The house is the connection's, whatever the order claims.
                        self.todo.push(Record {
                            frame: order.frame,
                            house: from,
                            body: order.body,
                        });
                    }
                }
                Msg::FrameSync {
                    frame,
                    sent,
                    game_id,
                } => {
                    if game_id != self.game_id {
                        self.over = Some(Step::Ended(End::GameIdMismatch));
                        return;
                    }
                    let peer = &mut self.peers[index];
                    peer.frame = peer.frame.max(i64::from(frame));
                    peer.sent = sent;
                }
                Msg::Ping { stamp } => self.send(&Msg::Pong { to: from, stamp }),
                Msg::Pong { to, stamp } => {
                    if to == self.local {
                        let round_trip = (now as u32).wrapping_sub(stamp);
                        self.peers[index].round_trip = Some(u64::from(round_trip));
                    }
                }
                Msg::Vote { target }
                    if !self.votes.contains(&(from, target)) => {
                        self.votes.push((from, target));
                    }
                // Lobby traffic has no meaning once the game runs.
                _ => {}
            }
        }
    }

    fn send(&mut self, msg: &Msg) {
        self.link.send(&msg.encode());
    }

    fn push(&mut self, body: Body) -> bool {
        if self.out.len() >= rules::OUT_LIST_SIZE {
            return false;
        }
        self.out.push_back((body, None));
        true
    }

    fn cooled(&mut self, now: u64) -> bool {
        if now < self.cooldown_until {
            return false;
        }
        self.cooldown_until = now + rules::ticks_to_ms(rules::CHANGE_COOLDOWN_TICKS);
        true
    }

    /// The worst PROCESS_TIME of the players still here; `None` while any is unknown.
    fn worst_process(&self) -> Option<u32> {
        let mut worst = u32::from(*self.process.get(&self.local)?);
        for peer in &self.peers {
            worst = worst.max(u32::from(*self.process.get(&peer.house)?));
        }
        Some(worst)
    }

    /// Not original behaviour: how it is measured (`Msg::Ping`).
    fn response_ticks(&self) -> u64 {
        let worst = self
            .peers
            .iter()
            .filter_map(|p| p.round_trip)
            .max()
            .unwrap_or(0);
        rules::ms_to_ticks(worst)
    }
}
