//! Full inspection from the command line.
//!
//! Runs the same scan the graphical interface does, subject to the same safety
//! guards and the same named consent. It does **not** apply any partitioning:
//! it ends by printing the diagnosis and the layouts that would be possible.
//!
//! Usage: `salvage-inspect <\\.\PhysicalDriveN> "<exact device name>"`

use std::io::Write;
use std::time::Instant;

use salvage_app::device::DeviceEnumerator;
use salvage_app::safety::{evaluate, DestructiveConsent, SafetyPolicy, SafetyVerdict};
use salvage_app::scan::{
    CancellationToken, ScanConfig, ScanObserver, ScanPhase, ScanProgress, Scanner,
};
use salvage_core::format;
use salvage_core::health::{diagnose, Assurance, FailureScenario};
use salvage_core::planner::{plan_layouts, FileSystem, PartitionRole, PlanningPolicy};
use salvage_core::sector_map::{SectorMap, SectorState};
use salvage_win32::{Access, RawBlockDevice, WindowsDeviceEnumerator};

/// Imprime o andamento sem inundar o terminal.
struct ConsoleObserver {
    started: Instant,
    phase_started: Instant,
    phase: Option<ScanPhase>,
    phase_base: u64,
    last_percent: i64,
}

impl ScanObserver for ConsoleObserver {
    fn on_progress(&mut self, p: &ScanProgress, _map: &SectorMap) {
        if self.phase != Some(p.phase) {
            if p.phase != ScanPhase::Refining {
                let name = match p.phase {
                    ScanPhase::Writing => "WRITING pattern (back to front)",
                    ScanPhase::Verifying => "VERIFYING reads (front to back)",
                    ScanPhase::Refining => "REFINING",
                };
                println!("\n--- {name} ---");
                self.phase = Some(p.phase);
                self.phase_started = Instant::now();
                self.phase_base = p.sectors_done;
                self.last_percent = -1;
            }
            return;
        }

        let pct = (p.fraction() * 100.0) as i64;
        if pct == self.last_percent {
            return;
        }
        self.last_percent = pct;

        let elapsed = self.phase_started.elapsed().as_secs_f64();
        let advanced = p.sectors_done.saturating_sub(self.phase_base);
        let (speed, eta) = if elapsed > 3.0 && advanced > 0 {
            let per_sec = advanced as f64 / elapsed;
            let remaining = p.sectors_total.saturating_sub(p.sectors_done) as f64;
            (
                format!("{}/s", format::bytes((per_sec * 512.0) as u64)),
                format::duration(remaining / per_sec),
            )
        } else {
            ("--".into(), "--".into())
        };

        print!("\r  {pct:3}% | {} | {eta} left | defects: {}    ", speed, p.defects_found);
        let _ = std::io::stdout().flush();
    }

    fn on_defect(&mut self, range: salvage_core::LbaRange, state: SectorState) {
        println!(
            "\n  ! {:?} defect at LBA {}..{} ({} sectors)",
            state,
            range.start(),
            range.end(),
            range.len()
        );
    }
}

/// Runs one full pass, terminating the process if it fails.
fn run_pass(
    raw: &mut RawBlockDevice,
    observer: &mut ConsoleObserver,
    config: ScanConfig,
) -> salvage_app::scan::ScanOutcome {
    match Scanner::new(config, CancellationToken::new()).run(raw, observer) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("\nscan failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Human-readable name of a layout strategy.
fn describe_strategy(s: salvage_core::planner::Strategy) -> &'static str {
    use salvage_core::planner::Strategy;
    match s {
        Strategy::LargestContiguous => "largest contiguous area",
        Strategy::MaximumSpace => "maximum usable space",
        Strategy::Conservative => "conservative (wider margin)",
        Strategy::SplicedFat32 => "spliced FAT32 (no guard band)",
    }
}

/// Human-readable summary of a failure scenario.
///
/// Presentation belongs here, not in the domain: the core returns a
/// classification, and each front end chooses its own wording.
fn describe_scenario(scenario: &FailureScenario) -> &'static str {
    match scenario {
        FailureScenario::Pristine => "no defects found",
        FailureScenario::CounterfeitCapacity { .. } => "A - counterfeit capacity",
        FailureScenario::ExhaustedSpare { .. } => "B - FTL spare blocks exhausted",
        FailureScenario::ActivelyDegrading { .. } => "C - actively degrading",
        FailureScenario::Indeterminate { .. } => "indeterminate - needs a second pass",
        FailureScenario::NotProven { .. } => "nothing proven - scan was not conclusive",
    }
}

/// What each assurance level means for the user's data.
fn describe_assurance(level: Assurance) -> &'static str {
    match level {
        Assurance::High => {
            "The preserved area is as reliable as the memory the card actually has. 
             The boundary is imposed by firmware and does not move with use."
        }
        Assurance::Moderate => {
            "Defects appear stable and fencing reduces the risk considerably, but this 
             card is near the end of its life. Keep a copy of anything stored here."
        }
        Assurance::Low => {
            "Fencing covers only the defects already known. This card may develop new 
             ones. Do not store anything irreplaceable here."
        }
        Assurance::None => {
            "There is no basis for any claim about this card. Either it degraded during 
             the inspection, or the scan verified no integrity at all."
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: salvage-inspect <\\\\.\\PhysicalDriveN> \"<device name>\" [--duas-passagens]"
        );
        std::process::exit(2);
    }
    let (target, typed) = (&args[1], &args[2]);

    // A single pass cannot separate a stable defect from ongoing degradation:
    // that requires comparing two measurements taken apart in time.
    let two_passes = args.iter().any(|a| a == "--two-passes");

    let devices = WindowsDeviceEnumerator::new().enumerate().expect("device enumeration failed");
    let Some(device) = devices.iter().find(|d| d.path.eq_ignore_ascii_case(target)) else {
        eprintln!("device {target} not found");
        std::process::exit(1);
    };

    println!("Target: {device}");
    println!("Fingerprint: {}", device.fingerprint());

    let policy = SafetyPolicy::default();
    match evaluate(device, &policy) {
        SafetyVerdict::Blocked { reasons } => {
            eprintln!("\nBLOCKED by the safety guards:");
            for r in reasons {
                eprintln!("  - {}", salvage_win32::text::block_message(&r));
            }
            std::process::exit(1);
        }
        SafetyVerdict::NeedsConfirmation { warnings } => {
            println!("\nWarnings:");
            for w in warnings {
                println!("  - {}", salvage_win32::text::warning_message(&w));
            }
        }
        SafetyVerdict::Allowed => println!("\nNo reservations."),
    }

    let consent = match DestructiveConsent::issue(device, typed, &policy) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\nconsent refused: {e}");
            std::process::exit(1);
        }
    };
    println!("Consent issued for {}\n", consent.fingerprint());

    let mut raw =
        RawBlockDevice::open(&device.path, Access::ReadWrite).expect("could not open the device");
    for problem in raw.lock_volumes_of_disk(device.index) {
        println!("warning: {problem}");
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9ABC_DEF0)
        | 1;

    let config = ScanConfig::new(nonce, device.geometry.sector_size());
    let mut observer = ConsoleObserver {
        started: Instant::now(),
        phase_started: Instant::now(),
        phase: None,
        phase_base: 0,
        last_percent: -1,
    };

    let first = run_pass(&mut raw, &mut observer, config);

    let (outcome, baseline) = if two_passes {
        println!("\n\n===== SECOND PASS =====");
        println!("Compared against the first to separate stable defects from active degradation.");

        // The seed must change. Reusing it, a sector failing this pass's write
        // would still hold the correct pattern from the previous one, pass
        // verification, and stay invisible — precisely the defect the second
        // pass exists to find.
        let config = ScanConfig { nonce: nonce.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, ..config };
        observer.phase = None;
        let second = run_pass(&mut raw, &mut observer, config);
        (second, Some(first))
    } else {
        (first, None)
    };
    let total_time = observer.started.elapsed().as_secs_f64();

    // ------------------------------------------------------------ resultado
    let ss = device.geometry.sector_size() as u64;
    let counts = outcome.map.counts();

    println!("\n\n========== RESULT ==========");
    println!("Total time: {}", format::duration(total_time));
    println!("\nSectors by state:");
    for (name, n) in [
        ("Approved", counts.good),
        ("Read failure", counts.bad_read),
        ("Write failure", counts.bad_write),
        ("Silent corruption", counts.corrupt),
        ("Nonexistent address", counts.aliased),
        ("Not inspected", counts.untested),
    ] {
        if n > 0 {
            println!("  {name:24} {:>14} sectors  {:>12}", n, format::bytes(n * ss));
        }
    }

    let report = diagnose(&outcome.map, baseline.as_ref().map(|o| &o.map));
    println!("\n--- DIAGNOSIS ---");
    println!("Scenario : {}", describe_scenario(&report.scenario));
    println!("Assurance: {}", report.assurance.as_str());
    println!("{}", describe_assurance(report.assurance));
    println!(
        "Largest approved contiguous area: {}",
        format::bytes(report.largest_usable_sectors * ss)
    );

    if baseline.is_none() {
        println!(
            "\n  Note: a single pass cannot separate a stable defect from active degradation.\
             \n        Repeat with --two-passes to obtain that distinction."
        );
    }

    if let salvage_core::health::FailureScenario::CounterfeitCapacity {
        real_capacity_sectors,
        reported_capacity_sectors,
        evidence_count,
    } = &report.scenario
    {
        println!("\n  ADVERTISED capacity : {}", format::bytes(reported_capacity_sectors * ss));
        println!("  REAL capacity       : {}", format::bytes(real_capacity_sectors * ss));
        println!("  Aliasing evidence   : {evidence_count}");
    }

    if !report.isolation_is_worthwhile {
        println!("\nThere is no isolation worth proposing for this card.");
        return;
    }

    println!("\n--- POSSIBLE LAYOUTS (nothing was applied) ---");
    let policy = PlanningPolicy::recommended_for(outcome.map.geometry());
    for plan in plan_layouts(&outcome.map, FileSystem::ExFat, &policy) {
        plan.validate(&outcome.map, 4).expect("an invalid plan reached the user");
        println!(
            "\n[{}]  usable area: {}",
            describe_strategy(plan.strategy),
            format::bytes(plan.usable_bytes)
        );
        println!("  sacrificed to margin/alignment: {}", format::bytes(plan.sacrificed_bytes));
        for p in &plan.partitions {
            let kind = match p.role {
                PartitionRole::Data => "VISIBLE",
                PartitionRole::Quarantine => "hidden ",
            };
            println!(
                "  {kind} {:14} LBA {:>11}..{:<11} {:>12}  type {:#04x}",
                p.label,
                p.range.start(),
                p.range.end(),
                format::bytes(p.range.len() * ss),
                p.mbr_type
            );
        }
    }
}
