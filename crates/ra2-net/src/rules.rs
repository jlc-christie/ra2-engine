//! The original's timing numbers and formulas, kept apart from the state machine so they can
//! be tested on their own. Everything here comes from.

/// The original counts time in 60 Hz system ticks; the session is driven in milliseconds.
pub const TICKS_PER_SECOND: u64 = 60;

/// PROCESS_TIME is queued every 128 frames.
pub const PROCESS_TIME_PERIOD: u32 = 128;
/// The host computes TIMING when the low
/// byte of Frame is 0, i.e. every 256 frames.
pub const TIMING_PERIOD: u32 = 256;
/// ... and only once more than 4 commands
/// have been sent.
pub const TIMING_MIN_SENT: u32 = 4;
/// 256 check values are kept, and
/// nothing is compared in the first 256 frames.
pub const HISTORY: usize = 256;
pub const SYNC_GRACE_FRAMES: u32 = 256;
/// Latency (and the multiplayer
/// dialog's speed slider) may change at most once per 900 ticks.
pub const CHANGE_COOLDOWN_TICKS: u64 = 900;
/// In game the FRAMESYNC resend interval
/// is 180 ticks whatever the table says.
pub const GAME_RESEND_TICKS: u64 = 180;
/// The out list holds 0x80 records and an
/// order issued while it is full is dropped silently.
pub const OUT_LIST_SIZE: usize = 0x80;
/// Latency level 0..3.
pub const MAX_LATENCY_LEVEL: u8 = 3;
/// The level starts at 1.
pub const START_LATENCY_LEVEL: u8 = 1;

pub fn ticks_to_ms(ticks: u64) -> u64 {
    ticks * 1000 / TICKS_PER_SECOND
}

/// Rounded up, so a short but non-zero time is never zero ticks.
pub fn ms_to_ticks(ms: u64) -> u64 {
    (ms * TICKS_PER_SECOND).div_ceil(1000)
}

pub fn roundup(value: u32, multiple: u32) -> u32 {
    let multiple = multiple.max(1);
    value.div_ceil(multiple) * multiple
}

/// With protocol 2 an order
/// drained on frame F is stamped roundup(F + MaxAhead, FrameSendRate).
pub fn stamp(frame: u32, max_ahead: u32, frame_send_rate: u32) -> u32 {
    roundup(frame + max_ahead, frame_send_rate)
}

/// The original's game types 3 and 4. Only the time-outs and the
/// FrameSendRate choice differ between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetKind {
    Lan,
    Internet,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StartValues {
    pub max_ahead: u32,
    pub fps: u32,
    pub frame_send_rate: u32,
}

/// Starting MaxAhead and frame
/// rate by game speed (0 fastest. 6).
///
/// FrameSendRate: the table sets 10 for speeds 0 and 1 and leaves it "unchanged" for
/// the others, where the (Internet quick match) path has 5. Which routine is the LAN
/// one and the LAN initial value are "Not determined", so LAN uses 10 at every speed
/// and Internet uses 10 / 5.
pub fn start_values(speed: u8, kind: NetKind) -> StartValues {
    let (max_ahead, fps) = match speed {
        0 => (40, 60),
        1 => (40, 45),
        2 => (30, 30),
        3 => (20, 20),
        4 => (20, 15),
        5 => (20, 12),
        _ => (10, 10),
    };
    let frame_send_rate = match (kind, speed) {
        (NetKind::Lan, _) | (_, 0 | 1) => 10,
        _ => 5,
    };
    StartValues {
        max_ahead,
        fps,
        frame_send_rate,
    }
}

/// Frame rate cap by game speed:
/// 60, 45, then 60 / speed (30, 20, 15, 12, 10).
pub fn fps_cap(speed: u8) -> u32 {
    match speed {
        0 => 60,
        1 => 45,
        s => 60 / u32::from(s.min(6)),
    }
}

/// Latency level -> response time
/// multiplier (as a fraction) and extra frames: x1 +0, x1.5 +10, x2 +20, x3 +30.
pub fn latency(level: u8) -> (u32, u32, u32) {
    match level {
        0 => (1, 1, 0),
        1 => (3, 2, 10),
        2 => (2, 1, 20),
        _ => (3, 1, 30),
    }
}

/// What the host feeds into the TIMING computation.
#[derive(Clone, Copy, Debug)]
pub struct TimingInput {
    /// The worst PROCESS_TIME reported by any player. The unit is "Not determined" in
    /// the notes, but the formula divides 1000 by it to get frames a second.
    pub worst_process: u32,
    pub speed: u8,
    /// Connection response time.
    pub response: u32,
    pub level: u8,
    pub max_ahead: u32,
    pub frame_send_rate: u32,
    pub kind: NetKind,
}

/// The payload of a TIMING order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timing {
    pub fps: u16,
    pub max_ahead: u16,
    pub frame_send_rate: u8,
}

/// The host's TIMING computation, line for
/// line as in the notes.
pub fn timing(i: TimingInput) -> Timing {
    let mut fps = 1000u32.checked_div(i.worst_process).map_or(60, |fps| fps.max(1));
    fps = fps.min(fps_cap(i.speed));
    let fsr = match i.kind {
        NetKind::Lan => i.frame_send_rate.max(1),
        NetKind::Internet => {
            if fps >= 31 {
                10
            } else {
                5
            }
        }
    };
    let (mul, div, extra) = latency(i.level);
    let resp = i.response * mul / div;
    let mut ahead = resp * fps / 120 + extra;
    // Grow one step at a time.
    if ahead > i.max_ahead && ahead >= i.max_ahead + fsr {
        ahead = i.max_ahead + fsr;
    }
    ahead = roundup(ahead, fsr);
    ahead = ahead.max(3 * fsr);
    ahead = ahead.min(((fsr + 249) / fsr) * fsr);
    Timing {
        fps: fps as u16,
        max_ahead: ahead as u16,
        frame_send_rate: fsr as u8,
    }
}

/// The time-out table, in ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timeouts {
    /// FRAMESYNC resend interval (used at the start; in game it is `GAME_RESEND_TICKS`).
    pub resend: u64,
    /// Stall time before the waiting dialog at the start of the game.
    pub start_dialog: u64,
    /// Stall time before the waiting dialog in game (or 3 x response time if that is more).
    pub game_dialog: u64,
    /// Stall time after which the laggard is dropped.
    pub hard: u64,
}

pub fn timeouts(kind: NetKind) -> Timeouts {
    match kind {
        NetKind::Lan => Timeouts {
            resend: 60,
            start_dialog: 900,
            game_dialog: 420,
            hard: 2400,
        },
        NetKind::Internet => Timeouts {
            resend: 120,
            start_dialog: 1800,
            game_dialog: 720,
            hard: 7200,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_table() {
        let ahead: Vec<u32> = (0..7)
            .map(|s| start_values(s, NetKind::Lan).max_ahead)
            .collect();
        assert_eq!(ahead, [40, 40, 30, 20, 20, 20, 10]);
        let fps: Vec<u32> = (0..7).map(|s| start_values(s, NetKind::Lan).fps).collect();
        assert_eq!(fps, [60, 45, 30, 20, 15, 12, 10]);
        // The starting frame rate is the cap for that speed.
        for s in 0..7 {
            assert_eq!(start_values(s, NetKind::Lan).fps, fps_cap(s));
        }
        assert_eq!(start_values(0, NetKind::Internet).frame_send_rate, 10);
        assert_eq!(start_values(3, NetKind::Internet).frame_send_rate, 5);
        assert_eq!(start_values(3, NetKind::Lan).frame_send_rate, 10);
    }

    #[test]
    fn stamping() {
        assert_eq!(stamp(0, 40, 10), 40);
        assert_eq!(stamp(7, 40, 10), 50);
        assert_eq!(stamp(10, 40, 10), 50);
        assert_eq!(stamp(11, 20, 5), 35);
    }

    #[test]
    fn seconds() {
        let lan = timeouts(NetKind::Lan);
        assert_eq!(ticks_to_ms(lan.game_dialog), 7_000);
        assert_eq!(ticks_to_ms(lan.hard), 40_000);
        assert_eq!(ticks_to_ms(lan.start_dialog), 15_000);
        let net = timeouts(NetKind::Internet);
        assert_eq!(ticks_to_ms(net.game_dialog), 12_000);
        assert_eq!(ticks_to_ms(net.hard), 120_000);
        assert_eq!(ticks_to_ms(net.start_dialog), 30_000);
        assert_eq!(ticks_to_ms(CHANGE_COOLDOWN_TICKS), 15_000);
    }

    fn input() -> TimingInput {
        TimingInput {
            worst_process: 0,
            speed: 0,
            response: 6,
            level: 1,
            max_ahead: 40,
            frame_send_rate: 10,
            kind: NetKind::Lan,
        }
    }

    #[test]
    fn timing_formula() {
        // 6 ticks x1.5 = 9; 9 * 60 / 120 = 4; + 10 = 14; round up 20; minimum 3 x 10.
        assert_eq!(
            timing(input()),
            Timing {
                fps: 60,
                max_ahead: 30,
                frame_send_rate: 10
            }
        );
        // Level 3: 18 * 60 / 120 = 9; + 30 = 39; from 30 that may grow one step, to 40.
        let t = timing(TimingInput {
            level: 3,
            max_ahead: 30,
            ..input()
        });
        assert_eq!(t.max_ahead, 40);
        // A big response time still grows only one FrameSendRate a step.
        let t = timing(TimingInput {
            response: 600,
            max_ahead: 30,
            ..input()
        });
        assert_eq!(t.max_ahead, 40);
        // Shrinking is not limited.
        let t = timing(TimingInput {
            level: 0,
            response: 0,
            max_ahead: 120,
            ..input()
        });
        assert_eq!(t.max_ahead, 30);
        // The top: ((fsr + 249) / fsr) * fsr.
        let t = timing(TimingInput {
            response: 6000,
            max_ahead: 250,
            ..input()
        });
        assert_eq!(t.max_ahead, 250);
        // Frame rate: 1000 / worst, capped by speed, never under 1.
        assert_eq!(
            timing(TimingInput {
                worst_process: 40,
                ..input()
            })
            .fps,
            25
        );
        assert_eq!(
            timing(TimingInput {
                worst_process: 40,
                speed: 4,
                ..input()
            })
            .fps,
            15
        );
        assert_eq!(
            timing(TimingInput {
                worst_process: 5000,
                ..input()
            })
            .fps,
            1
        );
        assert_eq!(
            timing(TimingInput {
                speed: 1,
                ..input()
            })
            .fps,
            45
        );
        // Internet picks the send rate from the frame rate.
        let net = TimingInput {
            kind: NetKind::Internet,
            ..input()
        };
        assert_eq!(timing(net).frame_send_rate, 10);
        let slow = timing(TimingInput { speed: 2, ..net });
        assert_eq!(
            (slow.fps, slow.frame_send_rate, slow.max_ahead),
            (30, 5, 15)
        );
    }
}
