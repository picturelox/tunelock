//! Traktor Kontrol S3 HID adapter (TL-07).
//!
//! The S3 exposes its control surface as a vendor-defined HID interface, not
//! standard MIDI. Discovery evidence (captured 2026-09-07):
//!
//! - VID `0x17CC`, PID `0x1900`, interface 3, usage page `0xff01`.
//! - Product "Traktor Kontrol S3", manufacturer "Native Instruments".
//! - Input reports are 63 bytes, sent continuously at ~140 Hz while the jog
//!   is moving and on state changes.
//!
//! Report layout (63 bytes, 0-indexed):
//! - byte 0: report ID (`0x01` continuous/jog, `0x02` button/state).
//! - bytes 1..21: buttons + jog + touch (bitmask area; exact button map is
//!   still being reverse-engineered).
//! - bytes 22..62: faders/knobs, 20 x 16-bit **big-endian** values in the
//!   12-bit range 0..4095 (center detent `0x07ff` = 2047).
//!
//! The jog wheel is a 4-byte value: byte 15 (Deck A) / byte 19 (Deck B) is a
//! 1-byte distance-tick counter, followed by a 3-byte timecode. The signed
//! per-report tick delta is computed by wrapping subtraction.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hidapi::{HidApi, HidDevice};
use serde::{Deserialize, Serialize};

pub const NI_VID: u16 = 0x17CC;
pub const S3_PID: u16 = 0x1900;
pub const REPORT_LEN: usize = 63;

/// A deck on the S3 (A = left, B = right).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Deck {
    A,
    B,
}

/// A semantic control action decoded from an S3 input report.
///
/// This is the hardware-neutral vocabulary that the S3 adapter emits; the
/// shared action model (TL-07) maps these onto the same session commands used
/// by mouse and keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum S3Action {
    Play { deck: Deck, pressed: bool },
    Cue { deck: Deck, pressed: bool },
    Sync { deck: Deck, pressed: bool },
    HotCue { deck: Deck, slot: u8, pressed: bool },
    Touch { deck: Deck, pressed: bool },
    Jog { deck: Deck, delta: i8 },
    Fader { index: u8, value: u16 },
}

/// Connection status emitted by the S3 reader thread.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Status {
    pub connected: bool,
    pub error: Option<String>,
}

/// Guards against spawning more than one S3 reader thread.
pub static S3_READER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Try to claim the single S3 reader slot. Returns `true` if this caller won.
pub fn claim_s3_reader() -> bool {
    !S3_READER_RUNNING.swap(true, Ordering::AcqRel)
}

/// Release the S3 reader slot.
pub fn release_s3_reader() {
    S3_READER_RUNNING.store(false, Ordering::Release);
}

// ── LED output (reverse-engineered from the Mixxx S3 mapping) ──────────────
//
// The S3 uses two output reports: 0x80 (button/state LEDs) and 0x81 (VU
// meters). Each LED is a single byte. Palette LEDs encode `color + brightness`
// where color is one of 18 values (0x00..0x44 in steps of 0x04) and brightness
// is 0..3. Single-color "basic" LEDs use 0x20 (off) / 0x77 (on).

/// Output report ID for button/state LEDs.
const LED_REPORT_ID: u8 = 0x80;
/// Output report length (report ID + 82 data bytes), matching Mixxx outputA.
const LED_REPORT_LEN: usize = 83;

/// Palette color base values (low 6 bits; brightness occupies the low 2 bits).
const COLOR_CARROT: u8 = 0x08;
/// Brightness levels added to a palette color.
const LED_DIM: u8 = 1;
const LED_BRIGHT: u8 = 3;
/// Single-color LED on/off values.
const BASIC_ON: u8 = 0x77;
const BASIC_OFF: u8 = 0x20;

// Byte offsets in the output report (report ID is byte 0). These match the
// Mixxx `defineOutput` offsets, which are already 1-based report indices.
const LED_PLAY_A: usize = 0x11;
const LED_PLAY_B: usize = 0x2A;
const LED_CUE_A: usize = 0x10;
const LED_CUE_B: usize = 0x29;
const LED_SYNC_A: usize = 0x0C;
const LED_SYNC_B: usize = 0x25;

/// Button backlight state mirrored onto the S3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3LedState {
    pub play_a: bool,
    pub play_b: bool,
    pub cue_a: bool,
    pub cue_b: bool,
    pub sync_a: bool,
    pub sync_b: bool,
}

impl Default for S3LedState {
    fn default() -> Self {
        Self {
            play_a: false,
            play_b: false,
            cue_a: false,
            cue_b: false,
            sync_a: false,
            sync_b: false,
        }
    }
}

/// Build the 0x80 output report for the given LED state.
pub fn build_led_report(state: &S3LedState) -> [u8; LED_REPORT_LEN] {
    let mut report = [0u8; LED_REPORT_LEN];
    report[0] = LED_REPORT_ID;

    // Play is a single-color LED.
    report[LED_PLAY_A] = if state.play_a { BASIC_ON } else { BASIC_OFF };
    report[LED_PLAY_B] = if state.play_b { BASIC_ON } else { BASIC_OFF };

    // Cue and Sync are palette LEDs (deck base color + brightness).
    report[LED_CUE_A] = COLOR_CARROT + if state.cue_a { LED_BRIGHT } else { LED_DIM };
    report[LED_CUE_B] = COLOR_CARROT + if state.cue_b { LED_BRIGHT } else { LED_DIM };
    report[LED_SYNC_A] = COLOR_CARROT + if state.sync_a { LED_BRIGHT } else { LED_DIM };
    report[LED_SYNC_B] = COLOR_CARROT + if state.sync_b { LED_BRIGHT } else { LED_DIM };

    report
}

/// Write a button backlight state to the S3, opening a short-lived handle.
pub fn write_leds(state: &S3LedState) -> Result<(), String> {
    let device = S3Device::open()?;
    device.write(&build_led_report(state))
}

// Button bit positions (byte index, bit mask) reverse-engineered from a
// controlled one-button-at-a-time capture on 2026-09-07.
const PLAY_A: (usize, u8) = (3, 0x01);
const CUE_A: (usize, u8) = (2, 0x80);
const SYNC_A: (usize, u8) = (2, 0x08);
const HOTCUE_A: [(usize, u8); 2] = [(3, 0x02), (3, 0x04)];
const PLAY_B: (usize, u8) = (6, 0x02);
const CUE_B: (usize, u8) = (6, 0x01);
const SYNC_B: (usize, u8) = (5, 0x10);
const TOUCH_A: (usize, u8) = (10, 0x10);

/// A parsed S3 input report.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct S3Report {
    /// Report ID (`0x01` or `0x02`).
    pub report_id: u8,
    /// Deck A jog distance-tick counter (byte 15).
    pub jog_a: u8,
    /// Deck B jog distance-tick counter (byte 19).
    pub jog_b: u8,
    /// Fader/knob values (bytes 22..62, 20 x 16-bit big-endian).
    pub faders: [u16; 20],
    /// Raw button/touch area (bytes 1..21) for protocol mapping.
    pub buttons: [u8; 21],
}

/// Parse a 63-byte S3 input report. Returns `None` if the buffer is short.
pub fn parse_report(data: &[u8]) -> Option<S3Report> {
    if data.len() < REPORT_LEN {
        return None;
    }

    let mut faders = [0u16; 20];
    for (i, slot) in faders.iter_mut().enumerate() {
        let idx = 22 + i * 2;
        *slot = u16::from_be_bytes([data[idx], data[idx + 1]]);
    }

    let mut buttons = [0u8; 21];
    buttons.copy_from_slice(&data[1..22]);

    Some(S3Report {
        report_id: data[0],
        jog_a: data[15],
        jog_b: data[19],
        faders,
        buttons,
    })
}

/// Signed delta between two 8-bit jog tick counters (wrapping subtraction).
/// Positive = forward, negative = reverse.
#[inline]
pub fn jog_delta(previous: u8, current: u8) -> i8 {
    current.wrapping_sub(previous) as i8
}

/// Decode the semantic actions that changed between two consecutive reports.
///
/// Button presses are edge-triggered (0 -> 1 = pressed, 1 -> 0 = released);
/// jog and fader changes are level-diffed.
pub fn decode_actions(prev: &S3Report, curr: &S3Report) -> Vec<S3Action> {
    let mut actions = Vec::new();

    // Button edge detection.
    let buttons: &[((usize, u8), Deck, S3ActionKind)] = &[
        (PLAY_A, Deck::A, S3ActionKind::Play),
        (CUE_A, Deck::A, S3ActionKind::Cue),
        (SYNC_A, Deck::A, S3ActionKind::Sync),
        (PLAY_B, Deck::B, S3ActionKind::Play),
        (CUE_B, Deck::B, S3ActionKind::Cue),
        (SYNC_B, Deck::B, S3ActionKind::Sync),
    ];
    for &((idx, mask), deck, kind) in buttons {
        let was = prev.buttons[idx] & mask != 0;
        let now = curr.buttons[idx] & mask != 0;
        if was != now {
            actions.push(kind.action(deck, now));
        }
    }

    // Hot cues (Deck A slots 1..2).
    for (slot, &(idx, mask)) in HOTCUE_A.iter().enumerate() {
        let was = prev.buttons[idx] & mask != 0;
        let now = curr.buttons[idx] & mask != 0;
        if was != now {
            actions.push(S3Action::HotCue {
                deck: Deck::A,
                slot: slot as u8 + 1,
                pressed: now,
            });
        }
    }

    // Touch (Deck A platter).
    let was_touch = prev.buttons[TOUCH_A.0] & TOUCH_A.1 != 0;
    let now_touch = curr.buttons[TOUCH_A.0] & TOUCH_A.1 != 0;
    if was_touch != now_touch {
        actions.push(S3Action::Touch {
            deck: Deck::A,
            pressed: now_touch,
        });
    }

    // Jog deltas.
    let da = jog_delta(prev.jog_a, curr.jog_a);
    if da != 0 {
        actions.push(S3Action::Jog {
            deck: Deck::A,
            delta: da,
        });
    }
    let db = jog_delta(prev.jog_b, curr.jog_b);
    if db != 0 {
        actions.push(S3Action::Jog {
            deck: Deck::B,
            delta: db,
        });
    }

    // Fader/knob changes.
    for (i, (&p, &c)) in prev.faders.iter().zip(curr.faders.iter()).enumerate() {
        if p != c {
            actions.push(S3Action::Fader {
                index: i as u8,
                value: c,
            });
        }
    }

    actions
}

/// Internal button kind used to build the semantic action table.
#[derive(Clone, Copy)]
enum S3ActionKind {
    Play,
    Cue,
    Sync,
}

impl S3ActionKind {
    fn action(self, deck: Deck, pressed: bool) -> S3Action {
        match self {
            S3ActionKind::Play => S3Action::Play { deck, pressed },
            S3ActionKind::Cue => S3Action::Cue { deck, pressed },
            S3ActionKind::Sync => S3Action::Sync { deck, pressed },
        }
    }
}

/// An opened S3 HID device. Reads input reports on the calling thread.
pub struct S3Device {
    device: HidDevice,
}

impl S3Device {
    /// Open the first Traktor Kontrol S3 HID interface.
    pub fn open() -> Result<Self, String> {
        let api = HidApi::new().map_err(|e| e.to_string())?;
        let device = api
            .open(NI_VID, S3_PID)
            .map_err(|e| format!("failed to open S3 HID device: {e}"))?;
        Ok(Self { device })
    }

    /// Block for up to `timeout` waiting for the next input report.
    /// Returns `Ok(None)` on timeout and `Ok(Some(bytes))` on a report.
    pub fn read_timeout(&self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let mut buf = [0u8; REPORT_LEN];
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        match self.device.read_timeout(&mut buf, ms) {
            Ok(0) => Ok(None),
            Ok(n) => Ok(Some(buf[..n].to_vec())),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Write an output report (e.g. LED backlight state) to the device.
    ///
    /// The caller supplies a complete HID output report, including report ID.
    pub fn write(&self, report: &[u8]) -> Result<(), String> {
        self.device
            .write(report)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from the S3 while Deck A jog was moving (2026-09-07).
    const JOG_REPORT: [u8; 63] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00,
        0x01, 0xc0, 0x1b, 0xf7, 0x00, 0x00, 0x00, 0x00, 0x07, 0xe5, 0x03, 0xeb, 0x07, 0xc1,
        0x0a, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xea, 0x09, 0x3b, 0x07, 0xff,
        0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff,
        0x07, 0xff, 0x07, 0x6c, 0x04, 0xfd, 0x06,
    ];

    #[test]
    fn parses_report_id_and_faders_big_endian() {
        let r = parse_report(&JOG_REPORT).expect("63-byte report must parse");
        assert_eq!(r.report_id, 0x01);
        // Faders are 16-bit big-endian, 12-bit range.
        assert_eq!(r.faders[0], 0x07e5);
        assert_eq!(r.faders[1], 0x03eb);
        assert_eq!(r.faders[2], 0x07c1);
        assert_eq!(r.faders[3], 0x0aff);
        assert_eq!(r.faders[4], 0x07ff); // center detent
        assert_eq!(r.faders[5], 0x07ff);
        assert_eq!(r.faders[6], 0x07ff);
        assert_eq!(r.faders[7], 0x07ea);
        assert_eq!(r.faders[8], 0x093b);
    }

    #[test]
    fn rejects_short_reports() {
        assert!(parse_report(&[0u8; 10]).is_none());
    }

    #[test]
    fn jog_delta_wraps_and_signs() {
        assert_eq!(jog_delta(100, 200), 100);
        assert_eq!(jog_delta(200, 100), -100);
        // Wrap-around: 255 -> 0 is +1 forward.
        assert_eq!(jog_delta(255, 0), 1);
        // 0 -> 255 is -1 reverse.
        assert_eq!(jog_delta(0, 255), -1);
    }

    fn report_with_button(byte: usize, mask: u8) -> S3Report {
        let mut r = parse_report(&JOG_REPORT).unwrap();
        r.buttons[byte] |= mask;
        r
    }

    #[test]
    fn decodes_deck_a_button_edges() {
        let baseline = parse_report(&JOG_REPORT).unwrap();

        let play = report_with_button(PLAY_A.0, PLAY_A.1);
        assert!(decode_actions(&baseline, &play)
            .contains(&S3Action::Play { deck: Deck::A, pressed: true }));
        assert!(decode_actions(&play, &baseline)
            .contains(&S3Action::Play { deck: Deck::A, pressed: false }));

        let cue = report_with_button(CUE_A.0, CUE_A.1);
        assert!(decode_actions(&baseline, &cue)
            .contains(&S3Action::Cue { deck: Deck::A, pressed: true }));

        let sync = report_with_button(SYNC_A.0, SYNC_A.1);
        assert!(decode_actions(&baseline, &sync)
            .contains(&S3Action::Sync { deck: Deck::A, pressed: true }));
    }

    #[test]
    fn decodes_deck_b_button_edges() {
        let baseline = parse_report(&JOG_REPORT).unwrap();

        let play = report_with_button(PLAY_B.0, PLAY_B.1);
        assert!(decode_actions(&baseline, &play)
            .contains(&S3Action::Play { deck: Deck::B, pressed: true }));

        let cue = report_with_button(CUE_B.0, CUE_B.1);
        assert!(decode_actions(&baseline, &cue)
            .contains(&S3Action::Cue { deck: Deck::B, pressed: true }));

        let sync = report_with_button(SYNC_B.0, SYNC_B.1);
        assert!(decode_actions(&baseline, &sync)
            .contains(&S3Action::Sync { deck: Deck::B, pressed: true }));
    }

    #[test]
    fn decodes_hot_cue_and_touch() {
        let baseline = parse_report(&JOG_REPORT).unwrap();

        let hot1 = report_with_button(HOTCUE_A[0].0, HOTCUE_A[0].1);
        assert!(decode_actions(&baseline, &hot1)
            .contains(&S3Action::HotCue { deck: Deck::A, slot: 1, pressed: true }));

        let hot2 = report_with_button(HOTCUE_A[1].0, HOTCUE_A[1].1);
        assert!(decode_actions(&baseline, &hot2)
            .contains(&S3Action::HotCue { deck: Deck::A, slot: 2, pressed: true }));

        let touch = report_with_button(TOUCH_A.0, TOUCH_A.1);
        assert!(decode_actions(&baseline, &touch)
            .contains(&S3Action::Touch { deck: Deck::A, pressed: true }));
    }

    #[test]
    fn decodes_jog_and_fader_deltas() {
        let baseline = parse_report(&JOG_REPORT).unwrap();
        let mut moved = baseline;
        moved.jog_a = baseline.jog_a.wrapping_add(30);
        moved.faders[0] = 0x0800;

        let actions = decode_actions(&baseline, &moved);
        assert!(actions.contains(&S3Action::Jog { deck: Deck::A, delta: 30 }));
        assert!(actions.contains(&S3Action::Fader { index: 0, value: 0x0800 }));
    }

    #[test]
    fn builds_led_report_with_expected_offsets_and_values() {
        let report = build_led_report(&S3LedState {
            play_a: true,
            play_b: false,
            cue_a: true,
            cue_b: false,
            sync_a: false,
            sync_b: true,
        });

        assert_eq!(report[0], 0x80, "report ID");
        // Play is a single-color LED: on = 0x77, off = 0x20.
        assert_eq!(report[0x11], 0x77, "play A on");
        assert_eq!(report[0x2A], 0x20, "play B off");
        // Cue/Sync are palette LEDs: CARROT (0x08) + bright (3) / dim (1).
        assert_eq!(report[0x10], 0x08 + 3, "cue A on");
        assert_eq!(report[0x29], 0x08 + 1, "cue B off");
        assert_eq!(report[0x0C], 0x08 + 1, "sync A off");
        assert_eq!(report[0x25], 0x08 + 3, "sync B on");
    }
}
