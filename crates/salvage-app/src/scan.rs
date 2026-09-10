//! Device scanning: write, read back, and classify every sector.
//!
//! # Write order is not a detail
//!
//! On a card with counterfeit capacity the controller serves address `L` from
//! physical cell `L mod C`, where `C` is the real capacity. Several logical
//! addresses share one cell, and **whoever writes last decides what is read**.
//!
//! Writing front to back, every high address overwrites the low one it collides
//! with. On read-back the **low** addresses — the ones that genuinely exist —
//! are the ones that come back altered, while the high, nonexistent ones return
//! exactly what was just written and pass. The diagnosis comes out inverted:
//! the good area is condemned and the area that does not exist is approved.
//!
//! The write pass therefore walks the device **back to front**. Each physical
//! cell then holds the pattern of the lowest address reaching it, and an
//! ascending read-back gives the expected result:
//!
//! - address below `C`: returns its own pattern, approved;
//! - address at or above `C`: returns the pattern of `L mod C`, exposing the
//!   alias and yielding `C` for free.
//!
//! # Large blocks, sufficient precision
//!
//! Scanning sector by sector across a 128 GB card would be impractical. The
//! scan works in blocks of a few megabytes and, when a block fails, refines the
//! result by bisection down to [`MIN_REFINE_SECTORS`].
//!
//! Refinement deliberately stops short of the individual sector. The extra
//! precision would cost hundreds of times more I/O operations — hours on a badly
//! damaged card — and would change nothing: planning dilates every condemned
//! region by a guard band and aligns it to the 4 MiB erase block. The rounding
//! always errs toward safety, condemning a few extra healthy sectors and never
//! letting a defective one through.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use salvage_core::geometry::LbaRange;
use salvage_core::pattern::{PatternGenerator, PatternKind, SectorVerdict};
use salvage_core::sector_map::{SectorMap, SectorState};
use serde::{Deserialize, Serialize};

use crate::device::{BlockDevice, DeviceError};

/// Default scan block size: 4 MiB, aligned to the erase block.
pub const DEFAULT_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// Maximum bisection depth when refining a failed block.
///
/// Each level doubles the operation count; twenty levels already isolate one
/// sector within a block of a million.
const MAX_REFINE_DEPTH: u32 = 20;

/// Finest bisection granularity, in sectors.
///
/// Refining down to the individual sector costs up to twice the block's sector
/// count in I/O operations — and the extra precision is never used. Planning
/// dilates every condemned region by a guard band and aligns it to the 4 MiB
/// erase block, so knowing the defect sits at sector 1500 or somewhere between
/// 1408 and 1663 leads to exactly the same layout.
///
/// Stopping at 128 KiB shrinks the bisection tree from over sixteen thousand
/// nodes to a few dozen, without changing any resulting partition plan.
const MIN_REFINE_SECTORS: u64 = 256;

/// How many of the block's sectors are probed before deciding to refine it.
///
/// Refining costs up to twice the block's sector count in I/O. That price is
/// not worth paying for a block where not even the samples write: it is either
/// entirely lost, or the failure is not the media's fault at all.
const WRITE_PROBE_SAMPLES: usize = 3;

/// Consecutive fully unwritable blocks that characterise a systemic failure
/// rather than a media defect.
///
/// Three blocks in a row where no sector accepts a write do not describe a card
/// with bad sectors: they describe a card that is not accepting writes at all.
const SYSTEMIC_FAILURE_THRESHOLD: u32 = 3;

/// Extra attempts before condemning a sector over an I/O error.
///
/// Not every failure is permanent. A dirty contact, a supply fluctuation, or a
/// marginal read the ECC nearly corrected produce an error on one attempt and
/// success on the next. Serious disk diagnostic tools — Victoria, MHDD, HDAT2 —
/// retry before marking a sector, and skipping that fills the map with defects
/// that do not exist, condemning good area.
///
/// Retrying is only worthwhile at fine granularity: on a large block the cost
/// would be high and the result imprecise.
const IO_RETRIES: u32 = 3;

/// Current scan phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    /// Writing the pattern, back to front.
    Writing,
    /// Reading back and verifying, front to back.
    Verifying,
    /// Isolating a failed block's sectors by bisection.
    Refining,
}

/// Scan configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanConfig {
    /// Sectors per block.
    pub chunk_sectors: u64,
    /// Pattern seed. Distinguishes this inspection from any earlier one.
    pub nonce: u64,
    /// Range to inspect. `None` means the whole device.
    pub range: Option<LbaRange>,
    /// Content written to each sector.
    pub pattern: PatternKind,
    /// Delay, in seconds, between finishing the writes and starting to verify.
    ///
    /// With no delay, verification measures only whether the cell accepted the
    /// data. A pause turns the scan into a **retention** test, which asks a
    /// different question: is the data still there after a while? Some cards
    /// answer yes to the first and no to the second — and those are precisely
    /// the ones that destroy files without warning.
    pub retention_delay_secs: u64,
}

impl ScanConfig {
    /// Default configuration for a geometry.
    pub fn new(nonce: u64, sector_size: u32) -> Self {
        Self {
            chunk_sectors: (DEFAULT_CHUNK_BYTES / sector_size as u64).max(1),
            nonce,
            range: None,
            pattern: PatternKind::Pseudorandom,
            retention_delay_secs: 0,
        }
    }
}

/// Progress reported during a scan.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScanProgress {
    /// Current phase.
    pub phase: ScanPhase,
    /// Sectors processed so far in this phase.
    pub sectors_done: u64,
    /// Total sectors in this phase.
    pub sectors_total: u64,
    /// Last LBA touched.
    pub current_lba: u64,
    /// Defects accumulated so far.
    pub defects_found: u64,
}

impl ScanProgress {
    /// Fraction of the phase completed, from 0.0 to 1.0.
    pub fn fraction(&self) -> f64 {
        if self.sectors_total == 0 {
            return 1.0;
        }
        (self.sectors_done as f64 / self.sectors_total as f64).clamp(0.0, 1.0)
    }
}

/// How far the enclosing phase has advanced.
///
/// Bisection reports progress on behalf of the phase that invoked it, so the
/// two counters travel together and are passed as one value rather than as a
/// pair of positional `u64`s that are trivial to swap at a call site.
#[derive(Debug, Clone, Copy)]
struct PhaseProgress {
    done: u64,
    total: u64,
}

impl PhaseProgress {
    const fn new(done: u64, total: u64) -> Self {
        Self { done, total }
    }
}

/// Receives progress updates and can interrupt the scan.
pub trait ScanObserver {
    /// Called periodically with progress and the partial map.
    ///
    /// Receiving the map under construction is what lets the interface fill the
    /// visualization during the scan rather than only at the end.
    /// Implementations doing heavy work — downsampling the map, serializing,
    /// emitting an event — must throttle themselves: this is called per block.
    fn on_progress(&mut self, progress: &ScanProgress, map: &SectorMap);

    /// Called when a defect is confirmed.
    fn on_defect(&mut self, _range: LbaRange, _state: SectorState) {}
}

/// Observer that discards everything. Useful in tests.
pub struct SilentObserver;

impl ScanObserver for SilentObserver {
    fn on_progress(&mut self, _progress: &ScanProgress, _map: &SectorMap) {}
}

/// Cancellation signal shareable across threads.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Creates a signal that has not been raised.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Scan failures.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The user interrupted the scan.
    #[error("scan cancelled by the user")]
    Cancelled,
    /// A destructive mode was requested on a read-only device.
    #[error("destructive mode requires a device opened for writing")]
    NotWritable,
    /// The device is refusing every write.
    ///
    /// Telling this apart from "a card full of bad sectors" is essential: they
    /// are different causes with different remedies, and insisting on bisecting
    /// every block would make the inspection look frozen for hours.
    #[error(
        "the device refused every write across {consecutive} consecutive blocks ({sectors} \
         sectors). This is not a media defect: the volume is probably still mounted, the card's \
         write-protect switch is engaged, or the program is not running elevated."
    )]
    SystemicWriteFailure {
        /// Consecutive blocks refused in full.
        consecutive: u32,
        /// Sectors involved.
        sectors: u64,
    },
    /// Unrecoverable access failure.
    #[error(transparent)]
    Device(#[from] DeviceError),
}

/// Scan result.
#[derive(Debug, Clone)]
pub struct ScanOutcome {
    /// Resulting map.
    pub map: SectorMap,
    /// Sectors actually inspected.
    pub sectors_scanned: u64,
}

/// Runs a scan over a device.
pub struct Scanner {
    config: ScanConfig,
    pattern: PatternGenerator,
    cancel: CancellationToken,
}

impl Scanner {
    /// Creates a scanner with the given configuration.
    pub fn new(config: ScanConfig, cancel: CancellationToken) -> Self {
        Self { pattern: PatternGenerator::with_kind(config.nonce, config.pattern), config, cancel }
    }

    /// Walks the device and returns the state map.
    pub fn run<D: BlockDevice, O: ScanObserver>(
        &self,
        device: &mut D,
        observer: &mut O,
    ) -> Result<ScanOutcome, ScanError> {
        let geometry = device.geometry();
        let span = self.config.range.unwrap_or_else(|| geometry.full_range());
        let mut map = SectorMap::new(geometry);

        // The only mode there is writes, so a read-only handle can never be
        // used. Refusing here rather than at the first write means the failure
        // arrives before anything has been touched.
        if !device.is_writable() {
            return Err(ScanError::NotWritable);
        }

        let sector_size = geometry.sector_size() as usize;
        let mut buf = vec![0u8; sector_size * self.config.chunk_sectors as usize];

        self.write_pass(device, &span, &mut buf, &mut map, observer)?;

        // The delay turns verification into a retention test: it measures
        // whether the data is still there, not merely that the cell took it.
        if self.config.retention_delay_secs > 0 {
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(self.config.retention_delay_secs);
            while std::time::Instant::now() < deadline {
                self.bail_if_cancelled()?;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }

        self.verify_pass(device, &span, &mut buf, &mut map, observer)?;

        debug_assert!(map.check_invariants().is_ok(), "sector map inconsistent at end of scan");

        Ok(ScanOutcome { map, sectors_scanned: span.len() })
    }

    /// Writes the pattern walking the device back to front.
    ///
    /// The reverse order is what makes aliasing surface at the nonexistent
    /// address rather than the valid one. See the module documentation.
    fn write_pass<D: BlockDevice, O: ScanObserver>(
        &self,
        device: &mut D,
        span: &LbaRange,
        buf: &mut [u8],
        map: &mut SectorMap,
        observer: &mut O,
    ) -> Result<(), ScanError> {
        let sector_size = device.geometry().sector_size() as usize;
        let chunks: Vec<LbaRange> = span.chunks(self.config.chunk_sectors).collect();
        let mut done = 0u64;
        let mut defects = 0u64;
        let mut consecutive_dead_chunks = 0u32;
        let mut dead_sectors = 0u64;

        for chunk in chunks.iter().rev() {
            self.bail_if_cancelled()?;

            let bytes = chunk.len() as usize * sector_size;
            let slice = &mut buf[..bytes];
            self.pattern.fill_range(chunk.start(), sector_size, slice);

            if device.write_at(chunk.start(), slice).is_err() {
                // Before spending a full bisection, check whether any sector
                // in this block accepts a write. If none does, there is nothing
                // to isolate — and the case must be told apart from a partially
                // bad block, because seeing it across the whole card means the
                // problem is not the media.
                let bad = if self.any_sector_writable(device, chunk, buf, sector_size) {
                    consecutive_dead_chunks = 0;
                    self.refine_write(
                        device,
                        chunk,
                        buf,
                        map,
                        observer,
                        PhaseProgress::new(done, span.len()),
                    )?
                } else {
                    consecutive_dead_chunks += 1;
                    dead_sectors = dead_sectors.saturating_add(chunk.len());
                    if consecutive_dead_chunks >= SYSTEMIC_FAILURE_THRESHOLD {
                        return Err(ScanError::SystemicWriteFailure {
                            consecutive: consecutive_dead_chunks,
                            sectors: dead_sectors,
                        });
                    }
                    vec![*chunk]
                };

                for r in &bad {
                    map.mark(*r, SectorState::BadWrite);
                    defects += r.len();
                    observer.on_defect(*r, SectorState::BadWrite);
                }
            } else {
                consecutive_dead_chunks = 0;
            }

            done += chunk.len();
            let progress = ScanProgress {
                phase: ScanPhase::Writing,
                sectors_done: done,
                sectors_total: span.len(),
                current_lba: chunk.start(),
                defects_found: defects,
            };
            observer.on_progress(&progress, map);
        }

        device.flush().ok();
        Ok(())
    }

    /// Reads the device back front to back and classifies every sector.
    fn verify_pass<D: BlockDevice, O: ScanObserver>(
        &self,
        device: &mut D,
        span: &LbaRange,
        buf: &mut [u8],
        map: &mut SectorMap,
        observer: &mut O,
    ) -> Result<(), ScanError> {
        let sector_size = device.geometry().sector_size() as usize;
        let mut done = 0u64;
        let mut defects = 0u64;

        for chunk in span.chunks(self.config.chunk_sectors) {
            self.bail_if_cancelled()?;

            let bytes = chunk.len() as usize * sector_size;
            let slice = &mut buf[..bytes];

            match device.read_at(chunk.start(), slice) {
                Ok(()) => {
                    defects += self.classify_chunk(&chunk, slice, sector_size, map, observer);
                }
                Err(_) => {
                    // The whole block failed: isolate the guilty sectors.
                    let outcome =
                        self.refine_read(device, &chunk, buf, sector_size, map, observer)?;
                    defects += outcome;
                }
            }

            done += chunk.len();
            let progress = ScanProgress {
                phase: ScanPhase::Verifying,
                sectors_done: done,
                sectors_total: span.len(),
                current_lba: chunk.start(),
                defects_found: defects,
            };
            observer.on_progress(&progress, map);
        }

        Ok(())
    }

    /// Classifies the sectors of a successfully read block.
    ///
    /// In read-only mode there is no written pattern to compare against, so a
    /// block that reads without error is exactly that: readable. Marking it
    /// approved would be a lie, because nothing about the content was checked.
    fn classify_chunk<O: ScanObserver>(
        &self,
        chunk: &LbaRange,
        data: &[u8],
        sector_size: usize,
        map: &mut SectorMap,
        observer: &mut O,
    ) -> u64 {
        let mut defects = 0u64;

        // Consecutive sectors sharing a verdict are recorded as one run, for
        // defects exactly as for approved area.
        //
        // Marking them one at a time is not merely untidy: on a card where most
        // sectors fail it produces one map operation and one observer
        // notification per sector — tens of millions of them — and the
        // bookkeeping ends up costing several times more than the I/O it
        // describes. A 128 GB card measured 17.5 MB/s while writing, which does
        // no such reporting, against 2.9 MB/s while verifying, which did.
        let mut run: Option<(u64, SectorState)> = None;

        let flush =
            |map: &mut SectorMap, observer: &mut O, run: Option<(u64, SectorState)>, end: u64| {
                let Some((start, state)) = run else {
                    return;
                };
                if end <= start {
                    return;
                }
                let range = LbaRange::from_bounds(start, end);
                map.mark(range, state);
                if state != SectorState::Good {
                    observer.on_defect(range, state);
                }
            };

        for (i, sector) in data.chunks_exact(sector_size).enumerate() {
            let lba = chunk.start() + i as u64;
            let state = match self.pattern.verify_sector(lba, sector) {
                SectorVerdict::Match => SectorState::Good,
                SectorVerdict::Aliased { actual_lba } => {
                    // Each alias names a different address, so the observation
                    // is recorded per sector even though the run is not.
                    map.record_alias(lba, actual_lba);
                    defects += 1;
                    SectorState::Aliased
                }
                // No valid header after a successful write means the sector
                // lost its content entirely, which is corruption either way.
                SectorVerdict::Corrupt { .. } | SectorVerdict::Foreign => {
                    defects += 1;
                    SectorState::Corrupt
                }
            };

            match run {
                Some((_, current)) if current == state => {}
                _ => {
                    flush(map, observer, run, lba);
                    run = Some((lba, state));
                }
            }
        }
        flush(map, observer, run, chunk.end());
        defects
    }

    /// Reads, retrying before giving up.
    ///
    /// Also returns how many attempts were needed, so a sector that only
    /// answers on the third try can be reported as marginal rather than simply
    /// approved.
    fn read_persistent<D: BlockDevice>(
        &self,
        device: &mut D,
        lba: u64,
        buf: &mut [u8],
    ) -> Result<u32, DeviceError> {
        let mut last: Option<DeviceError> = None;
        for attempt in 0..=IO_RETRIES {
            match device.read_at(lba, buf) {
                Ok(()) => return Ok(attempt),
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("ao menos uma tentativa foi feita"))
    }

    /// Writes, retrying before giving up.
    fn write_persistent<D: BlockDevice>(
        &self,
        device: &mut D,
        lba: u64,
        buf: &[u8],
    ) -> Result<u32, DeviceError> {
        let mut last: Option<DeviceError> = None;
        for attempt in 0..=IO_RETRIES {
            match device.write_at(lba, buf) {
                Ok(()) => return Ok(attempt),
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("ao menos uma tentativa foi feita"))
    }

    /// Probes a few sectors spread across the block to learn whether **any**
    /// of them accepts a write.
    ///
    /// A block failing as a whole does not, on its own, say whether it holds a
    /// few bad sectors or nothing writes there at all. The difference matters:
    /// in the first case bisection finds the culprits; in the second it walks
    /// the entire tree — up to twice the block sector count — to conclude what
    /// these three attempts would already have said.
    fn any_sector_writable<D: BlockDevice>(
        &self,
        device: &mut D,
        chunk: &LbaRange,
        buf: &mut [u8],
        sector_size: usize,
    ) -> bool {
        let len = chunk.len();
        let sample = &mut buf[..sector_size];
        for i in 0..WRITE_PROBE_SAMPLES {
            let offset = len.saturating_mul(i as u64) / WRITE_PROBE_SAMPLES as u64;
            let lba = chunk.start() + offset.min(len.saturating_sub(1));
            self.pattern.fill_sector(lba, sample);
            if device.write_at(lba, sample).is_ok() {
                return true;
            }
        }
        false
    }

    /// Isolates by bisection the sectors responsible for a write failure.
    fn refine_write<D: BlockDevice, O: ScanObserver>(
        &self,
        device: &mut D,
        chunk: &LbaRange,
        buf: &mut [u8],
        map: &SectorMap,
        observer: &mut O,
        phase: PhaseProgress,
    ) -> Result<Vec<LbaRange>, ScanError> {
        let sector_size = device.geometry().sector_size() as usize;
        let mut bad = Vec::new();
        let mut queue = vec![(*chunk, 0u32)];

        while let Some((range, depth)) = queue.pop() {
            self.bail_if_cancelled()?;

            let bytes = range.len() as usize * sector_size;
            let slice = &mut buf[..bytes];
            self.pattern.fill_range(range.start(), sector_size, slice);

            // Near the end of the bisection, insisting is cheap and avoids
            // condemning good area over a transient failure.
            let ok = if range.len() <= MIN_REFINE_SECTORS * 4 {
                self.write_persistent(device, range.start(), slice).is_ok()
            } else {
                device.write_at(range.start(), slice).is_ok()
            };
            if ok {
                continue;
            }
            if range.len() <= MIN_REFINE_SECTORS || depth >= MAX_REFINE_DEPTH {
                bad.push(range);
                continue;
            }

            let mid = range.start() + range.len() / 2;
            queue.push((LbaRange::from_bounds(range.start(), mid), depth + 1));
            queue.push((LbaRange::from_bounds(mid, range.end()), depth + 1));

            // A badly damaged block takes minutes to isolate. Without
            // reporting here, the interface would sit motionless during exactly
            // the slowest part of the work.
            let progress = ScanProgress {
                phase: ScanPhase::Refining,
                sectors_done: phase.done,
                sectors_total: phase.total,
                current_lba: range.start(),
                defects_found: bad.iter().map(|r| r.len()).sum(),
            };
            observer.on_progress(&progress, map);
        }

        bad.sort_unstable_by_key(|r| r.start());
        Ok(bad)
    }

    /// Isolates by bisection the sectors responsible for a read failure,
    /// classifying the readable ones along the way.
    fn refine_read<D: BlockDevice, O: ScanObserver>(
        &self,
        device: &mut D,
        chunk: &LbaRange,
        buf: &mut [u8],
        sector_size: usize,
        map: &mut SectorMap,
        observer: &mut O,
    ) -> Result<u64, ScanError> {
        let mut defects = 0u64;
        let mut queue = vec![(*chunk, 0u32)];

        while let Some((range, depth)) = queue.pop() {
            self.bail_if_cancelled()?;

            let bytes = range.len() as usize * sector_size;
            let slice = &mut buf[..bytes];

            match device.read_at(range.start(), slice) {
                Ok(()) => {
                    defects += self.classify_chunk(&range, slice, sector_size, map, observer);
                }
                Err(_) if range.len() <= MIN_REFINE_SECTORS || depth >= MAX_REFINE_DEPTH => {
                    // Last chance before condemning: retry the read. If the
                    // sector answers now, the error was transient and the area
                    // remains usable.
                    let bytes = range.len() as usize * sector_size;
                    match self.read_persistent(device, range.start(), &mut buf[..bytes]) {
                        Ok(_) => {
                            defects += self.classify_chunk(
                                &range,
                                &buf[..bytes],
                                sector_size,
                                map,
                                observer,
                            );
                        }
                        Err(_) => {
                            map.mark(range, SectorState::BadRead);
                            defects += range.len();
                            observer.on_defect(range, SectorState::BadRead);
                        }
                    }
                }
                Err(_) => {
                    let mid = range.start() + range.len() / 2;
                    queue.push((LbaRange::from_bounds(range.start(), mid), depth + 1));
                    queue.push((LbaRange::from_bounds(mid, range.end()), depth + 1));

                    let progress = ScanProgress {
                        phase: ScanPhase::Refining,
                        sectors_done: 0,
                        sectors_total: chunk.len(),
                        current_lba: range.start(),
                        defects_found: defects,
                    };
                    observer.on_progress(&progress, map);
                }
            }
        }
        Ok(defects)
    }

    #[inline]
    fn bail_if_cancelled(&self) -> Result<(), ScanError> {
        if self.cancel.is_cancelled() {
            Err(ScanError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulator::{SimulatedCard, SimulatedFault};
    use salvage_core::health::{diagnose, Assurance, FailureScenario};

    const SS: u32 = 512;
    /// 256 sectors per block keeps tests fast while still exercising the
    /// bisection across several levels.
    const CHUNK: u64 = 256;

    fn config() -> ScanConfig {
        ScanConfig {
            chunk_sectors: CHUNK,
            nonce: 0xA5A5_1234_DEAD_0001,
            range: None,
            pattern: PatternKind::Pseudorandom,
            retention_delay_secs: 0,
        }
    }

    fn scan(card: &mut SimulatedCard) -> ScanOutcome {
        Scanner::new(config(), CancellationToken::new())
            .run(card, &mut SilentObserver)
            .expect("the scan should have completed")
    }

    #[test]
    fn a_healthy_card_comes_out_entirely_good() {
        let mut card = SimulatedCard::healthy(SS, 4096);
        let out = scan(&mut card);
        out.map.check_invariants().unwrap();

        let c = out.map.counts();
        assert_eq!(c.good, 4096, "an intact card should be approved end to end");
        assert_eq!(c.defective(), 0);
        assert_eq!(diagnose(&out.map, None).scenario, FailureScenario::Pristine);
    }

    /// The tool central test: a card advertising 4096 sectors while holding
    /// 1024 must be unmasked, with its real capacity recovered.
    #[test]
    fn a_counterfeit_card_is_unmasked_and_its_real_capacity_recovered() {
        let real = 1024u64;
        let mut card = SimulatedCard::with_fault(SS, 4096, SimulatedFault::FakeCapacity { real });
        let out = scan(&mut card);
        out.map.check_invariants().unwrap();

        let c = out.map.counts();
        assert_eq!(c.good, real, "the whole of the real area should be approved");
        assert_eq!(c.aliased, 4096 - real, "every nonexistent sector should be detected");

        // Real sectors pass; invented ones are caught.
        assert_eq!(out.map.state_at(0), Some(SectorState::Good));
        assert_eq!(out.map.state_at(real - 1), Some(SectorState::Good));
        assert_eq!(out.map.state_at(real), Some(SectorState::Aliased));
        assert_eq!(out.map.state_at(4095), Some(SectorState::Aliased));

        let report = diagnose(&out.map, None);
        match report.scenario {
            FailureScenario::CounterfeitCapacity { real_capacity_sectors, .. } => {
                assert_eq!(real_capacity_sectors, real);
            }
            other => panic!("expected a counterfeit verdict, got {other:?}"),
        }
        assert_eq!(report.assurance, Assurance::High);
    }

    /// Proof that the reverse write order is required: writing front to back,
    /// the aliasing would surface in the good area instead.
    #[test]
    fn the_alias_is_reported_on_the_nonexistent_address_not_the_real_one() {
        let real = 1024u64;
        let mut card = SimulatedCard::with_fault(SS, 4096, SimulatedFault::FakeCapacity { real });
        let out = scan(&mut card);

        for a in out.map.aliases() {
            assert!(
                a.requested_lba >= real,
                "alias reported at {}, which is a real address",
                a.requested_lba
            );
            assert!(a.actual_lba < real, "the address served back should lie in the real area");
            assert_eq!(a.actual_lba, a.requested_lba % real);
        }
    }

    /// Refinement stops at [`MIN_REFINE_SECTORS`] granularity, so the condemned
    /// region is slightly larger than the defect. What must never happen is the
    /// opposite: a defective sector left out.
    #[test]
    fn every_bad_read_sector_is_condemned_within_the_refinement_granularity() {
        let bad = LbaRange::from_bounds(700, 705);
        let mut card =
            SimulatedCard::with_fault(SS, 4096, SimulatedFault::ReadError { range: bad });
        let out = scan(&mut card);
        out.map.check_invariants().unwrap();

        // No false negatives: data safety depends on exactly this.
        for lba in bad.start()..bad.end() {
            assert!(
                out.map.state_at(lba).unwrap().is_defective(),
                "defective sector {lba} escaped detection"
            );
        }

        // The over-condemnation is bounded by the granularity and does not
        // grow without limit.
        assert!(
            out.map.counts().defective() <= MIN_REFINE_SECTORS * 2,
            "condemned {} sectors over a defect of {}",
            out.map.counts().defective(),
            bad.len()
        );

        // Longe do defeito, o cartao continua aproveitavel.
        assert_eq!(out.map.state_at(0), Some(SectorState::Good));
        assert_eq!(out.map.state_at(2000), Some(SectorState::Good));
        assert_eq!(out.map.state_at(4095), Some(SectorState::Good));
    }

    #[test]
    fn write_errors_are_isolated_to_the_exact_sectors() {
        let bad = LbaRange::from_bounds(1500, 1503);
        let mut card =
            SimulatedCard::with_fault(SS, 4096, SimulatedFault::WriteError { range: bad });
        let out = scan(&mut card);
        out.map.check_invariants().unwrap();

        assert!(out.map.counts().defective() >= bad.len());
        assert_eq!(out.map.state_at(1499), Some(SectorState::Good));
        assert!(out.map.state_at(1500).unwrap().is_defective());
    }

    /// Silent corruption is the defect a read-only test never finds: the
    /// sector answers normally and hands back wrong data.
    #[test]
    fn silent_corruption_is_detected() {
        let bad = LbaRange::from_bounds(2000, 2010);
        let mut card =
            SimulatedCard::with_fault(SS, 4096, SimulatedFault::SilentCorruption { range: bad });
        let out = scan(&mut card);
        out.map.check_invariants().unwrap();

        assert_eq!(out.map.counts().corrupt, bad.len());
        assert_eq!(out.map.state_at(2000), Some(SectorState::Corrupt));
        assert_eq!(out.map.state_at(2010), Some(SectorState::Good));
    }

    /// A read-only handle is no longer a mode, so it is now an error. The
    /// scan has to say so before touching the device rather than failing at the
    /// first write, when the caller can still do something about it.
    #[test]
    fn a_read_only_device_is_refused_before_anything_is_written() {
        let mut card = SimulatedCard::healthy(SS, 1024);
        card.fill_with_marker(0x5A);
        card.set_writable(false);

        let err = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .expect_err("a write-protected device must not be scanned");

        assert!(matches!(err, ScanError::NotWritable), "unexpected error: {err:?}");
        assert!(card.is_entirely_marker(0x5A), "the device was written to despite the refusal");
        assert_eq!(card.sectors_written, 0, "a refused scan must not attempt a single write");
    }

    /// The diagnosis now refuses any verdict while a single sector remains
    /// uninspected. That makes total coverage a hard requirement rather than a
    /// nicety: a scan that quietly skipped sectors would report every card as
    /// unproven, and the tool would never conclude anything again.
    #[test]
    fn a_completed_scan_leaves_no_sector_uninspected() {
        let sectors = 4096;
        let cases: Vec<(&str, SimulatedFault)> = vec![
            ("healthy", SimulatedFault::None),
            ("read errors", SimulatedFault::ReadError { range: LbaRange::from_bounds(1000, 1100) }),
            (
                "write errors",
                SimulatedFault::WriteError { range: LbaRange::from_bounds(2000, 2400) },
            ),
            (
                "silent corruption",
                SimulatedFault::SilentCorruption { range: LbaRange::from_bounds(300, 900) },
            ),
            ("counterfeit", SimulatedFault::FakeCapacity { real: 1024 }),
            ("stuck at zero", SimulatedFault::StuckAtZero { range: LbaRange::from_bounds(0, 512) }),
            (
                "mixed",
                SimulatedFault::Multiple {
                    ranges: vec![
                        LbaRange::from_bounds(64, 128),
                        LbaRange::from_bounds(1500, 1700),
                        LbaRange::from_bounds(3000, 3100),
                    ],
                },
            ),
        ];

        for (name, fault) in cases {
            let mut card = SimulatedCard::with_fault(SS, sectors, fault);
            let out = scan(&mut card);
            let counts = out.map.counts();

            assert_eq!(counts.untested, 0, "{name}: {} sectors left uninspected", counts.untested);
            assert_eq!(counts.total(), sectors, "{name}: the map does not cover the device");
        }
    }

    /// The regression that made a real 128 GB card need twenty hours.
    ///
    /// Reporting each defective sector separately produced one map operation
    /// and one observer call per sector. On a card where nearly everything
    /// fails that is tens of millions of them, and the reporting cost several
    /// times more than the I/O it was describing.
    #[test]
    fn a_solidly_defective_region_is_reported_as_one_run_not_per_sector() {
        #[derive(Default)]
        struct CountingObserver {
            defects: usize,
        }
        impl ScanObserver for CountingObserver {
            fn on_progress(&mut self, _: &ScanProgress, _: &SectorMap) {}
            fn on_defect(&mut self, _: LbaRange, _: SectorState) {
                self.defects += 1;
            }
        }

        let sectors = 4096;
        let mut card = SimulatedCard::with_fault(
            SS,
            sectors,
            SimulatedFault::SilentCorruption { range: LbaRange::from_bounds(0, sectors) },
        );
        let mut observer = CountingObserver::default();
        let out = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut observer)
            .expect("the scan should complete");

        assert_eq!(out.map.counts().corrupt, sectors, "the corruption was not detected");
        assert!(
            observer.defects <= sectors as usize / 100,
            "{} notifications for {sectors} contiguous corrupt sectors: they are not being grouped",
            observer.defects
        );
        assert!(
            out.map.runs().len() <= 8,
            "the map fragmented into {} runs over one solid defect",
            out.map.runs().len()
        );
    }

    /// The regression that made the inspection look frozen on a real card.
    ///
    /// When nothing writes, bisection would walk each block entire tree —
    /// thousands of attempts each, across thousands of blocks — to conclude what
    /// three attempts already said. The user sees a motionless window for hours,
    /// with no error on screen at all.
    #[test]
    fn a_card_that_refuses_every_write_fails_fast_with_a_clear_cause() {
        let mut card = SimulatedCard::with_fault(SS, 65_536, SimulatedFault::WriteProtected);
        let err = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .unwrap_err();

        match err {
            ScanError::SystemicWriteFailure { consecutive, .. } => {
                assert_eq!(consecutive, SYSTEMIC_FAILURE_THRESHOLD);
            }
            other => panic!("expected a systemic-failure verdict, got: {other}"),
        }

        // Cost must be proportional to a few samples, not to a full bisection
        // of every block.
        let ceiling = (SYSTEMIC_FAILURE_THRESHOLD as u64 + 1) * (WRITE_PROBE_SAMPLES as u64 + 2);
        assert!(
            card.writes <= ceiling,
            "{} write attempts to detect the obvious (ceiling: {ceiling})",
            card.writes
        );
    }

    /// The message must point at the real cause rather than the media: someone
    /// reading "defective card" throws away a card that was merely mounted.
    #[test]
    fn the_systemic_failure_message_names_the_real_causes() {
        let err = ScanError::SystemicWriteFailure { consecutive: 3, sectors: 24_576 };
        let text = err.to_string().to_lowercase();
        assert!(text.contains("mounted"), "does not mention a mounted volume: {text}");
        assert!(text.contains("write-protect"), "does not mention write protection: {text}");
        assert!(text.contains("elevated"), "does not mention privilege: {text}");
        assert!(text.contains("not a media defect"), "does not rule out a media defect: {text}");
    }

    /// A localised defect is still refined normally: the systemic-failure
    /// guard must not interfere with the legitimate case.
    #[test]
    fn localised_write_defects_are_still_refined_precisely() {
        let bad = LbaRange::from_bounds(1500, 1503);
        let mut card =
            SimulatedCard::with_fault(SS, 8192, SimulatedFault::WriteError { range: bad });
        let out = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .expect("a localised defect must not be mistaken for systemic failure");

        out.map.check_invariants().unwrap();
        assert!(out.map.state_at(1500).unwrap().is_defective());
        assert_eq!(out.map.state_at(1499), Some(SectorState::Good));
        assert_eq!(out.map.state_at(1600), Some(SectorState::Good));
    }

    /// A transient failure must not condemn good area. Serious diagnostic
    /// tools retry before marking a sector.
    #[test]
    fn a_transient_read_error_does_not_condemn_the_sector() {
        let flaky = LbaRange::from_bounds(2000, 2004);
        let mut card = SimulatedCard::with_fault(
            SS,
            8192,
            SimulatedFault::FlakyRead { range: flaky, fails: 2 },
        );

        let out = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .expect("the scan should have completed");
        out.map.check_invariants().unwrap();

        assert_eq!(
            out.map.counts().bad_read,
            0,
            "a transient error became a permanent defect: the retry did not work"
        );
        for lba in flaky.start()..flaky.end() {
            assert_eq!(out.map.state_at(lba), Some(SectorState::Good));
        }
    }

    /// A genuinely permanent error is still condemned: retrying must not turn
    /// a real defect into an approval.
    #[test]
    fn a_permanent_read_error_is_still_condemned_after_retries() {
        let bad = LbaRange::from_bounds(2000, 2004);
        let mut card =
            SimulatedCard::with_fault(SS, 8192, SimulatedFault::ReadError { range: bad });
        let out = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .unwrap();
        assert!(out.map.state_at(2000).unwrap().is_defective());
    }

    /// Why several patterns exist: a cell stuck at zero matches half the bits
    /// of a pseudorandom pattern and can slip through, but fails immediately
    /// under all-ones.
    #[test]
    fn a_stuck_at_zero_bit_is_caught_by_the_all_ones_pattern() {
        let stuck = LbaRange::from_bounds(1000, 1001);

        let mut with_ones =
            SimulatedCard::with_fault(SS, 4096, SimulatedFault::StuckAtZero { range: stuck });
        let out = Scanner::new(
            ScanConfig { pattern: PatternKind::Ones, ..config() },
            CancellationToken::new(),
        )
        .run(&mut with_ones, &mut SilentObserver)
        .unwrap();

        assert!(
            out.map.state_at(1000).unwrap().is_defective(),
            "the all-ones pattern must expose a cell stuck at zero"
        );
    }

    /// Constant patterns must detect aliasing too: the self-identifying header
    /// is present in all of them.
    #[test]
    fn every_pattern_still_detects_a_counterfeit_card() {
        for kind in PatternKind::SWEEP {
            let real = 1024u64;
            let mut card =
                SimulatedCard::with_fault(SS, 4096, SimulatedFault::FakeCapacity { real });
            let out =
                Scanner::new(ScanConfig { pattern: kind, ..config() }, CancellationToken::new())
                    .run(&mut card, &mut SilentObserver)
                    .unwrap();

            assert_eq!(
                out.map.counts().aliased,
                4096 - real,
                "pattern {} lost counterfeit-capacity detection",
                kind.label()
            );
        }
    }

    #[test]
    fn a_destructive_scan_is_refused_on_a_read_only_device() {
        let mut card = SimulatedCard::healthy(SS, 1024);
        card.set_writable(false);
        let err = Scanner::new(config(), CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .unwrap_err();
        assert!(matches!(err, ScanError::NotWritable));
    }

    #[test]
    fn cancellation_stops_the_scan() {
        let mut card = SimulatedCard::healthy(SS, 65_536);
        let token = CancellationToken::new();
        token.cancel();
        let err = Scanner::new(config(), CancellationToken::clone(&token))
            .run(&mut card, &mut SilentObserver)
            .unwrap_err();
        assert!(matches!(err, ScanError::Cancelled));
    }

    #[test]
    fn progress_is_monotonic_and_completes() {
        struct Recorder {
            last: std::collections::HashMap<&'static str, u64>,
            final_verify: f64,
        }
        impl ScanObserver for Recorder {
            fn on_progress(&mut self, p: &ScanProgress, _map: &SectorMap) {
                if p.phase == ScanPhase::Refining {
                    return; // bisection does not advance the phase counter
                }
                let key = if p.phase == ScanPhase::Writing { "w" } else { "v" };
                let prev = self.last.get(key).copied().unwrap_or(0);
                assert!(p.sectors_done >= prev, "progress went backwards in phase {key}");
                self.last.insert(key, p.sectors_done);
                if p.phase == ScanPhase::Verifying {
                    self.final_verify = p.fraction();
                }
            }
        }

        let mut card = SimulatedCard::healthy(SS, 4096);
        let mut rec = Recorder { last: Default::default(), final_verify: 0.0 };
        Scanner::new(config(), CancellationToken::new()).run(&mut card, &mut rec).unwrap();
        assert!((rec.final_verify - 1.0).abs() < 1e-9, "the verify phase never reached 100%");
    }

    #[test]
    fn a_partial_range_leaves_the_rest_untested() {
        let mut card = SimulatedCard::healthy(SS, 4096);
        let cfg = ScanConfig { range: Some(LbaRange::from_bounds(1024, 2048)), ..config() };
        let out = Scanner::new(cfg, CancellationToken::new())
            .run(&mut card, &mut SilentObserver)
            .unwrap();
        out.map.check_invariants().unwrap();

        assert_eq!(out.map.counts().good, 1024);
        assert_eq!(out.map.counts().untested, 4096 - 1024);
        assert_eq!(out.map.state_at(0), Some(SectorState::Untested));
        assert_eq!(out.map.state_at(1024), Some(SectorState::Good));
    }

    /// A card with scattered defects must neither break the map structure nor
    /// escape detection.
    #[test]
    fn many_scattered_defects_are_all_found() {
        let ranges = vec![
            LbaRange::from_bounds(100, 103),
            LbaRange::from_bounds(900, 901),
            LbaRange::from_bounds(2500, 2530),
            LbaRange::from_bounds(4000, 4002),
        ];
        let expected: u64 = ranges.iter().map(|r| r.len()).sum();
        let mut card = SimulatedCard::with_fault(
            SS,
            4096,
            SimulatedFault::Multiple { ranges: ranges.clone() },
        );
        let out = scan(&mut card);
        out.map.check_invariants().unwrap();

        // Cada setor defeituoso precisa estar condenado, um a um.
        for r in &ranges {
            for lba in r.start()..r.end() {
                assert!(
                    out.map.state_at(lba).unwrap().is_defective(),
                    "the defect at {lba} went unnoticed"
                );
            }
        }

        // Rounding cost is bounded by the granularity, per region.
        let ceiling = expected + ranges.len() as u64 * MIN_REFINE_SECTORS * 2;
        assert!(
            out.map.counts().defective() <= ceiling,
            "condemned {} sectors for {expected} defective ones (ceiling {ceiling})",
            out.map.counts().defective()
        );

        // E a maior parte do cartao continua aproveitavel.
        assert!(out.map.counts().good > 3000, "too little usable space survived");
    }

    /// End to end: a counterfeit card inspected, diagnosed and partitioned,
    /// with the data partition entirely inside the real area.
    #[test]
    fn end_to_end_a_counterfeit_card_yields_a_safe_partition_plan() {
        use salvage_core::planner::{plan_layouts, FileSystem, PlanningPolicy};

        let real_sectors = 40 * 8192; // 160 MiB reais
        let fake_sectors = 200 * 8192; // 800 MiB anunciados
        let mut card = SimulatedCard::with_fault(
            SS,
            fake_sectors,
            SimulatedFault::FakeCapacity { real: real_sectors },
        );
        let out =
            Scanner::new(ScanConfig { chunk_sectors: 8192, ..config() }, CancellationToken::new())
                .run(&mut card, &mut SilentObserver)
                .unwrap();

        let report = diagnose(&out.map, None);
        assert_eq!(report.assurance, Assurance::High);
        assert!(report.isolation_is_worthwhile);

        let policy = PlanningPolicy::recommended_for(out.map.geometry());
        let plans = plan_layouts(&out.map, FileSystem::ExFat, &policy);
        assert!(!plans.is_empty(), "deveria haver ao menos um plano viavel");

        for plan in &plans {
            plan.validate(&out.map, 4).unwrap();
            for dp in plan.data_partitions() {
                assert!(
                    dp.range.end() <= real_sectors,
                    "partition '{}' ends at {} and reaches into nonexistent area (real: {})",
                    dp.label,
                    dp.range.end(),
                    real_sectors
                );
            }
        }
    }
}
