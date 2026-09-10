//! Device geometry and LBA interval algebra.
//!
//! An [`LbaRange`] is a half-open interval `[start, end)` measured in sectors.
//! All arithmetic here saturates rather than wrapping. A failing card can
//! report nonsense geometry, and a silent overflow in this module would turn
//! into a write landing somewhere it should never reach.

use serde::{Deserialize, Serialize};

/// Sector size assumed when a device reports none.
pub const DEFAULT_SECTOR_SIZE: u32 = 512;

/// Default partition alignment: 4 MiB.
///
/// This is the typical erase-block size of modern microSD NAND. Aligning to
/// that boundary keeps the data partition from sharing an erase block with a
/// region that has been condemned.
pub const DEFAULT_ALIGNMENT_BYTES: u64 = 4 * 1024 * 1024;

/// Errors produced while constructing a geometry.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GeometryError {
    /// Sector size is not one of the supported powers of two.
    #[error("invalid sector size: {0} (expected 512, 1024, 2048 or 4096)")]
    InvalidSectorSize(u32),
    /// The device reports no addressable sectors.
    #[error("device reports zero sectors")]
    EmptyDevice,
}

/// Geometry as reported by the device.
///
/// "Reported" is the operative word: a counterfeit card lies precisely here,
/// and exposing that lie is one of this tool's objectives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGeometry {
    sector_size: u32,
    total_sectors: u64,
}

impl DeviceGeometry {
    /// Builds a geometry, validating the sector size.
    pub fn new(sector_size: u32, total_sectors: u64) -> Result<Self, GeometryError> {
        if !matches!(sector_size, 512 | 1024 | 2048 | 4096) {
            return Err(GeometryError::InvalidSectorSize(sector_size));
        }
        if total_sectors == 0 {
            return Err(GeometryError::EmptyDevice);
        }
        Ok(Self { sector_size, total_sectors })
    }

    /// Sector size in bytes.
    #[inline]
    pub const fn sector_size(&self) -> u32 {
        self.sector_size
    }

    /// Total number of addressable sectors.
    #[inline]
    pub const fn total_sectors(&self) -> u64 {
        self.total_sectors
    }

    /// Total capacity in bytes, as reported by the device.
    #[inline]
    pub const fn total_bytes(&self) -> u64 {
        self.total_sectors.saturating_mul(self.sector_size as u64)
    }

    /// Range covering the whole device.
    #[inline]
    pub const fn full_range(&self) -> LbaRange {
        LbaRange { start: 0, end: self.total_sectors }
    }

    /// Converts a byte count into sectors, rounding up.
    #[inline]
    pub const fn bytes_to_sectors_ceil(&self, bytes: u64) -> u64 {
        bytes.div_ceil(self.sector_size as u64)
    }

    /// Default alignment expressed in sectors. Never zero.
    #[inline]
    pub const fn default_alignment_sectors(&self) -> u64 {
        let a = DEFAULT_ALIGNMENT_BYTES / self.sector_size as u64;
        if a == 0 {
            1
        } else {
            a
        }
    }
}

/// Half-open interval of logical block addresses: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LbaRange {
    start: u64,
    end: u64,
}

impl LbaRange {
    /// Builds from a start and a length. A zero length yields an empty range.
    #[inline]
    pub const fn new(start: u64, len: u64) -> Self {
        Self { start, end: start.saturating_add(len) }
    }

    /// Builds from bounds. If `end <= start`, the result is empty at `start`.
    #[inline]
    pub const fn from_bounds(start: u64, end: u64) -> Self {
        if end <= start {
            Self { start, end: start }
        } else {
            Self { start, end }
        }
    }

    /// First LBA in the range.
    #[inline]
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// First LBA past the range.
    #[inline]
    pub const fn end(&self) -> u64 {
        self.end
    }

    /// Number of sectors covered.
    #[inline]
    pub const fn len(&self) -> u64 {
        self.end - self.start
    }

    /// True when the range covers no sectors.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// True when `lba` falls inside the range.
    #[inline]
    pub const fn contains(&self, lba: u64) -> bool {
        lba >= self.start && lba < self.end
    }

    /// True when the two ranges share at least one sector.
    #[inline]
    pub const fn intersects(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// True when the ranges overlap or merely abut (`a.end == b.start`).
    #[inline]
    pub const fn touches_or_intersects(&self, other: &Self) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// The portion common to both ranges, if any.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let start = if self.start > other.start { self.start } else { other.start };
        let end = if self.end < other.end { self.end } else { other.end };
        (start < end).then_some(Self { start, end })
    }

    /// Smallest range containing both.
    #[inline]
    pub fn hull(&self, other: &Self) -> Self {
        Self { start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    /// Grows the range by `margin` sectors on each side, saturating at zero.
    #[inline]
    pub const fn expanded_by(&self, margin: u64) -> Self {
        Self { start: self.start.saturating_sub(margin), end: self.end.saturating_add(margin) }
    }

    /// Grows to `alignment` boundaries, always containing the original.
    ///
    /// Used so that a condemned region swallows the entire erase block holding
    /// it: neighbouring bad blocks tend to degrade together.
    pub fn aligned_outward(&self, alignment: u64) -> Self {
        if alignment <= 1 {
            return *self;
        }
        let start = self.start - (self.start % alignment);
        let rem = self.end % alignment;
        let end = if rem == 0 { self.end } else { self.end.saturating_add(alignment - rem) };
        Self { start, end }
    }

    /// Shrinks to `alignment` boundaries, always contained by the original.
    ///
    /// Used for partitions, which must never extend past the approved region.
    pub fn aligned_inward(&self, alignment: u64) -> Self {
        if alignment <= 1 {
            return *self;
        }
        let rem = self.start % alignment;
        let start = if rem == 0 { self.start } else { self.start.saturating_add(alignment - rem) };
        let end = self.end - (self.end % alignment);
        Self::from_bounds(start, end)
    }

    /// Restricts the range to the given bounds.
    pub fn clamped_to(&self, bounds: &Self) -> Self {
        self.intersection(bounds).unwrap_or(Self { start: bounds.start, end: bounds.start })
    }

    /// Removes `other`, returning the surviving left and right fragments.
    pub fn subtract(&self, other: &Self) -> (Option<Self>, Option<Self>) {
        if !self.intersects(other) {
            return (Some(*self), None);
        }
        let left =
            (self.start < other.start).then_some(Self { start: self.start, end: other.start });
        let right = (other.end < self.end).then_some(Self { start: other.end, end: self.end });
        (left, right)
    }

    /// Splits the range into chunks of at most `chunk` sectors.
    pub fn chunks(&self, chunk: u64) -> impl Iterator<Item = Self> + '_ {
        let chunk = chunk.max(1);
        let mut cursor = self.start;
        std::iter::from_fn(move || {
            if cursor >= self.end {
                return None;
            }
            let end = cursor.saturating_add(chunk).min(self.end);
            let r = Self { start: cursor, end };
            cursor = end;
            Some(r)
        })
    }

    /// Size of the range in bytes, given a geometry.
    #[inline]
    pub const fn byte_len(&self, geometry: &DeviceGeometry) -> u64 {
        self.len().saturating_mul(geometry.sector_size() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_rejects_invalid_input() {
        assert_eq!(DeviceGeometry::new(999, 100), Err(GeometryError::InvalidSectorSize(999)));
        assert_eq!(DeviceGeometry::new(512, 0), Err(GeometryError::EmptyDevice));
        assert!(DeviceGeometry::new(512, 100).is_ok());
        assert!(DeviceGeometry::new(4096, 100).is_ok());
    }

    #[test]
    fn total_bytes_saturates_instead_of_overflowing() {
        let g = DeviceGeometry::new(4096, u64::MAX).unwrap();
        assert_eq!(g.total_bytes(), u64::MAX);
    }

    #[test]
    fn range_basics() {
        let r = LbaRange::new(10, 5);
        assert_eq!((r.start(), r.end(), r.len()), (10, 15, 5));
        assert!(r.contains(10) && r.contains(14));
        assert!(!r.contains(15) && !r.contains(9));
        assert!(LbaRange::from_bounds(10, 10).is_empty());
        assert!(LbaRange::from_bounds(20, 10).is_empty());
    }

    #[test]
    fn intersection_and_subtraction() {
        let a = LbaRange::from_bounds(0, 100);
        let b = LbaRange::from_bounds(40, 60);
        assert_eq!(a.intersection(&b), Some(b));

        let (l, r) = a.subtract(&b);
        assert_eq!(l, Some(LbaRange::from_bounds(0, 40)));
        assert_eq!(r, Some(LbaRange::from_bounds(60, 100)));

        // Subtracting something disjoint returns the original untouched.
        assert_eq!(a.subtract(&LbaRange::from_bounds(200, 300)), (Some(a), None));

        // Subtracting a superset leaves nothing.
        assert_eq!(b.subtract(&a), (None, None));
    }

    #[test]
    fn touching_ranges_do_not_intersect_but_do_touch() {
        let a = LbaRange::from_bounds(0, 10);
        let b = LbaRange::from_bounds(10, 20);
        assert!(!a.intersects(&b));
        assert!(a.touches_or_intersects(&b));
    }

    #[test]
    fn outward_alignment_always_contains_the_original() {
        let r = LbaRange::from_bounds(9000, 9001);
        let a = r.aligned_outward(8192);
        assert_eq!(a, LbaRange::from_bounds(8192, 16384));
        assert!(a.start() <= r.start() && a.end() >= r.end());
    }

    #[test]
    fn inward_alignment_is_always_contained_by_the_original() {
        let r = LbaRange::from_bounds(9000, 20000);
        let a = r.aligned_inward(8192);
        assert_eq!(a, LbaRange::from_bounds(16384, 16384));
        assert!(a.start() >= r.start() && a.end() <= r.end());
    }

    #[test]
    fn inward_alignment_of_a_too_small_range_yields_empty() {
        assert!(LbaRange::from_bounds(9000, 9100).aligned_inward(8192).is_empty());
    }

    #[test]
    fn guard_band_saturates_at_zero() {
        assert_eq!(LbaRange::from_bounds(5, 10).expanded_by(100), LbaRange::from_bounds(0, 110));
    }

    #[test]
    fn chunks_cover_the_range_exactly_without_gaps_or_overlap() {
        let r = LbaRange::from_bounds(0, 250);
        let parts: Vec<_> = r.chunks(100).collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2], LbaRange::from_bounds(200, 250));
        assert_eq!(parts.iter().map(|p| p.len()).sum::<u64>(), r.len());
        for w in parts.windows(2) {
            assert_eq!(w[0].end(), w[1].start());
        }
    }
}
