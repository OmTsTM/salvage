//! Everything the window is handed.
//!
//! The one place a domain type becomes a shape JSON can carry. Splitting it out
//! of `main.rs` draws the line the module documentation there always claimed:
//! the command surface decides *when* to answer, and this decides *what an
//! answer looks like*.
//!
//! Two rules hold throughout. Nothing here ships a sentence — the domain
//! returns classifications and numbers, the window owns the wording, and it
//! owns it in four languages. And nothing here decides anything; a view that
//! computed a verdict would be a rule living outside the layers that are
//! tested for it.

use salvage_app::device::{BlockDevice, DeviceInfo};
use salvage_app::history::CardRecord;
use salvage_app::safety::{evaluate, SafetyPolicy, SafetyVerdict};
use salvage_app::scan::{RetentionWindow, ScanPhase, ScanProgress};
use salvage_core::health::{FailureScenario, HealthReport};
use salvage_core::mbr::{MasterBootRecord, PriorLayout, MBR_SIZE};
use salvage_core::planner::{PartitionPlan, PartitionRole};
use salvage_core::sector_map::{SectorCounts, SectorMap, SectorState};
use salvage_win32::RawBlockDevice;
use serde::Serialize;

use crate::VIEW_BUCKETS;

/// Numeric code for a state, used in the visualization's compact vector.
pub fn state_code(state: SectorState) -> u8 {
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

#[derive(Serialize, Clone)]
pub struct DeviceView {
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
    pub prior: Option<PriorLayoutView>,
    /// What an earlier session measured about this card, if anything.
    pub remembered: Option<RememberedView>,
}

/// A summary of what is remembered, offered to the window.
///
/// The map itself stays in the backend. The window needs to say what is known
/// and how old it is; handing it millions of sectors to make that sentence
/// would be a strange way to say it.
#[derive(Serialize, Clone)]
pub struct RememberedView {
    /// Seconds since the inspection, or `None` when the clock disagrees.
    age_seconds: Option<u64>,
    approved_bytes: u64,
    defective_bytes: u64,
    /// Whether the card's own partition table was kept, and so whether
    /// releasing it can restore that rather than inventing one.
    can_restore_table: bool,
    /// Whether the approved area can be read back and compared.
    ///
    /// False for a record written before the pattern seed was stored. Without
    /// it there is no expected content for a sector, and the only honest thing
    /// is to not offer the check rather than run it against a guess.
    can_recheck: bool,
}

impl RememberedView {
    /// What the window needs to say about a stored record.
    ///
    /// Deliberately not the record itself: the map inside it is megabytes on a
    /// failing card, and nothing on this screen reads a single sector of it.
    pub fn from_record(record: &CardRecord) -> Self {
        let sector_size = record.map.geometry().sector_size() as u64;
        let counts = record.map.counts();
        Self {
            age_seconds: record.age_seconds(),
            approved_bytes: counts.good * sector_size,
            defective_bytes: counts.defective() * sector_size,
            can_restore_table: record.table_before.is_some(),
            can_recheck: record.pattern.is_some() && counts.good > 0,
        }
    }
}

/// An earlier layout, in the terms the window needs to talk about it.
#[derive(Serialize, Clone)]
pub struct PriorLayoutView {
    /// Sectors the next inspection will cover.
    inspect_sectors: u64,
    /// Sectors the earlier layout withheld from data.
    fenced_sectors: u64,
    /// Data partitions that layout left behind.
    data_partitions: usize,
}

impl PriorLayoutView {
    pub fn from(prior: &PriorLayout) -> Self {
        Self {
            inspect_sectors: prior.data_span().map_or(0, |r| r.len()),
            fenced_sectors: prior.quarantined.iter().map(|r| r.len()).sum(),
            data_partitions: prior.data.len(),
        }
    }
}

impl DeviceView {
    /// Whether safety refused this device outright.
    ///
    /// A method rather than a public field: the verdict is a classification the
    /// window words, and nothing outside should be comparing it to a string.
    pub fn is_blocked(&self) -> bool {
        self.verdict == "blocked"
    }

    pub fn from(device: &DeviceInfo, policy: &SafetyPolicy) -> Self {
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
                // A key rather than a sentence, like everything else crossing
                // this boundary: a volume with no letter is the one entry here
                // that needs wording, and the window owns the four languages.
                .map(|v| v.drive_letter.map_or("spec.unlettered".into(), |c| format!("{c}:")))
                .collect(),
            verdict,
            blocks,
            warnings,
            prior: None,
            remembered: None,
        }
    }
}

/// Reads back the layout an earlier inspection left on the card.
///
/// The handle is opened read-only and dropped straight away: this answers a
/// question *about* the card and must not be able to change it. `None` when
/// the table cannot be read or was not written by this program — and then the
/// inspection covers the whole device, as it always did.
pub fn read_prior_layout(device: &DeviceInfo) -> Option<PriorLayout> {
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
pub enum ReportDetail {
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
pub struct ReportView {
    /// Stable scenario identifier. The window maps it to wording; the domain
    /// deliberately ships no user-facing prose.
    scenario_kind: String,
    assurance: String,
    isolation_worthwhile: bool,
    /// Largest run with no detected defect, including uninspected area.
    largest_usable_bytes: u64,
    /// How long each sector waited between being written and being read back.
    ///
    /// `None` for a map adopted from a stored record, which was measured in a
    /// session this one knows nothing about. The window then says nothing
    /// rather than quoting an interval it cannot vouch for.
    retention: Option<RetentionWindow>,
    details: Vec<ReportDetail>,
}

impl ReportView {
    pub fn from(
        report: &HealthReport,
        sector_size: u32,
        retention: Option<RetentionWindow>,
    ) -> Self {
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
            retention,
            details,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
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
    /// The unbroken defective span the scan is currently inside, if any.
    ///
    /// A measurement of what is behind the frontier, never a claim about what
    /// lies ahead. The window uses it to offer the choice to stop early, and
    /// says in the same breath what stopping keeps and what it forgoes.
    dead_run: Option<DeadRunView>,
    /// Whether this map was measured in this session.
    ///
    /// A map adopted from a stored record describes the card as it was, and
    /// every write the window can offer is refused on one. The window needs to
    /// know before it offers, rather than after the user has typed the device
    /// name into a confirmation.
    verified_now: bool,
}

/// What a re-read of an already-inspected area found, days later.
#[derive(Serialize, Clone)]
pub struct RecheckView {
    /// How much was re-read.
    pub examined_bytes: u64,
    /// How much of it no longer holds what was written.
    pub lost_bytes: u64,
    /// How long the pass took.
    pub elapsed_secs: u64,
    /// How long the data had been sitting there, which is the whole point.
    pub age_seconds: Option<u64>,
    /// Whether every sector still held its content.
    pub held: bool,
}

/// An unbroken stretch where nothing came back intact.
#[derive(Serialize, Clone)]
pub struct DeadRunView {
    /// Where the stretch begins, as an offset into the card.
    start_bytes: u64,
    /// How much of it has been examined so far.
    bytes: u64,
}

pub fn build_snapshot(
    map: &SectorMap,
    progress: Option<&ScanProgress>,
    report: Option<&HealthReport>,
    scanning: bool,
    view: Option<salvage_core::LbaRange>,
    verified_now: bool,
    retention: Option<RetentionWindow>,
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
        report: report.map(|r| ReportView::from(r, sector_size, retention)),
        // Only while a scan is running: on a finished map the frontier is the
        // end of the card, and a span touching it would describe nothing the
        // user can still act on.
        dead_run: progress
            .filter(|_| scanning)
            .and_then(|p| map.defective_run_at(p.current_lba))
            .map(|r| DeadRunView {
                start_bytes: r.start() * sector_size as u64,
                bytes: r.len() * sector_size as u64,
            }),
        verified_now,
    }
}

#[derive(Serialize, Clone)]
pub struct PartitionView {
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
pub struct PlanView {
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
    pub fn from(index: usize, plan: &PartitionPlan) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The canvas tells states apart by this number alone. Two sharing one
    /// would paint a defect as approved area, or the reverse — and the list
    /// comes from the enum, so a state added later is covered without anyone
    /// remembering to add it here.
    #[test]
    fn state_codes_are_distinct() {
        let codes: std::collections::HashSet<u8> =
            SectorState::ALL.iter().map(|s| state_code(*s)).collect();
        assert_eq!(codes.len(), SectorState::ALL.len());
    }
}
