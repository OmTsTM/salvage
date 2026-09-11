//! FAT32 image generation with defective clusters marked in the table itself.
//!
//! # Why this exists
//!
//! A partition is an interval: a start LBA and a length. There is no way to
//! express "a partition made of scattered pieces", so fencing defects behind
//! partition boundaries can only ever hand back one contiguous run — whatever
//! is left over between the defects is unreachable.
//!
//! A filesystem has no such restriction. FAT32 reserves the entry value
//! [`BAD_CLUSTER`] for exactly this purpose: a cluster marked with it is never
//! allocated to a file, by any driver, on any operating system. Building the
//! table ourselves lets a single volume span the whole usable area while the
//! defects inside it are simply never handed out. The good runs end up spliced
//! into one drive letter whose free space is their sum.
//!
//! # What it costs
//!
//! Partition fencing dilates every condemned region by a guard band and aligns
//! it outward to the erase block, so data keeps a margin from any known defect.
//! A cluster map has no margin: the cluster next to a dead one stays available.
//! That is the trade, and it is the caller's to make — see
//! [`crate::planner::Strategy`].
//!
//! Both approaches address logical sectors, and the flash translation layer
//! remaps logical to physical on every write. Neither survives a controller
//! that is still moving data around; both hold when the boundary is imposed by
//! firmware, as it is on a card lying about its capacity.
//!
//! # Scope
//!
//! This module is pure: it computes a layout and renders individual sectors on
//! demand. It performs no I/O and never touches a device. The infrastructure
//! layer writes the sectors it produces.

use crate::geometry::LbaRange;
use crate::sector_map::SectorMap;

/// FAT entry marking a cluster as defective. Never allocated by any driver.
pub const BAD_CLUSTER: u32 = 0x0FFF_FFF7;

/// FAT entry marking the last cluster of a chain.
pub const END_OF_CHAIN: u32 = 0x0FFF_FFFF;

/// Only the low 28 bits of a FAT32 entry carry the cluster number.
const ENTRY_MASK: u32 = 0x0FFF_FFFF;

/// The first cluster number that addresses data. Entries 0 and 1 are reserved.
const FIRST_DATA_CLUSTER: u32 = 2;

/// Below this cluster count Windows reads the volume as FAT16 and refuses it.
///
/// The threshold is part of the on-disk format: the filesystem type is derived
/// from the cluster count, not from a field, so a FAT32 volume with fewer
/// clusters than this is not a smaller FAT32 volume — it is a malformed one.
const MIN_FAT32_CLUSTERS: u32 = 65_525;

/// Reserved sectors before the first FAT. The value every Windows formatter
/// writes, and the one that leaves room for the backup boot sector at 6.
const RESERVED_SECTORS: u32 = 32;

/// Two copies of the table, as every mainstream formatter produces.
const NUM_FATS: u32 = 2;

/// Sector holding the FSInfo structure.
const FSINFO_SECTOR: u32 = 1;

/// Sector holding the backup copy of the boot sector.
const BACKUP_BOOT_SECTOR: u32 = 6;

/// Reasons a FAT32 volume cannot be produced for a given area.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Fat32Error {
    /// The area is too small to hold a valid FAT32 volume.
    #[error("area holds {clusters} clusters, and FAT32 requires at least {minimum}")]
    TooSmall {
        /// Clusters the area would yield.
        clusters: u32,
        /// Minimum the format demands.
        minimum: u32,
    },
    /// The area exceeds what a 32-bit sector count can address.
    #[error("area spans {sectors} sectors, beyond the {limit} a FAT32 volume can address")]
    TooLarge {
        /// Sectors requested.
        sectors: u64,
        /// Largest addressable.
        limit: u64,
    },
    /// A defect falls inside the metadata, which no cluster marking can hide.
    ///
    /// The boot sector, the FATs and the root directory are not user data:
    /// there is nowhere else to put them, and a volume whose table cannot be
    /// read is not a volume.
    #[error("the area's first {metadata_sectors} sectors hold filesystem metadata, and sector {bad_lba} within them is defective")]
    DefectiveMetadata {
        /// Absolute LBA of the offending sector.
        bad_lba: u64,
        /// How many sectors at the start of the area are metadata.
        metadata_sectors: u64,
    },
    /// Sector size the format does not admit.
    #[error("sector size {0} is not supported by FAT32")]
    UnsupportedSectorSize(u32),
}

/// Geometry of a FAT32 volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fat32Layout {
    bytes_per_sector: u32,
    sectors_per_cluster: u32,
    fat_sectors: u32,
    total_sectors: u32,
    cluster_count: u32,
    volume_id: u32,
}

impl Fat32Layout {
    /// Computes the layout for an area of `total_sectors`.
    ///
    /// `volume_id` is the serial Windows displays; deriving it from the scan
    /// nonce makes each formatting distinguishable from the last.
    pub fn plan(
        total_sectors: u64,
        bytes_per_sector: u32,
        volume_id: u32,
    ) -> Result<Self, Fat32Error> {
        if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096) {
            return Err(Fat32Error::UnsupportedSectorSize(bytes_per_sector));
        }
        if total_sectors > u32::MAX as u64 {
            return Err(Fat32Error::TooLarge { sectors: total_sectors, limit: u32::MAX as u64 });
        }
        let total = total_sectors as u32;
        let sectors_per_cluster = Self::cluster_size(total_sectors, bytes_per_sector);

        // Microsoft's sizing formula. It solves, in integer arithmetic, for the
        // FAT size that describes the data region that remains once the FATs
        // themselves are subtracted — a circular dependency that is why the
        // expression is not simply "clusters times four".
        let usable = total.saturating_sub(RESERVED_SECTORS);
        let per_fat_sector = (bytes_per_sector / 4) * sectors_per_cluster + NUM_FATS;
        let mut fat_sectors = usable.div_ceil(per_fat_sector.max(1));

        // The formula sizes the table for the data clusters and forgets that
        // entries 0 and 1 also occupy four bytes each, which leaves the last
        // cluster described by bytes past the end of the table. Growing the FAT
        // shrinks the data region, so this settles after an iteration or two.
        let entries_per_sector = bytes_per_sector / 4;
        let cluster_count = loop {
            let data_sectors = usable.saturating_sub(fat_sectors.saturating_mul(NUM_FATS));
            let clusters = data_sectors / sectors_per_cluster;
            let needed =
                (clusters.saturating_add(FIRST_DATA_CLUSTER)).div_ceil(entries_per_sector.max(1));
            if needed <= fat_sectors {
                break clusters;
            }
            fat_sectors = needed;
        };

        if cluster_count < MIN_FAT32_CLUSTERS {
            return Err(Fat32Error::TooSmall {
                clusters: cluster_count,
                minimum: MIN_FAT32_CLUSTERS,
            });
        }

        Ok(Self {
            bytes_per_sector,
            sectors_per_cluster,
            fat_sectors,
            total_sectors: total,
            cluster_count,
            volume_id,
        })
    }

    /// Cluster size, following the table Windows itself uses.
    ///
    /// Larger clusters waste more space in partial files but shrink the table:
    /// at 32 KiB a 128 GB volume needs a 16 MB FAT, and at 4 KiB it would need
    /// eight times that, which has to be read and written on every mount.
    fn cluster_size(total_sectors: u64, bytes_per_sector: u32) -> u32 {
        let bytes = total_sectors * bytes_per_sector as u64;
        const GB: u64 = 1024 * 1024 * 1024;
        let target_bytes: u64 = match bytes {
            b if b <= 8 * GB => 4 * 1024,
            b if b <= 16 * GB => 8 * 1024,
            b if b <= 32 * GB => 16 * 1024,
            _ => 32 * 1024,
        };
        (target_bytes / bytes_per_sector as u64).max(1) as u32
    }

    /// Bytes in one sector.
    pub const fn bytes_per_sector(&self) -> u32 {
        self.bytes_per_sector
    }

    /// Sectors in one cluster.
    pub const fn sectors_per_cluster(&self) -> u32 {
        self.sectors_per_cluster
    }

    /// Clusters the data region holds, numbered from 2.
    pub const fn cluster_count(&self) -> u32 {
        self.cluster_count
    }

    /// Sectors occupied by one copy of the table.
    pub const fn fat_sectors(&self) -> u32 {
        self.fat_sectors
    }

    /// First sector of the data region, relative to the start of the volume.
    pub const fn data_start_sector(&self) -> u32 {
        RESERVED_SECTORS + self.fat_sectors * NUM_FATS
    }

    /// Sectors of metadata at the front of the volume, including the root
    /// directory's cluster. Every one of them must be defect-free.
    pub const fn metadata_sectors(&self) -> u64 {
        self.data_start_sector() as u64 + self.sectors_per_cluster as u64
    }

    /// Volume-relative sector where a cluster begins.
    const fn cluster_start_sector(&self, cluster: u32) -> u64 {
        self.data_start_sector() as u64
            + (cluster - FIRST_DATA_CLUSTER) as u64 * self.sectors_per_cluster as u64
    }

    /// Cluster containing a volume-relative sector, if it lies in the data
    /// region at all.
    const fn cluster_of_sector(&self, sector: u64) -> Option<u32> {
        let data_start = self.data_start_sector() as u64;
        if sector < data_start {
            return None;
        }
        let index = (sector - data_start) / self.sectors_per_cluster as u64;
        if index >= self.cluster_count as u64 {
            return None;
        }
        Some(index as u32 + FIRST_DATA_CLUSTER)
    }
}

/// A run of consecutive clusters, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterRun {
    /// First cluster in the run.
    pub start: u32,
    /// One past the last cluster.
    pub end: u32,
}

impl ClusterRun {
    /// Clusters in the run.
    pub const fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the run covers nothing.
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    const fn contains(&self, cluster: u32) -> bool {
        cluster >= self.start && cluster < self.end
    }
}

/// Translates the map's condemned sectors into the clusters covering them.
///
/// Any cluster that touches a condemned sector is condemned whole: a cluster is
/// the smallest unit a filesystem allocates, so handing one out because most of
/// it is healthy would hand out the defective part along with it.
///
/// Sectors the scan never verified are condemned too. The rule is the same one
/// the partition planner applies — only proven-good area is ever offered — and
/// it is what keeps this path from being the weaker of the two on safety.
pub fn condemned_clusters(
    map: &SectorMap,
    area: &LbaRange,
    layout: &Fat32Layout,
) -> Result<Vec<ClusterRun>, Fat32Error> {
    let mut runs: Vec<ClusterRun> = Vec::new();
    let metadata = layout.metadata_sectors();
    let last_data_sector = layout
        .cluster_start_sector(layout.cluster_count() + FIRST_DATA_CLUSTER - 1)
        + layout.sectors_per_cluster() as u64
        - 1;

    let mut condemn = |rel_start: u64, rel_end: u64| -> Result<(), Fat32Error> {
        if rel_end <= rel_start {
            return Ok(());
        }
        if rel_start < metadata {
            return Err(Fat32Error::DefectiveMetadata {
                bad_lba: area.start() + rel_start,
                metadata_sectors: metadata,
            });
        }
        // Clamp to the data region: the tail of a volume that does not divide
        // evenly into clusters belongs to no cluster and needs no marking.
        let last = rel_end.saturating_sub(1).min(last_data_sector);
        let (Some(a), Some(b)) =
            (layout.cluster_of_sector(rel_start), layout.cluster_of_sector(last))
        else {
            return Ok(());
        };
        match runs.last_mut() {
            // Adjacent or overlapping runs merge, which keeps the list short
            // even when the defects are heavily fragmented.
            Some(prev) if a <= prev.end => prev.end = prev.end.max(b + 1),
            _ => runs.push(ClusterRun { start: a, end: b + 1 }),
        }
        Ok(())
    };

    // Walking the proven-good runs and condemning everything between them means
    // area the map never mentions — because the scan never reached it — is
    // withheld by construction rather than by remembering to check for it.
    let mut cursor = area.start();
    for run in map.runs() {
        if !run.state.is_usable() {
            continue;
        }
        let Some(good) = intersect(&run.range, area) else {
            continue;
        };
        if good.start() > cursor {
            condemn(cursor - area.start(), good.start() - area.start())?;
        }
        cursor = cursor.max(good.end());
    }
    condemn(cursor - area.start(), area.end() - area.start())?;

    Ok(runs)
}

fn intersect(a: &LbaRange, b: &LbaRange) -> Option<LbaRange> {
    let start = a.start().max(b.start());
    let end = a.end().min(b.end());
    (start < end).then(|| LbaRange::from_bounds(start, end))
}

/// Renders the sectors of a FAT32 volume on demand.
///
/// Only the metadata is rendered: the reserved sectors, both FATs and the root
/// directory. The data region needs no initialization — a cluster nobody
/// allocates is never read.
#[derive(Debug)]
pub struct Fat32Image {
    layout: Fat32Layout,
    condemned: Vec<ClusterRun>,
    label: [u8; 11],
    hidden_sectors: u32,
}

impl Fat32Image {
    /// Builds an image for `area`, condemning every cluster the map does not
    /// prove good.
    pub fn new(
        map: &SectorMap,
        area: &LbaRange,
        volume_id: u32,
        label: &str,
    ) -> Result<Self, Fat32Error> {
        let sector_size = map.geometry().sector_size();
        let layout = Fat32Layout::plan(area.len(), sector_size, volume_id)?;
        let condemned = condemned_clusters(map, area, &layout)?;
        Ok(Self {
            layout,
            condemned,
            label: encode_label(label),
            hidden_sectors: area.start().min(u32::MAX as u64) as u32,
        })
    }

    /// The volume's geometry.
    pub const fn layout(&self) -> &Fat32Layout {
        &self.layout
    }

    /// Clusters withheld from allocation.
    pub fn condemned_clusters(&self) -> u32 {
        self.condemned.iter().map(ClusterRun::len).sum()
    }

    /// Bytes the volume will actually offer, once the defects are withheld.
    pub fn usable_bytes(&self) -> u64 {
        let free = self.layout.cluster_count.saturating_sub(self.condemned_clusters());
        // The root directory occupies one cluster and is not free space.
        free.saturating_sub(1) as u64
            * self.layout.sectors_per_cluster as u64
            * self.layout.bytes_per_sector as u64
    }

    /// Sectors that must be written, counted from the start of the volume.
    pub const fn sectors_to_write(&self) -> u64 {
        self.layout.metadata_sectors()
    }

    /// Whether a cluster is withheld.
    fn is_condemned(&self, cluster: u32) -> bool {
        // The runs are sorted and merged, so a binary search settles it without
        // walking a list that can hold thousands of entries on a bad card.
        self.condemned
            .binary_search_by(|r| {
                if r.contains(cluster) {
                    std::cmp::Ordering::Equal
                } else if r.end <= cluster {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .is_ok()
    }

    /// The FAT entry for a cluster.
    fn fat_entry(&self, cluster: u32) -> u32 {
        match cluster {
            // Entry 0 carries the media descriptor in its low byte; entry 1 is
            // the "dirty" flags word. Both are format constants, not clusters.
            0 => 0x0FFF_FFF8,
            1 => END_OF_CHAIN,
            _ if cluster >= self.layout.cluster_count + FIRST_DATA_CLUSTER => 0,
            c if c == FIRST_DATA_CLUSTER => END_OF_CHAIN, // the root directory
            c if self.is_condemned(c) => BAD_CLUSTER,
            _ => 0,
        }
    }

    /// The table entry for a cluster, for tests in sibling modules that need to
    /// assert what the volume will and will not hand out.
    #[doc(hidden)]
    pub fn fat_entry_for_test(&self, cluster: u32) -> u32 {
        self.fat_entry(cluster)
    }

    /// Fills `out` with the content of one volume-relative sector.
    ///
    /// `out` must be exactly one sector long. Sectors outside the metadata
    /// region are left as zeros.
    pub fn render_sector(&self, sector: u64, out: &mut [u8]) {
        debug_assert_eq!(out.len(), self.layout.bytes_per_sector as usize);
        out.fill(0);

        let ss = self.layout.bytes_per_sector as u64;
        let fat_start = RESERVED_SECTORS as u64;
        let fat_len = self.layout.fat_sectors as u64;

        if sector == 0 || sector == BACKUP_BOOT_SECTOR as u64 {
            self.write_boot_sector(out);
        } else if sector == FSINFO_SECTOR as u64
            || sector == (BACKUP_BOOT_SECTOR + FSINFO_SECTOR) as u64
        {
            self.write_fsinfo(out);
        } else if sector >= fat_start && sector < fat_start + fat_len * NUM_FATS as u64 {
            // Both copies hold identical content, so the offset within whichever
            // copy this is gives the first cluster the sector describes.
            let within = (sector - fat_start) % fat_len;
            let entries_per_sector = ss / 4;
            let first = (within * entries_per_sector) as u32;
            for i in 0..entries_per_sector as usize {
                let cluster = first.saturating_add(i as u32);
                let value = self.fat_entry(cluster) & ENTRY_MASK;
                out[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        } else if sector == self.layout.cluster_start_sector(FIRST_DATA_CLUSTER) {
            // First sector of the root directory: the volume label lives here,
            // and the rest of the cluster stays zeroed, which reads as "no more
            // entries".
            out[..11].copy_from_slice(&self.label);
            out[11] = 0x08; // ATTR_VOLUME_ID
        }
    }

    fn write_boot_sector(&self, out: &mut [u8]) {
        let l = &self.layout;
        out[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]); // jmp short +0x58; nop
        out[3..11].copy_from_slice(b"MSWIN4.1"); // the OEM name Windows expects
        out[11..13].copy_from_slice(&(l.bytes_per_sector as u16).to_le_bytes());
        out[13] = l.sectors_per_cluster as u8;
        out[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes());
        out[16] = NUM_FATS as u8;
        out[17..19].copy_from_slice(&0u16.to_le_bytes()); // no fixed root dir
        out[19..21].copy_from_slice(&0u16.to_le_bytes()); // 16-bit count unused
        out[21] = 0xF8; // fixed media
        out[22..24].copy_from_slice(&0u16.to_le_bytes()); // 16-bit FAT size unused
        out[24..26].copy_from_slice(&63u16.to_le_bytes());
        out[26..28].copy_from_slice(&255u16.to_le_bytes());
        out[28..32].copy_from_slice(&self.hidden_sectors.to_le_bytes());
        out[32..36].copy_from_slice(&l.total_sectors.to_le_bytes());
        out[36..40].copy_from_slice(&l.fat_sectors.to_le_bytes());
        out[40..42].copy_from_slice(&0u16.to_le_bytes()); // both FATs live
        out[42..44].copy_from_slice(&0u16.to_le_bytes()); // version 0.0
        out[44..48].copy_from_slice(&FIRST_DATA_CLUSTER.to_le_bytes());
        out[48..50].copy_from_slice(&(FSINFO_SECTOR as u16).to_le_bytes());
        out[50..52].copy_from_slice(&(BACKUP_BOOT_SECTOR as u16).to_le_bytes());
        out[64] = 0x80; // drive number
        out[66] = 0x29; // extended boot signature: the three fields below are valid
        out[67..71].copy_from_slice(&l.volume_id.to_le_bytes());
        out[71..82].copy_from_slice(&self.label);
        out[82..90].copy_from_slice(b"FAT32   ");

        let end = self.layout.bytes_per_sector as usize;
        out[end - 2] = 0x55;
        out[end - 1] = 0xAA;
    }

    fn write_fsinfo(&self, out: &mut [u8]) {
        out[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes()); // "RRaA"
        out[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes()); // "rrAa"

        // Free count excludes both the condemned clusters and the root
        // directory. Windows trusts this number for the size it displays, so a
        // wrong one shows the user free space that does not exist.
        let free =
            self.layout.cluster_count.saturating_sub(self.condemned_clusters()).saturating_sub(1);
        out[488..492].copy_from_slice(&free.to_le_bytes());

        // Hint for the first free cluster. Pointing it past the root directory
        // saves the driver a scan; being wrong costs nothing but time.
        out[492..496].copy_from_slice(&(FIRST_DATA_CLUSTER + 1).to_le_bytes());

        let end = self.layout.bytes_per_sector as usize;
        out[end - 4..end].copy_from_slice(&[0x00, 0x00, 0x55, 0xAA]);
    }
}

/// Encodes a volume label into the 11-byte, space-padded, upper-case field.
fn encode_label(label: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    for (slot, ch) in out.iter_mut().zip(label.chars()) {
        // The field is OEM-encoded and rejects lower case and most punctuation.
        // Anything outside the safe set becomes an underscore rather than
        // producing a label Windows will not display.
        *slot = match ch.to_ascii_uppercase() {
            c @ ('A'..='Z' | '0'..='9' | '_' | '-') if c.is_ascii() => c as u8,
            _ => b'_',
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::DeviceGeometry;
    use crate::sector_map::SectorState;

    const SS: u32 = 512;

    /// 32 GB: large enough for a realistic layout, small enough to keep the
    /// rendered FAT cheap to walk in a test.
    const SECTORS: u64 = 62_914_560;

    fn map_with(defects: &[(u64, u64, SectorState)]) -> SectorMap {
        let mut m = SectorMap::new(DeviceGeometry::new(SS, SECTORS).unwrap());
        m.mark(LbaRange::from_bounds(0, SECTORS), SectorState::Good);
        for (a, b, s) in defects {
            m.mark(LbaRange::from_bounds(*a, *b), *s);
        }
        m
    }

    fn area() -> LbaRange {
        LbaRange::from_bounds(0, SECTORS)
    }

    fn image(defects: &[(u64, u64, SectorState)]) -> Fat32Image {
        Fat32Image::new(&map_with(defects), &area(), 0x1234_5678, "SALVAGE").unwrap()
    }

    fn sector(img: &Fat32Image, index: u64) -> Vec<u8> {
        let mut buf = vec![0u8; SS as usize];
        img.render_sector(index, &mut buf);
        buf
    }

    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    /// Reads one FAT entry by rendering the sector that holds it, which is how
    /// a driver would reach it.
    fn fat_entry(img: &Fat32Image, cluster: u32) -> u32 {
        let per_sector = SS / 4;
        let s = RESERVED_SECTORS as u64 + (cluster / per_sector) as u64;
        let b = sector(img, s);
        u32_at(&b, (cluster % per_sector) as usize * 4)
    }

    #[test]
    fn the_boot_sector_carries_the_fields_a_driver_reads() {
        let img = image(&[]);
        let b = sector(&img, 0);
        let l = img.layout();

        assert_eq!(&b[3..11], b"MSWIN4.1");
        assert_eq!(u16_at(&b, 11), SS as u16, "bytes per sector");
        assert_eq!(b[13] as u32, l.sectors_per_cluster(), "sectors per cluster");
        assert_eq!(u16_at(&b, 14), RESERVED_SECTORS as u16);
        assert_eq!(b[16] as u32, NUM_FATS);
        assert_eq!(u16_at(&b, 17), 0, "FAT32 has no fixed-size root directory");
        assert_eq!(u16_at(&b, 22), 0, "the 16-bit FAT size must be zero on FAT32");
        assert_eq!(u32_at(&b, 32), SECTORS as u32, "total sectors");
        assert_eq!(u32_at(&b, 36), l.fat_sectors(), "FAT size");
        assert_eq!(u32_at(&b, 44), FIRST_DATA_CLUSTER, "root cluster");
        assert_eq!(u16_at(&b, 48), FSINFO_SECTOR as u16);
        assert_eq!(u16_at(&b, 50), BACKUP_BOOT_SECTOR as u16);
        assert_eq!(&b[82..90], b"FAT32   ");
        assert_eq!(&b[510..512], &[0x55, 0xAA], "boot signature");
    }

    /// Windows reads the backup when the primary will not parse. If they differ,
    /// the recovery path lands on a different volume than the one written.
    #[test]
    fn the_backup_boot_sector_is_identical_to_the_primary() {
        let img = image(&[]);
        assert_eq!(sector(&img, 0), sector(&img, BACKUP_BOOT_SECTOR as u64));
        assert_eq!(
            sector(&img, FSINFO_SECTOR as u64),
            sector(&img, (BACKUP_BOOT_SECTOR + FSINFO_SECTOR) as u64)
        );
    }

    #[test]
    fn the_reserved_entries_and_the_root_directory_are_well_formed() {
        let img = image(&[]);
        assert_eq!(fat_entry(&img, 0), 0x0FFF_FFF8, "media descriptor entry");
        assert_eq!(fat_entry(&img, 1), END_OF_CHAIN, "flags entry");
        assert_eq!(fat_entry(&img, 2), END_OF_CHAIN, "root directory is a one-cluster chain");
    }

    /// The whole point of this module: a defective sector must reach the table
    /// as a cluster no driver will allocate.
    #[test]
    fn a_defective_sector_becomes_a_bad_cluster_in_the_table() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        // A sector in the middle of the data region, well past the metadata.
        let target = l.data_start_sector() as u64 + l.sectors_per_cluster() as u64 * 5_000 + 3;
        let img = image(&[(target, target + 1, SectorState::Corrupt)]);

        let cluster = FIRST_DATA_CLUSTER + 5_000;
        assert_eq!(fat_entry(&img, cluster), BAD_CLUSTER, "the defect did not reach the table");
        assert_eq!(fat_entry(&img, cluster - 1), 0, "a neighbour was condemned with it");
        assert_eq!(fat_entry(&img, cluster + 1), 0, "a neighbour was condemned with it");
        assert_eq!(img.condemned_clusters(), 1);
    }

    /// Both copies must agree: a driver may read either, and Windows compares
    /// them during repair.
    #[test]
    fn both_copies_of_the_table_are_identical() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        let target = l.data_start_sector() as u64 + l.sectors_per_cluster() as u64 * 900;
        let img = image(&[(target, target + 8, SectorState::BadRead)]);

        for offset in [0u64, 1, 17, l.fat_sectors() as u64 - 1] {
            let primary = sector(&img, RESERVED_SECTORS as u64 + offset);
            let backup = sector(&img, RESERVED_SECTORS as u64 + l.fat_sectors() as u64 + offset);
            assert_eq!(primary, backup, "the copies diverge at FAT sector {offset}");
        }
    }

    /// A cluster is the smallest unit a filesystem hands out, so a defect
    /// anywhere inside one condemns all of it — otherwise the allocation would
    /// include the dead sectors.
    #[test]
    fn one_bad_sector_condemns_its_whole_cluster_and_no_more() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        let spc = l.sectors_per_cluster() as u64;
        let base = l.data_start_sector() as u64 + spc * 100;

        // A single sector at the very end of a cluster.
        let img = image(&[(base + spc - 1, base + spc, SectorState::Corrupt)]);
        assert_eq!(img.condemned_clusters(), 1);
        assert_eq!(fat_entry(&img, FIRST_DATA_CLUSTER + 100), BAD_CLUSTER);
        assert_eq!(
            fat_entry(&img, FIRST_DATA_CLUSTER + 101),
            0,
            "a defect ending on the boundary spilled into the next cluster"
        );
    }

    /// Never-verified area is withheld exactly like proven defects. This is the
    /// invariant that keeps the spliced layout as safe as the fenced one on the
    /// question that matters: data only ever lands on proven-good sectors.
    #[test]
    fn unverified_area_is_withheld_just_like_a_defect() {
        let mut m = SectorMap::new(DeviceGeometry::new(SS, SECTORS).unwrap());
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        let cut = l.data_start_sector() as u64 + l.sectors_per_cluster() as u64 * 1_000;
        // Everything proven up to `cut`; the rest never looked at.
        m.mark(LbaRange::from_bounds(0, cut), SectorState::Good);

        let img = Fat32Image::new(&m, &area(), 0, "SALVAGE").unwrap();
        assert_eq!(
            img.condemned_clusters(),
            l.cluster_count() - 1_000,
            "uninspected clusters were left allocatable"
        );
        assert_eq!(fat_entry(&img, FIRST_DATA_CLUSTER + 1_000), BAD_CLUSTER);
    }

    /// There is nowhere else to put the boot sector, the tables or the root
    /// directory. A defect among them cannot be marked around, and pretending
    /// otherwise would produce a volume that fails on first mount.
    #[test]
    fn a_defect_in_the_metadata_is_refused_rather_than_marked() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        for bad in [0u64, 1, RESERVED_SECTORS as u64 + 5, l.data_start_sector() as u64 + 1] {
            let m = map_with(&[(bad, bad + 1, SectorState::Corrupt)]);
            let err = Fat32Image::new(&m, &area(), 0, "SALVAGE").unwrap_err();
            assert!(
                matches!(err, Fat32Error::DefectiveMetadata { .. }),
                "sector {bad} produced {err:?} instead of a refusal"
            );
        }
    }

    /// A FAT32 volume below the cluster threshold is not a small FAT32 volume;
    /// it is one Windows reads as FAT16 and rejects.
    #[test]
    fn an_area_too_small_for_the_format_is_refused() {
        let err = Fat32Layout::plan(1_000, SS, 0).unwrap_err();
        assert!(matches!(err, Fat32Error::TooSmall { .. }), "got {err:?}");

        // Just over the threshold must still work, or the boundary is wrong.
        let smallest = (MIN_FAT32_CLUSTERS as u64 + 16) * 8 + RESERVED_SECTORS as u64 + 4_096;
        assert!(Fat32Layout::plan(smallest, SS, 0).is_ok());
    }

    /// The FAT must actually fit in the sectors the layout reserved for it, or
    /// the tail clusters would be described by bytes the volume does not have.
    #[test]
    fn the_table_is_large_enough_for_every_cluster_it_describes() {
        for sectors in [SECTORS, 8_388_608, 125_829_120, 245_760_000] {
            let l = Fat32Layout::plan(sectors, SS, 0).unwrap();
            let entries = l.fat_sectors() as u64 * (SS / 4) as u64;
            let needed = l.cluster_count() as u64 + FIRST_DATA_CLUSTER as u64;
            assert!(entries >= needed, "{sectors} sectors: FAT holds {entries}, needs {needed}");

            let consumed = RESERVED_SECTORS as u64
                + l.fat_sectors() as u64 * NUM_FATS as u64
                + l.cluster_count() as u64 * l.sectors_per_cluster() as u64;
            assert!(consumed <= sectors, "{sectors} sectors: layout claims {consumed}");
        }
    }

    /// The sizes Windows will not format.
    ///
    /// `format.com` refuses FAT32 above 32 GB, so past that this is the only
    /// code that makes the volume — and nothing else would notice if the plan
    /// stopped being a legal FAT32 up there. Every entry must still address a
    /// cluster the 28-bit field can name, and the count must stay above the
    /// floor that separates FAT32 from FAT16.
    #[test]
    fn a_volume_larger_than_windows_will_format_is_still_a_legal_fat32() {
        // 33 GB, 64 GB, 128 GB and 256 GB, in 512-byte sectors.
        for gigabytes in [33u64, 64, 128, 256] {
            let sectors = gigabytes * 1024 * 1024 * 1024 / SS as u64;
            let l = Fat32Layout::plan(sectors, SS, 0)
                .unwrap_or_else(|e| panic!("{gigabytes} GB was refused: {e}"));

            assert!(
                l.cluster_count() >= MIN_FAT32_CLUSTERS,
                "{gigabytes} GB: {} clusters is FAT16 territory",
                l.cluster_count()
            );
            // The entry is 32 bits with the top four reserved, so no cluster
            // number may reach into them.
            assert!(
                (l.cluster_count() as u64 + FIRST_DATA_CLUSTER as u64) < 0x0FFF_FFF7,
                "{gigabytes} GB: the last cluster number collides with the bad-cluster mark"
            );

            let entries = l.fat_sectors() as u64 * (SS / 4) as u64;
            assert!(
                entries >= l.cluster_count() as u64 + FIRST_DATA_CLUSTER as u64,
                "{gigabytes} GB: the table cannot address its own clusters"
            );
            let consumed = RESERVED_SECTORS as u64
                + l.fat_sectors() as u64 * NUM_FATS as u64
                + l.cluster_count() as u64 * l.sectors_per_cluster() as u64;
            assert!(consumed <= sectors, "{gigabytes} GB: the layout claims {consumed} sectors");
        }
    }

    #[test]
    fn the_free_count_reported_to_windows_excludes_the_defects() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        let spc = l.sectors_per_cluster() as u64;
        let base = l.data_start_sector() as u64 + spc * 2_000;
        let img = image(&[(base, base + spc * 50, SectorState::Corrupt)]);

        let fsinfo = sector(&img, FSINFO_SECTOR as u64);
        assert_eq!(u32_at(&fsinfo, 0), 0x4161_5252);
        assert_eq!(u32_at(&fsinfo, 484), 0x6141_7272);
        assert_eq!(&fsinfo[508..512], &[0x00, 0x00, 0x55, 0xAA]);

        let free = u32_at(&fsinfo, 488);
        assert_eq!(free, l.cluster_count() - 50 - 1, "free count ignored the bad clusters");
        assert_eq!(img.usable_bytes(), free as u64 * spc * SS as u64);
    }

    #[test]
    fn the_volume_label_is_padded_and_sanitised() {
        assert_eq!(&encode_label("SALVAGE"), b"SALVAGE    ");
        assert_eq!(&encode_label("salvage"), b"SALVAGE    ");
        assert_eq!(&encode_label("a b"), b"A_B        ");
        assert_eq!(&encode_label("A VERY LONG LABEL"), b"A_VERY_LONG");
        assert_eq!(&encode_label(""), b"           ");
    }

    /// The root directory must be readable, or the volume mounts empty and
    /// Windows offers to format it.
    #[test]
    fn the_root_directory_holds_the_volume_label() {
        let img = image(&[]);
        let l = img.layout();
        let root = sector(&img, l.data_start_sector() as u64);
        assert_eq!(&root[..11], b"SALVAGE    ");
        assert_eq!(root[11], 0x08, "the entry is not marked as a volume label");
    }

    /// The condemned runs are searched by bisection, which is only valid while
    /// they are sorted and disjoint.
    #[test]
    fn condemned_runs_come_back_sorted_and_merged() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        let spc = l.sectors_per_cluster() as u64;
        let ds = l.data_start_sector() as u64;
        let img = image(&[
            (ds + spc * 700, ds + spc * 702, SectorState::Corrupt),
            (ds + spc * 100, ds + spc * 101, SectorState::BadRead),
            // Adjacent to the previous one: the two must merge into one run.
            (ds + spc * 101, ds + spc * 103, SectorState::Corrupt),
        ]);

        assert!(
            img.condemned.windows(2).all(|w| w[0].end < w[1].start),
            "runs are not sorted and disjoint: {:?}",
            img.condemned
        );
        assert_eq!(img.condemned_clusters(), 5);
        for c in [100u32, 101, 102, 700, 701] {
            assert_eq!(
                fat_entry(&img, FIRST_DATA_CLUSTER + c),
                BAD_CLUSTER,
                "cluster {c} was left allocatable"
            );
        }
        assert_eq!(fat_entry(&img, FIRST_DATA_CLUSTER + 103), 0);
    }

    /// Every entry the table can address must be one of the four legal values.
    /// A stray value is how a volume mounts and then corrupts itself.
    #[test]
    fn no_entry_is_ever_written_outside_the_legal_set() {
        let l = Fat32Layout::plan(SECTORS, SS, 0).unwrap();
        let spc = l.sectors_per_cluster() as u64;
        let ds = l.data_start_sector() as u64;
        let img = image(&[(ds + spc * 40, ds + spc * 44, SectorState::Aliased)]);

        // Walking every FAT sector is the only way to catch an entry produced
        // by the rendering rather than by `fat_entry`.
        let mut buf = vec![0u8; SS as usize];
        for s in 0..l.fat_sectors() as u64 {
            img.render_sector(RESERVED_SECTORS as u64 + s, &mut buf);
            for e in buf.chunks_exact(4) {
                let v = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
                assert!(
                    matches!(v, 0 | BAD_CLUSTER | END_OF_CHAIN | 0x0FFF_FFF8),
                    "illegal FAT entry {v:#010x} in sector {s}"
                );
            }
        }
    }
}
