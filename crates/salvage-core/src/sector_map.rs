//! Per-sector state map for a device.
//!
//! A 128 GB card holds 268 million sectors. Storing one byte per sector would
//! cost 268 MB of RAM to describe, almost always, "everything is fine". The map
//! is therefore run-length encoded: an ordered list of contiguous, disjoint
//! intervals covering exactly `[0, total_sectors)`. A healthy card fits in a
//! single entry; a worn one, in a few dozen.
//!
//! Three invariants are maintained and checked by
//! [`SectorMap::check_invariants`]: total coverage with no gaps, no overlap,
//! and no two adjacent runs sharing a state (full coalescing).

use serde::{Deserialize, Serialize};

use crate::geometry::{DeviceGeometry, LbaRange};

/// Verification state of a sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectorState {
    /// Not inspected yet.
    Untested,
    /// Written and read back exactly.
    Good,
    /// A read failed with an I/O error.
    BadRead,
    /// A write failed with an I/O error.
    BadWrite,
    /// Read without error, but returned content differing from what was written.
    Corrupt,
    /// Returned another LBA's intact content: the address does not exist.
    Aliased,
    /// Withheld from data without being a proven defect.
    ///
    /// Two things arrive here, and they share every consequence: a healthy
    /// sector condemned for sitting too close to a defect (the guard band),
    /// and a sector an earlier layout already fenced off, read back from the
    /// card's own partition table. Neither may hold data and neither is
    /// evidence of a defect at that address, which is exactly what this state
    /// means. What it must never be read as is *proof of health*: it is not
    /// [`SectorState::Good`] and never counts as usable.
    Fenced,
}

impl SectorState {
    /// Sectors that may safely hold user data.
    ///
    /// Only [`SectorState::Good`] qualifies — a sector whose content was
    /// written and read back intact. `Untested` never does: absence of proof
    /// is not proof of absence.
    #[inline]
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Good)
    }

    /// Sectors with a proven defect.
    #[inline]
    pub const fn is_defective(&self) -> bool {
        matches!(self, Self::BadRead | Self::BadWrite | Self::Corrupt | Self::Aliased)
    }

    /// Whether this state represents an actually conclusive verification.
    ///
    /// `Untested` concludes nothing; the diagnosis uses this to refuse a health
    /// verdict over a scan that did not cover the card.
    #[inline]
    pub const fn is_conclusive(&self) -> bool {
        !matches!(self, Self::Untested)
    }

    /// Severity ordering, used to pick a block's dominant colour when the map
    /// is downsampled for display.
    ///
    /// The worst state present always wins, so a single bad sector is never
    /// hidden behind thousands of healthy neighbours.
    #[inline]
    pub const fn severity(&self) -> u8 {
        match self {
            Self::Good => 0,
            Self::Untested => 1,
            Self::Fenced => 2,
            Self::BadRead => 3,
            Self::BadWrite => 4,
            Self::Corrupt => 5,
            Self::Aliased => 6,
        }
    }

    /// Stable identifier for serialization and for the presentation layer.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Untested => "untested",
            Self::Good => "good",
            Self::BadRead => "bad_read",
            Self::BadWrite => "bad_write",
            Self::Corrupt => "corrupt",
            Self::Aliased => "aliased",
            Self::Fenced => "fenced",
        }
    }

    /// Every state, in increasing order of severity.
    pub const ALL: [SectorState; 7] = [
        Self::Good,
        Self::Untested,
        Self::Fenced,
        Self::BadRead,
        Self::BadWrite,
        Self::Corrupt,
        Self::Aliased,
    ];
}

/// A contiguous run of sectors sharing one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Interval covered.
    pub range: LbaRange,
    /// State common to every sector in the interval.
    pub state: SectorState,
}

/// Record of a sector that returned another address's content.
///
/// The collection of these observations is what allows the true capacity of a
/// card lying about its size to be reconstructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasObservation {
    /// The LBA that was requested from the device.
    pub requested_lba: u64,
    /// The LBA whose content the device actually returned.
    pub actual_lba: u64,
}

impl AliasObservation {
    /// Distance between the requested and the returned address.
    ///
    /// Under modular wrap-around the real capacity divides this difference.
    /// The scan writes back-to-front precisely so the requested address is
    /// always the larger of the two, but the distance is taken as an absolute
    /// value so the estimate stays correct for observations from other sources.
    #[inline]
    pub const fn stride(&self) -> u64 {
        self.requested_lba.abs_diff(self.actual_lba)
    }
}

/// Sector counts per state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorCounts {
    /// Sectors never inspected.
    pub untested: u64,
    /// Sectors verified good.
    pub good: u64,
    /// Sectors that failed to read.
    pub bad_read: u64,
    /// Sectors that failed to write.
    pub bad_write: u64,
    /// Sectors that returned corrupted content.
    pub corrupt: u64,
    /// Sectors that do not exist and were served by aliasing.
    pub aliased: u64,
    /// Healthy sectors condemned by proximity to a defect.
    pub fenced: u64,
}

impl SectorCounts {
    /// Sum of every defective state.
    #[inline]
    pub const fn defective(&self) -> u64 {
        self.bad_read + self.bad_write + self.corrupt + self.aliased
    }

    /// Total sectors accounted for.
    #[inline]
    pub const fn total(&self) -> u64 {
        self.untested + self.good + self.fenced + self.defective()
    }

    /// Sectors already inspected.
    #[inline]
    pub const fn tested(&self) -> u64 {
        self.total() - self.untested
    }

    /// Adds `sectors` to the tally for the given state.
    fn add(&mut self, state: SectorState, sectors: u64) {
        let slot = match state {
            SectorState::Untested => &mut self.untested,
            SectorState::Good => &mut self.good,
            SectorState::BadRead => &mut self.bad_read,
            SectorState::BadWrite => &mut self.bad_write,
            SectorState::Corrupt => &mut self.corrupt,
            SectorState::Aliased => &mut self.aliased,
            SectorState::Fenced => &mut self.fenced,
        };
        *slot = slot.saturating_add(sectors);
    }
}

/// Summary of one display block after downsampling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapBucket {
    /// LBA interval this block represents.
    pub range: LbaRange,
    /// Worst state present in the interval; determines the block's colour.
    pub dominant: SectorState,
    /// Detailed tally, for tooltips and accessible fallbacks.
    pub counts: SectorCounts,
}

/// Structural invariant violations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MapInvariantError {
    /// The map holds no runs at all.
    #[error("map contains no runs")]
    Empty,
    /// The first run does not begin at LBA 0.
    #[error("map starts at LBA {0}, expected 0")]
    DoesNotStartAtZero(u64),
    /// A gap or overlap exists between consecutive runs.
    #[error("discontinuity: run ends at {prev_end} but the next starts at {next_start}")]
    Discontinuity {
        /// End of the preceding run.
        prev_end: u64,
        /// Start of the following run.
        next_start: u64,
    },
    /// Coverage does not reach the end of the device.
    #[error("map ends at {actual}, expected {expected}")]
    DoesNotCoverDevice {
        /// Where coverage actually stops.
        actual: u64,
        /// Where it should stop.
        expected: u64,
    },
    /// Two neighbouring runs share a state and were not coalesced.
    #[error("adjacent runs at {at} were not coalesced (both {state:?})")]
    NotCoalesced {
        /// Boundary where it occurs.
        at: u64,
        /// The repeated state.
        state: SectorState,
    },
    /// An empty run survived an operation.
    #[error("empty run at {0}")]
    EmptyRun(u64),
}

/// Complete state map for a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorMap {
    geometry: DeviceGeometry,
    runs: Vec<Run>,
    aliases: Vec<AliasObservation>,
}

impl SectorMap {
    /// Creates a map with the whole device marked as uninspected.
    pub fn new(geometry: DeviceGeometry) -> Self {
        Self {
            geometry,
            runs: vec![Run { range: geometry.full_range(), state: SectorState::Untested }],
            aliases: Vec::new(),
        }
    }

    /// Geometry of the mapped device.
    #[inline]
    pub const fn geometry(&self) -> &DeviceGeometry {
        &self.geometry
    }

    /// Runs making up the map, in ascending LBA order.
    #[inline]
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Accumulated aliasing observations.
    #[inline]
    pub fn aliases(&self) -> &[AliasObservation] {
        &self.aliases
    }

    /// Marks an interval with a state, preserving all invariants.
    ///
    /// The interval is clipped to the device bounds; marks past the end of the
    /// media are ignored rather than causing a panic.
    pub fn mark(&mut self, range: LbaRange, state: SectorState) {
        let range = range.clamped_to(&self.geometry.full_range());
        if range.is_empty() {
            return;
        }

        let mut next: Vec<Run> = Vec::with_capacity(self.runs.len() + 2);
        for run in &self.runs {
            let (left, right) = run.range.subtract(&range);
            for part in [left, right].into_iter().flatten() {
                if !part.is_empty() {
                    next.push(Run { range: part, state: run.state });
                }
            }
        }
        next.push(Run { range, state });
        next.sort_unstable_by_key(|r| r.range.start());
        self.runs = coalesce(next);
    }

    /// Marks a single sector.
    #[inline]
    pub fn mark_sector(&mut self, lba: u64, state: SectorState) {
        self.mark(LbaRange::new(lba, 1), state);
    }

    /// Marks everything outside `span` as withheld from data.
    ///
    /// For an inspection deliberately narrowed to part of the device. Sectors
    /// nobody looked at are [`SectorState::Untested`], and untested rightly
    /// denies the run any verdict about the card — but area left out on
    /// purpose was not skipped out of ignorance. Calling it untested does two
    /// wrong things at once: it turns a narrowed inspection into one that
    /// proved nothing, and it leaves the planner free to hand that area out to
    /// a partition wherever the untested policy is relaxed.
    ///
    /// [`SectorState::Fenced`] is the honest word for it: not usable, and not
    /// evidence of a defect at that address either. What this can never do is
    /// approve anything — only a sector written and read back intact reaches
    /// [`SectorState::Good`], and nothing inside `span` is touched here.
    pub fn withhold_outside(&mut self, span: LbaRange) {
        let full = self.geometry.full_range();
        if span.start() > full.start() {
            self.mark(LbaRange::from_bounds(full.start(), span.start()), SectorState::Fenced);
        }
        if span.end() < full.end() {
            self.mark(LbaRange::from_bounds(span.end(), full.end()), SectorState::Fenced);
        }
    }

    /// Records an aliasing observation and marks the sector accordingly.
    pub fn record_alias(&mut self, requested_lba: u64, actual_lba: u64) {
        self.aliases.push(AliasObservation { requested_lba, actual_lba });
        self.mark_sector(requested_lba, SectorState::Aliased);
    }

    /// State of a specific LBA, or `None` past the end of the device.
    pub fn state_at(&self, lba: u64) -> Option<SectorState> {
        let idx = self
            .runs
            .binary_search_by(|r| {
                if r.range.end() <= lba {
                    std::cmp::Ordering::Less
                } else if r.range.start() > lba {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()?;
        Some(self.runs[idx].state)
    }

    /// Intervals in a specific state.
    pub fn ranges_with_state(&self, state: SectorState) -> impl Iterator<Item = LbaRange> + '_ {
        self.runs.iter().filter(move |r| r.state == state).map(|r| r.range)
    }

    /// Intervals with a proven defect.
    pub fn defective_ranges(&self) -> impl Iterator<Item = LbaRange> + '_ {
        self.runs.iter().filter(|r| r.state.is_defective()).map(|r| r.range)
    }

    /// Intervals approved for use.
    pub fn usable_ranges(&self) -> impl Iterator<Item = LbaRange> + '_ {
        self.runs.iter().filter(|r| r.state.is_usable()).map(|r| r.range)
    }

    /// Sector counts per state.
    pub fn counts(&self) -> SectorCounts {
        self.counts_in(self.geometry.full_range())
    }

    /// The unbroken defective span touching `lba`, if there is one.
    ///
    /// Runs of different defective states are one span here: a stretch that
    /// failed to read and then returned altered content is not two findings to
    /// someone watching a progress bar, it is one region where nothing answers.
    ///
    /// Both sides of `lba` are considered because the two phases travel in
    /// opposite directions — writing runs back to front, verification front to
    /// back — so the sectors just examined lie above the frontier in one and
    /// below it in the other.
    ///
    /// This measures what is behind, never what is ahead. Nothing here licenses
    /// a claim about area that was not examined.
    pub fn defective_run_at(&self, lba: u64) -> Option<LbaRange> {
        let at = |probe: u64| {
            self.runs.iter().position(|r| r.range.contains(probe) && r.state.is_defective())
        };
        let seed = at(lba).or_else(|| at(lba.checked_sub(1)?))?;

        // Runs tile the device without gaps, so walking the list is walking the
        // addresses.
        let mut first = seed;
        while first > 0 && self.runs[first - 1].state.is_defective() {
            first -= 1;
        }
        let mut last = seed;
        while last + 1 < self.runs.len() && self.runs[last + 1].state.is_defective() {
            last += 1;
        }
        Some(LbaRange::from_bounds(self.runs[first].range.start(), self.runs[last].range.end()))
    }

    /// Tallies states inside one interval of the device.
    ///
    /// For a view that is about part of the card rather than all of it. The
    /// tally then describes what is on screen, which is what the legend beside
    /// it claims to be describing.
    pub fn counts_in(&self, range: LbaRange) -> SectorCounts {
        let range = range.clamped_to(&self.geometry.full_range());
        let mut c = SectorCounts::default();
        for run in &self.runs {
            if run.range.start() >= range.end() {
                break;
            }
            if let Some(overlap) = run.range.intersection(&range) {
                c.add(run.state, overlap.len());
            }
        }
        c
    }

    /// Fraction of the device already inspected, from 0.0 to 1.0.
    pub fn progress(&self) -> f64 {
        let total = self.geometry.total_sectors();
        if total == 0 {
            return 1.0;
        }
        self.counts().tested() as f64 / total as f64
    }

    /// Reduces the map to `buckets` uniform blocks for display.
    ///
    /// Each block reports the worst state it contains, so a single defective
    /// sector stays visible even when it occupies a minuscule fraction of the
    /// block's area.
    pub fn downsample(&self, buckets: usize) -> Vec<MapBucket> {
        self.downsample_range(self.geometry.full_range(), buckets)
    }

    /// Downsamples one interval of the device into `buckets` blocks.
    ///
    /// For a re-inspection restricted to the area an earlier layout left in
    /// use: the picture covers that area and the ruler beside it measures that
    /// area. A picture spanning the whole card under a ruler measuring part of
    /// it would put every label next to the wrong place, which is worse than
    /// either scale on its own.
    pub fn downsample_range(&self, range: LbaRange, buckets: usize) -> Vec<MapBucket> {
        let range = range.clamped_to(&self.geometry.full_range());
        let span = range.len();
        let buckets = buckets.max(1).min(span.max(1) as usize);
        let per = span.div_ceil(buckets as u64).max(1);

        let mut out = Vec::with_capacity(buckets);
        // Runs are ordered, so the search for each bucket can resume where the
        // previous one stopped instead of rescanning from the beginning.
        let mut cursor = 0usize;

        for i in 0..buckets {
            let start = range.start() + (i as u64).saturating_mul(per);
            if start >= range.end() {
                break;
            }
            let bucket_range = LbaRange::from_bounds(start, (start + per).min(range.end()));

            let mut counts = SectorCounts::default();
            let mut dominant = SectorState::Good;
            let mut seen_any = false;

            while cursor < self.runs.len() && self.runs[cursor].range.end() <= bucket_range.start()
            {
                cursor += 1;
            }
            for run in &self.runs[cursor..] {
                if run.range.start() >= bucket_range.end() {
                    break;
                }
                if let Some(overlap) = run.range.intersection(&bucket_range) {
                    counts.add(run.state, overlap.len());
                    if !seen_any || run.state.severity() > dominant.severity() {
                        dominant = run.state;
                        seen_any = true;
                    }
                }
            }

            out.push(MapBucket { range: bucket_range, dominant, counts });
        }
        out
    }

    /// Verifies the map's structural invariants.
    ///
    /// Called from tests and debug assertions. An inconsistent map would lead
    /// to an incorrect partition plan and, in the worst case, to a write over
    /// the very area the plan meant to protect.
    pub fn check_invariants(&self) -> Result<(), MapInvariantError> {
        let first = self.runs.first().ok_or(MapInvariantError::Empty)?;
        if first.range.start() != 0 {
            return Err(MapInvariantError::DoesNotStartAtZero(first.range.start()));
        }
        for run in &self.runs {
            if run.range.is_empty() {
                return Err(MapInvariantError::EmptyRun(run.range.start()));
            }
        }
        for w in self.runs.windows(2) {
            if w[0].range.end() != w[1].range.start() {
                return Err(MapInvariantError::Discontinuity {
                    prev_end: w[0].range.end(),
                    next_start: w[1].range.start(),
                });
            }
            if w[0].state == w[1].state {
                return Err(MapInvariantError::NotCoalesced {
                    at: w[0].range.end(),
                    state: w[0].state,
                });
            }
        }
        let end = self.runs.last().expect("non-empty").range.end();
        if end != self.geometry.total_sectors() {
            return Err(MapInvariantError::DoesNotCoverDevice {
                actual: end,
                expected: self.geometry.total_sectors(),
            });
        }
        Ok(())
    }
}

/// Merges adjacent runs that share a state.
fn coalesce(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for run in runs {
        if run.range.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.state == run.state && last.range.end() == run.range.start() => {
                last.range = LbaRange::from_bounds(last.range.start(), run.range.end());
            }
            _ => out.push(run),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(total: u64) -> SectorMap {
        SectorMap::new(DeviceGeometry::new(512, total).unwrap())
    }

    /// Different defective states in a row are one region to someone waiting,
    /// not three findings. The span has to merge them or the figure offered to
    /// the user would be a fraction of what actually failed.
    #[test]
    fn adjacent_defects_of_different_kinds_are_one_span() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 2_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(2_000, 3_000), SectorState::Corrupt);
        m.mark(LbaRange::from_bounds(3_000, 3_500), SectorState::BadRead);
        m.mark(LbaRange::from_bounds(3_500, 6_000), SectorState::Corrupt);
        m.mark(LbaRange::from_bounds(6_000, 7_000), SectorState::Good);

        let span = m.defective_run_at(4_000).expect("inside the damage");
        assert_eq!(span, LbaRange::from_bounds(2_000, 6_000));
        assert_eq!(span.len(), 4_000);
    }

    /// Verification runs front to back, so the frontier sits just past the
    /// sector last examined. Asking about the frontier has to find the span
    /// behind it, or the offer to stop would never appear.
    #[test]
    fn the_span_is_found_from_the_frontier_just_past_it() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 1_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(1_000, 5_000), SectorState::Corrupt);

        assert_eq!(m.defective_run_at(5_000), Some(LbaRange::from_bounds(1_000, 5_000)));
    }

    #[test]
    fn good_ground_reports_no_span() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 4_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(4_000, 5_000), SectorState::Corrupt);
        m.mark(LbaRange::from_bounds(5_000, 10_000), SectorState::Good);

        assert!(m.defective_run_at(2_000).is_none(), "no defect anywhere near 2000");
        assert!(m.defective_run_at(8_000).is_none(), "8000 is good, and 7999 with it");
    }

    /// Untested is not a defect. Treating it as one would let the span grow
    /// through area nobody has looked at, which is the claim this whole
    /// program refuses to make.
    #[test]
    fn unexamined_area_never_extends_the_span() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 1_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(1_000, 3_000), SectorState::Corrupt);
        // 3_000..10_000 is left untested.

        assert_eq!(m.defective_run_at(2_000), Some(LbaRange::from_bounds(1_000, 3_000)));
        assert!(m.defective_run_at(9_000).is_none());
    }

    #[test]
    fn a_span_running_to_the_end_of_the_card_is_reported_whole() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 6_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(6_000, 10_000), SectorState::Corrupt);

        assert_eq!(m.defective_run_at(9_999), Some(LbaRange::from_bounds(6_000, 10_000)));
    }

    #[test]
    fn a_new_map_is_one_untested_run() {
        let m = map(1000);
        assert_eq!(m.runs().len(), 1);
        assert_eq!(m.runs()[0].state, SectorState::Untested);
        assert_eq!(m.counts().untested, 1000);
        assert_eq!(m.progress(), 0.0);
        m.check_invariants().unwrap();
    }

    #[test]
    fn marking_the_middle_splits_into_three_runs() {
        let mut m = map(1000);
        m.mark(LbaRange::from_bounds(400, 600), SectorState::BadRead);
        assert_eq!(m.runs().len(), 3);
        assert_eq!(m.state_at(399), Some(SectorState::Untested));
        assert_eq!(m.state_at(400), Some(SectorState::BadRead));
        assert_eq!(m.state_at(599), Some(SectorState::BadRead));
        assert_eq!(m.state_at(600), Some(SectorState::Untested));
        m.check_invariants().unwrap();
    }

    #[test]
    fn adjacent_equal_states_are_coalesced() {
        let mut m = map(1000);
        m.mark(LbaRange::from_bounds(0, 500), SectorState::Good);
        m.mark(LbaRange::from_bounds(500, 1000), SectorState::Good);
        assert_eq!(m.runs().len(), 1, "the two halves should merge into one run");
        assert_eq!(m.counts().good, 1000);
        m.check_invariants().unwrap();
    }

    #[test]
    fn overwriting_a_region_replaces_the_previous_state() {
        let mut m = map(1000);
        m.mark(LbaRange::from_bounds(100, 900), SectorState::BadRead);
        m.mark(LbaRange::from_bounds(0, 1000), SectorState::Good);
        assert_eq!(m.runs().len(), 1);
        assert_eq!(m.counts().good, 1000);
        assert_eq!(m.counts().bad_read, 0);
        m.check_invariants().unwrap();
    }

    #[test]
    fn a_mark_spanning_several_runs_absorbs_them_all() {
        let mut m = map(1000);
        m.mark(LbaRange::from_bounds(100, 200), SectorState::BadRead);
        m.mark(LbaRange::from_bounds(300, 400), SectorState::Corrupt);
        m.mark(LbaRange::from_bounds(500, 600), SectorState::BadWrite);
        assert!(m.runs().len() > 3);
        m.mark(LbaRange::from_bounds(150, 550), SectorState::Good);
        m.check_invariants().unwrap();
        assert_eq!(m.state_at(350), Some(SectorState::Good));
        assert_eq!(m.state_at(120), Some(SectorState::BadRead));
        assert_eq!(m.state_at(580), Some(SectorState::BadWrite));
    }

    #[test]
    fn marks_are_clipped_to_the_device_instead_of_panicking() {
        let mut m = map(1000);
        m.mark(LbaRange::from_bounds(900, 5000), SectorState::Good);
        m.check_invariants().unwrap();
        assert_eq!(m.counts().good, 100);
        assert_eq!(m.counts().total(), 1000);

        m.mark(LbaRange::from_bounds(2000, 3000), SectorState::BadRead);
        m.check_invariants().unwrap();
        assert_eq!(m.counts().bad_read, 0);
    }

    #[test]
    fn empty_marks_are_a_no_op() {
        let mut m = map(1000);
        let before = m.clone();
        m.mark(LbaRange::from_bounds(500, 500), SectorState::Good);
        assert_eq!(m, before);
    }

    /// A narrowed inspection has to come back with the area it skipped marked
    /// as withheld — never untested, which would deny the run any verdict, and
    /// never approved, which would hand condemned space to a partition.
    #[test]
    fn area_outside_the_inspected_span_is_withheld_and_never_approved() {
        let geometry = DeviceGeometry::new(512, 200_000).unwrap();
        let mut map = SectorMap::new(geometry);
        let span = LbaRange::from_bounds(8192, 100_000);
        map.mark(span, SectorState::Good);

        map.withhold_outside(span);

        let counts = map.counts();
        assert_eq!(counts.untested, 0, "nothing outside the span may stay untested");
        assert_eq!(counts.good, span.len(), "no sector outside the span became approved");
        assert_eq!(counts.fenced, 200_000 - span.len());
        assert_eq!(map.state_at(0), Some(SectorState::Fenced));
        assert_eq!(map.state_at(8191), Some(SectorState::Fenced));
        assert_eq!(map.state_at(8192), Some(SectorState::Good));
        assert_eq!(map.state_at(99_999), Some(SectorState::Good));
        assert_eq!(map.state_at(100_000), Some(SectorState::Fenced));
        assert_eq!(map.state_at(199_999), Some(SectorState::Fenced));
        assert!(map.check_invariants().is_ok());
    }

    #[test]
    fn a_span_covering_the_device_withholds_nothing() {
        let geometry = DeviceGeometry::new(512, 200_000).unwrap();
        let mut map = SectorMap::new(geometry);
        let full = geometry.full_range();
        map.mark(full, SectorState::Good);

        map.withhold_outside(full);

        assert_eq!(map.counts().fenced, 0);
        assert_eq!(map.counts().good, 200_000);
    }

    /// Defects found inside the span survive: withholding describes what was
    /// left out, and must not repaint what was measured.
    #[test]
    fn withholding_leaves_every_measurement_inside_the_span_alone() {
        let geometry = DeviceGeometry::new(512, 200_000).unwrap();
        let mut map = SectorMap::new(geometry);
        let span = LbaRange::from_bounds(8192, 100_000);
        map.mark(span, SectorState::Good);
        map.mark(LbaRange::from_bounds(50_000, 50_100), SectorState::BadRead);

        map.withhold_outside(span);

        assert_eq!(map.counts().bad_read, 100);
        assert_eq!(map.state_at(50_050), Some(SectorState::BadRead));
    }

    /// The view of a re-inspected card has to cover the area in use and stop
    /// there: the first bucket starts where the area starts, the last one ends
    /// where it ends, and the ruler drawn from the same figures lands on it.
    #[test]
    fn a_ranged_downsample_covers_exactly_that_range() {
        let mut map = SectorMap::new(DeviceGeometry::new(512, 200_000).unwrap());
        let span = LbaRange::from_bounds(8192, 100_000);
        map.mark(span, SectorState::Good);
        map.withhold_outside(span);

        let buckets = map.downsample_range(span, 64);
        assert_eq!(buckets.first().unwrap().range.start(), span.start());
        assert_eq!(buckets.last().unwrap().range.end(), span.end());
        // Nothing withheld leaks into a picture that claims to be the area in
        // use — that is the whole reason for drawing the range and not the card.
        assert!(
            buckets.iter().all(|b| b.dominant == SectorState::Good),
            "a bucket outside the inspected area reached the view"
        );
    }

    #[test]
    fn a_ranged_downsample_leaves_no_gap_in_its_range() {
        let map = SectorMap::new(DeviceGeometry::new(512, 200_000).unwrap());
        let span = LbaRange::from_bounds(8192, 100_000);
        let buckets = map.downsample_range(span, 97);
        for pair in buckets.windows(2) {
            assert_eq!(pair[0].range.end(), pair[1].range.start(), "gap between buckets");
        }
    }

    #[test]
    fn a_range_covering_the_device_downsamples_like_the_whole_device() {
        let mut map = SectorMap::new(DeviceGeometry::new(512, 40_000).unwrap());
        map.mark(LbaRange::from_bounds(1000, 1100), SectorState::Corrupt);
        let full = map.geometry().full_range();
        assert_eq!(map.downsample_range(full, 128), map.downsample(128));
    }

    #[test]
    fn counts_in_tallies_only_what_falls_inside_the_range() {
        let mut map = SectorMap::new(DeviceGeometry::new(512, 200_000).unwrap());
        let span = LbaRange::from_bounds(8192, 100_000);
        map.mark(span, SectorState::Good);
        map.withhold_outside(span);
        map.mark(LbaRange::from_bounds(50_000, 50_100), SectorState::BadRead);

        let counts = map.counts_in(span);
        assert_eq!(counts.total(), span.len(), "the tally must account for the range exactly");
        assert_eq!(counts.fenced, 0, "area outside the range is not part of this tally");
        assert_eq!(counts.bad_read, 100);
        assert_eq!(counts.good, span.len() - 100);
    }

    /// A range reaching past the end of the card is clamped, not trusted: the
    /// span an inspection uses comes off the media's own partition table.
    #[test]
    fn counts_in_clamps_a_range_that_overruns_the_device() {
        let map = SectorMap::new(DeviceGeometry::new(512, 1000).unwrap());
        let counts = map.counts_in(LbaRange::from_bounds(500, 999_999));
        assert_eq!(counts.total(), 500);
    }

    #[test]
    fn counts_always_sum_to_the_device_size() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 3000), SectorState::Good);
        m.mark(LbaRange::from_bounds(3000, 3100), SectorState::BadRead);
        m.mark(LbaRange::from_bounds(5000, 5050), SectorState::Corrupt);
        m.mark(LbaRange::from_bounds(9000, 9500), SectorState::Aliased);
        let c = m.counts();
        assert_eq!(c.total(), 10_000);
        assert_eq!(c.defective(), 100 + 50 + 500);
        m.check_invariants().unwrap();
    }

    #[test]
    fn only_verified_sectors_count_as_usable() {
        assert!(SectorState::Good.is_usable());
        assert!(!SectorState::Untested.is_usable());
        assert!(!SectorState::Fenced.is_usable());
    }

    #[test]
    fn recording_an_alias_marks_the_sector_and_keeps_the_evidence() {
        let mut m = map(10_000);
        m.record_alias(9000, 1000);
        assert_eq!(m.state_at(9000), Some(SectorState::Aliased));
        assert_eq!(m.aliases().len(), 1);
        assert_eq!(m.aliases()[0].stride(), 8000);
        m.check_invariants().unwrap();
    }

    #[test]
    fn state_at_returns_none_past_the_end_of_the_device() {
        let m = map(1000);
        assert_eq!(m.state_at(1000), None);
        assert_eq!(m.state_at(u64::MAX), None);
        assert_eq!(m.state_at(999), Some(SectorState::Untested));
    }

    #[test]
    fn downsample_covers_the_device_without_gaps() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 10_000), SectorState::Good);
        let b = m.downsample(100);
        assert_eq!(b.len(), 100);
        assert_eq!(b[0].range.start(), 0);
        assert_eq!(b.last().unwrap().range.end(), 10_000);
        for w in b.windows(2) {
            assert_eq!(w[0].range.end(), w[1].range.start());
        }
    }

    /// A single bad sector among thousands of healthy ones must stay visible:
    /// it is exactly what the user needs to see.
    #[test]
    fn a_single_bad_sector_dominates_its_bucket() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 10_000), SectorState::Good);
        m.mark_sector(5_050, SectorState::Corrupt);
        let buckets = m.downsample(100);
        let hit = &buckets[50];
        assert_eq!(hit.dominant, SectorState::Corrupt);
        assert_eq!(hit.counts.corrupt, 1);
        assert_eq!(hit.counts.good, 99);
        assert_eq!(buckets[49].dominant, SectorState::Good);
    }

    #[test]
    fn downsample_handles_more_buckets_than_sectors() {
        let m = map(10);
        let b = m.downsample(1000);
        assert!(!b.is_empty() && b.len() <= 10);
        assert_eq!(b.last().unwrap().range.end(), 10);
    }

    #[test]
    fn downsample_bucket_counts_match_the_bucket_size() {
        let mut m = map(9_999);
        m.mark(LbaRange::from_bounds(0, 4_000), SectorState::Good);
        for b in m.downsample(77) {
            assert_eq!(b.counts.total(), b.range.len());
        }
    }

    #[test]
    fn progress_tracks_the_inspected_fraction() {
        let mut m = map(1000);
        m.mark(LbaRange::from_bounds(0, 250), SectorState::Good);
        assert!((m.progress() - 0.25).abs() < 1e-9);
        m.mark(LbaRange::from_bounds(250, 1000), SectorState::Good);
        assert!((m.progress() - 1.0).abs() < 1e-9);
    }

    /// Bulk random marking must not be able to break the structure.
    #[test]
    fn invariants_survive_many_overlapping_marks() {
        let mut m = map(50_000);
        let states = SectorState::ALL;
        let mut seed = 0x1234_5678u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..500 {
            let a = rnd() % 50_000;
            let b = rnd() % 50_000;
            let range = LbaRange::from_bounds(a.min(b), a.max(b) + 1);
            m.mark(range, states[i % states.len()]);
            m.check_invariants().unwrap_or_else(|e| panic!("iteration {i}: {e}"));
        }
        assert_eq!(m.counts().total(), 50_000);
    }
}
