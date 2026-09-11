//! The diagnostic log.
//!
//! The window runs without a console. Without this file a failure inside the
//! scan thread would vanish, and the interface would sit still with nobody able
//! to say where it stopped — which is the situation that put the log here in
//! the first place.
//!
//! Split out of `main.rs` because none of it is bridge work: nothing here
//! serialises anything to the window or touches a device. It is the one part of
//! the binary every other part calls into.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Process start time, the basis for log timestamps.
pub static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Path to the log file.
///
/// It sits under `LOCALAPPDATA`, in a subdirectory of its own, rather than in
/// the temporary directory. An elevated process writing to a path named by
/// `TMP` can be induced to write elsewhere — someone need only control that
/// environment variable and leave a symbolic link where the file should be. The
/// user's data directory offers no such detour.
pub fn log_path() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("Salvage");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("salvage.log")
}

/// Largest log kept before the previous one is set aside. A scan of a failing
/// card produces a lot of lines, and without a ceiling the file grows without
/// bound on the user's system drive.
const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

/// Defect runs written to the log before it switches to counting them.
pub const MAX_LOGGED_DEFECTS: u64 = 500;

/// Bytes this process has written to the log.
///
/// Rotation only happens at startup, which bounds the file across runs but not
/// within one: a single inspection once left a 2.6 GB file behind. The defect
/// cap is the reason that cannot happen again, and this is the floor under it —
/// past the ceiling the file simply stops growing. Stopping rather than
/// rotating mid-run is deliberate: half a run in one file and half in another
/// is worse for reading back than a run that ends early, and what explains a
/// failure is almost always near the beginning.
static LOG_BYTES: AtomicU64 = AtomicU64::new(0);

/// Moves an oversized log aside, keeping exactly one previous copy.
///
/// Run once per process, before the handle is opened: rotating while a scan is
/// writing would leave part of the run in one file and part in another.
fn rotate_if_large(path: &std::path::Path) {
    let too_big = std::fs::metadata(path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false);
    if too_big {
        let _ = std::fs::rename(path, path.with_extension("log.previous"));
    }
}

/// Appends one line to the diagnostic log.
///
/// The graphical window runs without a console: without this file, a failure
/// inside the scan thread would vanish without a trace, and the interface would
/// sit still with nobody knowing where it stopped.
pub fn log(msg: &str) {
    use std::io::Write;
    let elapsed = START.get_or_init(Instant::now).elapsed().as_secs_f64();

    // Line breaks would come in handy for anyone wanting to forge whole log
    // entries; each message occupies exactly one line.
    let clean: String =
        msg.chars().map(|c| if c.is_control() { ' ' } else { c }).take(2000).collect();

    // The handle is opened once and kept. Re-opening it per line also meant
    // creating the directory per line, and during a scan of a failing card that
    // turned logging into the slowest part of the program by a wide margin.
    static FILE: std::sync::OnceLock<Option<Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let handle = FILE.get_or_init(|| {
        let path = log_path();
        rotate_if_large(&path);
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    });

    // Twelve bytes of timestamp and framing, plus the message itself.
    let width = clean.len() as u64 + 12;
    let before = LOG_BYTES.fetch_add(width, Ordering::Relaxed);
    if before >= MAX_LOG_BYTES {
        return;
    }

    if let Some(lock) = handle {
        if let Ok(mut f) = lock.lock() {
            let _ = writeln!(f, "[{elapsed:9.3}s] {clean}");
            // Exactly one caller crosses the line, so exactly one says so.
            if before + width >= MAX_LOG_BYTES {
                let _ = writeln!(
                    f,
                    "[{elapsed:9.3}s] log ceiling of {MAX_LOG_BYTES} bytes reached;                      nothing further will be written to this file"
                );
            }
        }
    }
}
