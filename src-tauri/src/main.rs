// No console window behind the interface in a release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! Bridge between the toolchain core and the window.
//!
//! This file holds no business rules: it maps domain types into serializable
//! structures, drives the scan on its own thread, and forwards progress to the
//! window. Every decision about what is safe stays in the layers below, which
//! are the tested ones.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use salvage_app::device::{BlockDevice, DeviceEnumerator, DeviceInfo};
use salvage_app::safety::{evaluate, DestructiveConsent, SafetyPolicy, SafetyVerdict};
use salvage_app::scan::{
    CancellationToken, ScanConfig, ScanObserver, ScanPhase, ScanProgress, Scanner,
};
use salvage_core::health::{diagnose, FailureScenario, HealthReport};
use salvage_core::mbr::{MasterBootRecord, PriorLayout, MBR_SIZE};
use salvage_core::planner::{
    fenced_view, layout_requirements, plan_layouts, FileSystem, LayoutRequirements, PartitionPlan,
    PartitionRole, PlanningPolicy,
};
use salvage_core::sector_map::{SectorCounts, SectorMap, SectorState};
use salvage_win32::{apply_plan, RawBlockDevice, WindowsDeviceEnumerator};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

/// Blocks handed to the visualization. A 96 by 64 grid.
const VIEW_BUCKETS: usize = 6144;

/// Minimum interval between updates pushed to the window.
const EMIT_INTERVAL: Duration = Duration::from_millis(120);

// ----------------------------------------------------------------- logging

/// Process start time, the basis for log timestamps.
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

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
const MAX_LOGGED_DEFECTS: u64 = 500;

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
fn log(msg: &str) {
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

// -------------------------------------------------------------------- abertura

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

// ---------------------------------------------------------------- utilidades

/// Numeric code for a state, used in the visualization's compact vector.
fn state_code(state: SectorState) -> u8 {
    match state {
        SectorState::Untested => 0,
        SectorState::Good => 1,
        SectorState::BadRead => 2,
        SectorState::BadWrite => 3,
        SectorState::Corrupt => 4,
        SectorState::Aliased => 5,
        SectorState::Fenced => 6,
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

// -------------------------------------------------------------------- views

#[derive(Serialize, Clone)]
struct DeviceView {
    path: String,
    index: u32,
    name: String,
    bus: String,
    removable: bool,
    capacity_bytes: u64,
    sector_size: u32,
    total_sectors: u64,
    volumes: Vec<String>,
    verdict: String,
    /// Absolute impediments, if any.
    blocks: Vec<salvage_app::safety::SafetyBlock>,
    /// Risks the user may knowingly accept.
    warnings: Vec<salvage_app::safety::SafetyWarning>,
    /// What an earlier inspection already fenced off, when the card carries
    /// such a layout. `None` also covers "not looked at yet": the list does
    /// not read a table for every disk it names, only for the one selected.
    prior: Option<PriorLayoutView>,
}

/// An earlier layout, in the terms the window needs to talk about it.
#[derive(Serialize, Clone)]
struct PriorLayoutView {
    /// Sectors the next inspection will cover.
    inspect_sectors: u64,
    /// Sectors the earlier layout withheld from data.
    fenced_sectors: u64,
    /// Data partitions that layout left behind.
    data_partitions: usize,
}

impl PriorLayoutView {
    fn from(prior: &PriorLayout) -> Self {
        Self {
            inspect_sectors: prior.data_span().map_or(0, |r| r.len()),
            fenced_sectors: prior.quarantined.iter().map(|r| r.len()).sum(),
            data_partitions: prior.data.len(),
        }
    }
}

impl DeviceView {
    fn from(device: &DeviceInfo, policy: &SafetyPolicy) -> Self {
        // The window receives classifications and picks its own wording; the
        // application layer ships no user-facing prose.
        let (verdict, blocks, warnings) = match evaluate(device, policy) {
            SafetyVerdict::Allowed => ("allowed".to_string(), Vec::new(), Vec::new()),
            SafetyVerdict::NeedsConfirmation { warnings } => {
                ("needs_confirmation".to_string(), Vec::new(), warnings)
            }
            SafetyVerdict::Blocked { reasons } => ("blocked".to_string(), reasons, Vec::new()),
        };

        Self {
            path: device.path.clone(),
            index: device.index,
            name: device.display_name(),
            bus: device.bus_type.as_str().to_string(),
            removable: device.removable_media,
            capacity_bytes: device.capacity_bytes(),
            sector_size: device.geometry.sector_size(),
            total_sectors: device.geometry.total_sectors(),
            volumes: device
                .volumes
                .iter()
                .map(|v| v.drive_letter.map_or("sem letra".into(), |c| format!("{c}:")))
                .collect(),
            verdict,
            blocks,
            warnings,
            prior: None,
        }
    }
}

/// Reads back the layout an earlier inspection left on the card.
///
/// The handle is opened read-only and dropped straight away: this answers a
/// question *about* the card and must not be able to change it. `None` when
/// the table cannot be read or was not written by this program — and then the
/// inspection covers the whole device, as it always did.
fn read_prior_layout(device: &DeviceInfo) -> Option<PriorLayout> {
    let mut raw = RawBlockDevice::open(&device.path, salvage_win32::Access::Read).ok()?;
    // A whole sector, because that is the smallest unit a block device hands
    // over; the record occupies the first 512 bytes of it whatever the sector
    // size happens to be.
    let mut first = vec![0u8; device.geometry.sector_size() as usize];
    raw.read_at(0, &mut first).ok()?;
    let mbr = MasterBootRecord::from_bytes(first.get(..MBR_SIZE)?).ok()?;
    PriorLayout::read(&mbr)
}

/// One measured fact behind a verdict.
///
/// A kind and its numbers, never a sentence. The window owns the wording, and
/// it now owns it in four languages — a `Vec<String>` assembled here would have
/// pinned the whole report to whichever one this file was written in, and
/// pinned the number formatting with it: a thousands separator is not the same
/// character in every language that reads this screen.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ReportDetail {
    /// Capacity the card claims.
    AnnouncedCapacity { sectors: u64 },
    /// Capacity it actually has.
    RealCapacity { sectors: u64 },
    /// Addresses that served another address's content.
    AliasEvidence { count: usize },
    /// Distinct defective regions.
    DefectRegions { count: usize },
    /// The second pass found the same defects in the same places.
    SecondPassIdentical,
    /// Sectors that passed the first pass and failed the second.
    NewlyFailedSectors { sectors: u64 },
    /// Regions that appeared between the two passes.
    NewRegions { count: usize },
    /// Area the inspection never reached.
    UnverifiedArea { sectors: u64 },
    /// Only one pass ran, which cannot judge stability.
    OnePassOnly,
}

#[derive(Serialize, Clone)]
struct ReportView {
    /// Stable scenario identifier. The window maps it to wording; the domain
    /// deliberately ships no user-facing prose.
    scenario_kind: String,
    assurance: String,
    isolation_worthwhile: bool,
    /// Largest run with no detected defect, including uninspected area.
    largest_usable_bytes: u64,
    details: Vec<ReportDetail>,
}

impl ReportView {
    fn from(report: &HealthReport, sector_size: u32) -> Self {
        // Measurements only. The scenario name, the mechanism behind it and the
        // wording of what the numbers are worth all live in the window, which is
        // the presentation layer and the only place that should hold a language.
        let details = match &report.scenario {
            FailureScenario::Pristine => Vec::new(),
            FailureScenario::CounterfeitCapacity {
                real_capacity_sectors,
                reported_capacity_sectors,
                evidence_count,
            } => vec![
                ReportDetail::AnnouncedCapacity { sectors: *reported_capacity_sectors },
                ReportDetail::RealCapacity { sectors: *real_capacity_sectors },
                ReportDetail::AliasEvidence { count: *evidence_count },
            ],
            FailureScenario::ExhaustedSpare { defect_regions } => vec![
                ReportDetail::DefectRegions { count: *defect_regions },
                ReportDetail::SecondPassIdentical,
            ],
            FailureScenario::ActivelyDegrading { newly_failed_sectors, newly_failed_regions } => {
                vec![
                    ReportDetail::NewlyFailedSectors { sectors: *newly_failed_sectors },
                    ReportDetail::NewRegions { count: newly_failed_regions.len() },
                ]
            }
            FailureScenario::NotProven { unverified_sectors, defect_regions } => {
                let mut out = vec![ReportDetail::UnverifiedArea { sectors: *unverified_sectors }];
                if *defect_regions > 0 {
                    out.push(ReportDetail::DefectRegions { count: *defect_regions });
                }
                out
            }
            FailureScenario::Indeterminate { defect_regions } => vec![
                ReportDetail::DefectRegions { count: *defect_regions },
                ReportDetail::OnePassOnly,
            ],
        };

        Self {
            scenario_kind: report.scenario.kind().to_string(),
            assurance: report.assurance.as_str().to_string(),
            isolation_worthwhile: report.isolation_is_worthwhile,
            largest_usable_bytes: report.largest_usable_sectors * sector_size as u64,
            details,
        }
    }
}

#[derive(Serialize, Clone)]
struct Snapshot {
    scanning: bool,
    phase: String,
    fraction: f64,
    sectors_done: u64,
    sectors_total: u64,
    current_lba: u64,
    defects_found: u64,
    buckets: Vec<u8>,
    /// Blocks that hold approved sectors but are not painted as approved.
    ///
    /// A block takes the worst state inside it, so one bad sector hides
    /// thousands of good ones — deliberately, because the reverse would hide a
    /// defect. On a card where the survivors are a fraction of a percent that
    /// leaves them invisible, and "where is my good space" becomes unanswerable
    /// from the picture. These are marked on top instead: the fill keeps
    /// telling the truth about the damage, and the marks say the area exists.
    approved_marks: Vec<u32>,
    counts: SectorCounts,
    sector_size: u32,
    capacity_bytes: u64,
    report: Option<ReportView>,
}

fn build_snapshot(
    map: &SectorMap,
    progress: Option<&ScanProgress>,
    report: Option<&HealthReport>,
    scanning: bool,
    view: Option<salvage_core::LbaRange>,
) -> Snapshot {
    let sector_size = map.geometry().sector_size();

    // Whatever the inspection is about is what the window draws and what it
    // measures: the whole card, or the area an earlier layout left in use. One
    // interval feeds the picture, the tally and the ruler beside them, so the
    // three cannot end up describing different things.
    let view = view.unwrap_or_else(|| map.geometry().full_range());
    let counts = map.counts_in(view);
    let sampled = map.downsample_range(view, VIEW_BUCKETS);
    let approved_marks = sampled
        .iter()
        .enumerate()
        .filter(|(_, b)| b.counts.good > 0 && b.dominant != SectorState::Good)
        .map(|(i, _)| i as u32)
        .collect();
    let buckets = sampled.into_iter().map(|b| state_code(b.dominant)).collect();

    let (phase, fraction, done, total, lba, defects) = match progress {
        Some(p) => (
            match p.phase {
                ScanPhase::Writing => "writing",
                ScanPhase::Verifying => "verifying",
                ScanPhase::Refining => "refining",
            },
            p.fraction(),
            p.sectors_done,
            p.sectors_total,
            p.current_lba,
            p.defects_found,
        ),
        None => {
            let fraction =
                if view.is_empty() { 1.0 } else { counts.tested() as f64 / view.len() as f64 };
            ("idle", fraction, 0, view.len(), 0, 0)
        }
    };

    Snapshot {
        scanning,
        phase: phase.to_string(),
        fraction,
        sectors_done: done,
        sectors_total: total,
        current_lba: lba,
        defects_found: defects,
        buckets,
        approved_marks,
        counts,
        sector_size,
        capacity_bytes: view.len() * sector_size as u64,
        report: report.map(|r| ReportView::from(r, sector_size)),
    }
}

#[derive(Serialize, Clone)]
struct PartitionView {
    label: String,
    role: String,
    start_lba: u64,
    sectors: u64,
    size_bytes: u64,
    mbr_type: String,
    offset_percent: f64,
    length_percent: f64,
}

#[derive(Serialize, Clone)]
struct PlanView {
    index: usize,
    strategy: String,
    usable_bytes: u64,
    /// Approved area the guard band and alignment keep out of the partition.
    /// Zero for the splicing strategy, which gives up nothing — and a sentence
    /// explaining a cost of zero contradicts itself, so the window needs the
    /// figure and not merely its label.
    sacrificed_bytes: u64,
    partitions: Vec<PartitionView>,
}

impl PlanView {
    fn from(index: usize, plan: &PartitionPlan) -> Self {
        let total = plan.geometry.total_sectors().max(1) as f64;
        Self {
            index,
            strategy: plan.strategy.kind().to_string(),
            usable_bytes: plan.usable_bytes,
            sacrificed_bytes: plan.sacrificed_bytes,
            partitions: plan
                .partitions
                .iter()
                .map(|p| PartitionView {
                    label: p.label.clone(),
                    role: match p.role {
                        PartitionRole::Data => "data".into(),
                        PartitionRole::Quarantine => "quarantine".into(),
                    },
                    start_lba: p.range.start(),
                    sectors: p.range.len(),
                    size_bytes: p.byte_len(&plan.geometry),
                    mbr_type: format!("{:#04x}", p.mbr_type),
                    offset_percent: p.range.start() as f64 / total * 100.0,
                    length_percent: p.range.len() as f64 / total * 100.0,
                })
                .collect(),
        }
    }
}

// -------------------------------------------------------------------- estado

#[derive(Default)]
struct AppState {
    devices: Vec<DeviceInfo>,
    selected: Option<DeviceInfo>,
    map: Option<SectorMap>,
    baseline: Option<SectorMap>,
    report: Option<HealthReport>,
    plans: Vec<PartitionPlan>,
    consent: Option<DestructiveConsent>,
    cancel: CancellationToken,
    scanning: bool,
    /// Layout found on the selected card, read from its own partition table.
    prior: Option<PriorLayout>,
    /// Interval the inspection and the window are about. `None` is the whole
    /// device, which is the case for any card this program has not fenced.
    view_span: Option<salvage_core::LbaRange>,
}

type Shared = Arc<Mutex<AppState>>;

/// Observer that downsamples the map and pushes progress to the window.
struct WindowObserver {
    app: AppHandle,
    /// Interval the window is drawing. See [`build_snapshot`].
    view: Option<salvage_core::LbaRange>,
    last_emit: Instant,
    emitted: u64,
    last_logged_percent: i64,
    defects_seen: u64,
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

        let snapshot = build_snapshot(map, Some(progress), None, true, self.view);
        match self.app.emit("scan:progress", snapshot) {
            Ok(()) => self.emitted += 1,
            // If the event never reaches the window, the interface sits still
            // while the scan runs normally. That needs to be known.
            Err(e) => log(&format!("FALHA ao emitir progresso: {e}")),
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
enum ScanEnd {
    /// The user asked for it.
    Cancelled,
    /// Something went wrong. Carries the text for the log.
    Failed(String),
}

/// Failures the window has wording for, in every language it speaks.
///
/// A command's `Err` is a lookup key, not a sentence. The alternative was
/// Portuguese assembled here, which no amount of translating the window could
/// ever have reached. Anything without a key still arrives as its own text —
/// the window's lookup falls through to whatever it was handed — so a rare
/// technical failure gets reported verbatim instead of swallowed.
mod err {
    pub const STATE: &str = "err.state";
    pub const ENUMERATE: &str = "err.enumerate";
    pub const DEVICE_GONE: &str = "err.device_gone";
    pub const SCAN_RUNNING: &str = "err.scan_running";
    pub const NO_DEVICE: &str = "err.no_device";
    pub const NO_MAP: &str = "err.no_map";
    pub const NO_PLAN: &str = "err.no_plan";
    pub const PLAN_REJECTED: &str = "err.plan_rejected";
    pub const NAME_MISMATCH: &str = "err.name_mismatch";
    pub const DEVICE_BLOCKED: &str = "err.device_blocked";
    pub const OPEN_FAILED: &str = "err.open_failed";
    pub const OPEN_LOG: &str = "err.open_log";
    pub const SCAN_NOT_WRITABLE: &str = "err.scan.not_writable";
    pub const SCAN_ALL_WRITES_FAILED: &str = "err.scan.all_writes_failed";
}

/// Turns a consent refusal into a key, leaving the detail for the log.
fn consent_key(e: &salvage_app::safety::ConfirmationError) -> &'static str {
    use salvage_app::safety::ConfirmationError;
    match e {
        ConfirmationError::DeviceBlocked => err::DEVICE_BLOCKED,
        ConfirmationError::NameMismatch { .. } => err::NAME_MISMATCH,
    }
}

// ------------------------------------------------------------------ comandos

#[tauri::command]
fn list_devices(state: State<'_, Shared>) -> Result<Vec<DeviceView>, String> {
    let devices = WindowsDeviceEnumerator::new().enumerate().map_err(|e| {
        log(&format!("FALHA ao enumerar: {e}"));
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
            if view.verdict != "blocked" {
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
    Ok(view)
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

            let mut observer = WindowObserver {
                app: app.clone(),
                view: config.range,
                last_emit: Instant::now(),
                emitted: 0,
                last_logged_percent: -100,
                defects_seen: 0,
            };
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
                let snapshot = build_snapshot(&map, None, Some(&report), false, config.range);
                // This pass becomes the next one's baseline, which is what
                // distinguishes a stable defect from active degradation.
                guard.baseline = Some(map.clone());
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
                log(&format!("ERRO: {message}"));
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

    let outcome = apply_plan(&device, &plan, &map, &consent, filesystem, &label)
        .map_err(|e| e.to_string())?;

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
    Ok(guard
        .map
        .as_ref()
        .map(|m| build_snapshot(m, None, guard.report.as_ref(), guard.scanning, guard.view_span)))
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
const AUTHOR_URL: &str = "https://x.com/omtstm";

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
    fn state_codes_are_distinct() {
        let codes: std::collections::HashSet<u8> =
            SectorState::ALL.iter().map(|s| state_code(*s)).collect();
        assert_eq!(codes.len(), SectorState::ALL.len());
    }

    #[test]
    fn session_nonce_is_never_zero() {
        assert_ne!(session_nonce(), 0);
    }
}
