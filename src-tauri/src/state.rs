//! What a session holds, and what watches it.
//!
//! One card, one map, one diagnosis, one set of layouts — and the two booleans
//! that decide what may be written from them. Split out of `main.rs` so that
//! the rules governing the state sit beside the state rather than scattered
//! through seventeen commands.
//!
//! `verified_now` is the one worth reading twice. It is false for a map adopted
//! from a stored record, and the apply path refuses to write a data partition
//! while it is: a record says where to look, never that the area is still good.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use salvage_app::device::DeviceInfo;
use salvage_app::history::CardRecord;
use salvage_app::safety::DestructiveConsent;
use salvage_app::scan::{CancellationToken, ScanObserver, ScanProgress};
use salvage_core::health::HealthReport;
use salvage_core::mbr::PriorLayout;
use salvage_core::planner::PartitionPlan;
use salvage_core::sector_map::{SectorMap, SectorState};
use tauri::{AppHandle, Emitter};

use crate::diagnostics::{log, MAX_LOGGED_DEFECTS};
use crate::err;
use crate::views::build_snapshot;
use crate::EMIT_INTERVAL;

#[derive(Default)]
pub struct AppState {
    pub devices: Vec<DeviceInfo>,
    pub selected: Option<DeviceInfo>,
    pub map: Option<SectorMap>,
    pub baseline: Option<SectorMap>,
    pub report: Option<HealthReport>,
    pub plans: Vec<PartitionPlan>,
    pub consent: Option<DestructiveConsent>,
    pub cancel: CancellationToken,
    pub scanning: bool,
    /// Layout found on the selected card, read from its own partition table.
    pub prior: Option<PriorLayout>,
    /// Interval the inspection and the window are about. `None` is the whole
    /// device, which is the case for any card this program has not fenced.
    pub view_span: Option<salvage_core::LbaRange>,
    /// What an earlier session measured about the selected card, held so the
    /// user can adopt it without a second read from disk.
    pub remembered: Option<CardRecord>,
    /// How long each sector waited between write and verification, for the
    /// inspection that produced the working map.
    pub retention: Option<salvage_app::scan::RetentionWindow>,
    /// Whether the working map was produced by an inspection in this session.
    ///
    /// False when it came from a record. A map in that state describes hardware
    /// as it was, and the apply path will not write a data partition from it
    /// until the area has been read back today.
    pub verified_now: bool,
}

pub type Shared = Arc<Mutex<AppState>>;

/// Observer that downsamples the map and pushes progress to the window.
pub struct WindowObserver {
    pub app: AppHandle,
    /// Interval the window is drawing. See [`build_snapshot`].
    pub view: Option<salvage_core::LbaRange>,
    pub last_emit: Instant,
    pub emitted: u64,
    pub last_logged_percent: i64,
    pub defects_seen: u64,
}

impl ScanObserver for WindowObserver {
    fn on_progress(&mut self, progress: &ScanProgress, map: &SectorMap) {
        let finished = progress.sectors_done >= progress.sectors_total;
        if !finished && self.last_emit.elapsed() < EMIT_INTERVAL {
            return;
        }
        self.last_emit = Instant::now();

        let percent = (progress.fraction() * 100.0) as i64;
        if self.emitted == 0 || percent / 5 != self.last_logged_percent / 5 {
            log(&format!(
                "progress: phase {:?}, {percent}%, LBA {}, defects {}",
                progress.phase, progress.current_lba, progress.defects_found
            ));
            self.last_logged_percent = percent;
        }

        let snapshot = build_snapshot(map, Some(progress), None, true, self.view, true, None);
        match self.app.emit("scan:progress", snapshot) {
            Ok(()) => self.emitted += 1,
            // If the event never reaches the window, the interface sits still
            // while the scan runs normally. That needs to be known.
            Err(e) => log(&format!("failed to emit progress: {e}")),
        }
    }

    fn on_defect(&mut self, range: salvage_core::LbaRange, state: SectorState) {
        self.defects_seen += 1;

        // The log exists to explain a failure after the fact, and the first
        // few hundred defects explain it as well as a million would. Past the
        // ceiling only the count is kept: writing every one of them is what
        // once made the inspection slower than the card it was inspecting.
        if self.defects_seen <= MAX_LOGGED_DEFECTS {
            log(&format!(
                "defect {:?} at LBA {}..{} ({} sectors)",
                state,
                range.start(),
                range.end(),
                range.len()
            ));
            if self.defects_seen == MAX_LOGGED_DEFECTS {
                log("defect log capped; further defects are counted, not listed");
            }
        }
    }
}

/// How an inspection ended when it produced no map.
///
/// Cancellation is not a failure: the user asked for it, and the window says
/// so in its own words on its own terms. Telling the two apart here is what
/// keeps a deliberate stop from arriving at the window dressed as an error —
/// and dressed in English, since a `ScanError`'s text is written for the log
/// and the command-line tools, never for this window.
pub enum ScanEnd {
    /// The user asked for it.
    Cancelled,
    /// Something went wrong. Carries the text for the log.
    Failed(String),
}

/// Failures the window has wording for, in every language it speaks.
/// Turns a consent refusal into a key, leaving the detail for the log.
pub fn consent_key(e: &salvage_app::safety::ConfirmationError) -> &'static str {
    use salvage_app::safety::ConfirmationError;
    match e {
        ConfirmationError::DeviceBlocked => err::DEVICE_BLOCKED,
        ConfirmationError::NameMismatch { .. } => err::NAME_MISMATCH,
    }
}
