//! Command-line diagnostic: lists visible devices and the safety verdict for
//! each. Writes nothing, anywhere.

use salvage_app::device::DeviceEnumerator;
use salvage_app::safety::{evaluate, SafetyPolicy, SafetyVerdict};
use salvage_win32::WindowsDeviceEnumerator;

fn main() {
    println!("Visible devices (read-only, nothing is modified)\n");

    let devices = match WindowsDeviceEnumerator::new().enumerate() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("enumeration failed: {e}");
            std::process::exit(1);
        }
    };

    if devices.is_empty() {
        println!("No accessible device. Run as Administrator.");
        return;
    }

    let policy = SafetyPolicy::default();
    for d in &devices {
        println!("{}", d.path);
        println!("  model       : {}", d.display_name());
        println!("  bus         : {}", d.bus_type.as_str());
        println!("  removable   : {}", if d.removable_media { "yes" } else { "no" });
        println!(
            "  capacity    : {:.2} GB ({} sectors of {} B)",
            d.capacity_bytes() as f64 / 1e9,
            d.geometry.total_sectors(),
            d.geometry.sector_size()
        );
        let letters: Vec<String> = d
            .volumes
            .iter()
            .map(|v| v.drive_letter.map_or("?".into(), |c| format!("{c}:")))
            .collect();
        println!(
            "  volumes     : {}",
            if letters.is_empty() { "nenhum".into() } else { letters.join(" ") }
        );

        match evaluate(d, &policy) {
            SafetyVerdict::Allowed => println!("  SEGURANCA   : LIBERADO"),
            SafetyVerdict::NeedsConfirmation { warnings } => {
                println!("  SEGURANCA   : EXIGE CONFIRMACAO");
                for w in warnings {
                    println!("      - {}", salvage_win32::text::warning_message(&w));
                }
            }
            SafetyVerdict::Blocked { reasons } => {
                println!("  SEGURANCA   : BLOQUEADO");
                for r in reasons {
                    println!("      - {}", salvage_win32::text::block_message(&r));
                }
            }
        }
        println!();
    }
}
