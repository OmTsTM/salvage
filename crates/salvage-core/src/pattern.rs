//! Self-identifying verification pattern, derived from the sector's own LBA.
//!
//! # Why the pattern carries its own address
//!
//! The naive test writes random data and checks that it reads back unchanged.
//! That catches write errors and read errors, but it is blind to the defect
//! that destroys the most data: a counterfeit card. A controller that reports
//! 128 GB while holding 8 GB serves address `N` from physical cell
//! `N mod real_capacity`. Writing and re-reading the same address always
//! agrees, because the aliasing is *consistent*. The card passes, and destroys
//! the user's files later.
//!
//! Here every sector carries a header containing its own LBA, a session nonce,
//! and a checksum binding the two. When the device returns another address's
//! contents, the header names **which** address came back — and the smallest
//! observed collision reveals the true capacity.
//!
//! The session nonce prevents leftovers from a previous inspection being read
//! as approval of a new one.
//!
//! # Why several payload kinds exist
//!
//! Different patterns expose different faults, and one is not enough. A
//! pseudorandom payload has balanced bit density and is the best general
//! detector, but it masks stuck bits: a cell frozen at zero matches half the
//! bits by chance. Saturated patterns drive every cell to one extreme and
//! expose that case immediately. A checkerboard stresses coupling between
//! adjacent cells, which neither of the others reaches.
//!
//! The header is present in all of them. Dropping it for a constant fill would
//! cost the aliasing detection that justifies this module's existence.

use serde::{Deserialize, Serialize};

/// Signature marking a sector as written by this tool.
const MAGIC: u64 = 0x5344_4D41_5031_00A5;

/// Header bytes at the start of every sector: magic, LBA, nonce, checksum.
pub const HEADER_LEN: usize = 32;

/// Smallest supported sector: the header must fit with room for a payload.
pub const MIN_SECTOR_SIZE: usize = 64;

/// MurmurHash3 finalizer, used to derive seeds and checksums.
#[inline]
const fn fmix64(mut z: u64) -> u64 {
    z ^= z >> 33;
    z = z.wrapping_mul(0xff51_afd7_ed55_8ccd);
    z ^= z >> 33;
    z = z.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    z ^= z >> 33;
    z
}

/// One step of the SplitMix64 generator.
///
/// Chosen over a cryptographic PRNG because the requirement here is speed and
/// determinism, not unpredictability: an adversary is not trying to guess the
/// pattern, and generating tens of gigabytes must not become the bottleneck.
#[inline]
const fn splitmix64(state: u64) -> (u64, u64) {
    let next = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = next;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (next, z ^ (z >> 31))
}

/// Verdict for a single verified sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectorVerdict {
    /// The sector returned exactly what was written to it.
    Match,
    /// The sector returned another LBA's contents, intact.
    ///
    /// This is the unambiguous signature of aliasing: the device does not hold
    /// the requested address and silently remapped it.
    Aliased {
        /// The LBA whose contents were actually returned.
        actual_lba: u64,
    },
    /// Header is valid and for the right LBA, but the payload diverged.
    ///
    /// Silent corruption: the sector accepted the write and returned wrong
    /// data, reporting no error at any point.
    Corrupt {
        /// Byte offset within the sector of the first mismatching byte.
        first_bad_offset: usize,
    },
    /// No header from this session was found.
    ///
    /// The sector was never written, lost its header entirely, or holds data
    /// belonging to something else.
    Foreign,
}

impl SectorVerdict {
    /// True only for a fully verified sector.
    #[inline]
    pub const fn is_ok(&self) -> bool {
        matches!(self, Self::Match)
    }
}

/// Payload content written after the self-identifying header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// Pseudorandom, seeded by `(nonce, lba)`. The general-purpose default.
    Pseudorandom,
    /// All bits zero. Exposes cells that cannot hold a low state.
    Zeros,
    /// All bits one. Exposes cells that cannot hold a high state.
    Ones,
    /// Alternating `0xAA` / `0x55`. Stresses coupling between neighbours.
    Checkerboard,
}

impl PatternKind {
    /// Recommended sequence for a thorough inspection.
    ///
    /// Starts with the pseudorandom pattern, which resolves most cases on its
    /// own, then moves to the saturated ones, which add information only when
    /// stuck cells are present.
    pub const SWEEP: [PatternKind; 4] =
        [Self::Pseudorandom, Self::Ones, Self::Zeros, Self::Checkerboard];

    /// Short label for display.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Pseudorandom => "pseudorandom",
            Self::Zeros => "all zeros",
            Self::Ones => "all ones",
            Self::Checkerboard => "checkerboard",
        }
    }
}

/// Deterministic pattern generator, parameterised by a session nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternGenerator {
    nonce: u64,
    kind: PatternKind,
}

impl PatternGenerator {
    /// Creates a pseudorandom generator with the given nonce.
    #[inline]
    pub const fn new(nonce: u64) -> Self {
        Self { nonce, kind: PatternKind::Pseudorandom }
    }

    /// Creates a generator producing a specific payload kind.
    #[inline]
    pub const fn with_kind(nonce: u64, kind: PatternKind) -> Self {
        Self { nonce, kind }
    }

    /// The payload kind this generator produces.
    #[inline]
    pub const fn kind(&self) -> PatternKind {
        self.kind
    }

    /// This session's nonce.
    #[inline]
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Header checksum binding magic, LBA and nonce together.
    ///
    /// Distinguishes a genuine header from noise that happens to contain the
    /// magic value, so that random garbage is never reported as aliasing.
    #[inline]
    const fn header_checksum(&self, lba: u64) -> u64 {
        fmix64(MAGIC ^ fmix64(lba ^ fmix64(self.nonce)))
    }

    /// Payload seed for an LBA.
    #[inline]
    const fn payload_seed(&self, lba: u64) -> u64 {
        fmix64(self.nonce.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ lba)
    }

    /// Fills `sector` with the pattern for `lba`.
    ///
    /// # Panics
    /// If `sector` is smaller than [`MIN_SECTOR_SIZE`].
    pub fn fill_sector(&self, lba: u64, sector: &mut [u8]) {
        assert!(
            sector.len() >= MIN_SECTOR_SIZE,
            "sector of {} bytes is below the {MIN_SECTOR_SIZE} byte minimum",
            sector.len()
        );
        sector[0..8].copy_from_slice(&MAGIC.to_le_bytes());
        sector[8..16].copy_from_slice(&lba.to_le_bytes());
        sector[16..24].copy_from_slice(&self.nonce.to_le_bytes());
        sector[24..32].copy_from_slice(&self.header_checksum(lba).to_le_bytes());

        let payload = &mut sector[HEADER_LEN..];
        match self.kind {
            PatternKind::Zeros => payload.fill(0x00),
            PatternKind::Ones => payload.fill(0xFF),
            PatternKind::Checkerboard => {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b = if i % 2 == 0 { 0xAA } else { 0x55 };
                }
            }
            PatternKind::Pseudorandom => {
                let mut state = self.payload_seed(lba);
                let mut chunks = payload.chunks_exact_mut(8);
                for c in &mut chunks {
                    let (next, value) = splitmix64(state);
                    state = next;
                    c.copy_from_slice(&value.to_le_bytes());
                }
                let tail = chunks.into_remainder();
                if !tail.is_empty() {
                    let (_, value) = splitmix64(state);
                    let bytes = value.to_le_bytes();
                    tail.copy_from_slice(&bytes[..tail.len()]);
                }
            }
        }
    }

    /// Fills a buffer spanning consecutive sectors starting at `start_lba`.
    ///
    /// # Panics
    /// If `buf` is not an exact multiple of `sector_size`.
    pub fn fill_range(&self, start_lba: u64, sector_size: usize, buf: &mut [u8]) {
        assert!(
            sector_size >= MIN_SECTOR_SIZE && buf.len() % sector_size == 0,
            "buffer of {} bytes is not a multiple of the {sector_size} byte sector",
            buf.len()
        );
        for (i, sector) in buf.chunks_exact_mut(sector_size).enumerate() {
            self.fill_sector(start_lba.saturating_add(i as u64), sector);
        }
    }

    /// Checks a sector that was read back against the pattern expected at `lba`.
    ///
    /// Allocation-free: the expected payload is recomputed and compared in
    /// place rather than materialised.
    pub fn verify_sector(&self, lba: u64, sector: &[u8]) -> SectorVerdict {
        if sector.len() < MIN_SECTOR_SIZE {
            return SectorVerdict::Foreign;
        }
        let magic = u64::from_le_bytes(sector[0..8].try_into().expect("8-byte slice"));
        if magic != MAGIC {
            return SectorVerdict::Foreign;
        }
        let stored_lba = u64::from_le_bytes(sector[8..16].try_into().expect("8-byte slice"));
        let stored_nonce = u64::from_le_bytes(sector[16..24].try_into().expect("8-byte slice"));
        let stored_sum = u64::from_le_bytes(sector[24..32].try_into().expect("8-byte slice"));

        // A header from another session, or one whose checksum does not bind,
        // proves nothing and must not be reported as aliasing.
        if stored_nonce != self.nonce || stored_sum != self.header_checksum(stored_lba) {
            return SectorVerdict::Foreign;
        }

        // Intact header, but for a different address: the device remapped.
        if stored_lba != lba {
            return SectorVerdict::Aliased { actual_lba: stored_lba };
        }

        match self.first_payload_mismatch(lba, &sector[HEADER_LEN..]) {
            Some(offset) => SectorVerdict::Corrupt { first_bad_offset: HEADER_LEN + offset },
            None => SectorVerdict::Match,
        }
    }

    /// Offset of the first diverging payload byte, if any.
    fn first_payload_mismatch(&self, lba: u64, payload: &[u8]) -> Option<usize> {
        // Constant patterns are checked byte by byte, with no generator.
        match self.kind {
            PatternKind::Zeros => return payload.iter().position(|b| *b != 0x00),
            PatternKind::Ones => return payload.iter().position(|b| *b != 0xFF),
            PatternKind::Checkerboard => {
                return payload
                    .iter()
                    .enumerate()
                    .position(|(i, b)| *b != if i % 2 == 0 { 0xAA } else { 0x55 });
            }
            PatternKind::Pseudorandom => {}
        }

        let mut state = self.payload_seed(lba);
        let mut chunks = payload.chunks_exact(8);
        let mut offset = 0usize;
        for c in &mut chunks {
            let (next, value) = splitmix64(state);
            state = next;
            let expected = value.to_le_bytes();
            if c != expected {
                let local = c.iter().zip(expected).position(|(a, b)| *a != b).unwrap_or(0);
                return Some(offset + local);
            }
            offset += 8;
        }
        let tail = chunks.remainder();
        if !tail.is_empty() {
            let (_, value) = splitmix64(state);
            let expected = value.to_le_bytes();
            if let Some(local) = tail.iter().zip(expected).position(|(a, b)| *a != b) {
                return Some(offset + local);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: usize = 512;

    fn gen() -> PatternGenerator {
        PatternGenerator::new(0xDEAD_BEEF_CAFE_1234)
    }

    #[test]
    fn round_trip_is_accepted() {
        let g = gen();
        let mut buf = vec![0u8; SS];
        g.fill_sector(42, &mut buf);
        assert_eq!(g.verify_sector(42, &buf), SectorVerdict::Match);
    }

    #[test]
    fn distinct_lbas_produce_distinct_content() {
        let g = gen();
        let (mut a, mut b) = (vec![0u8; SS], vec![0u8; SS]);
        g.fill_sector(1000, &mut a);
        g.fill_sector(1001, &mut b);
        assert_ne!(a, b, "neighbouring sectors must not share content");
    }

    #[test]
    fn generation_is_deterministic() {
        let (mut a, mut b) = (vec![0u8; SS], vec![0u8; SS]);
        gen().fill_sector(777, &mut a);
        gen().fill_sector(777, &mut b);
        assert_eq!(a, b);
    }

    /// The central test: a counterfeit card returns, at a high address, the
    /// contents of the low address its controller mapped the write to.
    #[test]
    fn wrap_around_is_reported_with_the_aliased_address() {
        let g = gen();
        let real_capacity = 1000u64;
        let requested = 4321u64;
        let actual = requested % real_capacity; // 321

        let mut buf = vec![0u8; SS];
        g.fill_sector(actual, &mut buf);

        assert_eq!(g.verify_sector(requested, &buf), SectorVerdict::Aliased { actual_lba: actual });
    }

    #[test]
    fn silent_corruption_in_the_payload_is_caught() {
        let g = gen();
        let mut buf = vec![0u8; SS];
        g.fill_sector(42, &mut buf);
        buf[200] ^= 0x01; // a single flipped bit
        assert_eq!(g.verify_sector(42, &buf), SectorVerdict::Corrupt { first_bad_offset: 200 });
    }

    #[test]
    fn corruption_in_the_trailing_partial_word_is_caught() {
        let g = gen();
        let odd = MIN_SECTOR_SIZE + 5; // payload not a multiple of 8
        let mut buf = vec![0u8; odd];
        g.fill_sector(9, &mut buf);
        let last = odd - 1;
        buf[last] ^= 0xFF;
        assert_eq!(g.verify_sector(9, &buf), SectorVerdict::Corrupt { first_bad_offset: last });
    }

    #[test]
    fn unwritten_and_third_party_sectors_are_foreign() {
        let g = gen();
        assert_eq!(g.verify_sector(1, &vec![0u8; SS]), SectorVerdict::Foreign);
        assert_eq!(g.verify_sector(1, &vec![0xFFu8; SS]), SectorVerdict::Foreign);
    }

    #[test]
    fn a_previous_session_is_never_accepted_as_proof() {
        let mut buf = vec![0u8; SS];
        PatternGenerator::new(1).fill_sector(42, &mut buf);
        assert_eq!(PatternGenerator::new(2).verify_sector(42, &buf), SectorVerdict::Foreign);
    }

    /// Garbage carrying the right magic but an invalid checksum must not turn
    /// into a false aliasing report.
    #[test]
    fn a_forged_header_without_a_valid_checksum_is_foreign() {
        let g = gen();
        let mut buf = vec![0u8; SS];
        g.fill_sector(42, &mut buf);
        buf[8..16].copy_from_slice(&999u64.to_le_bytes()); // LBA changed, checksum not
        assert_eq!(g.verify_sector(42, &buf), SectorVerdict::Foreign);
    }

    #[test]
    fn fill_range_matches_sector_by_sector_generation() {
        let g = gen();
        let mut many = vec![0u8; SS * 4];
        g.fill_range(500, SS, &mut many);
        for i in 0..4u64 {
            let s = &many[i as usize * SS..(i as usize + 1) * SS];
            assert_eq!(g.verify_sector(500 + i, s), SectorVerdict::Match);
        }
    }

    #[test]
    fn the_pseudorandom_payload_is_not_constant() {
        let g = gen();
        let mut buf = vec![0u8; SS];
        g.fill_sector(12345, &mut buf);
        let payload = &buf[HEADER_LEN..];
        let distinct = payload.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(distinct > 100, "payload has only {distinct} distinct values");
    }

    #[test]
    fn constant_patterns_round_trip_and_reject_corruption() {
        for kind in PatternKind::SWEEP {
            let g = PatternGenerator::with_kind(7, kind);
            let mut buf = vec![0u8; SS];
            g.fill_sector(33, &mut buf);
            assert_eq!(g.verify_sector(33, &buf), SectorVerdict::Match, "{kind:?}");

            buf[HEADER_LEN + 10] ^= 0xFF;
            assert!(
                matches!(g.verify_sector(33, &buf), SectorVerdict::Corrupt { .. }),
                "{kind:?} failed to detect a flipped byte"
            );
        }
    }

    /// Every pattern keeps the header, so every pattern keeps the ability to
    /// unmask a counterfeit card.
    #[test]
    fn every_pattern_kind_still_detects_aliasing() {
        for kind in PatternKind::SWEEP {
            let g = PatternGenerator::with_kind(11, kind);
            let mut buf = vec![0u8; SS];
            g.fill_sector(100, &mut buf);
            assert_eq!(
                g.verify_sector(8292, &buf),
                SectorVerdict::Aliased { actual_lba: 100 },
                "{kind:?} lost aliasing detection"
            );
        }
    }

    /// An all-zero payload must not be confused with an unwritten sector: the
    /// header is what separates "deliberately zeroed" from "never touched".
    #[test]
    fn a_zero_payload_is_distinguishable_from_a_blank_sector() {
        let g = PatternGenerator::with_kind(5, PatternKind::Zeros);
        let mut written = vec![0u8; SS];
        g.fill_sector(60, &mut written);

        assert_eq!(g.verify_sector(60, &written), SectorVerdict::Match);
        assert_eq!(g.verify_sector(60, &vec![0u8; SS]), SectorVerdict::Foreign);
    }
}
