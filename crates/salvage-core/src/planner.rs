//! Partition layouts that fence defects away from user data.
//!
//! # The invariant the whole tool rests on
//!
//! No data partition may contain a sector that was not explicitly approved.
//! Excluding the sectors that failed is not enough: sectors that were never
//! inspected stay out too, because absence of proof is not proof of absence.
//! [`PartitionPlan::validate`] rejects any plan violating this, and the
//! application layer calls it again immediately before writing a single byte.
//!
//! # Why a guard band exists
//!
//! A defective sector is almost never alone. NAND erases in blocks of several
//! megabytes, and when one cell degrades its neighbours in the same erase block
//! are usually on the same path. Each condemned region is therefore dilated by
//! a margin and aligned outward to the erase-block boundary, so it swallows the
//! whole block containing it.

use serde::{Deserialize, Serialize};

use crate::fat32::{Fat32Image, Fat32Layout};
use crate::geometry::{DeviceGeometry, LbaRange};
use crate::sector_map::{SectorMap, SectorState};

/// Role of a partition in the final layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionRole {
    /// Visible, formatted area where the user stores files.
    Data,
    /// Condemned area, tagged with a type Windows will not mount.
    Quarantine,
}

/// Filesystem for the visible partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileSystem {
    /// exFAT: files larger than 4 GiB, universal support on modern media.
    ExFat,
    /// FAT32: maximum compatibility, 4 GiB per-file limit.
    Fat32,
}

impl FileSystem {
    /// MBR partition type byte.
    pub const fn mbr_type(&self) -> u8 {
        match self {
            // 0x07 covers IFS: NTFS and exFAT.
            Self::ExFat => 0x07,
            // 0x0C: FAT32 with LBA access.
            Self::Fat32 => 0x0C,
        }
    }

    /// Name accepted by the Windows format utility.
    pub const fn format_name(&self) -> &'static str {
        match self {
            Self::ExFat => "exFAT",
            Self::Fat32 => "FAT32",
        }
    }
}

/// MBR partition type used for quarantine.
///
/// `0xDA` means "non-filesystem data". Windows assigns no drive letter and does
/// not attempt to mount it, so the region disappears from Explorer. At the same
/// time the space shows as allocated to any partitioning tool, which stops
/// someone from creating a volume there by mistake.
pub const QUARANTINE_MBR_TYPE: u8 = 0xDA;

/// A planned partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedPartition {
    /// LBA interval occupied.
    pub range: LbaRange,
    /// Role of the partition.
    pub role: PartitionRole,
    /// Type byte written to the partition table.
    pub mbr_type: u8,
    /// Stable identifier used by the presentation layer.
    pub label: String,
}

impl PlannedPartition {
    /// Size in bytes.
    pub const fn byte_len(&self, g: &DeviceGeometry) -> u64 {
        self.range.byte_len(g)
    }
}

/// How a plan keeps user data off sectors that were not proven good.
///
/// Both variants deliver the same guarantee — nothing is ever written to a
/// sector the scan did not approve — but they enforce it at different layers,
/// and the layer decides what margin is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Containment {
    /// The partition boundary itself: every sector inside a data partition was
    /// proven good, and condemned regions are dilated by a guard band.
    PartitionBoundary,
    /// The filesystem's allocation table: the partition spans defects, and each
    /// cluster touching one is marked so no driver hands it out.
    FilesystemClusterMap,
}

/// Strategy for making use of the surviving space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// A single partition over the largest contiguous approved area.
    ///
    /// Yields less space, but a single drive letter and the layout that is
    /// easiest to verify by eye.
    LargestContiguous,
    /// Several partitions over the largest approved areas.
    ///
    /// Recovers more space at the cost of multiple drive letters.
    MaximumSpace,
    /// Like the first, with an enlarged guard band.
    ///
    /// Sacrifices usable space in exchange for greater distance between the
    /// data and any known defect.
    Conservative,
    /// One FAT32 volume spanning the defects, which its own table withholds.
    ///
    /// A partition is an interval, so fencing can only return one contiguous
    /// run and everything between the defects is lost. A filesystem has no such
    /// limit: clusters marked [`crate::fat32::BAD_CLUSTER`] are never allocated
    /// by any driver, so the good runs are spliced into a single drive letter
    /// whose free space is their sum.
    ///
    /// The cost is the guard band. Fencing keeps data a margin away from every
    /// known defect; a cluster map marks exactly the cells that failed and
    /// leaves their neighbours allocatable.
    SplicedFat32,
}

impl Strategy {
    /// Stable identifier for serialization and presentation lookup.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::LargestContiguous => "largest_contiguous",
            Self::MaximumSpace => "maximum_space",
            Self::Conservative => "conservative",
            Self::SplicedFat32 => "spliced_fat32",
        }
    }
}

/// Planning parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningPolicy {
    /// Margin, in sectors, applied around every condemned region.
    pub guard_band_sectors: u64,
    /// Boundary alignment, in sectors.
    pub alignment_sectors: u64,
    /// Minimum size of a data partition, in sectors.
    pub min_partition_sectors: u64,
    /// Maximum number of partition table entries.
    pub max_partitions: usize,
    /// Whether never-inspected sectors count as unsafe.
    ///
    /// Defaults to `true`. Turning it off is the most direct way to hand the
    /// user a partition with defects inside it.
    pub treat_untested_as_unsafe: bool,
}

impl PlanningPolicy {
    /// Default policy for a geometry: 8 MiB guard band, 4 MiB erase-block
    /// alignment, 8 MiB minimum partition.
    pub fn recommended_for(g: &DeviceGeometry) -> Self {
        let align = g.default_alignment_sectors();
        Self {
            guard_band_sectors: align * 2,
            alignment_sectors: align,
            min_partition_sectors: align * 2,
            max_partitions: 4,
            treat_untested_as_unsafe: true,
        }
    }

    /// Conservative variant: guard band multiplied by four.
    pub fn conservative_for(g: &DeviceGeometry) -> Self {
        let base = Self::recommended_for(g);
        Self { guard_band_sectors: base.guard_band_sectors * 4, ..base }
    }
}

/// Reasons a plan can be rejected.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    /// A data partition covers a sector that was not approved.
    #[error("data partition '{label}' covers LBA {lba}, whose state is {state:?}")]
    DataPartitionCoversUnsafeSector {
        /// Label of the offending partition.
        label: String,
        /// First problematic LBA found.
        lba: u64,
        /// State of that LBA.
        state: SectorState,
    },
    /// The filesystem's table cannot express the exclusion this area needs.
    #[error("data partition '{label}' cannot withhold its defects: {source}")]
    ClusterMapCannotContainDefects {
        /// Label of the offending partition.
        label: String,
        /// Why the image could not be built.
        #[source]
        source: crate::fat32::Fat32Error,
    },
    /// Two partitions overlap.
    #[error("partitions '{a}' and '{b}' overlap")]
    OverlappingPartitions {
        /// First partition.
        a: String,
        /// Second partition.
        b: String,
    },
    /// A partition extends past the end of the device.
    #[error("partition '{label}' ends at {end}, past the device's {total} sectors")]
    OutOfBounds {
        /// Label of the partition.
        label: String,
        /// End of the partition.
        end: u64,
        /// Device size.
        total: u64,
    },
    /// More partitions than the table can hold.
    #[error("{count} partitions exceed the maximum of {max}")]
    TooManyPartitions {
        /// Number planned.
        count: usize,
        /// The limit.
        max: usize,
    },
    /// An empty partition survived planning.
    #[error("partition '{0}' is empty")]
    EmptyPartition(String),
    /// A partition would sit on the partition table's own sector.
    #[error("partition '{label}' starts at LBA {start}, over the partition table")]
    OverlapsPartitionTable {
        /// Label of the partition.
        label: String,
        /// The invalid start.
        start: u64,
    },
}

/// Complete layout proposed for a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionPlan {
    /// Strategy that produced this plan.
    pub strategy: Strategy,
    /// Partitions, in ascending LBA order.
    pub partitions: Vec<PlannedPartition>,
    /// Geometry of the target device.
    pub geometry: DeviceGeometry,
    /// Total bytes usable by the user.
    pub usable_bytes: u64,
    /// Approved bytes discarded to margin, alignment, or table capacity.
    pub sacrificed_bytes: u64,
    /// Mechanism that keeps data off unapproved sectors.
    pub containment: Containment,
}

impl PartitionPlan {
    /// Partitions visible to the user.
    pub fn data_partitions(&self) -> impl Iterator<Item = &PlannedPartition> {
        self.partitions.iter().filter(|p| p.role == PartitionRole::Data)
    }

    /// Quarantine partitions.
    pub fn quarantine_partitions(&self) -> impl Iterator<Item = &PlannedPartition> {
        self.partitions.iter().filter(|p| p.role == PartitionRole::Quarantine)
    }

    /// Checks that the plan is safe and structurally valid.
    ///
    /// This is the last barrier before any write. It verifies, against every
    /// run in the map, that no data partition touches anything other than
    /// [`SectorState::Good`].
    pub fn validate(&self, map: &SectorMap, max_partitions: usize) -> Result<(), PlanError> {
        if self.partitions.len() > max_partitions {
            return Err(PlanError::TooManyPartitions {
                count: self.partitions.len(),
                max: max_partitions,
            });
        }

        let total = self.geometry.total_sectors();
        for p in &self.partitions {
            if p.range.is_empty() {
                return Err(PlanError::EmptyPartition(p.label.clone()));
            }
            if p.range.end() > total {
                return Err(PlanError::OutOfBounds {
                    label: p.label.clone(),
                    end: p.range.end(),
                    total,
                });
            }
            if p.range.start() == 0 {
                return Err(PlanError::OverlapsPartitionTable { label: p.label.clone(), start: 0 });
            }
        }

        for (i, a) in self.partitions.iter().enumerate() {
            for b in &self.partitions[i + 1..] {
                if a.range.intersects(&b.range) {
                    return Err(PlanError::OverlappingPartitions {
                        a: a.label.clone(),
                        b: b.label.clone(),
                    });
                }
            }
        }

        // The central check. The promise is identical either way — no user data
        // on a sector that was not proven good — but the mechanism differs, so
        // what has to be proven differs with it.
        match self.containment {
            // The boundary carries the guarantee, so the interval itself must
            // hold nothing but approved sectors.
            Containment::PartitionBoundary => {
                for p in self.data_partitions() {
                    for run in map.runs() {
                        if run.state.is_usable() {
                            continue;
                        }
                        if let Some(bad) = run.range.intersection(&p.range) {
                            return Err(PlanError::DataPartitionCoversUnsafeSector {
                                label: p.label.clone(),
                                lba: bad.start(),
                                state: run.state,
                            });
                        }
                    }
                }
            }
            // The allocation table carries it instead. Rebuilding the image is
            // what proves the table can actually express the exclusion: it
            // fails when a defect lands in the metadata, where no marking can
            // hide it because there is nowhere else to put the metadata.
            Containment::FilesystemClusterMap => {
                for p in self.data_partitions() {
                    let image = Fat32Image::new(map, &p.range, 0, "SALVAGE").map_err(|source| {
                        PlanError::ClusterMapCannotContainDefects { label: p.label.clone(), source }
                    })?;
                    if image.usable_bytes() == 0 {
                        return Err(PlanError::EmptyPartition(p.label.clone()));
                    }
                }
            }
        }

        Ok(())
    }
}

/// Merges ranges that overlap or abut. Input may be in any order.
fn merge_ranges(mut ranges: Vec<LbaRange>) -> Vec<LbaRange> {
    ranges.retain(|r| !r.is_empty());
    ranges.sort_unstable_by_key(|r| r.start());
    let mut out: Vec<LbaRange> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if last.touches_or_intersects(&r) => *last = last.hull(&r),
            _ => out.push(r),
        }
    }
    out
}

/// Complement of `ranges` within `bounds`. Expects `ranges` already merged.
fn complement(bounds: LbaRange, ranges: &[LbaRange]) -> Vec<LbaRange> {
    let mut out = Vec::new();
    let mut cursor = bounds.start();
    for r in ranges {
        if r.end() <= cursor {
            continue;
        }
        if r.start() > cursor {
            let gap = LbaRange::from_bounds(cursor, r.start().min(bounds.end()));
            if !gap.is_empty() {
                out.push(gap);
            }
        }
        cursor = cursor.max(r.end());
        if cursor >= bounds.end() {
            break;
        }
    }
    if cursor < bounds.end() {
        out.push(LbaRange::from_bounds(cursor, bounds.end()));
    }
    out
}

/// Regions that may not receive data, already dilated and aligned outward.
///
/// Covers everything not approved: defects, fenced area, and — when the policy
/// says so — anything never inspected.
pub fn condemned_ranges(map: &SectorMap, policy: &PlanningPolicy) -> Vec<LbaRange> {
    let raw: Vec<LbaRange> = map
        .runs()
        .iter()
        .filter(|run| {
            if run.state.is_usable() {
                return false;
            }
            if run.state == SectorState::Untested {
                return policy.treat_untested_as_unsafe;
            }
            true
        })
        .map(|run| {
            run.range
                .expanded_by(policy.guard_band_sectors)
                .aligned_outward(policy.alignment_sectors)
        })
        .collect();
    merge_ranges(raw)
}

/// Copy of the map with approved sectors falling inside condemned regions
/// marked as [`SectorState::Fenced`].
///
/// Exists purely for display: it shows **how much good space the guard band
/// cost**, which the raw map hides. A card may have a single defective sector
/// and still lose tens of megabytes to dilation and erase-block alignment;
/// without this view, that cost is invisible.
///
/// The returned map does **not** replace the original during validation. The
/// original remains the evidence of what was measured; this is a derived
/// reading, and turning measurement into interpretation would lose the
/// distinction.
pub fn fenced_view(map: &SectorMap, policy: &PlanningPolicy) -> SectorMap {
    let mut out = map.clone();
    for region in condemned_ranges(map, policy) {
        for run in map.runs() {
            if run.state != SectorState::Good {
                continue;
            }
            if let Some(overlap) = run.range.intersection(&region) {
                out.mark(overlap, SectorState::Fenced);
            }
        }
    }
    out
}

/// Areas eligible to hold data: aligned inward, filtered by minimum size, and
/// sorted by descending size.
fn candidate_ranges(map: &SectorMap, policy: &PlanningPolicy) -> Vec<LbaRange> {
    let g = map.geometry();
    let condemned = condemned_ranges(map, policy);

    // Sector zero holds the MBR; the data area starts at the first alignment
    // multiple, never before.
    let head = policy.alignment_sectors.max(2048);
    let usable_span = LbaRange::from_bounds(head.min(g.total_sectors()), g.total_sectors());

    let mut out: Vec<LbaRange> = complement(usable_span, &condemned)
        .into_iter()
        .map(|r| r.aligned_inward(policy.alignment_sectors))
        .filter(|r| r.len() >= policy.min_partition_sectors)
        .collect();

    out.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then(a.start().cmp(&b.start())));
    out
}

/// What a layout would have needed, reported when none could be produced.
///
/// "No usable area" is a true statement and a useless one when the card shows
/// 94 MB approved. The numbers below are what turn it into an explanation: the
/// approved space is real, it is simply in pieces, and each mechanism has a
/// minimum contiguous size that none of the pieces reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRequirements {
    /// Approved sectors on the whole card.
    pub approved_sectors: u64,
    /// How many separate runs they are split across.
    pub approved_runs: usize,
    /// Sectors in the largest single run.
    pub largest_run_sectors: u64,
    /// Contiguous approved sectors a fenced partition needs.
    ///
    /// A run sitting between two defects loses a guard band at each end, and
    /// what survives still has to reach the minimum partition size.
    pub fenced_needs_sectors: u64,
    /// Contiguous approved sectors the spliced volume's metadata needs.
    ///
    /// `None` when the area could not host a FAT32 volume at all, at which
    /// point the metadata size is not the reason it was refused.
    pub spliced_needs_sectors: Option<u64>,
}

/// Computes what the card would have had to offer for any layout to exist.
pub fn layout_requirements(map: &SectorMap, policy: &PlanningPolicy) -> LayoutRequirements {
    let runs: Vec<LbaRange> = map.usable_ranges().collect();
    let approved_sectors = runs.iter().map(|r| r.len()).sum();
    let largest_run_sectors = runs.iter().map(|r| r.len()).max().unwrap_or(0);

    // The spliced volume spans from the first approved sector to the last, so
    // that is the span whose table has to fit in healthy ground.
    let spliced_needs_sectors = match (runs.first(), runs.last()) {
        (Some(first), Some(last)) => Fat32Layout::plan(
            last.end().saturating_sub(first.start()),
            map.geometry().sector_size(),
            0,
        )
        .ok()
        .map(|l| l.metadata_sectors()),
        _ => None,
    };

    LayoutRequirements {
        approved_sectors,
        approved_runs: runs.len(),
        largest_run_sectors,
        fenced_needs_sectors: policy
            .guard_band_sectors
            .saturating_mul(2)
            .saturating_add(policy.min_partition_sectors),
        spliced_needs_sectors,
    }
}

/// Builds the spliced plan: one FAT32 volume covering as much of the device as
/// its own table can police.
///
/// The volume must begin on defect-free ground, because the boot sector, both
/// copies of the table and the root directory have no alternative home. Every
/// approved run is tried as a starting point, earliest first, since an earlier
/// start spans more of the card — and everything defective inside the span is
/// withheld cluster by cluster rather than cut away.
fn spliced_layout(map: &SectorMap, policy: &PlanningPolicy) -> Option<PartitionPlan> {
    let g = *map.geometry();
    let head = policy.alignment_sectors.max(2048);

    // The volume stops at the last approved sector rather than at the end of
    // the device. Everything past it is condemned anyway, so including it would
    // only inflate the declared size — and on a card lying about its capacity
    // those addresses alias onto real ones, so publishing a partition over them
    // invites another tool to write there and destroy the area that works.
    let total = map.runs().iter().filter(|r| r.state.is_usable()).map(|r| r.range.end()).max()?;

    let mut starts: Vec<u64> = map
        .runs()
        .iter()
        .filter(|r| r.state.is_usable())
        .map(|r| r.range.start().max(head).next_multiple_of(policy.alignment_sectors.max(1)))
        .filter(|s| *s < total)
        .collect();
    starts.sort_unstable();
    starts.dedup();

    for start in starts {
        let area = LbaRange::from_bounds(start, total);
        let Ok(image) = Fat32Image::new(map, &area, 0, "SALVAGE") else {
            continue;
        };
        let usable_bytes = image.usable_bytes();
        if usable_bytes == 0 || area.len() < policy.min_partition_sectors {
            continue;
        }

        let approved: u64 = map
            .runs()
            .iter()
            .filter(|r| r.state.is_usable())
            .filter_map(|r| r.range.intersection(&area))
            .map(|r| r.byte_len(&g))
            .sum();

        let mut partitions = vec![PlannedPartition {
            range: area,
            role: PartitionRole::Data,
            mbr_type: FileSystem::Fat32.mbr_type(),
            label: "spliced".into(),
        }];

        // Anything ahead of the volume is unreachable to it, so it is fenced the
        // usual way rather than left looking like free space.
        if start > head {
            partitions.insert(
                0,
                PlannedPartition {
                    range: LbaRange::from_bounds(head, start),
                    role: PartitionRole::Quarantine,
                    mbr_type: QUARANTINE_MBR_TYPE,
                    label: "quarantine-head".into(),
                },
            );
        }

        return Some(PartitionPlan {
            strategy: Strategy::SplicedFat32,
            partitions,
            geometry: g,
            usable_bytes,
            sacrificed_bytes: approved.saturating_sub(usable_bytes),
            containment: Containment::FilesystemClusterMap,
        });
    }
    None
}

/// Assembles a plan from the chosen data areas.
fn assemble(
    strategy: Strategy,
    map: &SectorMap,
    policy: &PlanningPolicy,
    data_ranges: Vec<LbaRange>,
    filesystem: FileSystem,
) -> PartitionPlan {
    let g = *map.geometry();
    let mut partitions: Vec<PlannedPartition> = Vec::with_capacity(policy.max_partitions);

    for (i, r) in data_ranges.iter().enumerate() {
        partitions.push(PlannedPartition {
            range: *r,
            role: PartitionRole::Data,
            mbr_type: filesystem.mbr_type(),
            label: if data_ranges.len() == 1 {
                "data".to_string()
            } else {
                format!("data-{}", i + 1)
            },
        });
    }

    // Remaining slots become quarantine over the largest condemned regions.
    // Covering all of them is not required for safety: space belonging to no
    // partition is already unreachable. Quarantine exists to document the
    // layout and to stop another tool from offering that space as free.
    let mut condemned = condemned_ranges(map, policy);
    condemned.sort_unstable_by_key(|r| std::cmp::Reverse(r.len()));

    for r in condemned {
        if partitions.len() >= policy.max_partitions {
            break;
        }
        let clipped = r.clamped_to(&LbaRange::from_bounds(1, g.total_sectors()));
        if clipped.is_empty() || partitions.iter().any(|p| p.range.intersects(&clipped)) {
            continue;
        }
        let n = partitions.iter().filter(|p| p.role == PartitionRole::Quarantine).count() + 1;
        partitions.push(PlannedPartition {
            range: clipped,
            role: PartitionRole::Quarantine,
            mbr_type: QUARANTINE_MBR_TYPE,
            label: format!("quarantine-{n}"),
        });
    }

    partitions.sort_unstable_by_key(|p| p.range.start());

    let usable_sectors: u64 = data_ranges.iter().map(|r| r.len()).sum();
    let total_good: u64 = map.usable_ranges().map(|r| r.len()).sum();
    let sacrificed_sectors = total_good.saturating_sub(usable_sectors);

    PartitionPlan {
        strategy,
        partitions,
        geometry: g,
        usable_bytes: usable_sectors.saturating_mul(g.sector_size() as u64),
        sacrificed_bytes: sacrificed_sectors.saturating_mul(g.sector_size() as u64),
        containment: Containment::PartitionBoundary,
    }
}

/// Generates the applicable layout strategies, from most conservative to most
/// space-efficient, always respecting the safety invariant.
///
/// Degenerate plans with no data area are discarded: there is nothing to offer
/// a user when no approved space remains.
pub fn plan_layouts(
    map: &SectorMap,
    filesystem: FileSystem,
    policy: &PlanningPolicy,
) -> Vec<PartitionPlan> {
    let mut plans = Vec::new();
    let candidates = candidate_ranges(map, policy);

    if let Some(largest) = candidates.first() {
        plans.push(assemble(Strategy::LargestContiguous, map, policy, vec![*largest], filesystem));
    }

    // Reserve at least one slot for quarantine when there is something to fence.
    let has_condemned = !condemned_ranges(map, policy).is_empty();
    let data_slots = if has_condemned {
        policy.max_partitions.saturating_sub(1).max(1)
    } else {
        policy.max_partitions
    };

    if candidates.len() > 1 && data_slots > 1 {
        let chosen: Vec<LbaRange> = candidates.iter().take(data_slots).copied().collect();
        let mut sorted = chosen.clone();
        sorted.sort_unstable_by_key(|r| r.start());
        plans.push(assemble(Strategy::MaximumSpace, map, policy, sorted, filesystem));
    }

    let cons_policy =
        PlanningPolicy { guard_band_sectors: policy.guard_band_sectors * 4, ..*policy };
    if let Some(largest) = candidate_ranges(map, &cons_policy).first() {
        let plan = assemble(Strategy::Conservative, map, &cons_policy, vec![*largest], filesystem);
        // Only worth offering when it actually differs from the first plan.
        if plans.first().is_none_or(|p| p.partitions != plan.partitions) {
            plans.push(plan);
        }
    }

    // Offered last: it recovers the most space, and it is the one option that
    // trades away the guard band, so it should not be what the eye lands on
    // first.
    if let Some(spliced) = spliced_layout(map, policy) {
        plans.push(spliced);
    }

    plans.retain(|p| p.data_partitions().count() > 0);
    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: u32 = 512;
    /// 4 MiB in 512-byte sectors.
    const ALIGN: u64 = 8192;

    fn geometry(sectors: u64) -> DeviceGeometry {
        DeviceGeometry::new(SS, sectors).unwrap()
    }

    fn healthy_map(sectors: u64) -> SectorMap {
        let mut m = SectorMap::new(geometry(sectors));
        m.mark(LbaRange::from_bounds(0, sectors), SectorState::Good);
        m
    }

    fn policy() -> PlanningPolicy {
        PlanningPolicy {
            guard_band_sectors: ALIGN,
            alignment_sectors: ALIGN,
            min_partition_sectors: ALIGN,
            max_partitions: 4,
            treat_untested_as_unsafe: true,
        }
    }

    #[test]
    fn merge_ranges_joins_overlapping_and_touching() {
        let merged = merge_ranges(vec![
            LbaRange::from_bounds(10, 20),
            LbaRange::from_bounds(20, 30),
            LbaRange::from_bounds(15, 18),
            LbaRange::from_bounds(100, 110),
        ]);
        assert_eq!(merged, vec![LbaRange::from_bounds(10, 30), LbaRange::from_bounds(100, 110)]);
    }

    #[test]
    fn complement_returns_the_gaps() {
        let bounds = LbaRange::from_bounds(0, 100);
        let taken = vec![LbaRange::from_bounds(20, 30), LbaRange::from_bounds(60, 70)];
        assert_eq!(
            complement(bounds, &taken),
            vec![
                LbaRange::from_bounds(0, 20),
                LbaRange::from_bounds(30, 60),
                LbaRange::from_bounds(70, 100),
            ]
        );
    }

    #[test]
    fn complement_of_full_coverage_is_empty() {
        let bounds = LbaRange::from_bounds(0, 100);
        assert!(complement(bounds, &[LbaRange::from_bounds(0, 100)]).is_empty());
    }

    #[test]
    fn a_healthy_card_yields_one_data_partition_and_no_quarantine() {
        let m = healthy_map(ALIGN * 128);
        let plans = plan_layouts(&m, FileSystem::ExFat, &policy());
        assert!(!plans.is_empty());
        let p = &plans[0];
        assert_eq!(p.data_partitions().count(), 1);
        assert_eq!(p.quarantine_partitions().count(), 0);
        p.validate(&m, 4).unwrap();
    }

    #[test]
    fn no_partition_ever_starts_on_the_partition_table() {
        let m = healthy_map(ALIGN * 128);
        for plan in plan_layouts(&m, FileSystem::ExFat, &policy()) {
            for p in &plan.partitions {
                assert!(p.range.start() > 0, "partition '{}' starts at LBA 0", p.label);
            }
        }
    }

    #[test]
    fn a_defect_is_excluded_together_with_its_guard_band() {
        let total = ALIGN * 128;
        let mut m = healthy_map(total);
        let defect = LbaRange::from_bounds(ALIGN * 60, ALIGN * 60 + 1);
        m.mark(defect, SectorState::BadRead);

        let plans = plan_layouts(&m, FileSystem::ExFat, &policy());
        assert!(!plans.is_empty());
        for plan in &plans {
            plan.validate(&m, 4).unwrap();
            for dp in plan.data_partitions() {
                match plan.containment {
                    // The boundary is the mechanism, so it has to clear both the
                    // defect and the margin around it.
                    Containment::PartitionBoundary => {
                        assert!(!dp.range.intersects(&defect), "'{}' covers the defect", dp.label);
                        let padded = defect.expanded_by(ALIGN);
                        assert!(
                            !dp.range.intersects(&padded),
                            "'{}' enters the guard band",
                            dp.label
                        );
                    }
                    // Here the partition is meant to span the defect; what must
                    // hold is that the table refuses to hand out the clusters
                    // covering it.
                    Containment::FilesystemClusterMap => {
                        let image = Fat32Image::new(&m, &dp.range, 0, "T")
                            .expect("the spliced plan validated, so the image must build");
                        assert!(
                            image.condemned_clusters() > 0,
                            "'{}' spans the defect without withholding a single cluster",
                            dp.label
                        );
                    }
                }
            }
        }
    }

    /// The reason the spliced strategy exists. With the good area broken into
    /// pieces, a partition boundary can only return the largest single piece;
    /// the cluster map returns their sum.
    #[test]
    fn splicing_recovers_more_than_the_largest_contiguous_run() {
        // Large enough that FAT32 is a legal format for the area at all.
        let total = 80_000_000u64;
        let mut m = healthy_map(total);

        // Four defects chopping the card into five comparable pieces.
        for i in 1..=4u64 {
            let at = total / 5 * i;
            m.mark(LbaRange::from_bounds(at, at + 4096), SectorState::Corrupt);
        }

        let plans = plan_layouts(&m, FileSystem::Fat32, &policy());
        let fenced = plans
            .iter()
            .find(|p| p.strategy == Strategy::LargestContiguous)
            .expect("the fenced plan should still be offered");
        let spliced = plans
            .iter()
            .find(|p| p.strategy == Strategy::SplicedFat32)
            .expect("the spliced plan was not offered");

        assert_eq!(spliced.containment, Containment::FilesystemClusterMap);
        assert_eq!(fenced.containment, Containment::PartitionBoundary);
        assert_eq!(spliced.data_partitions().count(), 1, "splicing yields one drive letter");
        assert!(
            spliced.usable_bytes > fenced.usable_bytes * 3,
            "splicing recovered {} against {} for the largest run — it should be several times more",
            spliced.usable_bytes,
            fenced.usable_bytes
        );
        spliced.validate(&m, 4).expect("the spliced plan must validate");
    }

    /// The safety equivalence the whole design rests on. The spliced partition
    /// spans defects on purpose, so the guarantee has to be re-proved at the
    /// layer that actually enforces it: every cluster the volume will hand out
    /// must map to sectors the scan approved.
    #[test]
    fn no_cluster_the_spliced_volume_offers_touches_an_unapproved_sector() {
        let total = 80_000_000u64;
        let mut m = healthy_map(total);
        let defects = [
            LbaRange::from_bounds(9_000_000, 9_004_096),
            LbaRange::from_bounds(30_000_000, 30_100_000),
            LbaRange::from_bounds(61_234_567, 61_234_568),
        ];
        for d in defects {
            m.mark(d, SectorState::Corrupt);
        }
        // A stretch the scan never reached, which must be withheld just as
        // firmly as a proven defect.
        m.mark(LbaRange::from_bounds(70_000_000, 71_000_000), SectorState::Untested);

        let plan = plan_layouts(&m, FileSystem::Fat32, &policy())
            .into_iter()
            .find(|p| p.strategy == Strategy::SplicedFat32)
            .expect("the spliced plan was not offered");
        plan.validate(&m, 4).unwrap();

        let area = plan.data_partitions().next().unwrap().range;
        let image = Fat32Image::new(&m, &area, 0, "T").unwrap();
        let layout = *image.layout();
        let spc = layout.sectors_per_cluster() as u64;

        // Walk every cluster the volume could allocate and confirm the sectors
        // behind it were approved. This is the assertion that would fail if the
        // cluster arithmetic were off by one anywhere.
        let mut checked = 0u64;
        for cluster in 3..layout.cluster_count() + 2 {
            if image.fat_entry_for_test(cluster) != 0 {
                continue; // withheld, or the root directory
            }
            let start =
                area.start() + layout.data_start_sector() as u64 + (cluster as u64 - 2) * spc;
            for lba in start..start + spc {
                let state = m.state_at(lba).unwrap_or(SectorState::Untested);
                assert!(
                    state.is_usable(),
                    "allocatable cluster {cluster} covers LBA {lba}, which is {state:?}"
                );
            }
            checked += 1;
        }
        assert!(
            checked > 1_000,
            "only {checked} clusters were allocatable; the test proved little"
        );
    }

    /// The card that prompted this: 94 MB approved, none of it usable, and a
    /// bare "no usable area" that reads as a contradiction. The requirements
    /// are what make the refusal explicable — and each mechanism is refused for
    /// its own reason, at its own threshold.
    #[test]
    fn requirements_explain_a_refusal_on_a_card_with_approved_area() {
        let total = 245_760_000u64;
        let mut m = SectorMap::new(DeviceGeometry::new(512, total).unwrap());
        m.mark(LbaRange::from_bounds(0, total), SectorState::Corrupt);
        // Approved area in scattered pieces, the largest well under what either
        // mechanism needs.
        for i in 0..36u64 {
            let at = 1_000_000 + i * 5_000_000;
            m.mark(LbaRange::from_bounds(at, at + 5_000), SectorState::Good);
        }
        let policy = PlanningPolicy::recommended_for(m.geometry());

        assert!(
            plan_layouts(&m, FileSystem::ExFat, &policy).is_empty(),
            "no layout should survive on this card"
        );

        let req = layout_requirements(&m, &policy);
        assert_eq!(req.approved_runs, 36);
        assert_eq!(req.approved_sectors, 36 * 5_000);
        assert_eq!(req.largest_run_sectors, 5_000);
        assert!(
            req.fenced_needs_sectors > req.largest_run_sectors,
            "fencing would have fit, so the refusal needs another explanation"
        );
        let spliced = req.spliced_needs_sectors.expect("the span can host FAT32");
        assert!(
            spliced > req.largest_run_sectors,
            "the table would have fit, so the refusal needs another explanation"
        );
    }

    /// The counterpart: when a run is big enough, the requirements must not
    /// claim otherwise, or the explanation would be a fabrication.
    #[test]
    fn requirements_are_met_when_a_layout_does_exist() {
        let total = 245_760_000u64;
        let mut m = SectorMap::new(DeviceGeometry::new(512, total).unwrap());
        m.mark(LbaRange::from_bounds(0, total), SectorState::Corrupt);
        m.mark(LbaRange::from_bounds(1_000_000, 61_000_000), SectorState::Good);
        let policy = PlanningPolicy::recommended_for(m.geometry());

        let req = layout_requirements(&m, &policy);
        assert!(req.largest_run_sectors >= req.fenced_needs_sectors);
        assert!(!plan_layouts(&m, FileSystem::ExFat, &policy).is_empty());
    }

    /// The most dangerous regression possible: a data partition over a bad
    /// sector. Built deliberately here to prove validation rejects it.
    #[test]
    fn validate_rejects_a_data_partition_covering_a_bad_sector() {
        let total = ALIGN * 32;
        let mut m = healthy_map(total);
        m.mark(LbaRange::from_bounds(ALIGN * 10, ALIGN * 11), SectorState::Corrupt);

        let bad_plan = PartitionPlan {
            strategy: Strategy::LargestContiguous,
            partitions: vec![PlannedPartition {
                range: LbaRange::from_bounds(ALIGN, ALIGN * 20),
                role: PartitionRole::Data,
                mbr_type: 0x07,
                label: "data".into(),
            }],
            geometry: geometry(total),
            usable_bytes: 0,
            sacrificed_bytes: 0,
            containment: Containment::PartitionBoundary,
        };

        match bad_plan.validate(&m, 4) {
            Err(PlanError::DataPartitionCoversUnsafeSector { state, .. }) => {
                assert_eq!(state, SectorState::Corrupt);
            }
            other => panic!("validation should have rejected the plan, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_untested_sectors_inside_a_data_partition() {
        let total = ALIGN * 32;
        let mut m = SectorMap::new(geometry(total));
        m.mark(LbaRange::from_bounds(0, ALIGN * 20), SectorState::Good);
        // From ALIGN*20 onward the map stays Untested.

        let plan = PartitionPlan {
            strategy: Strategy::LargestContiguous,
            partitions: vec![PlannedPartition {
                range: LbaRange::from_bounds(ALIGN, ALIGN * 25),
                role: PartitionRole::Data,
                mbr_type: 0x07,
                label: "data".into(),
            }],
            geometry: geometry(total),
            usable_bytes: 0,
            sacrificed_bytes: 0,
            containment: Containment::PartitionBoundary,
        };

        assert!(matches!(
            plan.validate(&m, 4),
            Err(PlanError::DataPartitionCoversUnsafeSector { state: SectorState::Untested, .. })
        ));
    }

    #[test]
    fn validate_rejects_overlapping_partitions() {
        let total = ALIGN * 32;
        let m = healthy_map(total);
        let plan = PartitionPlan {
            strategy: Strategy::LargestContiguous,
            partitions: vec![
                PlannedPartition {
                    range: LbaRange::from_bounds(ALIGN, ALIGN * 10),
                    role: PartitionRole::Data,
                    mbr_type: 0x07,
                    label: "a".into(),
                },
                PlannedPartition {
                    range: LbaRange::from_bounds(ALIGN * 9, ALIGN * 12),
                    role: PartitionRole::Quarantine,
                    mbr_type: QUARANTINE_MBR_TYPE,
                    label: "b".into(),
                },
            ],
            geometry: geometry(total),
            usable_bytes: 0,
            sacrificed_bytes: 0,
            containment: Containment::PartitionBoundary,
        };
        assert!(matches!(plan.validate(&m, 4), Err(PlanError::OverlappingPartitions { .. })));
    }

    #[test]
    fn validate_rejects_a_partition_past_the_end_of_the_device() {
        let total = ALIGN * 32;
        let m = healthy_map(total);
        let plan = PartitionPlan {
            strategy: Strategy::LargestContiguous,
            partitions: vec![PlannedPartition {
                range: LbaRange::from_bounds(ALIGN, total + ALIGN),
                role: PartitionRole::Data,
                mbr_type: 0x07,
                label: "data".into(),
            }],
            geometry: geometry(total),
            usable_bytes: 0,
            sacrificed_bytes: 0,
            containment: Containment::PartitionBoundary,
        };
        assert!(matches!(plan.validate(&m, 4), Err(PlanError::OutOfBounds { .. })));
    }

    /// Typical scenario A: the card advertises 128 units and only the first 32
    /// exist. The plan must fence off the entire fake area.
    #[test]
    fn a_counterfeit_card_keeps_only_the_real_region() {
        let total = ALIGN * 128;
        let real = ALIGN * 32;
        let mut m = SectorMap::new(geometry(total));
        m.mark(LbaRange::from_bounds(0, real), SectorState::Good);
        m.mark(LbaRange::from_bounds(real, total), SectorState::Aliased);

        let plans = plan_layouts(&m, FileSystem::ExFat, &policy());
        let plan = &plans[0];
        plan.validate(&m, 4).unwrap();

        let data: Vec<_> = plan.data_partitions().collect();
        assert_eq!(data.len(), 1);
        assert!(data[0].range.end() <= real, "partition reaches into nonexistent area");
        assert_eq!(plan.quarantine_partitions().count(), 1);

        let q = plan.quarantine_partitions().next().unwrap();
        assert_eq!(q.mbr_type, QUARANTINE_MBR_TYPE);
        assert!(q.range.start() >= real - ALIGN);
    }

    #[test]
    fn maximum_space_uses_more_than_one_region_when_it_helps() {
        let total = ALIGN * 128;
        let mut m = healthy_map(total);
        // A defect in the middle splits the card into two large halves.
        m.mark(LbaRange::from_bounds(ALIGN * 64, ALIGN * 65), SectorState::BadRead);

        let plans = plan_layouts(&m, FileSystem::ExFat, &policy());
        let max_plan = plans
            .iter()
            .find(|p| p.strategy == Strategy::MaximumSpace)
            .expect("a maximum-space strategy should exist");
        assert_eq!(max_plan.data_partitions().count(), 2);
        max_plan.validate(&m, 4).unwrap();

        let largest = plans.iter().find(|p| p.strategy == Strategy::LargestContiguous).unwrap();
        assert!(
            max_plan.usable_bytes > largest.usable_bytes,
            "maximum space should beat a single contiguous area"
        );
    }

    #[test]
    fn the_conservative_strategy_trades_space_for_distance() {
        let total = ALIGN * 128;
        let mut m = healthy_map(total);
        m.mark(LbaRange::from_bounds(ALIGN * 64, ALIGN * 65), SectorState::BadRead);

        let plans = plan_layouts(&m, FileSystem::ExFat, &policy());
        if let Some(cons) = plans.iter().find(|p| p.strategy == Strategy::Conservative) {
            cons.validate(&m, 4).unwrap();
            let largest = plans.iter().find(|p| p.strategy == Strategy::LargestContiguous).unwrap();
            assert!(cons.usable_bytes <= largest.usable_bytes);
        }
    }

    #[test]
    fn plans_never_exceed_the_partition_table_size() {
        let total = ALIGN * 256;
        let mut m = healthy_map(total);
        // Many scattered defects: more condemned regions than table slots.
        for i in 1..20u64 {
            m.mark(LbaRange::from_bounds(ALIGN * i * 12, ALIGN * i * 12 + 1), SectorState::Corrupt);
        }
        let plans = plan_layouts(&m, FileSystem::ExFat, &policy());
        assert!(!plans.is_empty());
        for plan in &plans {
            assert!(plan.partitions.len() <= 4, "plan has {} partitions", plan.partitions.len());
            plan.validate(&m, 4).unwrap();
        }
    }

    /// When no approved space remains, the planner must not invent one: better
    /// to offer nothing than to offer something unsafe.
    #[test]
    fn a_fully_defective_card_produces_no_plan() {
        let total = ALIGN * 32;
        let mut m = SectorMap::new(geometry(total));
        m.mark(LbaRange::from_bounds(0, total), SectorState::Corrupt);
        assert!(plan_layouts(&m, FileSystem::ExFat, &policy()).is_empty());
    }

    #[test]
    fn an_uninspected_card_produces_no_plan() {
        let m = SectorMap::new(geometry(ALIGN * 32));
        assert!(plan_layouts(&m, FileSystem::ExFat, &policy()).is_empty());
    }

    #[test]
    fn every_generated_plan_is_aligned_to_the_erase_block() {
        let total = ALIGN * 128;
        let mut m = healthy_map(total);
        m.mark(LbaRange::from_bounds(ALIGN * 33 + 77, ALIGN * 33 + 90), SectorState::Corrupt);
        for plan in plan_layouts(&m, FileSystem::ExFat, &policy()) {
            for p in plan.data_partitions() {
                assert_eq!(p.range.start() % ALIGN, 0, "'{}' start unaligned", p.label);
                assert_eq!(p.range.len() % ALIGN, 0, "'{}' length unaligned", p.label);
            }
        }
    }

    #[test]
    fn filesystem_type_bytes_are_the_expected_ones() {
        assert_eq!(FileSystem::ExFat.mbr_type(), 0x07);
        assert_eq!(FileSystem::Fat32.mbr_type(), 0x0C);
        assert_eq!(QUARANTINE_MBR_TYPE, 0xDA);
    }

    /// A single bad sector condemns an entire erase block. The fenced view must
    /// make that cost visible instead of leaving it implicit.
    #[test]
    fn fenced_view_reveals_the_good_space_lost_to_the_guard_band() {
        let total = ALIGN * 64;
        let mut m = healthy_map(total);
        m.mark_sector(ALIGN * 30, SectorState::Corrupt);

        let view = fenced_view(&m, &policy());
        view.check_invariants().unwrap();

        let before = m.counts();
        let after = view.counts();

        assert_eq!(before.fenced, 0, "the measured map carries no fenced area");
        assert!(after.fenced > 0, "the view should expose the sacrificed area");
        assert_eq!(after.corrupt, before.corrupt, "the defect must not be reclassified");
        assert_eq!(
            after.good + after.fenced,
            before.good,
            "fencing may only move sectors from approved to fenced"
        );
        assert_eq!(after.total(), total, "the view must cover the same device");
    }

    #[test]
    fn fenced_view_of_a_healthy_card_changes_nothing() {
        let m = healthy_map(ALIGN * 64);
        let view = fenced_view(&m, &policy());
        assert_eq!(view.counts().fenced, 0);
        assert_eq!(view.counts().good, m.counts().good);
    }

    /// The view is for human reading; validation still uses the measured map,
    /// and plans must remain valid against both.
    #[test]
    fn plans_stay_valid_against_the_fenced_view() {
        let total = ALIGN * 128;
        let mut m = healthy_map(total);
        m.mark(LbaRange::from_bounds(ALIGN * 64, ALIGN * 65), SectorState::BadRead);

        let view = fenced_view(&m, &policy());
        for plan in plan_layouts(&m, FileSystem::ExFat, &policy()) {
            plan.validate(&m, 4).unwrap();
            plan.validate(&view, 4).expect("plan should also hold against the view");
        }
    }

    /// Exhaustive check: across many defect patterns, no plan may ever place
    /// data on a sector that was not approved.
    #[test]
    fn no_generated_plan_ever_violates_the_safety_invariant() {
        let total = ALIGN * 200;
        let mut seed = 0xABCD_EF01u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for case in 0..60 {
            let mut m = healthy_map(total);
            for _ in 0..(case % 9) + 1 {
                let start = rnd() % total;
                let len = (rnd() % (ALIGN * 3)) + 1;
                let state = match rnd() % 4 {
                    0 => SectorState::BadRead,
                    1 => SectorState::BadWrite,
                    2 => SectorState::Corrupt,
                    _ => SectorState::Aliased,
                };
                m.mark(LbaRange::new(start, len), state);
            }
            m.check_invariants().unwrap();

            for plan in plan_layouts(&m, FileSystem::ExFat, &policy()) {
                plan.validate(&m, 4)
                    .unwrap_or_else(|e| panic!("case {case}, plan {:?}: {e}", plan.strategy));
            }
        }
    }
}
