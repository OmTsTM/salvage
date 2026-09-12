// No console window behind the interface in a release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! Bridge between the toolchain core and the window.
//!
//! This file holds no business rules. Every decision about what is safe stays
//! in the layers below, which are the tested ones; what lives here is the
//! command surface the window calls, and the launch that puts it on screen.
//!
//! The rest of the bridge sits beside it, split along the lines the work
//! actually falls on:
//!
//! - [`diagnostics`] — the log, which everything else calls into and which
//!   depends on nothing.
//! - [`views`] — the one place a domain type becomes a shape JSON can carry.
//!   It ships no sentences: the window owns the wording, in four languages.
//! - [`state`] — what a session holds, and the observer that watches a scan
//!   and pushes progress out.
//!
//! They were one file of 1,661 lines until the boundary between "decide when
//! to answer" and "decide what an answer looks like" was worth drawing.

mod diagnostics;
mod state;
mod views;

use diagnostics::{log, log_path, START};
use state::{consent_key, ScanEnd, Shared, WindowObserver};
use views::{
    build_snapshot, read_prior_layout, state_code, DeviceView, PlanView, PriorLayoutView,
    RecheckView, RememberedView, Snapshot,
};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use salvage_app::device::{DeviceEnumerator, DeviceInfo};
use salvage_app::history::{CardHistory, CardRecord};
use salvage_app::safety::{DestructiveConsent, SafetyPolicy};
use salvage_app::scan::{CancellationToken, ScanConfig, Scanner};
use salvage_core::health::diagnose;
use salvage_core::mbr::PriorLayout;
use salvage_core::pattern::PatternKind;
use salvage_core::planner::{
    fenced_view, layout_requirements, plan_layouts, FileSystem, LayoutRequirements, PlanningPolicy,
};
use salvage_core::sector_map::{SectorCounts, SectorMap, SectorState};
use salvage_win32::{apply_plan, FileHistory, RawBlockDevice, WindowsDeviceEnumerator};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

/// Blocks handed to the visualization. A 96 by 64 grid.
const VIEW_BUCKETS: usize = 6144;

/// Minimum interval between updates pushed to the window.
const EMIT_INTERVAL: Duration = Duration::from_millis(120);

// ------------------------------------------------------------------- launch

/// Shortest the splash stays on screen.
///
/// Measured from process start, so a slow launch waits for nothing extra. It
/// exists because the launch is usually faster than this: a splash that flashes
/// for a tenth of a second reads as a glitch rather than as the program
/// starting, and the eye is left with an impression of something going wrong.
///
/// Two and a half seconds is long for a splash, and deliberate: at 1.2s the
/// line under the wordmark could not be read before the window replaced it,
/// which makes the whole screen decoration. This is the one number to change
/// if it ever feels like a wait rather than an opening.
const SPLASH_MIN_MILLIS: u64 = 2500;

/// Longest the splash may hold the screen on its own.
///
/// Generous: a cold WebView2 start and the first sweep of the machine's disks
/// both happen inside it. Firing early costs nothing — the window appears and
/// the splash goes, which is what was going to happen anyway.
const SPLASH_DEADLINE_SECS: u64 = 10;

/// Whether the window has already been put on screen.
static REVEALED: AtomicBool = AtomicBool::new(false);

/// Shows the window and dismisses the splash.
///
/// Called either by the window itself, once it has drawn and listed the
/// devices, or by the deadline below when it never does. Idempotent, because
/// whichever arrives second must not undo the first.
fn reveal_main(app: &AppHandle, why: &str) {
    if REVEALED.swap(true, Ordering::SeqCst) {
        return;
    }
    log(&format!("showing the window ({why})"));

    if let Some(main) = app.get_webview_window("main") {
        // Centred before it is shown, so it does not appear in one place and
        // jump to another.
        let _ = main.center();
        let _ = main.show();
        let _ = main.set_focus();
    }
    // Destroyed rather than closed: closing asks, and the close request is
    // handled below as a question for the user about a running inspection.
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.destroy();
    }
}

// ---------------------------------------------------------------- utilities

/// Stores what was just measured, so a later session need not measure it again.
///
/// Best effort by design. A record that cannot be written is a convenience
/// lost, and the inspection the user waited hours for must not fail with it.
/// The one thing kept from any earlier record is the card's original partition
/// table: that is what makes fencing reversible, and it exists only once.
fn remember(device: &DeviceInfo, map: &SectorMap, wrote: (u64, PatternKind)) {
    let store = FileHistory::in_user_data();
    let fingerprint = device.fingerprint();
    let previous = store.load(&fingerprint).ok().flatten();

    // The seed goes in with the map: without it the area cannot be read back
    // later, and reading it back later is the only way to learn whether the
    // card still holds what it took.
    let mut record = CardRecord::new(&fingerprint, map.clone()).wrote(wrote.0, wrote.1);
    if let Some(table) = previous.and_then(|r| r.table_before) {
        record.remember_table(&table);
    }
    match store.save(&record) {
        Ok(()) => log("card record stored"),
        Err(e) => log(&format!("could not store the card record: {e}")),
    }
}

/// Keeps the table a card carried before this program overwrote it.
///
/// Called after an apply, with the bytes the apply read on its way past. There
/// is exactly one moment this can be captured — just before it stops existing —
/// and missing it makes the fencing one-way.
fn remember_table(device: &DeviceInfo, table: &[u8]) {
    let store = FileHistory::in_user_data();
    let fingerprint = device.fingerprint();
    let Ok(Some(mut record)) = store.load(&fingerprint) else {
        log("no card record to attach the original table to");
        return;
    };
    if record.table_before.is_some() {
        return;
    }
    record.remember_table(table);
    if let Err(e) = store.save(&record) {
        log(&format!("could not store the original table: {e}"));
    }
}

/// Session seed derived from the clock.
fn session_nonce() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5D_A9_1C_3B_7E_44_02_91)
        | 1
}

/// A command's `Err` is a lookup key, not a sentence. The alternative was
/// Portuguese assembled here, which no amount of translating the window could
/// ever have reached. Anything without a key still arrives as its own text —
/// the window's lookup falls through to whatever it was handed — so a rare
/// technical failure gets reported verbatim instead of swallowed.
pub mod err {
    pub const STATE: &str = "err.state";
    pub const ENUMERATE: &str = "err.enumerate";
    pub const DEVICE_GONE: &str = "err.device_gone";
    pub const SCAN_RUNNING: &str = "err.scan_running";
    pub const NOTHING_REMEMBERED: &str = "err.nothing_remembered";
    /// The record predates the field that makes a re-read possible.
    pub const NO_PATTERN: &str = "err.no_pattern";
    /// Nothing was approved, so there is no area worth re-reading.
    pub const NOTHING_APPROVED: &str = "err.nothing_approved";
    pub const REMEMBERED_OTHER_CARD: &str = "err.remembered_other_card";
    pub const STALE_MAP: &str = "err.stale_map";
    pub const NO_DEVICE: &str = "err.no_device";
    pub const NO_MAP: &str = "err.no_map";
    /// The card was not wholly proven good, so one volume over all of it is
    /// the wrong shape. A card with defects has layouts, or has release.
    pub const NOT_PRISTINE: &str = "err.not_pristine";
    pub const NO_PLAN: &str = "err.no_plan";
    pub const PLAN_REJECTED: &str = "err.plan_rejected";
    pub const NAME_MISMATCH: &str = "err.name_mismatch";
    pub const DEVICE_BLOCKED: &str = "err.device_blocked";
    pub const OPEN_FAILED: &str = "err.open_failed";
    pub const OPEN_LOG: &str = "err.open_log";
    pub const SCAN_NOT_WRITABLE: &str = "err.scan.not_writable";
    pub const SCAN_ALL_WRITES_FAILED: &str = "err.scan.all_writes_failed";
}

// ------------------------------------------------------------------ commands

#[tauri::command]
fn list_devices(state: State<'_, Shared>) -> Result<Vec<DeviceView>, String> {
    let devices = WindowsDeviceEnumerator::new().enumerate().map_err(|e| {
        log(&format!("failed to enumerate devices: {e}"));
        err::ENUMERATE.to_string()
    })?;
    log(&format!("{} devices found", devices.len()));

    let policy = SafetyPolicy::default();
    // The listing names capacities, and on a card this program already fenced
    // the figure that matters is what the earlier layout left. One sector read
    // per card answers that, and it is skipped for anything the guards refuse:
    // the device certain to be blocked is the system disk, and there is nothing
    // to learn from its table.
    let views = devices
        .iter()
        .map(|d| {
            let mut view = DeviceView::from(d, &policy);
            if !view.is_blocked() {
                view.prior = read_prior_layout(d).as_ref().map(PriorLayoutView::from);
            }
            view
        })
        .collect();

    let mut guard = state.lock().map_err(|_| err::STATE)?;
    guard.devices = devices;
    Ok(views)
}

#[tauri::command]
fn select_device(path: String, state: State<'_, Shared>) -> Result<DeviceView, String> {
    let mut guard = state.lock().map_err(|_| err::STATE)?;
    let device = guard.devices.iter().find(|d| d.path == path).cloned().ok_or(err::DEVICE_GONE)?;

    // Switching devices invalidates every previous result.
    guard.map = None;
    guard.baseline = None;
    guard.report = None;
    guard.plans.clear();
    guard.consent = None;
    guard.remembered = None;
    guard.verified_now = false;
    guard.selected = Some(device.clone());

    // Read here rather than in the listing: it costs a handle and a sector per
    // card, and only the selected one is about to be inspected.
    let prior = read_prior_layout(&device);
    match &prior {
        Some(p) => log(&format!(
            "{} carries a layout from an earlier run: {} data, {} quarantined",
            device.path,
            p.data.len(),
            p.quarantined.len()
        )),
        None => log(&format!("{} carries no layout of ours", device.path)),
    }
    guard.prior = prior.clone();
    // Clamped here, once, so the window and the inspection cannot be handed
    // different intervals: the figure comes off the media's own table.
    guard.view_span = prior
        .as_ref()
        .and_then(PriorLayout::data_span)
        .map(|r| r.clamped_to(&device.geometry.full_range()));

    let mut view = DeviceView::from(&device, &SafetyPolicy::default());
    view.prior = prior.as_ref().map(PriorLayoutView::from);

    // What an earlier session measured. Loaded but not adopted: it becomes the
    // working map only if the user asks for it, and even then it approves
    // nothing on its own.
    let store = FileHistory::in_user_data();
    match store.load(&device.fingerprint()) {
        Ok(Some(record)) => {
            log(&format!(
                "card remembered from an earlier session, age {:?}s",
                record.age_seconds()
            ));
            view.remembered = Some(RememberedView::from_record(&record));
            guard.remembered = Some(record);
        }
        Ok(None) => guard.remembered = None,
        Err(e) => {
            log(&format!("card record unreadable: {e}"));
            guard.remembered = None;
        }
    }
    Ok(view)
}

/// Adopts what an earlier session measured, without re-reading the card.
///
/// The record becomes the working map, so the diagnosis and the layouts appear
/// as they did then. It approves nothing: `verified_now` stays false, and the
/// apply path refuses to write a data partition from a map in that state until
/// the area has been read back today.
#[tauri::command]
fn use_remembered(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let mut guard = state.lock().map_err(|_| err::STATE)?;
    let record = guard.remembered.clone().ok_or(err::NOTHING_REMEMBERED)?;
    let device = guard.selected.clone().ok_or(err::NO_DEVICE)?;

    if !record.matches(&device.fingerprint()) {
        return Err(err::REMEMBERED_OTHER_CARD.into());
    }

    log(&format!("adopting the remembered map, age {:?}s", record.age_seconds()));
    let report = diagnose(&record.map, None);
    let snapshot =
        build_snapshot(&record.map, None, Some(&report), false, guard.view_span, false, None);
    guard.map = Some(record.map);
    guard.report = Some(report);
    guard.verified_now = false;
    guard.baseline = None;
    let _ = app.emit("scan:done", snapshot);
    Ok(())
}

/// Removes this program's layout and gives the card its capacity back.
///
/// # What this is
///
/// The way out. Fencing is otherwise a one-way door: a card given a layout is
/// limited to the sliver that survived, for good.
///
/// # What it is not
///
/// It does not undo the inspection. The pattern was written over every sector
/// long before any layout existed, and the card's original contents went with
/// it. And it removes the protection rather than the damage: a card fenced
/// because almost all of it is dead comes back as one full-capacity volume that
/// will accept files and lose them. That is a legitimate thing to want, and it
/// is the caller's to choose — the window says so before asking.
#[tauri::command]
fn release_card(
    typed_name: String,
    filesystem: String,
    state: State<'_, Shared>,
) -> Result<ApplyView, String> {
    let (device, table) = {
        let guard = state.lock().map_err(|_| err::STATE)?;
        let device = guard.selected.clone().ok_or(err::NO_DEVICE)?;
        let table = guard.remembered.as_ref().and_then(|r| r.table_before.clone());
        (device, table)
    };

    // The same named consent as any other destructive operation: this rewrites
    // the partition table of a disk.
    let consent = DestructiveConsent::issue(&device, &typed_name, &SafetyPolicy::default())
        .map_err(|e| {
            log(&format!("consent refused: {e}"));
            consent_key(&e).to_string()
        })?;

    let filesystem = match filesystem.as_str() {
        "fat32" => FileSystem::Fat32,
        _ => FileSystem::ExFat,
    };

    log(&format!(
        "releasing {} (original table {})",
        device.path,
        if table.is_some() { "restored" } else { "not kept; using full capacity" }
    ));
    let outcome =
        salvage_win32::release_card(&device, &consent, table.as_deref(), filesystem, "SALVAGE")
            .map_err(|e| e.to_string())?;

    // The card no longer carries a layout, so the next inspection covers all of
    // it again. Everything measured about the fenced state is now a description
    // of a card that does not exist.
    if let Ok(mut guard) = state.lock() {
        guard.prior = None;
        guard.view_span = None;
        guard.map = None;
        guard.baseline = None;
        guard.report = None;
        guard.plans.clear();
    }

    Ok(ApplyView { steps: outcome.steps, drive_letter: outcome.data_volume_letter, prior: None })
}

/// Writes one full-capacity volume on a card the inspection found intact.
///
/// # Why this exists
///
/// The inspection writes a pattern over every sector, so a card leaves it
/// erased and unpartitioned whatever the verdict. A card with defects then
/// flows into a layout, which ends with a formatted volume. A card with none
/// had nowhere to flow: the better result left the user with an unusable card
/// and no next step. This is that step.
///
/// # Why only for a clean verdict
///
/// One volume over the whole card is the right shape only when the whole card
/// is good. Where defects exist, the same operation is [`release_card`] — same
/// partition table, different claim: it hands back capacity known to be
/// unreliable, and says so first. Offering it here as "format the card" would
/// dress that up as a convenience.
#[tauri::command]
fn prepare_card(
    typed_name: String,
    filesystem: String,
    label: String,
    state: State<'_, Shared>,
) -> Result<ApplyView, String> {
    let (device, map, consent) = {
        let guard = state.lock().map_err(|_| err::STATE)?;
        let device = guard.selected.clone().ok_or(err::NO_DEVICE)?;
        let map = guard.map.clone().ok_or(err::NO_MAP)?;

        // Same gate as `apply`, for the same reason: a map adopted from an
        // earlier session says the card was intact then. Writing a volume over
        // the whole card on that basis would place files everywhere on a claim
        // no one has checked today.
        if !guard.verified_now {
            log("prepare refused: the working map was not verified in this session");
            return Err(err::STALE_MAP.into());
        }

        // The window only offers this on a clean verdict; the check is repeated
        // here because the window is not what makes it safe.
        let report = guard.report.as_ref().ok_or(err::NO_MAP)?;
        if report.counts.defective() > 0 || report.counts.untested > 0 {
            log("prepare refused: the card is not wholly proven good");
            return Err(err::NOT_PRISTINE.into());
        }

        let consent = DestructiveConsent::issue(&device, &typed_name, &SafetyPolicy::default())
            .map_err(|e| {
                log(&format!("consent refused: {e}"));
                consent_key(&e).to_string()
            })?;
        (device, map, consent)
    };

    let filesystem = match filesystem.as_str() {
        "fat32" => FileSystem::Fat32,
        _ => FileSystem::ExFat,
    };

    log(&format!("preparing {} as {}", device.path, filesystem.format_name()));
    let outcome = salvage_win32::prepare_card(&device, &map, &consent, filesystem, &label)
        .map_err(|e| e.to_string())?;

    // Kept for the same reason fencing keeps it: whatever was on the card
    // before this program wrote a table is the only way back to it.
    if let Some(table) = outcome.table_before.as_deref() {
        remember_table(&device, table);
    }

    Ok(ApplyView { steps: outcome.steps, drive_letter: outcome.data_volume_letter, prior: None })
}

/// Re-reads the approved area of a stored record, days later, without writing.
///
/// # The question a scan cannot answer
///
/// An inspection proves a cell took the data and gave it back. On the last run
/// of a 252 GB card the shortest interval between the two was a quarter of a
/// second, and the longest two hours — so what it proved was that the cells
/// accept data, not that they keep it. A worn cell answers the first correctly
/// and the second badly, which is how a card passes an inspection and loses
/// files overnight.
///
/// Running this the next day asks the second question, over exactly the area a
/// layout would use.
///
/// # Why it asks for no consent
///
/// It writes nothing, and the handle it opens cannot: the device is opened for
/// reading, so the guarantee is the operating system's rather than this
/// function's good intentions. There is nothing to consent to.
#[tauri::command]
fn recheck_retention(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let (device, record, cancel) = {
        let mut guard = state.lock().map_err(|_| err::STATE)?;
        if guard.scanning {
            return Err(err::SCAN_RUNNING.into());
        }
        let device = guard.selected.clone().ok_or(err::NO_DEVICE)?;
        let record = guard.remembered.clone().ok_or(err::NOTHING_REMEMBERED)?;
        if record.pattern.is_none() {
            return Err(err::NO_PATTERN.into());
        }
        guard.cancel = CancellationToken::new();
        guard.scanning = true;
        (device, record, guard.cancel.clone())
    };

    let spans: Vec<salvage_core::LbaRange> = record.map.usable_ranges().collect();
    let approved: u64 = spans.iter().map(|r| r.len()).sum();
    if approved == 0 {
        if let Ok(mut guard) = state.lock() {
            guard.scanning = false;
        }
        return Err(err::NOTHING_APPROVED.into());
    }

    let pattern = record.pattern.expect("checked above");
    log(&format!(
        "re-checking {} approved sectors on {} against the pattern from {}s ago",
        approved,
        device.path,
        record.age_seconds().unwrap_or(0)
    ));

    let shared: Shared = Arc::clone(state.inner());
    std::thread::spawn(move || {
        let outcome = (|| -> Result<salvage_app::scan::ReverifyOutcome, ScanEnd> {
            // Read-only, so the operating system enforces what the comment
            // above only promises.
            let mut raw =
                RawBlockDevice::open(&device.path, salvage_win32::Access::Read).map_err(|e| {
                    log(&format!("failed to open for re-check: {e}"));
                    ScanEnd::Failed(e.to_string())
                })?;

            let mut config = ScanConfig::new(pattern.nonce, device.geometry.sector_size());
            config.pattern = pattern.kind;

            let mut observer = WindowObserver::new(app.clone(), None);
            Scanner::new(config, cancel).reverify(&mut raw, &spans, &mut observer).map_err(|e| {
                match e {
                    salvage_app::scan::ScanError::Cancelled => ScanEnd::Cancelled,
                    other => ScanEnd::Failed(other.to_string()),
                }
            })
        })();

        let mut guard = match shared.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.scanning = false;

        match outcome {
            Ok(out) => {
                log(&format!(
                    "re-check finished in {}s: {} matched, {} corrupt, {} foreign, {} aliased, {} unreadable",
                    out.elapsed_secs,
                    out.tally.matched,
                    out.tally.corrupt,
                    out.tally.foreign,
                    out.tally.aliased,
                    out.tally.unreadable
                ));

                // Most of the area answering with no recognisable header means
                // something wrote to this card since the inspection. The
                // comparison is then between two unrelated things, and
                // reporting it as damage would condemn a healthy card.
                if !out.reference_survived() {
                    log("re-check abandoned: the pattern is no longer on this card");
                    let _ = app.emit("recheck:reference-gone", ());
                    return;
                }

                let held = out.tally.defects() == 0;
                let view = RecheckView {
                    examined_bytes: out.sectors_examined * device.geometry.sector_size() as u64,
                    lost_bytes: out.tally.defects() * device.geometry.sector_size() as u64,
                    elapsed_secs: out.elapsed_secs,
                    age_seconds: record.age_seconds(),
                    held,
                };

                // Merged onto the stored map rather than replacing it. The
                // re-read covers only the approved area, so everything else
                // comes back untested — and adopting that wholesale would throw
                // away what the original scan proved about the condemned half,
                // turning a diagnosis into "nothing proven". What the re-read
                // measured overwrites what the record said; what it did not
                // look at keeps standing.
                let mut merged = record.map.clone();
                for run in out.map.runs() {
                    if run.state != SectorState::Untested {
                        merged.mark(run.range, run.state);
                    }
                }

                // The area a layout could use has now been read back today,
                // which is exactly what the apply gate asks for.
                guard.map = Some(merged);
                guard.verified_now = true;
                guard.retention = None;
                if let Some(map) = guard.map.as_ref() {
                    let report = diagnose(map, None);
                    let snapshot =
                        build_snapshot(map, None, Some(&report), false, None, true, None);
                    guard.report = Some(report);
                    let _ = app.emit("scan:done", snapshot);
                }
                let _ = app.emit("recheck:done", view);
            }
            Err(ScanEnd::Cancelled) => {
                log("re-check cancelled by the user");
                let _ = app.emit("scan:cancelled", ());
            }
            Err(ScanEnd::Failed(e)) => {
                log(&format!("re-check failed: {e}"));
                let _ = app.emit("scan:error", e);
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn start_scan(typed_name: String, app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let (device, cancel, view_span) = {
        let mut guard = state.lock().map_err(|_| err::STATE)?;
        if guard.scanning {
            return Err(err::SCAN_RUNNING.into());
        }
        let device = guard.selected.clone().ok_or(err::NO_DEVICE)?;
        log(&format!("start_scan requested: {}", device.path));

        // The inspection destroys the content, so it demands the same named
        // consent as repartitioning.
        let consent = DestructiveConsent::issue(&device, &typed_name, &SafetyPolicy::default())
            .map_err(|e| {
                log(&format!("consent refused: {e}"));
                consent_key(&e).to_string()
            })?;
        guard.consent = Some(consent);
        log("consent accepted");

        guard.cancel = CancellationToken::new();
        guard.scanning = true;
        guard.map = None;
        guard.report = None;
        guard.plans.clear();
        (device, guard.cancel.clone(), guard.view_span)
    };

    let shared: Shared = Arc::clone(state.inner());
    std::thread::spawn(move || {
        log("scan thread started");
        let access = salvage_win32::Access::ReadWrite;

        // The whole device, unless this program already fenced part of it off.
        // A partial inspection cannot approve area, so narrowing one is only
        // defensible when what is left out was already condemned — and the
        // card's own partition table is what says so. Anything else gets
        // inspected end to end, as it always did.
        let mut config = ScanConfig::new(session_nonce(), device.geometry.sector_size());
        config.range = view_span;
        let span = config.range.unwrap_or_else(|| device.geometry.full_range());

        let result = (|| -> Result<salvage_app::scan::ScanOutcome, ScanEnd> {
            log(&format!("opening {} as {:?}", device.path, access));
            let mut raw = RawBlockDevice::open(&device.path, access).map_err(|e| {
                log(&format!("failed to open: {e}"));
                ScanEnd::Failed(e.to_string())
            })?;
            log("device opened");

            log("locking and dismounting volumes");
            for problem in raw.lock_volumes_of_disk(device.index) {
                log(&format!("volume warning: {problem}"));
                let _ = app.emit("scan:warning", problem);
            }
            log("volumes handled");

            log(&format!(
                "starting scan: LBA {}..{} of {} sectors, blocks of {} sectors",
                span.start(),
                span.end(),
                device.geometry.total_sectors(),
                config.chunk_sectors
            ));

            let mut observer = WindowObserver::new(app.clone(), config.range);
            let result = Scanner::new(config, cancel).run(&mut raw, &mut observer).map_err(|e| {
                use salvage_app::scan::ScanError;
                // The two a user can act on get wording of their own, in their
                // own language. The rest arrive as their own text: it is
                // technical, it is in the log either way, and showing it beats
                // replacing it with a shrug.
                log(&format!("scan failed: {e}"));
                match e {
                    ScanError::Cancelled => ScanEnd::Cancelled,
                    ScanError::NotWritable => ScanEnd::Failed(err::SCAN_NOT_WRITABLE.into()),
                    ScanError::SystemicWriteFailure { .. } => {
                        ScanEnd::Failed(err::SCAN_ALL_WRITES_FAILED.into())
                    }
                    other => ScanEnd::Failed(other.to_string()),
                }
            });
            log(&format!("scan finished, {} events emitted", observer.emitted));
            result
        })();

        let mut guard = match shared.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.scanning = false;

        match result {
            Ok(outcome) => {
                let mut map = outcome.map;
                map.withhold_outside(span);
                let report = diagnose(&map, guard.baseline.as_ref());
                let snapshot = build_snapshot(
                    &map,
                    None,
                    Some(&report),
                    false,
                    config.range,
                    true,
                    Some(outcome.retention),
                );
                // This pass becomes the next one's baseline, which is what
                // distinguishes a stable defect from active degradation.
                guard.baseline = Some(map.clone());
                guard.verified_now = true;
                guard.retention = Some(outcome.retention);
                remember(&device, &map, (config.nonce, config.pattern));
                guard.map = Some(map);
                guard.report = Some(report);
                let _ = app.emit("scan:done", snapshot);
            }
            Err(ScanEnd::Cancelled) => {
                log("scan cancelled by the user");
                // No payload: the window owns the wording, and there is
                // nothing here it does not already know.
                let _ = app.emit("scan:cancelled", ());
            }
            Err(ScanEnd::Failed(message)) => {
                log(&format!("error: {message}"));
                let _ = app.emit("scan:error", message);
            }
        }
    });

    Ok(())
}

/// Stops the inspection and closes the program once the card has been let go.
///
/// Ending the process on the spot would skip the scan thread's unwinding, and
/// that thread is what unlocks and remounts the volumes it dismounted to get
/// exclusive access. Killing it there leaves the card invisible to Windows
/// until it is physically unplugged — a worse state than the one the user was
/// trying to leave. So cancellation is raised, the thread is given time to
/// release the device, and only then does the process end.
#[tauri::command]
fn stop_and_close(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    {
        let guard = state.lock().map_err(|_| err::STATE)?;
        guard.cancel.cancel();
    }
    log("stop requested from the close prompt");

    let shared: Shared = Arc::clone(state.inner());
    std::thread::spawn(move || {
        // A block is the granularity at which the scan notices it was asked to
        // stop, and a slow card takes seconds over one. Past this deadline the
        // waiting is itself the failure: the user asked to leave, the window is
        // already gone, and nothing further is coming.
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match shared.lock() {
                Ok(guard) if !guard.scanning => break,
                Err(_) => break,
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        log("closing");
        app.exit(0);
    });
    Ok(())
}

#[tauri::command]
fn cancel_scan(state: State<'_, Shared>) -> Result<(), String> {
    let guard = state.lock().map_err(|_| err::STATE)?;
    guard.cancel.cancel();
    Ok(())
}

/// Planning response: the layouts plus a view with fenced area highlighted, so
/// the cost of the guard band is visible on the card.
#[derive(Serialize)]
struct PlansResponse {
    plans: Vec<PlanView>,
    buckets: Vec<u8>,
    /// Same purpose as on the snapshot: the fenced repaint condemns even more
    /// area, so without these the surviving space disappears from the picture
    /// exactly when the user is choosing what to do with it.
    approved_marks: Vec<u32>,
    counts: SectorCounts,
    fenced_bytes: u64,
    /// Present only when no layout could be produced, and the reason it could
    /// not. Returning this instead of an error is what lets the window say
    /// "94 MB approved, largest piece 10 MB, fencing needs 25 MB" rather than
    /// "no usable area", which reads as a contradiction of the figure above it.
    refusal: Option<RefusalView>,
}

/// The measurements behind a refusal to produce any layout.
///
/// Bytes, not formatted text: the window decides the wording and the decimal
/// separator, which differ per language.
#[derive(Serialize, Clone)]
struct RefusalView {
    approved_bytes: u64,
    approved_runs: usize,
    largest_run_bytes: u64,
    fenced_needs_bytes: u64,
    /// Absent when the approved span could not host a FAT32 volume at all.
    spliced_needs_bytes: Option<u64>,
}

impl RefusalView {
    fn from(req: &LayoutRequirements, sector_size: u32) -> Self {
        let ss = sector_size as u64;
        Self {
            approved_bytes: req.approved_sectors.saturating_mul(ss),
            approved_runs: req.approved_runs,
            largest_run_bytes: req.largest_run_sectors.saturating_mul(ss),
            fenced_needs_bytes: req.fenced_needs_sectors.saturating_mul(ss),
            spliced_needs_bytes: req.spliced_needs_sectors.map(|n| n.saturating_mul(ss)),
        }
    }
}

#[tauri::command]
fn build_plans(filesystem: String, state: State<'_, Shared>) -> Result<PlansResponse, String> {
    let mut guard = state.lock().map_err(|_| err::STATE)?;
    let map = guard.map.clone().ok_or(err::NO_MAP)?;

    let filesystem = match filesystem.as_str() {
        "fat32" => FileSystem::Fat32,
        _ => FileSystem::ExFat,
    };

    let policy = PlanningPolicy::recommended_for(map.geometry());
    let plans = plan_layouts(&map, filesystem, &policy);

    if plans.is_empty() {
        let req = layout_requirements(&map, &policy);
        let sector_size = map.geometry().sector_size();
        log(&format!(
            "no layout: {} approved in {} runs, largest {}, fencing needs {}",
            req.approved_sectors,
            req.approved_runs,
            req.largest_run_sectors,
            req.fenced_needs_sectors
        ));
        return Ok(PlansResponse {
            plans: Vec::new(),
            buckets: Vec::new(),
            counts: map.counts(),
            approved_marks: Vec::new(),
            fenced_bytes: 0,
            refusal: Some(RefusalView::from(&req, sector_size)),
        });
    }

    // No plan reaches the interface without passing safety validation.
    for plan in &plans {
        plan.validate(&map, 4).map_err(|e| {
            log(&format!("plan rejected in validation: {e}"));
            err::PLAN_REJECTED.to_string()
        })?;
    }

    // The fenced view is for reading only: the stored map remains the measured
    // one, and that is what validates plans at apply time.
    let fenced = fenced_view(&map, &policy);
    let sector_size = map.geometry().sector_size() as u64;
    let span = guard.view_span.unwrap_or_else(|| map.geometry().full_range());

    // Restricted to the interval on screen, and this one matters: on a card an
    // earlier layout already fenced, the whole condemned tail is fenced too,
    // and counting it here would present a previous run's quarantine as the
    // price of this run's guard band — off by the size of the failure.
    let fenced_bytes = fenced.counts_in(span).fenced * sector_size;
    let fenced_sample = fenced.downsample_range(span, VIEW_BUCKETS);

    let views = plans.iter().enumerate().map(|(i, p)| PlanView::from(i, p)).collect();
    guard.plans = plans;

    Ok(PlansResponse {
        plans: views,
        buckets: fenced_sample.iter().map(|b| state_code(b.dominant)).collect(),
        approved_marks: fenced_sample
            .iter()
            .enumerate()
            .filter(|(_, b)| b.counts.good > 0 && b.dominant != SectorState::Good)
            .map(|(i, _)| i as u32)
            .collect(),
        counts: fenced.counts_in(span),
        fenced_bytes,
        refusal: None,
    })
}

#[derive(Serialize)]
struct ApplyView {
    /// Structured record of what was done; the window supplies the wording.
    steps: Vec<salvage_win32::apply::ApplyStep>,
    drive_letter: Option<char>,
    /// The layout now on the card. Every figure in the window is about the
    /// area it left, and this is the moment those figures change.
    prior: Option<PriorLayoutView>,
}

#[tauri::command]
fn apply(
    plan_index: usize,
    typed_name: String,
    label: String,
    filesystem: String,
    state: State<'_, Shared>,
) -> Result<ApplyView, String> {
    let (device, plan, map, consent) = {
        let guard = state.lock().map_err(|_| err::STATE)?;
        let device = guard.selected.clone().ok_or(err::NO_DEVICE)?;
        let plan = guard.plans.get(plan_index).cloned().ok_or(err::NO_PLAN)?;
        let map = guard.map.clone().ok_or(err::NO_MAP)?;

        // A map adopted from an earlier session describes the card as it was.
        // Flash degrades and the translation layer moves addresses, so writing
        // a data partition from one would place files on sectors last proven
        // months ago. The record is a map of where to look; only an inspection
        // run today approves anything.
        if !guard.verified_now {
            log("apply refused: the working map was not verified in this session");
            return Err(err::STALE_MAP.into());
        }

        // Consent is reissued from the name typed now: approving the
        // inspection does not count as approving the repartitioning.
        let consent = DestructiveConsent::issue(&device, &typed_name, &SafetyPolicy::default())
            .map_err(|e| {
                log(&format!("consent refused: {e}"));
                consent_key(&e).to_string()
            })?;
        (device, plan, map, consent)
    };

    let filesystem = match filesystem.as_str() {
        "fat32" => FileSystem::Fat32,
        _ => FileSystem::ExFat,
    };

    // The one destructive write that left no trace. A 0.5.6 log showed a card
    // reporting "carries no layout of ours", then a layout appearing out of
    // nowhere thirty-nine seconds later, because a successful repartitioning
    // wrote nothing here — only a refused one did. A file whose purpose is to
    // explain a failure after the fact has to record the operation most likely
    // to have caused it.
    log(&format!(
        "applying {:?} to {} as {}: {} visible, {} hidden, {} usable, {} to margin",
        plan.strategy,
        device.path,
        filesystem.format_name(),
        plan.data_partitions().count(),
        plan.partitions.len() - plan.data_partitions().count(),
        plan.usable_bytes,
        plan.sacrificed_bytes
    ));

    let outcome = apply_plan(&device, &plan, &map, &consent, filesystem, &label).map_err(|e| {
        log(&format!("apply failed: {e}"));
        e.to_string()
    })?;
    log(&format!(
        "layout written to {}{}",
        device.path,
        outcome.data_volume_letter.map_or(String::new(), |l| format!(", mounted at {l}:"))
    ));

    if let Some(table) = outcome.table_before.as_deref() {
        remember_table(&device, table);
    }

    // The card carries a layout now, so the next inspection covers the area it
    // left. Read back off the table just written rather than taken from the
    // plan: the media is what the next run will read, and where the two could
    // ever disagree the media is the one that is right.
    let prior = read_prior_layout(&device);
    let prior_view = prior.as_ref().map(PriorLayoutView::from);
    if let Ok(mut guard) = state.lock() {
        guard.view_span = prior
            .as_ref()
            .and_then(PriorLayout::data_span)
            .map(|r| r.clamped_to(&device.geometry.full_range()));
        guard.prior = prior;
    }

    Ok(ApplyView {
        steps: outcome.steps,
        drive_letter: outcome.data_volume_letter,
        prior: prior_view,
    })
}

#[tauri::command]
fn snapshot(state: State<'_, Shared>) -> Result<Option<Snapshot>, String> {
    let guard = state.lock().map_err(|_| err::STATE)?;
    Ok(guard.map.as_ref().map(|m| {
        build_snapshot(
            m,
            None,
            guard.report.as_ref(),
            guard.scanning,
            guard.view_span,
            guard.verified_now,
            guard.retention,
        )
    }))
}

/// Records a message from the window, so a JavaScript error also lands in the
/// same diagnostic file.
/// The running version, for the header badge.
///
/// Read from the crate rather than written into the interface: the window, the
/// title bar and the installer then all name the same number, and a release
/// cannot ship a build that misreports which one it is.
#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn log_front(message: String) {
    log(&format!("[janela] {message}"));
}

/// Reports that the window has finished loading and may be shown.
#[tauri::command]
fn finish_launch(app: AppHandle) {
    let waited = START.get_or_init(Instant::now).elapsed();
    let remaining = Duration::from_millis(SPLASH_MIN_MILLIS).saturating_sub(waited);
    if remaining.is_zero() {
        reveal_main(&app, "requested by the window");
        return;
    }
    // On a thread: sleeping here would hold the command up, and the window is
    // waiting on the answer.
    std::thread::spawn(move || {
        std::thread::sleep(remaining);
        reveal_main(&app, "requested by the window, after the minimum dwell");
    });
}

/// The author's page, reachable from the footer.
///
/// A constant, and the command below takes no argument. This process runs as
/// Administrator, and a command that opened whatever address the window handed
/// it would be an arbitrary-launch surface inside an elevated process — the
/// window being the part of this program most exposed to what renders in it.
const AUTHOR_URL: &str = "https://ko-fi.com/omtstm";

/// Opens the author's page in the user's browser.
///
/// Through `explorer.exe`, not the browser and not `cmd /c start`. Everything
/// an elevated process spawns inherits Administrator, and an elevated browser
/// is a genuinely bad thing to hand somebody. Explorer is the exception: asked
/// by an elevated process, it routes the request to the desktop shell already
/// running unelevated, so the page opens at the integrity level it should.
#[tauri::command]
fn open_author_page() -> Result<(), String> {
    log("opening the author page");
    // By absolute path, never as a bare name: this process runs elevated, and
    // a bare name is resolved by walking `PATH`, where anything planted ahead
    // of the real Explorer would be launched as Administrator.
    let explorer = salvage_win32::windows_executable("explorer.exe").map_err(|e| {
        log(&format!("could not resolve explorer.exe: {e}"));
        err::OPEN_FAILED.to_string()
    })?;
    std::process::Command::new(explorer).arg(AUTHOR_URL).spawn().map(|_| ()).map_err(|e| {
        log(&format!("failed to open the author page: {e}"));
        err::OPEN_FAILED.to_string()
    })
}

/// Opens the diagnostic file in whatever application handles it.
///
/// The path comes from `log_path()` and never from the window. That is the
/// same rule as the author page, and it carries more weight here because the
/// argument would be a filesystem path: a command that opened whatever path
/// the webview named would hand an elevated process an arbitrary target.
///
/// Routed through `explorer.exe` for the same reason as well, and again it
/// matters more: an elevated text editor can read and overwrite anything on
/// the machine, which is a poor thing to leave open behind somebody who only
/// wanted to read a log.
#[tauri::command]
fn open_diagnostics() -> Result<(), String> {
    let path = log_path();
    // Before the first line is written there is no file to open, and the
    // directory that will hold it is the next best answer.
    let target = if path.exists() {
        path
    } else {
        path.parent().map(std::path::Path::to_path_buf).unwrap_or(path)
    };

    log(&format!("opening diagnostics: {}", target.display()));
    let explorer = salvage_win32::windows_executable("explorer.exe").map_err(|e| {
        log(&format!("could not resolve explorer.exe: {e}"));
        err::OPEN_LOG.to_string()
    })?;
    std::process::Command::new(explorer).arg(&target).spawn().map(|_| ()).map_err(|e| {
        log(&format!("failed to open the diagnostics: {e}"));
        err::OPEN_LOG.to_string()
    })
}

/// Path to the diagnostic file, for display in the interface.
#[tauri::command]
fn diagnostics_path() -> String {
    log_path().display().to_string()
}

fn main() {
    START.get_or_init(Instant::now);
    log("================ application started ================");

    tauri::Builder::default()
        .manage(Shared::default())
        // The floor under the splash. Nothing in the window can be allowed to
        // make the splash the whole program: a script that fails to parse
        // never asks for the window, and one release shipped exactly that —
        // the user would be left watching a logo with no way forward. Past the
        // deadline the window appears regardless, and whatever is wrong with
        // it becomes visible instead of invisible.
        .setup(|app| {
            // Stamped at startup rather than in the config, which has no way to
            // interpolate the crate version.
            let version = app.package_info().version.to_string();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&format!("Salvage {version}"));
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(SPLASH_DEADLINE_SECS));
                reveal_main(&handle, "deadline reached without the window asking");
            });
            Ok(())
        })
        // Closing the window mid-inspection would abandon a card with its
        // volumes dismounted and a pattern half written over it. The close is
        // held back and the question put to the user, who is the only one who
        // knows whether the run still matters.
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let scanning =
                    window.state::<Shared>().lock().map(|guard| guard.scanning).unwrap_or(false);
                if scanning {
                    api.prevent_close();
                    log("close requested during a scan; asking");
                    let _ = window.emit("app:close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            list_devices,
            use_remembered,
            release_card,
            recheck_retention,
            prepare_card,
            select_device,
            start_scan,
            cancel_scan,
            stop_and_close,
            build_plans,
            apply,
            snapshot,
            log_front,
            diagnostics_path,
            finish_launch,
            open_author_page,
            open_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the window");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_nonce_is_never_zero() {
        assert_ne!(session_nonce(), 0);
    }
}
