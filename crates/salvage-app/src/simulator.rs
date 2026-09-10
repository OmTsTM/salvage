//! In-memory simulated card with programmable defects.
//!
//! It exists for a practical reason: a tool that repartitions cards cannot be
//! developed with a genuinely defective card as its only test bench — those are
//! scarce, slow, and destroy the evidence on every attempt. The simulator
//! reproduces the defects that matter — counterfeit capacity, read errors,
//! write errors and silent corruption — deterministically and instantly.
//!
//! The counterfeit-capacity model is that of a reprogrammed controller: address
//! `L` reaches cell `L mod C`. It is the behaviour observed in cards of dubious
//! origin, and it is what makes old data vanish with no error reported.

use std::io;

use salvage_core::geometry::{DeviceGeometry, LbaRange};

use crate::device::{BlockDevice, DeviceError};

/// Defect exhibited by the simulated card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulatedFault {
    /// Healthy card.
    None,
    /// Counterfeit capacity: only `real` sectors exist; the rest is aliased.
    FakeCapacity {
        /// Sectors actually present.
        real: u64,
    },
    /// Reads fail with an I/O error across the given range.
    ReadError {
        /// Affected range.
        range: LbaRange,
    },
    /// Writes fail with an I/O error across the given range.
    WriteError {
        /// Affected range.
        range: LbaRange,
    },
    /// The write is accepted, but the content is stored altered.
    SilentCorruption {
        /// Affected range.
        range: LbaRange,
    },
    /// Several defective ranges, cycling through the three failure kinds.
    Multiple {
        /// Affected ranges.
        ranges: Vec<LbaRange>,
    },
    /// Reads fail the first `fails` times and then succeed.
    ///
    /// Reproduces a transient error — dirty contact, supply fluctuation, a
    /// marginal read the ECC nearly corrected. A tool that does not retry
    /// condemns good area because of it.
    FlakyRead {
        /// Affected range.
        range: LbaRange,
        /// How many attempts fail before the sector answers.
        fails: u32,
    },
    /// Every payload bit is stuck at zero.
    ///
    /// Under a pseudorandom pattern half the bits match by chance; under
    /// all-ones the failure is immediate. This is why more than one pattern
    /// exists.
    StuckAtZero {
        /// Affected range.
        range: LbaRange,
    },
    /// Acknowledges every write but retains only the most recent sectors.
    ///
    /// Reproduces the most treacherous defect a card can have: no operation
    /// fails, nothing is reported, and the data simply is not there afterwards.
    /// While the read-back happens right after the write, the content is still
    /// in the controller buffer and everything looks perfect — which is why only
    /// a test that writes everything before verifying anything sees it.
    VolatileWrites {
        /// How many sectors the device can actually hold.
        cache_sectors: usize,
    },
    /// No write is accepted anywhere.
    ///
    /// Reproduces the physical write-protect switch, a volume that stayed
    /// mounted with Windows refusing direct writes, or the program running
    /// without administrative privilege. None of these is a media defect, and
    /// the scan must tell them apart.
    WriteProtected,
}

impl SimulatedFault {
    /// Failure kind affecting an LBA, if any.
    fn at(&self, lba: u64) -> Option<FaultKind> {
        match self {
            Self::None | Self::FakeCapacity { .. } => None,
            Self::ReadError { range } => range.contains(lba).then_some(FaultKind::Read),
            Self::WriteError { range } => range.contains(lba).then_some(FaultKind::Write),
            Self::SilentCorruption { range } => range.contains(lba).then_some(FaultKind::Corrupt),
            Self::WriteProtected => Some(FaultKind::Write),
            // Handled outside this dispatch, as they depend on state or alter
            // content rather than failing the operation.
            Self::FlakyRead { .. } | Self::StuckAtZero { .. } | Self::VolatileWrites { .. } => None,
            Self::Multiple { ranges } => ranges.iter().enumerate().find_map(|(i, r)| {
                r.contains(lba).then_some(match i % 3 {
                    0 => FaultKind::Read,
                    1 => FaultKind::Write,
                    _ => FaultKind::Corrupt,
                })
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultKind {
    Read,
    Write,
    Corrupt,
}

/// Simulated memory card.
pub struct SimulatedCard {
    /// Read attempts already spent per LBA, for the intermittent failure.
    read_attempts: std::collections::HashMap<u64, u32>,
    /// Written sectors the device still retains, oldest first. Used by
    /// [`SimulatedFault::VolatileWrites`].
    retained: std::collections::VecDeque<u64>,
    geometry: DeviceGeometry,
    /// Cells that actually exist. Smaller than advertised on a fake card.
    storage: Vec<u8>,
    physical_sectors: u64,
    fault: SimulatedFault,
    writable: bool,
    /// Counters used by tests to inspect the access pattern.
    pub reads: u64,
    /// Write operations performed.
    pub writes: u64,
    /// Total sectors read.
    ///
    /// Volume, not operation count, determines how long a scan takes on a real
    /// card: transfer rate dominates per-call latency.
    pub sectors_read: u64,
    /// Total sectors written.
    pub sectors_written: u64,
}

impl SimulatedCard {
    /// Healthy card of the given capacity.
    pub fn healthy(sector_size: u32, sectors: u64) -> Self {
        Self::with_fault(sector_size, sectors, SimulatedFault::None)
    }

    /// Card advertising `sectors` sectors and exhibiting the given defect.
    pub fn with_fault(sector_size: u32, sectors: u64, fault: SimulatedFault) -> Self {
        let physical_sectors = match &fault {
            SimulatedFault::FakeCapacity { real } => (*real).min(sectors).max(1),
            _ => sectors,
        };
        let bytes = (physical_sectors * sector_size as u64) as usize;
        Self {
            read_attempts: std::collections::HashMap::new(),
            retained: std::collections::VecDeque::new(),
            geometry: DeviceGeometry::new(sector_size, sectors).expect("geometria valida"),
            storage: vec![0u8; bytes],
            physical_sectors,
            fault,
            writable: true,
            reads: 0,
            writes: 0,
            sectors_read: 0,
            sectors_written: 0,
        }
    }

    /// Sets whether the card accepts writes.
    pub fn set_writable(&mut self, writable: bool) {
        self.writable = writable;
    }

    /// Fills all real memory with a byte, to test non-destructiveness.
    pub fn fill_with_marker(&mut self, marker: u8) {
        self.storage.fill(marker);
    }

    /// Whether all real memory still holds the marker.
    pub fn is_entirely_marker(&self, marker: u8) -> bool {
        self.storage.iter().all(|b| *b == marker)
    }

    /// Maps a logical address to the physical one actually reached.
    ///
    /// On a healthy card this is the identity. On a counterfeit one it is the
    /// modulo operation that makes old data vanish without warning.
    #[inline]
    fn physical(&self, lba: u64) -> u64 {
        lba % self.physical_sectors
    }

    #[inline]
    fn byte_range(&self, physical_lba: u64) -> std::ops::Range<usize> {
        let ss = self.geometry.sector_size() as usize;
        let at = physical_lba as usize * ss;
        at..at + ss
    }

    fn io_error(lba: u64, what: &str) -> DeviceError {
        DeviceError::Io { lba, source: io::Error::other(format!("simulated {what} failure")) }
    }
}

impl BlockDevice for SimulatedCard {
    fn geometry(&self) -> DeviceGeometry {
        self.geometry
    }

    fn read_at(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DeviceError> {
        let sectors = self.check_access(lba, buf.len())?;
        let ss = self.geometry.sector_size() as usize;
        self.reads += 1;
        self.sectors_read += sectors;

        for i in 0..sectors {
            let current = lba + i;
            if self.fault.at(current) == Some(FaultKind::Read) {
                return Err(Self::io_error(current, "read"));
            }
            // Intermittent failure: errs on the first attempts, then relents.
            //
            // The counter belongs to the whole range, not to each address. A
            // real transient error — flickering contact, unstable supply —
            // disrupts the read operation rather than one isolated sector at a
            // time; counting per LBA would make a four-sector range need eight
            // attempts to relent, which matches no hardware.
            let flaky = match &self.fault {
                SimulatedFault::FlakyRead { range, fails } if range.contains(current) => {
                    Some((range.start(), *fails))
                }
                _ => None,
            };
            if let Some((key, fails)) = flaky {
                let used = self.read_attempts.entry(key).or_insert(0);
                if *used < fails {
                    *used += 1;
                    return Err(Self::io_error(current, "intermittent read"));
                }
            }
            let src = self.byte_range(self.physical(current));
            let at = i as usize * ss;
            buf[at..at + ss].copy_from_slice(&self.storage[src]);
        }
        Ok(())
    }

    fn write_at(&mut self, lba: u64, buf: &[u8]) -> Result<(), DeviceError> {
        if !self.writable {
            return Err(DeviceError::ReadOnly);
        }
        let sectors = self.check_access(lba, buf.len())?;
        let ss = self.geometry.sector_size() as usize;
        self.writes += 1;
        self.sectors_written += sectors;

        for i in 0..sectors {
            let current = lba + i;
            // A real device writes what it managed before failing, and the
            // bisection has to cope with that partial write.
            if self.fault.at(current) == Some(FaultKind::Write) {
                return Err(Self::io_error(current, "write"));
            }
            let dst = self.byte_range(self.physical(current));
            let at = i as usize * ss;
            self.storage[dst.clone()].copy_from_slice(&buf[at..at + ss]);

            if let SimulatedFault::StuckAtZero { range } = &self.fault {
                if range.contains(current) {
                    // The header survives; the payload is stuck at zero, which
                    // is how a degraded cell behaves.
                    let payload = dst.start + 32;
                    self.storage[payload..dst.end].fill(0);
                }
            }

            // Limited retention: the oldest sector loses its content as soon
            // as the retention capacity is exceeded.
            if let SimulatedFault::VolatileWrites { cache_sectors } = self.fault {
                self.retained.push_back(current);
                while self.retained.len() > cache_sectors {
                    if let Some(evicted) = self.retained.pop_front() {
                        let range = self.byte_range(self.physical(evicted));
                        // Garbage, not zeros: a sector that lost its content
                        // does not conveniently come back blank.
                        for (i, b) in self.storage[range].iter_mut().enumerate() {
                            *b = (i as u8) ^ 0x5A;
                        }
                    }
                }
            }

            if self.fault.at(current) == Some(FaultKind::Corrupt) {
                // A single flipped bit: the hardest defect to notice and the
                // one that ruins files most quietly.
                self.storage[dst.start + ss / 2] ^= 0x01;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        Ok(())
    }

    fn is_writable(&self) -> bool {
        self.writable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: u32 = 512;

    #[test]
    fn a_healthy_card_round_trips_data() {
        let mut c = SimulatedCard::healthy(SS, 100);
        let out = vec![0xABu8; SS as usize];
        c.write_at(10, &out).unwrap();
        let mut back = vec![0u8; SS as usize];
        c.read_at(10, &mut back).unwrap();
        assert_eq!(back, out);
    }

    /// The essential behaviour of a counterfeit card: writing to a high
    /// address silently destroys the data at a low one.
    #[test]
    fn a_fake_card_silently_overwrites_the_low_address() {
        let mut c = SimulatedCard::with_fault(SS, 100, SimulatedFault::FakeCapacity { real: 10 });
        let low = vec![0x11u8; SS as usize];
        let high = vec![0x22u8; SS as usize];

        c.write_at(3, &low).unwrap();
        c.write_at(43, &high).unwrap(); // 43 mod 10 == 3

        let mut back = vec![0u8; SS as usize];
        c.read_at(3, &mut back).unwrap();
        assert_eq!(back, high, "o endereco 3 deveria ter sido sobrescrito pelo 43");
    }

    #[test]
    fn access_past_the_end_is_refused() {
        let mut c = SimulatedCard::healthy(SS, 10);
        let mut buf = vec![0u8; SS as usize];
        assert!(matches!(c.read_at(10, &mut buf), Err(DeviceError::OutOfBounds { .. })));
        assert!(matches!(
            c.write_at(9, &vec![0u8; SS as usize * 2]),
            Err(DeviceError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn unaligned_buffers_are_refused() {
        let mut c = SimulatedCard::healthy(SS, 10);
        let mut buf = vec![0u8; 100];
        assert!(matches!(c.read_at(0, &mut buf), Err(DeviceError::UnalignedBuffer { .. })));
    }

    #[test]
    fn faults_apply_only_inside_their_range() {
        let f = SimulatedFault::ReadError { range: LbaRange::from_bounds(10, 20) };
        assert_eq!(f.at(9), None);
        assert_eq!(f.at(10), Some(FaultKind::Read));
        assert_eq!(f.at(19), Some(FaultKind::Read));
        assert_eq!(f.at(20), None);
    }

    #[test]
    fn a_read_only_card_refuses_writes() {
        let mut c = SimulatedCard::healthy(SS, 10);
        c.set_writable(false);
        assert!(matches!(c.write_at(0, &vec![0u8; SS as usize]), Err(DeviceError::ReadOnly)));
    }

    #[test]
    fn silent_corruption_alters_the_stored_content() {
        let mut c = SimulatedCard::with_fault(
            SS,
            10,
            SimulatedFault::SilentCorruption { range: LbaRange::from_bounds(5, 6) },
        );
        let out = vec![0x00u8; SS as usize];
        c.write_at(5, &out).unwrap();
        let mut back = vec![0u8; SS as usize];
        c.read_at(5, &mut back).unwrap();
        assert_ne!(back, out, "silent corruption changed nothing");
    }
}
