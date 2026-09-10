//! Diagnosis: turns a sector map into a failure classification, and from that
//! into the level of assurance the tool may honestly offer.
//!
//! This is the module that decides what the program has the right to promise.
//! Fencing bad sectors protects real data when the defect boundary is stable,
//! and protects nothing when the controller is collapsing progressively.
//! Conflating those two cases is the one way this tool could cause data loss,
//! so the distinction is explicit and carries its own type.
//!
//! # No presentation text lives here
//!
//! This module returns classifications and numbers, never sentences. Wording,
//! translation and emphasis belong to the presentation layer; a domain that
//! hard-codes user-facing prose cannot be reused by a CLI, a different
//! language, or an automated report without dragging one interface's voice
//! into all of them.

use serde::{Deserialize, Serialize};

use crate::geometry::LbaRange;
use crate::sector_map::{SectorCounts, SectorMap, SectorState};

/// Greatest common divisor, used to recover the wrap-around modulus.
const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Nature of the device's failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureScenario {
    /// No defect found in the inspected area.
    Pristine,

    /// Scenario A: the card lies about its own capacity.
    ///
    /// The controller accepts addresses that do not exist and remaps them into
    /// the real area. The boundary is set by firmware rather than wear, so it
    /// does not move over time: fencing the real region resolves the problem
    /// permanently.
    CounterfeitCapacity {
        /// Estimated real capacity, in sectors.
        real_capacity_sectors: u64,
        /// Capacity advertised by the device, in sectors.
        reported_capacity_sectors: u64,
        /// How many independent aliasing observations support the conclusion.
        evidence_count: usize,
    },

    /// Scenario B: the FTL has exhausted its spare blocks.
    ///
    /// Defects stopped being remapped and froze onto fixed addresses. Two
    /// passes produced exactly the same defect set, which suggests stability —
    /// but the card is already at the end of its service life.
    ExhaustedSpare {
        /// Number of distinct defective regions.
        defect_regions: usize,
    },

    /// Scenario C: the device is deteriorating during the inspection itself.
    ///
    /// Sectors that passed one pass failed the next. No partition layout
    /// protects against this, because tomorrow's bad sectors do not exist yet.
    ActivelyDegrading {
        /// Sectors that passed the first verification and failed the second.
        newly_failed_sectors: u64,
        /// Regions where the degradation appeared.
        newly_failed_regions: Vec<LbaRange>,
    },

    /// Defects exist, but only one pass was run: B and C cannot be separated.
    Indeterminate {
        /// Number of distinct defective regions.
        defect_regions: usize,
    },

    /// The scan proved nothing about the device's integrity.
    ///
    /// Raised when the map contains merely-readable sectors, that is, when a
    /// The scan did not cover the whole device, so nothing can be concluded
    /// about it as a whole.
    ///
    /// A run that was cancelled, or deliberately restricted to a range, leaves
    /// sectors nobody looked at. Reporting "no defects found" from that would
    /// turn a partial inspection into a certificate of health for the parts it
    /// skipped.
    ///
    /// This variant exists so that silence is never presented as evidence.
    NotProven {
        /// Sectors the scan never reached.
        unverified_sectors: u64,
        /// Defects found regardless — these are real.
        defect_regions: usize,
    },
}

impl FailureScenario {
    /// Stable identifier for serialization and presentation lookup.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Pristine => "pristine",
            Self::CounterfeitCapacity { .. } => "counterfeit_capacity",
            Self::ExhaustedSpare { .. } => "exhausted_spare",
            Self::ActivelyDegrading { .. } => "actively_degrading",
            Self::Indeterminate { .. } => "indeterminate",
            Self::NotProven { .. } => "not_proven",
        }
    }
}

/// How far the result can be trusted after fencing.
///
/// There is deliberately no "guaranteed" variant. No software can guarantee
/// integrity on hardware that has already demonstrated a defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assurance {
    /// No basis for any claim: the card is collapsing, or nothing was verified.
    None,
    /// Fencing helps, but the defect boundary may move.
    Low,
    /// Defects appear stable; fencing is a solid mitigation.
    Moderate,
    /// Boundary set by firmware rather than wear. Fencing addresses the cause,
    /// and the preserved area is as reliable as the NAND the card actually has.
    High,
}

impl Assurance {
    /// Stable identifier for serialization and presentation lookup.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }
}

/// Complete diagnosis of a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    /// Nature of the failure.
    pub scenario: FailureScenario,
    /// Level of assurance achievable through fencing.
    pub assurance: Assurance,
    /// Sector counts per state.
    pub counts: SectorCounts,
    /// Largest contiguous verified-good area, in sectors.
    pub largest_usable_sectors: u64,
    /// Largest contiguous run free of any proven defect.
    ///
    /// Includes never-inspected area, and therefore authorises nothing on its
    /// own. It answers a different question from `largest_usable_sectors`:
    /// not "where may data go", but "where is it worth spending a full scan".
    pub largest_defect_free: LbaRange,
    /// Whether fencing is worth offering for this device.
    pub isolation_is_worthwhile: bool,
}

/// Smallest divisor of `n` strictly greater than `floor`.
///
/// Runs in time proportional to the square root of `n`, which for any real
/// sector count stays in the tens of thousands of iterations.
fn smallest_divisor_above(n: u64, floor: u64) -> Option<u64> {
    if n == 0 {
        return None;
    }
    let mut best: Option<u64> = None;
    let mut i = 1u64;
    while i.saturating_mul(i) <= n {
        if n % i == 0 {
            for d in [i, n / i] {
                if d > floor {
                    best = Some(best.map_or(d, |b: u64| b.min(d)));
                }
            }
        }
        i += 1;
    }
    best
}

/// Estimates the real capacity of a card that lies about its size.
///
/// Under modular wrap-around the device returns `req mod C` when `req` is
/// requested, so `C` divides `req - act` for every observation, and therefore
/// `C` divides the GCD of all the differences.
///
/// The GCD itself is **not** the answer. When observations land on the same
/// remainder — which happens whenever the scan advances in regular steps — the
/// GCD converges to a multiple of `C` rather than to `C`. Accepting it would
/// overestimate the capacity, and overestimating here means declaring an area
/// safe that does not exist.
///
/// The correct inference uses the second available fact: every address the
/// card actually returned does exist, so `C` is greater than the largest of
/// them. Among the divisors of the GCD, the smallest one satisfying that floor
/// is the most conservative estimate consistent with the evidence. The result
/// is further capped by the lowest address observed aliasing, which is a
/// guaranteed ceiling.
pub fn estimate_real_capacity(map: &SectorMap) -> Option<u64> {
    let aliases = map.aliases();
    if aliases.is_empty() {
        return None;
    }

    let lowest_aliased = aliases.iter().map(|a| a.requested_lba).min()?;
    let highest_actual = aliases.iter().map(|a| a.actual_lba).max()?;

    let modulus = aliases.iter().map(|a| a.stride()).filter(|s| *s > 0).fold(0u64, gcd);

    match smallest_divisor_above(modulus, highest_actual) {
        Some(c) if c <= lowest_aliased => Some(c),
        // No plausible divisor: fall back to the guaranteed observed ceiling.
        _ => Some(lowest_aliased),
    }
}

/// Largest contiguous run free of any proven defect.
///
/// This differs from the largest usable run in a way that confuses anyone
/// reading the card diagram: that one counts only **verified** area, this one
/// counts everything that has not **failed**, including never-inspected space.
///
/// After a sampled triage the two numbers diverge wildly. Approved samples are
/// isolated blocks a few dozen kilobytes wide, so the largest verified run is
/// tiny; meanwhile half the card may not have produced a single defect.
/// Reporting only the first number makes it look as though nothing is left,
/// when in fact a candidate region remains that simply has not been examined.
///
/// This run **authorises nothing**. It answers where a full scan is worth
/// spending.
pub fn largest_defect_free_run(map: &SectorMap) -> LbaRange {
    let mut best = LbaRange::from_bounds(0, 0);
    let mut current: Option<LbaRange> = None;

    for run in map.runs() {
        if run.state.is_defective() {
            current = None;
            continue;
        }
        let extended = match current {
            Some(c) => LbaRange::from_bounds(c.start(), run.range.end()),
            None => run.range,
        };
        if extended.len() > best.len() {
            best = extended;
        }
        current = Some(extended);
    }
    best
}

/// Sectors that passed in `previous` and failed in `current`.
///
/// The comparison is the only way to separate a stable card from a collapsing
/// one: a single map carries no time dimension.
pub fn newly_failed_regions(previous: &SectorMap, current: &SectorMap) -> Vec<LbaRange> {
    let mut out = Vec::new();
    for run in current.runs() {
        if !run.state.is_defective() {
            continue;
        }
        // Restrict the comparison to stretches that were previously approved.
        for prev in previous.runs() {
            if prev.state != SectorState::Good {
                continue;
            }
            if let Some(overlap) = prev.range.intersection(&run.range) {
                out.push(overlap);
            }
        }
    }
    out.sort_unstable_by_key(|r| r.start());
    out
}

/// Classifies a device from its current map and, when available, an earlier
/// pass for temporal comparison.
pub fn diagnose(current: &SectorMap, previous: Option<&SectorMap>) -> HealthReport {
    let counts = current.counts();
    let largest_usable_sectors = current.usable_ranges().map(|r| r.len()).max().unwrap_or(0);
    let largest_defect_free = largest_defect_free_run(current);
    let defect_regions = current.defective_ranges().count();

    // Aliasing takes precedence over every other signal: it is the only
    // evidence that identifies the root cause rather than just the symptom.
    if !current.aliases().is_empty() {
        let reported = current.geometry().total_sectors();
        let real = estimate_real_capacity(current).unwrap_or(reported);
        return HealthReport {
            scenario: FailureScenario::CounterfeitCapacity {
                real_capacity_sectors: real,
                reported_capacity_sectors: reported,
                evidence_count: current.aliases().len(),
            },
            assurance: Assurance::High,
            counts,
            largest_usable_sectors,
            largest_defect_free,
            isolation_is_worthwhile: true,
        };
    }

    // Sectors nobody inspected block every verdict about the card as a whole.
    // Without this, a run restricted to a range — or cancelled early — would
    // find no defects in the sliver it covered and report the entire card
    // pristine. The check comes before every other conclusion, including the
    // defect ones: those defects are real, but the silence of everything the
    // scan skipped still means nothing.
    if counts.untested > 0 {
        return HealthReport {
            scenario: FailureScenario::NotProven {
                unverified_sectors: counts.untested,
                defect_regions,
            },
            assurance: Assurance::None,
            counts,
            largest_usable_sectors,
            largest_defect_free,
            // The card as a whole gets no verdict, but the sectors that were
            // verified were genuinely verified, and a plan is built from those
            // alone. A full inspection of a large card runs for hours; refusing
            // to use a run that stopped at 80% would discard real evidence and
            // teach the user that stopping is catastrophic. What is withheld is
            // the health verdict, not the work.
            isolation_is_worthwhile: largest_usable_sectors > 0,
        };
    }

    if counts.defective() == 0 {
        return HealthReport {
            scenario: FailureScenario::Pristine,
            assurance: Assurance::High,
            counts,
            largest_usable_sectors,
            largest_defect_free,
            isolation_is_worthwhile: false,
        };
    }

    match previous {
        Some(prev) => {
            let regressions = newly_failed_regions(prev, current);
            if regressions.is_empty() {
                HealthReport {
                    scenario: FailureScenario::ExhaustedSpare { defect_regions },
                    assurance: Assurance::Moderate,
                    counts,
                    largest_usable_sectors,
                    largest_defect_free,
                    isolation_is_worthwhile: true,
                }
            } else {
                let newly_failed_sectors = regressions.iter().map(|r| r.len()).sum();
                HealthReport {
                    scenario: FailureScenario::ActivelyDegrading {
                        newly_failed_sectors,
                        newly_failed_regions: regressions,
                    },
                    assurance: Assurance::None,
                    counts,
                    largest_usable_sectors,
                    largest_defect_free,
                    isolation_is_worthwhile: false,
                }
            }
        }
        None => HealthReport {
            scenario: FailureScenario::Indeterminate { defect_regions },
            assurance: Assurance::Low,
            counts,
            largest_usable_sectors,
            largest_defect_free,
            isolation_is_worthwhile: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::DeviceGeometry;

    fn map(total: u64) -> SectorMap {
        SectorMap::new(DeviceGeometry::new(512, total).unwrap())
    }

    fn all_good(total: u64) -> SectorMap {
        let mut m = map(total);
        m.mark(LbaRange::from_bounds(0, total), SectorState::Good);
        m
    }

    #[test]
    fn a_clean_card_needs_no_isolation() {
        let r = diagnose(&all_good(10_000), None);
        assert_eq!(r.scenario, FailureScenario::Pristine);
        assert_eq!(r.assurance, Assurance::High);
        assert!(!r.isolation_is_worthwhile);
        assert_eq!(r.largest_usable_sectors, 10_000);
    }

    /// The crux of re-inspecting a card this program already fenced: the area
    /// left out was condemned by an earlier run, not skipped, and the run that
    /// covered what remains has to be allowed to conclude something about it.
    /// Untested area denies every verdict — rightly — so the narrowing has to
    /// be recorded as withheld, and this is the test that says it worked.
    #[test]
    fn a_deliberately_narrowed_inspection_still_reaches_a_verdict() {
        let span = LbaRange::from_bounds(8192, 100_000);
        let mut m = map(200_000);
        m.mark(span, SectorState::Good);
        m.withhold_outside(span);

        let r = diagnose(&m, None);
        assert_eq!(r.scenario, FailureScenario::Pristine);
        assert_eq!(r.assurance, Assurance::High);
        // Only what the run actually read back may be counted as usable.
        assert_eq!(r.largest_usable_sectors, span.len());
    }

    /// And the opposite case has to keep working: a run that stopped early
    /// leaves real gaps, and those still refuse a verdict.
    #[test]
    fn a_run_that_stopped_early_still_proves_nothing() {
        let mut m = map(200_000);
        m.mark(LbaRange::from_bounds(0, 100_000), SectorState::Good);

        let r = diagnose(&m, None);
        assert_eq!(r.scenario.kind(), "not_proven");
        assert_eq!(r.assurance, Assurance::None);
    }

    #[test]
    fn aliasing_is_diagnosed_as_counterfeit_with_high_assurance() {
        let mut m = all_good(10_000);
        m.record_alias(9_000, 1_000);
        let r = diagnose(&m, None);
        assert!(matches!(r.scenario, FailureScenario::CounterfeitCapacity { .. }));
        assert_eq!(r.assurance, Assurance::High);
        assert!(r.isolation_is_worthwhile);
    }

    /// Observations landing on the same remainder produce a GCD that is only a
    /// multiple of the capacity. Accepting the raw GCD would declare a
    /// nonexistent area safe, so the estimate must descend to the smallest
    /// viable divisor.
    #[test]
    fn real_capacity_is_not_overestimated_when_the_gcd_is_a_multiple() {
        let real = 8_000u64;
        let mut m = all_good(100_000);
        for req in [20_000u64, 36_000, 52_000] {
            m.record_alias(req, req % real);
        }
        // Strides are 16k, 32k and 48k; the GCD is 16,000 — twice the capacity.
        assert_eq!(estimate_real_capacity(&m), Some(real));
    }

    #[test]
    fn real_capacity_is_recovered_from_varied_remainders() {
        let real = 8_000u64;
        let mut m = all_good(100_000);
        for req in [20_000u64, 33_000, 51_000] {
            m.record_alias(req, req % real);
        }
        assert_eq!(estimate_real_capacity(&m), Some(real));
    }

    /// A lone observation does not determine the capacity. The estimate settles
    /// on the smallest divisor consistent with what is known to exist, which
    /// errs low.
    #[test]
    fn a_single_observation_yields_the_smallest_viable_divisor() {
        let mut m = all_good(100_000);
        m.record_alias(50_000, 10_000);
        // Stride 40,000; divisors above 10,000 are 20,000 and 40,000.
        assert_eq!(estimate_real_capacity(&m), Some(20_000));
    }

    /// The estimate may never exceed the lowest address observed aliasing:
    /// above it, the card is known to have no memory.
    #[test]
    fn the_estimate_never_exceeds_the_observed_ceiling() {
        let mut seed = 0x5EED_1234u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let real = (rnd() % 50_000) + 1_000;
            let mut m = all_good(1_000_000);
            for _ in 0..3 {
                let req = real + (rnd() % 500_000) + 1;
                m.record_alias(req, req % real);
            }
            let ceiling = m.aliases().iter().map(|a| a.requested_lba).min().unwrap();
            let floor = m.aliases().iter().map(|a| a.actual_lba).max().unwrap();
            let est = estimate_real_capacity(&m).unwrap();
            assert!(est <= ceiling, "estimate {est} above ceiling {ceiling}");
            assert!(est > floor, "estimate {est} below a known-existing address {floor}");
        }
    }

    #[test]
    fn an_implausible_modulus_falls_back_to_the_observed_bound() {
        let mut m = all_good(100_000);
        m.record_alias(11_000, 10_000);
        assert_eq!(estimate_real_capacity(&m), Some(11_000));
    }

    #[test]
    fn no_aliases_means_no_capacity_estimate() {
        assert_eq!(estimate_real_capacity(&all_good(1000)), None);
    }

    #[test]
    fn defects_without_a_second_pass_are_indeterminate() {
        let mut m = all_good(10_000);
        m.mark(LbaRange::from_bounds(5_000, 5_100), SectorState::BadRead);
        let r = diagnose(&m, None);
        assert!(matches!(r.scenario, FailureScenario::Indeterminate { defect_regions: 1 }));
        assert_eq!(r.assurance, Assurance::Low);
        assert!(r.isolation_is_worthwhile);
    }

    #[test]
    fn identical_passes_indicate_stable_defects() {
        let mut first = all_good(10_000);
        first.mark(LbaRange::from_bounds(5_000, 5_100), SectorState::BadRead);
        let second = first.clone();
        let r = diagnose(&second, Some(&first));
        assert!(matches!(r.scenario, FailureScenario::ExhaustedSpare { defect_regions: 1 }));
        assert_eq!(r.assurance, Assurance::Moderate);
        assert!(r.isolation_is_worthwhile);
    }

    /// The case the tool must get right: a sector approved before and rejected
    /// now means no layout can protect the user.
    #[test]
    fn a_sector_that_regresses_between_passes_forbids_any_promise() {
        let first = all_good(10_000);
        let mut second = all_good(10_000);
        second.mark(LbaRange::from_bounds(7_000, 7_050), SectorState::Corrupt);

        let r = diagnose(&second, Some(&first));
        match &r.scenario {
            FailureScenario::ActivelyDegrading { newly_failed_sectors, newly_failed_regions } => {
                assert_eq!(*newly_failed_sectors, 50);
                assert_eq!(newly_failed_regions.len(), 1);
            }
            other => panic!("expected ActivelyDegrading, got {other:?}"),
        }
        assert_eq!(r.assurance, Assurance::None);
        assert!(!r.isolation_is_worthwhile, "a collapsing card must not be offered fencing");
    }

    #[test]
    fn a_pre_existing_defect_is_not_counted_as_degradation() {
        let mut first = all_good(10_000);
        first.mark(LbaRange::from_bounds(7_000, 7_050), SectorState::Corrupt);
        let second = first.clone();
        assert!(newly_failed_regions(&first, &second).is_empty());
    }

    #[test]
    fn counterfeit_takes_precedence_over_degradation() {
        let first = all_good(10_000);
        let mut second = all_good(10_000);
        second.mark(LbaRange::from_bounds(7_000, 7_050), SectorState::Corrupt);
        second.record_alias(9_000, 1_000);
        let r = diagnose(&second, Some(&first));
        assert!(matches!(r.scenario, FailureScenario::CounterfeitCapacity { .. }));
    }

    /// The most dangerous regression this tool could have: certifying area
    /// nobody inspected. Every sector the scan skipped is a sector about which
    /// the only honest statement is that nothing is known.
    #[test]
    fn a_scan_that_covered_nothing_is_never_reported_as_healthy() {
        let m = map(10_000);

        let r = diagnose(&m, None);
        assert!(
            matches!(r.scenario, FailureScenario::NotProven { .. }),
            "an unproven scan became {:?}",
            r.scenario
        );
        assert_eq!(r.assurance, Assurance::None, "an untouched card cannot yield assurance");
        assert!(!r.isolation_is_worthwhile, "no partition is planned over unproven area");
        assert_ne!(r.scenario, FailureScenario::Pristine);
    }

    /// The concrete hole this guards: a scan restricted to a range, or stopped
    /// early, finds no defects in the sliver it covered. Without the check it
    /// reports the whole card pristine with high assurance — on the strength of
    /// the 10% it looked at.
    #[test]
    fn a_clean_partial_scan_does_not_certify_the_untouched_remainder() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 1_000), SectorState::Good);

        let r = diagnose(&m, None);
        assert!(
            matches!(r.scenario, FailureScenario::NotProven { unverified_sectors: 9_000, .. }),
            "a 10% scan concluded {:?}",
            r.scenario
        );
        assert_eq!(r.assurance, Assurance::None);
    }

    /// Nor does finding real defects rescue the verdict: they are evidence
    /// about where they were found and nowhere else.
    #[test]
    fn defects_found_in_a_partial_scan_do_not_complete_it() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 8_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(8_000, 9_000), SectorState::Corrupt);

        let r = diagnose(&m, None);
        assert!(matches!(r.scenario, FailureScenario::NotProven { .. }));
        assert_eq!(r.assurance, Assurance::None);
    }

    /// A long inspection that was stopped part way still yields something to
    /// build on. The verdict stays refused — nothing is known about the part
    /// nobody looked at — but the approved area is approved, and the planner
    /// only ever uses that.
    #[test]
    fn a_partial_scan_still_offers_the_area_it_did_verify() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 6_000), SectorState::Good);

        let r = diagnose(&m, None);
        assert!(matches!(r.scenario, FailureScenario::NotProven { .. }));
        assert_eq!(r.assurance, Assurance::None, "a partial scan cannot yield assurance");
        assert!(r.isolation_is_worthwhile, "six thousand verified sectors were thrown away");
        assert_eq!(r.largest_usable_sectors, 6_000);
    }

    /// With nothing verified there is nothing to offer either.
    #[test]
    fn a_scan_that_verified_nothing_offers_no_isolation() {
        let r = diagnose(&map(10_000), None);
        assert!(!r.isolation_is_worthwhile);
    }

    /// The counterpart: a scan that did cover everything must still reach a
    /// real verdict, or the check above would have made the tool useless.
    #[test]
    fn a_complete_scan_still_reaches_a_verdict() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 10_000), SectorState::Good);

        let r = diagnose(&m, None);
        assert_eq!(r.scenario, FailureScenario::Pristine);
        assert_eq!(r.assurance, Assurance::High);
    }

    #[test]
    fn assurance_is_ordered_from_worst_to_best() {
        assert!(Assurance::None < Assurance::Low);
        assert!(Assurance::Low < Assurance::Moderate);
        assert!(Assurance::Moderate < Assurance::High);
    }

    #[test]
    fn largest_usable_reports_the_biggest_contiguous_run() {
        let mut m = map(10_000);
        m.mark(LbaRange::from_bounds(0, 2_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(2_000, 2_100), SectorState::BadRead);
        m.mark(LbaRange::from_bounds(2_100, 9_000), SectorState::Good);
        m.mark(LbaRange::from_bounds(9_000, 10_000), SectorState::Untested);
        assert_eq!(diagnose(&m, None).largest_usable_sectors, 6_900);
    }

    /// The case that made the on-screen numbers confusing: after a triage, the
    /// verified area is a handful of isolated samples while half the card never
    /// failed. Reporting only the first number suggests nothing is left.
    #[test]
    fn a_sampled_map_reports_a_tiny_verified_area_but_a_large_candidate() {
        let mut m = map(100_000);
        m.mark(LbaRange::from_bounds(50_000, 100_000), SectorState::Corrupt);
        for i in 0..50 {
            m.mark(LbaRange::new(i * 1_000, 128), SectorState::Good);
        }

        let r = diagnose(&m, None);
        assert_eq!(r.largest_usable_sectors, 128, "verified run should be one sample wide");
        assert_eq!(r.largest_defect_free.len(), 50_000, "candidate should span the clean half");
        assert_eq!(r.largest_defect_free.start(), 0);
    }

    #[test]
    fn the_defect_free_run_stops_at_a_defect() {
        let mut m = map(1_000);
        m.mark(LbaRange::from_bounds(400, 410), SectorState::BadRead);
        assert_eq!(largest_defect_free_run(&m), LbaRange::from_bounds(410, 1_000));
    }

    #[test]
    fn a_card_with_no_defects_has_one_defect_free_run_covering_everything() {
        let m = all_good(10_000);
        assert_eq!(largest_defect_free_run(&m), LbaRange::from_bounds(0, 10_000));
    }

    /// The defect-free run must never be mistaken for approved area: it
    /// includes everything that was never looked at.
    #[test]
    fn the_defect_free_run_includes_untested_area() {
        let m = map(10_000);
        assert_eq!(largest_defect_free_run(&m).len(), 10_000);
        assert_eq!(diagnose(&m, None).largest_usable_sectors, 0);
    }

    #[test]
    fn scenario_and_assurance_expose_stable_identifiers() {
        assert_eq!(FailureScenario::Pristine.kind(), "pristine");
        assert_eq!(Assurance::High.as_str(), "high");
        let kinds: std::collections::HashSet<&str> = [
            FailureScenario::Pristine.kind(),
            FailureScenario::Indeterminate { defect_regions: 0 }.kind(),
            FailureScenario::NotProven { unverified_sectors: 0, defect_regions: 0 }.kind(),
            FailureScenario::ExhaustedSpare { defect_regions: 0 }.kind(),
        ]
        .into_iter()
        .collect();
        assert_eq!(kinds.len(), 4, "identifiers must be distinct");
    }
}
