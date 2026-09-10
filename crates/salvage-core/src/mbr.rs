//! Master Boot Record serialization and parsing.
//!
//! MBR was chosen over GPT deliberately. Card readers built into cameras,
//! consoles and older devices commonly understand only MBR, and the goal here
//! is a card that keeps working everywhere. The format's limits — four primary
//! partitions and 2 TiB — constrain no microSD in existence.
//!
//! The sector is assembled byte by byte, with no `unsafe` and no reliance on
//! the in-memory layout of any struct.

use serde::{Deserialize, Serialize};

use crate::geometry::LbaRange;
use crate::planner::{PartitionPlan, QUARANTINE_MBR_TYPE};

/// Size of the MBR sector.
pub const MBR_SIZE: usize = 512;
/// Offset of the first partition table entry.
pub const PARTITION_TABLE_OFFSET: usize = 446;
/// Size of each entry.
pub const PARTITION_ENTRY_SIZE: usize = 16;
/// Number of primary entries.
pub const MAX_PRIMARY_PARTITIONS: usize = 4;
/// Offset of the disk signature.
pub const DISK_SIGNATURE_OFFSET: usize = 440;

/// Largest sector count an MBR entry can address (32-bit field).
pub const MAX_ADDRESSABLE_SECTORS: u64 = u32::MAX as u64;

/// Conventional CHS geometry used by practically all firmware.
const HEADS_PER_CYLINDER: u64 = 255;
const SECTORS_PER_TRACK: u64 = 63;

/// Errors raised while building an MBR.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MbrError {
    /// More partitions than the table can hold.
    #[error("{0} partitions exceed the MBR's {MAX_PRIMARY_PARTITIONS} primary entries")]
    TooManyPartitions(usize),
    /// A partition exceeds the 32-bit addressing limit.
    #[error(
        "partition at {start}..{end} exceeds the MBR limit of {MAX_ADDRESSABLE_SECTORS} sectors"
    )]
    ExceedsAddressableRange {
        /// First LBA.
        start: u64,
        /// Last LBA, exclusive.
        end: u64,
    },
    /// The buffer is not 512 bytes.
    #[error("sector is {0} bytes, expected {MBR_SIZE}")]
    WrongSectorSize(usize),
    /// The 0x55AA boot signature is missing.
    #[error("boot signature missing: found {found:#06x}, expected 0xAA55")]
    MissingBootSignature {
        /// The value read.
        found: u16,
    },
    /// An empty partition reached the table.
    #[error("empty partition at index {0}")]
    EmptyPartition(usize),
}

/// One partition table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionEntry {
    /// `0x80` marks the partition as active.
    pub bootable: bool,
    /// Type byte.
    pub partition_type: u8,
    /// First LBA.
    pub start_lba: u32,
    /// Sector count.
    pub sector_count: u32,
}

impl PartitionEntry {
    /// A null entry, representing a free slot.
    pub const EMPTY: Self =
        Self { bootable: false, partition_type: 0x00, start_lba: 0, sector_count: 0 };

    /// True when the slot is free.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.partition_type == 0x00 || self.sector_count == 0
    }

    /// LBA interval covered.
    #[inline]
    pub const fn range(&self) -> LbaRange {
        LbaRange::new(self.start_lba as u64, self.sector_count as u64)
    }

    /// Serializes the entry's 16 bytes.
    ///
    /// A free slot is written as sixteen zeros. Serializing the CHS of an empty
    /// slot would leave a residual `0x01` in the sector field, and some
    /// partitioning tools read that residue as a real, half-corrupt entry.
    fn write_to(&self, out: &mut [u8]) {
        debug_assert_eq!(out.len(), PARTITION_ENTRY_SIZE);
        if self.is_empty() {
            out.fill(0);
            return;
        }
        out[0] = if self.bootable { 0x80 } else { 0x00 };
        out[1..4].copy_from_slice(&encode_chs(self.start_lba as u64));
        out[4] = self.partition_type;
        let last = (self.start_lba as u64 + self.sector_count as u64).saturating_sub(1);
        out[5..8].copy_from_slice(&encode_chs(last));
        out[8..12].copy_from_slice(&self.start_lba.to_le_bytes());
        out[12..16].copy_from_slice(&self.sector_count.to_le_bytes());
    }

    /// Reads the entry's 16 bytes.
    fn read_from(src: &[u8]) -> Self {
        debug_assert_eq!(src.len(), PARTITION_ENTRY_SIZE);
        Self {
            bootable: src[0] == 0x80,
            partition_type: src[4],
            start_lba: u32::from_le_bytes(src[8..12].try_into().expect("4 bytes")),
            sector_count: u32::from_le_bytes(src[12..16].try_into().expect("4 bytes")),
        }
    }
}

/// Converts an LBA into the MBR's three CHS bytes.
///
/// Addresses beyond what CHS can express receive the saturation value
/// `0xFE 0xFF 0xFF`, the convention every modern firmware reads as "use LBA and
/// ignore these fields".
fn encode_chs(lba: u64) -> [u8; 3] {
    let cylinder = lba / (HEADS_PER_CYLINDER * SECTORS_PER_TRACK);
    if cylinder > 1023 {
        return [0xFE, 0xFF, 0xFF];
    }
    let head = (lba / SECTORS_PER_TRACK) % HEADS_PER_CYLINDER;
    let sector = (lba % SECTORS_PER_TRACK) + 1;
    [
        head as u8,
        ((sector as u8) & 0x3F) | (((cylinder >> 2) as u8) & 0xC0),
        (cylinder & 0xFF) as u8,
    ]
}

/// A complete Master Boot Record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterBootRecord {
    /// 32-bit disk signature, used by Windows to identify the media.
    pub disk_signature: u32,
    /// The four primary entries.
    pub partitions: [PartitionEntry; MAX_PRIMARY_PARTITIONS],
}

impl MasterBootRecord {
    /// An MBR with no partitions.
    pub const fn empty(disk_signature: u32) -> Self {
        Self { disk_signature, partitions: [PartitionEntry::EMPTY; MAX_PRIMARY_PARTITIONS] }
    }

    /// Builds an MBR from a partition plan.
    ///
    /// No partition is marked bootable: the card stores data and has no
    /// business competing for the machine's boot order.
    pub fn from_plan(plan: &PartitionPlan, disk_signature: u32) -> Result<Self, MbrError> {
        if plan.partitions.len() > MAX_PRIMARY_PARTITIONS {
            return Err(MbrError::TooManyPartitions(plan.partitions.len()));
        }

        let mut mbr = Self::empty(disk_signature);
        for (i, p) in plan.partitions.iter().enumerate() {
            if p.range.is_empty() {
                return Err(MbrError::EmptyPartition(i));
            }
            if p.range.end() > MAX_ADDRESSABLE_SECTORS {
                return Err(MbrError::ExceedsAddressableRange {
                    start: p.range.start(),
                    end: p.range.end(),
                });
            }
            mbr.partitions[i] = PartitionEntry {
                bootable: false,
                partition_type: p.mbr_type,
                start_lba: p.range.start() as u32,
                sector_count: p.range.len() as u32,
            };
        }
        Ok(mbr)
    }

    /// Serializes the 512-byte sector.
    ///
    /// The bootstrap area is left zeroed: the card boots nothing, and a blank
    /// bootstrap removes any chance of the machine attempting to execute code
    /// from it.
    pub fn to_bytes(&self) -> [u8; MBR_SIZE] {
        let mut out = [0u8; MBR_SIZE];
        out[DISK_SIGNATURE_OFFSET..DISK_SIGNATURE_OFFSET + 4]
            .copy_from_slice(&self.disk_signature.to_le_bytes());
        // Bytes 444..446 stay 0x0000 (reserved).
        for (i, p) in self.partitions.iter().enumerate() {
            let at = PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE;
            p.write_to(&mut out[at..at + PARTITION_ENTRY_SIZE]);
        }
        out[510] = 0x55;
        out[511] = 0xAA;
        out
    }

    /// Parses an MBR from a raw sector.
    pub fn from_bytes(sector: &[u8]) -> Result<Self, MbrError> {
        if sector.len() != MBR_SIZE {
            return Err(MbrError::WrongSectorSize(sector.len()));
        }
        let sig = u16::from_le_bytes([sector[510], sector[511]]);
        if sig != 0xAA55 {
            return Err(MbrError::MissingBootSignature { found: sig });
        }
        let disk_signature = u32::from_le_bytes(
            sector[DISK_SIGNATURE_OFFSET..DISK_SIGNATURE_OFFSET + 4].try_into().expect("4 bytes"),
        );
        let mut partitions = [PartitionEntry::EMPTY; MAX_PRIMARY_PARTITIONS];
        for (i, slot) in partitions.iter_mut().enumerate() {
            let at = PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE;
            *slot = PartitionEntry::read_from(&sector[at..at + PARTITION_ENTRY_SIZE]);
        }
        Ok(Self { disk_signature, partitions })
    }

    /// Entries actually in use.
    pub fn used_partitions(&self) -> impl Iterator<Item = (usize, &PartitionEntry)> {
        self.partitions.iter().enumerate().filter(|(_, p)| !p.is_empty())
    }
}

/// What an earlier run of this program left recorded on the card.
///
/// The partition table is the only thing that survives unplugging the card, so
/// it is where the question "has this one already been fenced?" gets answered.
/// A quarantine entry is the signature: the type byte exists precisely so that
/// nothing mounts that area, and finding one beside a data partition means an
/// earlier inspection already condemned what it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorLayout {
    /// Intervals an earlier layout withheld from data, in address order.
    pub quarantined: Vec<LbaRange>,
    /// Intervals it left available for data, in address order.
    pub data: Vec<LbaRange>,
}

impl PriorLayout {
    /// Reads the layout back out of a partition table.
    ///
    /// `None` when the table carries no quarantine, or no data partition
    /// beside it: either way it is not a layout this program produced, and
    /// nothing on the card licenses skipping any part of it.
    pub fn read(mbr: &MasterBootRecord) -> Option<Self> {
        let mut quarantined = Vec::new();
        let mut data = Vec::new();
        for (_, p) in mbr.used_partitions() {
            if p.partition_type == QUARANTINE_MBR_TYPE {
                quarantined.push(p.range());
            } else {
                data.push(p.range());
            }
        }
        if quarantined.is_empty() || data.is_empty() {
            return None;
        }
        quarantined.sort_by_key(LbaRange::start);
        data.sort_by_key(LbaRange::start);
        Some(Self { quarantined, data })
    }

    /// Smallest interval covering every data partition.
    ///
    /// One interval, because that is what an inspection covers. When an
    /// earlier layout scattered data across several partitions, this hull
    /// spans the quarantine sitting between them — re-reading a few condemned
    /// sectors costs minutes, whereas leaving one of those data partitions
    /// outside the inspection would leave it unproven, which costs the point
    /// of running at all.
    pub fn data_span(&self) -> Option<LbaRange> {
        self.data.iter().copied().reduce(|a, b| a.hull(&b))
    }
}

/// One line describing a single partition table change.
///
/// Structured rather than pre-formatted so the presentation layer controls the
/// wording; the domain reports what changes, not how to phrase it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableChange {
    /// Whether the entry is being added or removed.
    pub added: bool,
    /// One-based slot index in the partition table.
    pub slot: usize,
    /// Partition type byte.
    pub partition_type: u8,
    /// First LBA.
    pub start_lba: u64,
    /// Sector count.
    pub sectors: u64,
}

/// Describes the difference between the current table and the proposed one.
///
/// Feeds the preview shown before any write: the user sees what leaves and what
/// arrives, and only then confirms.
pub fn describe_change(
    before: Option<&MasterBootRecord>,
    after: &MasterBootRecord,
) -> Vec<TableChange> {
    let mut out = Vec::new();
    if let Some(b) = before {
        for (i, p) in b.used_partitions() {
            out.push(TableChange {
                added: false,
                slot: i + 1,
                partition_type: p.partition_type,
                start_lba: p.start_lba as u64,
                sectors: p.sector_count as u64,
            });
        }
    }
    for (i, p) in after.used_partitions() {
        out.push(TableChange {
            added: true,
            slot: i + 1,
            partition_type: p.partition_type,
            start_lba: p.start_lba as u64,
            sectors: p.sector_count as u64,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::DeviceGeometry;
    use crate::planner::{
        Containment, FileSystem, PartitionRole, PlannedPartition, Strategy, QUARANTINE_MBR_TYPE,
    };

    fn sample_plan() -> PartitionPlan {
        PartitionPlan {
            strategy: Strategy::LargestContiguous,
            partitions: vec![
                PlannedPartition {
                    range: LbaRange::from_bounds(8192, 1_000_000),
                    role: PartitionRole::Data,
                    mbr_type: FileSystem::ExFat.mbr_type(),
                    label: "data".into(),
                },
                PlannedPartition {
                    range: LbaRange::from_bounds(1_000_000, 2_000_000),
                    role: PartitionRole::Quarantine,
                    mbr_type: QUARANTINE_MBR_TYPE,
                    label: "quarantine-1".into(),
                },
            ],
            geometry: DeviceGeometry::new(512, 2_000_000).unwrap(),
            usable_bytes: 0,
            sacrificed_bytes: 0,
            containment: Containment::PartitionBoundary,
        }
    }

    #[test]
    fn serialised_sector_has_the_boot_signature_and_right_size() {
        let bytes = MasterBootRecord::from_plan(&sample_plan(), 0x1234_5678).unwrap().to_bytes();
        assert_eq!(bytes.len(), MBR_SIZE);
        assert_eq!((bytes[510], bytes[511]), (0x55, 0xAA));
    }

    #[test]
    fn the_bootstrap_area_is_left_zeroed() {
        let bytes = MasterBootRecord::from_plan(&sample_plan(), 0xDEAD_BEEF).unwrap().to_bytes();
        assert!(
            bytes[..DISK_SIGNATURE_OFFSET].iter().all(|b| *b == 0),
            "the bootstrap area must stay blank"
        );
    }

    #[test]
    fn round_trip_preserves_every_field() {
        let original = MasterBootRecord::from_plan(&sample_plan(), 0xCAFE_BABE).unwrap();
        let parsed = MasterBootRecord::from_bytes(&original.to_bytes()).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(parsed.disk_signature, 0xCAFE_BABE);
    }

    #[test]
    fn entries_land_at_the_documented_offsets() {
        let mbr = MasterBootRecord::from_plan(&sample_plan(), 0).unwrap();
        let b = mbr.to_bytes();
        assert_eq!(b[PARTITION_TABLE_OFFSET + 4], FileSystem::ExFat.mbr_type());
        assert_eq!(
            u32::from_le_bytes(
                b[PARTITION_TABLE_OFFSET + 8..PARTITION_TABLE_OFFSET + 12].try_into().unwrap()
            ),
            8192
        );
        assert_eq!(b[PARTITION_TABLE_OFFSET + 16 + 4], QUARANTINE_MBR_TYPE);
    }

    #[test]
    fn unused_slots_stay_zeroed() {
        let mbr = MasterBootRecord::from_plan(&sample_plan(), 0).unwrap();
        let b = mbr.to_bytes();
        for i in 2..4 {
            let at = PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE;
            assert!(b[at..at + PARTITION_ENTRY_SIZE].iter().all(|x| *x == 0));
        }
        assert_eq!(mbr.used_partitions().count(), 2);
    }

    #[test]
    fn no_partition_is_ever_marked_bootable() {
        let mbr = MasterBootRecord::from_plan(&sample_plan(), 0).unwrap();
        assert!(mbr.partitions.iter().all(|p| !p.bootable));
        assert_eq!(mbr.to_bytes()[PARTITION_TABLE_OFFSET], 0x00);
    }

    #[test]
    fn parsing_rejects_a_sector_without_the_signature() {
        let mut b = MasterBootRecord::empty(0).to_bytes();
        b[511] = 0x00;
        assert!(matches!(
            MasterBootRecord::from_bytes(&b),
            Err(MbrError::MissingBootSignature { .. })
        ));
    }

    #[test]
    fn parsing_rejects_a_wrong_sized_buffer() {
        assert_eq!(MasterBootRecord::from_bytes(&[0u8; 256]), Err(MbrError::WrongSectorSize(256)));
    }

    #[test]
    fn a_partition_beyond_32_bits_is_refused_rather_than_truncated() {
        let plan = PartitionPlan {
            strategy: Strategy::LargestContiguous,
            partitions: vec![PlannedPartition {
                range: LbaRange::from_bounds(8192, MAX_ADDRESSABLE_SECTORS + 10),
                role: PartitionRole::Data,
                mbr_type: 0x07,
                label: "huge".into(),
            }],
            geometry: DeviceGeometry::new(512, u64::MAX / 512).unwrap(),
            usable_bytes: 0,
            sacrificed_bytes: 0,
            containment: Containment::PartitionBoundary,
        };
        assert!(matches!(
            MasterBootRecord::from_plan(&plan, 0),
            Err(MbrError::ExceedsAddressableRange { .. })
        ));
    }

    #[test]
    fn chs_saturates_for_addresses_it_cannot_express() {
        assert_eq!(encode_chs(u32::MAX as u64), [0xFE, 0xFF, 0xFF]);
        assert_eq!(encode_chs(0), [0x00, 0x01, 0x00]);
        assert_eq!(encode_chs(62), [0x00, 0x3F, 0x00]);
        assert_eq!(encode_chs(63), [0x01, 0x01, 0x00]);
    }

    #[test]
    fn entry_range_matches_the_planned_range() {
        let mbr = MasterBootRecord::from_plan(&sample_plan(), 0).unwrap();
        assert_eq!(mbr.partitions[0].range(), LbaRange::from_bounds(8192, 1_000_000));
        assert_eq!(mbr.partitions[1].range(), LbaRange::from_bounds(1_000_000, 2_000_000));
    }

    #[test]
    fn change_description_lists_removals_and_additions() {
        let before = MasterBootRecord::from_plan(&sample_plan(), 1).unwrap();
        let after = MasterBootRecord::empty(1);
        let changes = describe_change(Some(&before), &after);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| !c.added));
    }

    /// The card is the only witness that survives being unplugged. Reading the
    /// layout back out of it is what lets a second inspection skip what the
    /// first one already condemned.
    #[test]
    fn a_layout_this_program_wrote_is_recognised_when_read_back() {
        let bytes = MasterBootRecord::from_plan(&sample_plan(), 0x1234_5678).unwrap().to_bytes();
        let parsed = MasterBootRecord::from_bytes(&bytes).unwrap();
        let prior = PriorLayout::read(&parsed).expect("our own layout should be recognised");

        assert_eq!(prior.data, vec![LbaRange::from_bounds(8192, 1_000_000)]);
        assert_eq!(prior.quarantined, vec![LbaRange::from_bounds(1_000_000, 2_000_000)]);
        assert_eq!(prior.data_span(), Some(LbaRange::from_bounds(8192, 1_000_000)));
    }

    #[test]
    fn a_table_with_no_quarantine_is_never_taken_for_one_of_ours() {
        let mut mbr = MasterBootRecord::empty(7);
        mbr.partitions[0] = PartitionEntry {
            bootable: false,
            partition_type: 0x07,
            start_lba: 8192,
            sector_count: 1_000_000,
        };
        assert_eq!(PriorLayout::read(&mbr), None);
    }

    /// Quarantine alone says nothing was left to use. Skipping part of the
    /// card on that basis would mean inspecting nothing at all.
    #[test]
    fn quarantine_with_no_data_partition_yields_no_layout() {
        let mut mbr = MasterBootRecord::empty(7);
        mbr.partitions[0] = PartitionEntry {
            bootable: false,
            partition_type: QUARANTINE_MBR_TYPE,
            start_lba: 8192,
            sector_count: 1_000_000,
        };
        assert_eq!(PriorLayout::read(&mbr), None);
    }

    #[test]
    fn an_empty_table_yields_no_layout() {
        assert_eq!(PriorLayout::read(&MasterBootRecord::empty(0)), None);
    }

    /// Every data partition has to fall inside the span, including one sitting
    /// past a stretch of quarantine: an interval that left it out would leave
    /// a mounted partition unproven.
    #[test]
    fn the_data_span_reaches_every_data_partition() {
        let mut mbr = MasterBootRecord::empty(7);
        mbr.partitions[0] = PartitionEntry {
            bootable: false,
            partition_type: 0x07,
            start_lba: 8192,
            sector_count: 100_000,
        };
        mbr.partitions[1] = PartitionEntry {
            bootable: false,
            partition_type: QUARANTINE_MBR_TYPE,
            start_lba: 108_192,
            sector_count: 50_000,
        };
        mbr.partitions[2] = PartitionEntry {
            bootable: false,
            partition_type: 0x07,
            start_lba: 158_192,
            sector_count: 200_000,
        };

        let prior = PriorLayout::read(&mbr).expect("data and quarantine are both present");
        let span = prior.data_span().unwrap();
        assert_eq!(span, LbaRange::from_bounds(8192, 358_192));
        for d in &prior.data {
            assert!(span.intersection(d).as_ref() == Some(d), "{d:?} fell outside the span");
        }
    }

    #[test]
    fn change_description_handles_an_unreadable_previous_table() {
        let after = MasterBootRecord::from_plan(&sample_plan(), 1).unwrap();
        let changes = describe_change(None, &after);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.added));
    }
}
