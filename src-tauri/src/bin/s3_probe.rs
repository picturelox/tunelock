// s3-probe — capture the Traktor Kontrol S3 vendor-defined HID event trace.
//
// The S3 exposes its control surface (jog wheels, touch, buttons, faders,
// encoders) as a vendor-defined HID interface, not standard MIDI. This probe
// enumerates Native Instruments HID devices and prints raw input reports with
// timestamps so the protocol can be reverse-engineered against real evidence.
//
// Run with:
//   cargo run --bin s3-probe
//   cargo run --bin s3-probe -- --descriptor-only
//   cargo run --bin s3-probe -- --led-test
//
// Then operate the S3 (move jogs, touch platters, press buttons, move faders)
// while the probe prints reports. Ctrl+C to stop.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Instant;

use hidapi::HidApi;
use tunelock_lib::controller::{build_led_report, S3LedState};

const NI_VID: u16 = 0x17CC;
const S3_PID: u16 = 0x1900;

#[derive(Debug, Default, PartialEq, Eq)]
struct ReportLengths {
    input: BTreeMap<u8, usize>,
    output: BTreeMap<u8, usize>,
    feature: BTreeMap<u8, usize>,
}

/// Extract byte lengths per report ID from a HID report descriptor.
///
/// Lengths include the report-ID byte when IDs are present, matching the
/// buffers accepted/returned by hidapi.
fn report_lengths(descriptor: &[u8]) -> Result<ReportLengths, String> {
    let mut input_bits = BTreeMap::<u8, usize>::new();
    let mut output_bits = BTreeMap::<u8, usize>::new();
    let mut feature_bits = BTreeMap::<u8, usize>::new();
    let mut report_size_bits = 0usize;
    let mut report_count = 0usize;
    let mut report_id = 0u8;
    let mut uses_report_ids = false;
    let mut offset = 0usize;

    while offset < descriptor.len() {
        let prefix = descriptor[offset];
        offset += 1;
        if prefix == 0xFE {
            if offset + 2 > descriptor.len() {
                return Err("truncated HID long item".to_string());
            }
            let size = descriptor[offset] as usize;
            offset += 2; // size + long-item tag
            if offset + size > descriptor.len() {
                return Err("truncated HID long-item payload".to_string());
            }
            offset += size;
            continue;
        }

        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if offset + size > descriptor.len() {
            return Err("truncated HID short-item payload".to_string());
        }
        let mut value = 0u32;
        for byte in 0..size {
            value |= (descriptor[offset + byte] as u32) << (byte * 8);
        }
        offset += size;

        let item_type = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0F;
        match (item_type, tag) {
            (1, 7) => report_size_bits = value as usize,
            (1, 8) => {
                report_id = value as u8;
                uses_report_ids = true;
            }
            (1, 9) => report_count = value as usize,
            (0, 8 | 9 | 11) => {
                let bits = report_size_bits.saturating_mul(report_count);
                let map = match tag {
                    8 => &mut input_bits,
                    9 => &mut output_bits,
                    _ => &mut feature_bits,
                };
                *map.entry(report_id).or_default() += bits;
            }
            _ => {}
        }
    }

    let to_bytes = |reports: BTreeMap<u8, usize>| {
        reports
            .into_iter()
            .map(|(id, bits)| {
                let payload_bytes = bits.saturating_add(7) / 8;
                (id, payload_bytes + usize::from(uses_report_ids))
            })
            .collect()
    };
    Ok(ReportLengths {
        input: to_bytes(input_bits),
        output: to_bytes(output_bits),
        feature: to_bytes(feature_bits),
    })
}

fn main() {
    let api = HidApi::new().expect("failed to initialize hidapi");

    println!("=== Native Instruments HID devices ===");
    let mut found = 0;
    for info in api.device_list() {
        if info.vendor_id() == NI_VID {
            found += 1;
            println!(
                "VID {:04x} PID {:04x} interface {} usage_page {:04x} usage {:04x}",
                info.vendor_id(),
                info.product_id(),
                info.interface_number(),
                info.usage_page(),
                info.usage(),
            );
            println!("  path: {}", info.path().to_string_lossy());
            if let Some(p) = info.product_string() {
                println!("  product: {p}");
            }
            if let Some(m) = info.manufacturer_string() {
                println!("  manufacturer: {m}");
            }
            if let Some(s) = info.serial_number() {
                println!("  serial: {s}");
            }
        }
    }
    if found == 0 {
        eprintln!("No Native Instruments HID devices found. Is the S3 connected and powered?");
        std::process::exit(1);
    }

    println!(
        "\n=== Opening S3 (VID {:04x} PID {:04x}) ===",
        NI_VID, S3_PID
    );
    let device = match api.open(NI_VID, S3_PID) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open S3: {e}");
            std::process::exit(1);
        }
    };

    // Print product info for the opened device.
    if let Ok(Some(p)) = device.get_product_string() {
        println!("opened product: {p}");
    }

    let mut descriptor = [0u8; 4096];
    match device.get_report_descriptor(&mut descriptor) {
        Ok(n) => {
            println!("report descriptor: {n} bytes");
            match report_lengths(&descriptor[..n]) {
                Ok(lengths) => {
                    println!("  input report lengths: {:?}", lengths.input);
                    println!("  output report lengths: {:?}", lengths.output);
                    println!("  feature report lengths: {:?}", lengths.feature);
                }
                Err(error) => eprintln!("could not parse report descriptor: {error}"),
            }
        }
        Err(error) => eprintln!("could not read report descriptor: {error}"),
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--descriptor-only") {
        return;
    }

    if args.iter().any(|arg| arg == "--led-test") {
        let on = build_led_report(&S3LedState {
            play_a: true,
            play_b: true,
            cue_a: true,
            cue_b: true,
            sync_a: true,
            sync_b: true,
        });
        let written = device
            .write(&on)
            .expect("failed to write S3 LED test report");
        assert_eq!(written, on.len(), "short S3 LED test write");
        std::thread::sleep(std::time::Duration::from_millis(750));
        let baseline = build_led_report(&S3LedState::default());
        let written = device
            .write(&baseline)
            .expect("failed to write baseline S3 LED report");
        assert_eq!(written, baseline.len(), "short S3 baseline LED write");
        println!("LED test wrote complete test and baseline {written}-byte reports");
        return;
    }

    println!("\nReading input reports (Ctrl+C to stop)...");
    println!("format: [seconds.millis] <byte count> bytes: <hex>");
    println!();

    let mut buf = [0u8; 512];
    let start = Instant::now();
    let mut report_count: u64 = 0;

    loop {
        match device.read_timeout(&mut buf, 1000) {
            Ok(0) => {
                // Timeout: no report in the last second. Keep waiting.
                let elapsed = start.elapsed();
                if elapsed.as_secs() % 5 == 0 && elapsed.subsec_millis() < 100 {
                    println!(
                        "[heartbeat] {} reports so far, still listening...",
                        report_count
                    );
                    let _ = io::stdout().flush();
                }
            }
            Ok(n) => {
                report_count += 1;
                let elapsed = start.elapsed();
                let hex: Vec<String> = buf[..n].iter().map(|b| format!("{b:02x}")).collect();
                println!(
                    "[{:>6}.{:03}] {:>3} bytes: {}",
                    elapsed.as_secs(),
                    elapsed.subsec_millis(),
                    n,
                    hex.join(" ")
                );
                // Flush so output is visible immediately when piped.
                let _ = io::stdout().flush();
            }
            Err(_) => {
                // Device error. Keep waiting.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_report_id_lengths_including_id_byte() {
        // Vendor usage; ID 1 input = 62 bytes, ID 0x80 output = 82 bytes.
        let descriptor = [
            0x06, 0x01, 0xFF, // Usage Page (vendor)
            0x85, 0x01, // Report ID 1
            0x75, 0x08, // Report Size 8
            0x95, 0x3E, // Report Count 62
            0x81, 0x02, // Input
            0x85, 0x80, // Report ID 0x80
            0x75, 0x08, // Report Size 8
            0x95, 0x52, // Report Count 82
            0x91, 0x02, // Output
        ];
        let lengths = report_lengths(&descriptor).unwrap();
        assert_eq!(lengths.input.get(&0x01), Some(&63));
        assert_eq!(lengths.output.get(&0x80), Some(&83));
    }

    #[test]
    fn rejects_truncated_items() {
        assert!(report_lengths(&[0x76, 0x08]).is_err());
        assert!(report_lengths(&[0xFE, 0x04, 0x01, 0x00]).is_err());
    }
}
