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
//! Report layout (63 bytes, 0-indexed, including report ID):
//! - byte 0: report ID (`0x01` buttons/jog, `0x02` faders/knobs).
//! - report `0x01`: buttons at fixed byte/bit offsets and four-byte,
//!   little-endian jog values at `0x0e` (A) / `0x12` (B).
//! - report `0x02`: 16-bit little-endian controls at fixed odd offsets. The
//!   values use the 12-bit range 0..4095 (center detent 2047).
//!
//! The jog wheel's low byte is a distance-tick counter and its upper three
//! bytes are a 400 kHz timecode. The signed tick delta is computed by wrapping
//! subtraction and paired with the wrapping 24-bit time delta.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hidapi::{HidApi, HidDevice};
use serde::{Deserialize, Serialize};

pub const NI_VID: u16 = 0x17CC;
pub const S3_PID: u16 = 0x1900;
pub const REPORT_LEN: usize = 63;
const SHORT_REPORT_ID: u8 = 0x01;
const LONG_REPORT_ID: u8 = 0x02;

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
    Play {
        deck: Deck,
        pressed: bool,
    },
    Cue {
        deck: Deck,
        pressed: bool,
    },
    Sync {
        deck: Deck,
        pressed: bool,
    },
    HotCue {
        deck: Deck,
        slot: u8,
        pressed: bool,
    },
    Touch {
        deck: Deck,
        pressed: bool,
    },
    Jog {
        deck: Deck,
        #[serde(rename = "tickDelta")]
        tick_delta: i8,
        #[serde(rename = "timeDelta")]
        time_delta: u32,
    },
    Control {
        deck: Option<Deck>,
        control: S3Control,
        value: u16,
    },
}

/// Named continuous controls decoded from report `0x02`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum S3Control {
    Tempo,
    Volume,
    Gain,
    EqHigh,
    EqMid,
    EqLow,
    Crossfader,
    HeadphoneMix,
    HeadphoneGain,
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

// Raw button offsets include the report ID at byte 0, matching Mixxx's S3
// mapping and the buffers returned by hidapi.
const PLAY_A: (usize, u8) = (3, 0x01);
const CUE_A: (usize, u8) = (2, 0x80);
const SYNC_A: (usize, u8) = (2, 0x08);
const HOTCUE_A: [(usize, u8); 8] = [
    (3, 0x02),
    (3, 0x04),
    (3, 0x08),
    (3, 0x10),
    (3, 0x20),
    (3, 0x40),
    (3, 0x80),
    (4, 0x01),
];
const PLAY_B: (usize, u8) = (6, 0x02);
const CUE_B: (usize, u8) = (6, 0x01);
const SYNC_B: (usize, u8) = (5, 0x10);
const HOTCUE_B: [(usize, u8); 8] = [
    (6, 0x04),
    (6, 0x08),
    (6, 0x10),
    (6, 0x20),
    (6, 0x40),
    (6, 0x80),
    (7, 0x01),
    (7, 0x02),
];
const TOUCH_A: (usize, u8) = (10, 0x10);
const TOUCH_B: (usize, u8) = (10, 0x20);
const JOG_A_OFFSET: usize = 0x0E;
const JOG_B_OFFSET: usize = 0x12;

// (raw byte offset, deck, semantic control). Multibyte S3 input fields are
// little-endian, as defined by the Mixxx common HID packet parser.
const CONTROLS: &[(usize, Option<Deck>, S3Control)] = &[
    (0x01, Some(Deck::A), S3Control::Tempo),
    (0x0D, Some(Deck::B), S3Control::Tempo),
    (0x05, Some(Deck::A), S3Control::Volume),
    (0x07, Some(Deck::B), S3Control::Volume),
    (0x11, Some(Deck::A), S3Control::Gain),
    (0x13, Some(Deck::B), S3Control::Gain),
    (0x25, Some(Deck::A), S3Control::EqHigh),
    (0x27, Some(Deck::A), S3Control::EqMid),
    (0x29, Some(Deck::A), S3Control::EqLow),
    (0x2B, Some(Deck::B), S3Control::EqHigh),
    (0x2D, Some(Deck::B), S3Control::EqMid),
    (0x2F, Some(Deck::B), S3Control::EqLow),
    (0x0B, None, S3Control::Crossfader),
    (0x1D, None, S3Control::HeadphoneMix),
    (0x1B, None, S3Control::HeadphoneGain),
];

/// A parsed S3 input report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct S3Report {
    /// Report ID (`0x01` or `0x02`).
    pub report_id: u8,
    /// Complete report bytes, including the report ID at offset 0.
    pub data: [u8; REPORT_LEN],
}

impl Default for S3Report {
    fn default() -> Self {
        Self {
            report_id: 0,
            data: [0; REPORT_LEN],
        }
    }
}

/// Parse a 63-byte S3 input report. Returns `None` if the buffer is short.
pub fn parse_report(data: &[u8]) -> Option<S3Report> {
    if data.len() < REPORT_LEN {
        return None;
    }

    let mut report = [0u8; REPORT_LEN];
    report.copy_from_slice(&data[..REPORT_LEN]);

    Some(S3Report {
        report_id: data[0],
        data: report,
    })
}

/// Signed delta between two 8-bit jog tick counters (wrapping subtraction).
/// Positive = forward, negative = reverse.
#[inline]
pub fn jog_delta(previous: u8, current: u8) -> i8 {
    current.wrapping_sub(previous) as i8
}

/// Wrapping delta for the S3's 24-bit, 400 kHz jog timecode.
#[inline]
pub fn jog_time_delta(previous: u32, current: u32) -> u32 {
    current.wrapping_sub(previous) & 0x00FF_FFFF
}

fn button(report: &S3Report, position: (usize, u8)) -> bool {
    report.data[position.0] & position.1 != 0
}

fn read_u16_le(report: &S3Report, offset: usize) -> u16 {
    u16::from_le_bytes([report.data[offset], report.data[offset + 1]])
}

fn read_jog(report: &S3Report, offset: usize) -> (u8, u32) {
    let tick = report.data[offset];
    let time = u32::from_le_bytes([
        report.data[offset + 1],
        report.data[offset + 2],
        report.data[offset + 3],
        0,
    ]);
    (tick, time)
}

/// Decode the semantic actions that changed between two consecutive reports.
///
/// Button presses are edge-triggered (0 -> 1 = pressed, 1 -> 0 = released);
/// jog and continuous-control changes are level-diffed.
pub fn decode_actions(prev: &S3Report, curr: &S3Report) -> Vec<S3Action> {
    let mut actions = Vec::new();

    // Short and long reports are independent state streams. Comparing across
    // report IDs creates false button and continuous-control changes.
    if prev.report_id != curr.report_id {
        return actions;
    }

    if curr.report_id == LONG_REPORT_ID {
        for &(offset, deck, control) in CONTROLS {
            let previous = read_u16_le(prev, offset);
            let current = read_u16_le(curr, offset);
            if previous != current {
                actions.push(S3Action::Control {
                    deck,
                    control,
                    value: current,
                });
            }
        }
        return actions;
    }

    if curr.report_id != SHORT_REPORT_ID {
        return actions;
    }

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
        let was = button(prev, (idx, mask));
        let now = button(curr, (idx, mask));
        if was != now {
            actions.push(kind.action(deck, now));
        }
    }

    // Hot cues (eight pads per physical deck).
    for (deck, pads) in [(Deck::A, &HOTCUE_A), (Deck::B, &HOTCUE_B)] {
        for (slot, &position) in pads.iter().enumerate() {
            let was = button(prev, position);
            let now = button(curr, position);
            if was != now {
                actions.push(S3Action::HotCue {
                    deck,
                    slot: slot as u8 + 1,
                    pressed: now,
                });
            }
        }
    }

    // Platter touch.
    for (deck, position) in [(Deck::A, TOUCH_A), (Deck::B, TOUCH_B)] {
        let was = button(prev, position);
        let now = button(curr, position);
        if was != now {
            actions.push(S3Action::Touch { deck, pressed: now });
        }
    }

    // Jog distance plus device time, used together to derive platter velocity.
    for (deck, offset) in [(Deck::A, JOG_A_OFFSET), (Deck::B, JOG_B_OFFSET)] {
        let (previous_tick, previous_time) = read_jog(prev, offset);
        let (current_tick, current_time) = read_jog(curr, offset);
        let tick_delta = jog_delta(previous_tick, current_tick);
        if tick_delta != 0 {
            actions.push(S3Action::Jog {
                deck,
                tick_delta,
                // Very small deltas yield unstable velocity. Mixxx documents
                // 500 device ticks as a safe lower bound for this hardware.
                time_delta: jog_time_delta(previous_time, current_time).max(500),
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
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x01,
        0xc0, 0x1b, 0xf7, 0x00, 0x00, 0x00, 0x00, 0x07, 0xe5, 0x03, 0xeb, 0x07, 0xc1, 0x0a, 0xff,
        0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xea, 0x09, 0x3b, 0x07, 0xff, 0x07, 0xff, 0x07,
        0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0xff, 0x07, 0x6c,
        0x04, 0xfd, 0x06,
    ];

    #[test]
    fn parses_report_id_and_raw_offsets() {
        let r = parse_report(&JOG_REPORT).expect("63-byte report must parse");
        assert_eq!(r.report_id, 0x01);
        assert_eq!(r.data[PLAY_A.0], JOG_REPORT[PLAY_A.0]);
        assert_eq!(read_jog(&r, JOG_A_OFFSET), (0x01, 0xF71BC0));
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
        assert_eq!(jog_time_delta(0xFF_FF00, 0x00_0100), 0x200);
    }

    fn report_with_button(byte: usize, mask: u8) -> S3Report {
        let mut r = parse_report(&JOG_REPORT).unwrap();
        r.data[byte] |= mask;
        r
    }

    #[test]
    fn decodes_deck_a_button_edges() {
        let baseline = parse_report(&JOG_REPORT).unwrap();

        let play = report_with_button(PLAY_A.0, PLAY_A.1);
        assert!(decode_actions(&baseline, &play).contains(&S3Action::Play {
            deck: Deck::A,
            pressed: true
        }));
        assert!(decode_actions(&play, &baseline).contains(&S3Action::Play {
            deck: Deck::A,
            pressed: false
        }));

        let cue = report_with_button(CUE_A.0, CUE_A.1);
        assert!(decode_actions(&baseline, &cue).contains(&S3Action::Cue {
            deck: Deck::A,
            pressed: true
        }));

        let sync = report_with_button(SYNC_A.0, SYNC_A.1);
        assert!(decode_actions(&baseline, &sync).contains(&S3Action::Sync {
            deck: Deck::A,
            pressed: true
        }));
    }

    #[test]
    fn decodes_deck_b_button_edges() {
        let baseline = parse_report(&JOG_REPORT).unwrap();

        let play = report_with_button(PLAY_B.0, PLAY_B.1);
        assert!(decode_actions(&baseline, &play).contains(&S3Action::Play {
            deck: Deck::B,
            pressed: true
        }));

        let cue = report_with_button(CUE_B.0, CUE_B.1);
        assert!(decode_actions(&baseline, &cue).contains(&S3Action::Cue {
            deck: Deck::B,
            pressed: true
        }));

        let sync = report_with_button(SYNC_B.0, SYNC_B.1);
        assert!(decode_actions(&baseline, &sync).contains(&S3Action::Sync {
            deck: Deck::B,
            pressed: true
        }));
    }

    #[test]
    fn decodes_hot_cue_and_touch() {
        let baseline = parse_report(&JOG_REPORT).unwrap();

        let hot1 = report_with_button(HOTCUE_A[0].0, HOTCUE_A[0].1);
        assert!(
            decode_actions(&baseline, &hot1).contains(&S3Action::HotCue {
                deck: Deck::A,
                slot: 1,
                pressed: true
            })
        );

        let hot8_b = report_with_button(HOTCUE_B[7].0, HOTCUE_B[7].1);
        assert!(
            decode_actions(&baseline, &hot8_b).contains(&S3Action::HotCue {
                deck: Deck::B,
                slot: 8,
                pressed: true
            })
        );

        let touch = report_with_button(TOUCH_A.0, TOUCH_A.1);
        assert!(
            decode_actions(&baseline, &touch).contains(&S3Action::Touch {
                deck: Deck::A,
                pressed: true
            })
        );

        let touch_b = report_with_button(TOUCH_B.0, TOUCH_B.1);
        assert!(
            decode_actions(&baseline, &touch_b).contains(&S3Action::Touch {
                deck: Deck::B,
                pressed: true
            })
        );
    }

    #[test]
    fn decodes_jog_distance_and_device_time() {
        let baseline = parse_report(&JOG_REPORT).unwrap();
        let mut moved = baseline;
        let (tick, time) = read_jog(&baseline, JOG_A_OFFSET);
        moved.data[JOG_A_OFFSET] = tick.wrapping_add(30);
        let next_time = time + 1_000;
        moved.data[JOG_A_OFFSET + 1..JOG_A_OFFSET + 4]
            .copy_from_slice(&next_time.to_le_bytes()[..3]);

        let actions = decode_actions(&baseline, &moved);
        assert!(actions.contains(&S3Action::Jog {
            deck: Deck::A,
            tick_delta: 30,
            time_delta: 1_000,
        }));
    }

    #[test]
    fn decodes_named_long_report_controls_as_little_endian() {
        let mut before = S3Report {
            report_id: LONG_REPORT_ID,
            data: [0; REPORT_LEN],
        };
        before.data[0] = LONG_REPORT_ID;
        let mut after = before;
        after.data[0x05..0x07].copy_from_slice(&2047u16.to_le_bytes());
        after.data[0x2D..0x2F].copy_from_slice(&4095u16.to_le_bytes());

        let actions = decode_actions(&before, &after);
        assert!(actions.contains(&S3Action::Control {
            deck: Some(Deck::A),
            control: S3Control::Volume,
            value: 2047,
        }));
        assert!(actions.contains(&S3Action::Control {
            deck: Some(Deck::B),
            control: S3Control::EqMid,
            value: 4095,
        }));
    }

    #[test]
    fn does_not_compare_interleaved_report_types() {
        let short = parse_report(&JOG_REPORT).unwrap();
        let mut long = short;
        long.report_id = LONG_REPORT_ID;
        long.data[0] = LONG_REPORT_ID;
        assert!(decode_actions(&short, &long).is_empty());
    }

    #[test]
    fn serializes_jog_contract_for_typescript() {
        let json = serde_json::to_value(S3Action::Jog {
            deck: Deck::B,
            tick_delta: -7,
            time_delta: 2_500,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "jog",
                "deck": "b",
                "tickDelta": -7,
                "timeDelta": 2_500,
            })
        );
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
