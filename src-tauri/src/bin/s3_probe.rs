// s3-probe — capture the Traktor Kontrol S3 vendor-defined HID event trace.
//
// The S3 exposes its control surface (jog wheels, touch, buttons, faders,
// encoders) as a vendor-defined HID interface, not standard MIDI. This probe
// enumerates Native Instruments HID devices and prints raw input reports with
// timestamps so the protocol can be reverse-engineered against real evidence.
//
// Run with:
//   cargo run --bin s3-probe
//
// Then operate the S3 (move jogs, touch platters, press buttons, move faders)
// while the probe prints reports. Ctrl+C to stop.

use std::io::{self, Write};
use std::time::Instant;

use hidapi::HidApi;

const NI_VID: u16 = 0x17CC;
const S3_PID: u16 = 0x1900;

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

    println!("\n=== Opening S3 (VID {:04x} PID {:04x}) ===", NI_VID, S3_PID);
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
                    println!("[heartbeat] {} reports so far, still listening...", report_count);
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
