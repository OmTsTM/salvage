//! Human-readable formatting for the quantities this tool reports.
//!
//! Byte counts are formatted in decimal units (kB, MB, GB) rather than binary
//! ones (KiB, MiB, GiB). That is a deliberate choice: card manufacturers print
//! decimal capacities on the packaging, and a user comparing our output against
//! the label on a "32 GB" card should see numbers that line up with it. Binary
//! units would report the same card as 29.8 GiB and invite the conclusion that
//! the tool found missing capacity when it did not.
//!
//! Sector counts and LBA arithmetic stay in binary throughout the codebase;
//! only the presentation layer converts.

/// Formats a byte count using decimal SI units.
///
/// Values below one kilobyte are printed as an exact integer, because rounding
/// a sector size to "0.51 kB" hides the very precision that matters at that
/// scale.
///
/// ```
/// use salvage_core::format::bytes;
/// assert_eq!(bytes(512), "512 B");
/// assert_eq!(bytes(1_000), "1.00 kB");
/// assert_eq!(bytes(33_560_000_000), "33.56 GB");
/// ```
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = count as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{count} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Formats a duration in seconds as a coarse, human-scale estimate.
///
/// Precision is deliberately dropped as the value grows: an inspection with two
/// hours left does not benefit from knowing the seconds, and showing them
/// suggests an accuracy the estimate does not have.
///
/// ```
/// use salvage_core::format::duration;
/// assert_eq!(duration(45.0), "45 s");
/// assert_eq!(duration(600.0), "10 min");
/// assert_eq!(duration(7_500.0), "2 h 05 min");
/// ```
pub fn duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "—".to_string();
    }
    if seconds < 90.0 {
        return format!("{} s", seconds.round() as u64);
    }
    let minutes = (seconds / 60.0).round() as u64;
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h {:02} min", minutes / 60, minutes % 60)
    }
}

/// Formats a fraction in the range `0.0..=1.0` as a percentage.
///
/// ```
/// use salvage_core::format::percent;
/// assert_eq!(percent(0.529), "52.9%");
/// ```
pub fn percent(fraction: f64) -> String {
    format!("{:.1}%", (fraction * 100.0).clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_values_stay_exact() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(999), "999 B");
    }

    #[test]
    fn units_step_at_decimal_boundaries() {
        assert_eq!(bytes(1_000), "1.00 kB");
        assert_eq!(bytes(1_000_000), "1.00 MB");
        assert_eq!(bytes(64_000_000_000), "64.00 GB");
    }

    /// A 32 GB card reports roughly 33.5 decimal gigabytes. Reporting binary
    /// units here would show 31.25 and look like missing capacity.
    #[test]
    fn a_nominal_32gb_card_reads_close_to_its_label() {
        let sectors: u64 = 65_538_048;
        let text = bytes(sectors * 512);
        assert!(text.starts_with("33."), "expected ~33 GB, got {text}");
    }

    #[test]
    fn the_largest_unit_saturates_rather_than_overflowing() {
        assert!(bytes(u64::MAX).ends_with("TB"));
    }

    #[test]
    fn durations_lose_precision_as_they_grow() {
        assert_eq!(duration(5.0), "5 s");
        assert_eq!(duration(89.0), "89 s");
        assert_eq!(duration(90.0), "2 min");
        assert_eq!(duration(3_600.0), "1 h 00 min");
        assert_eq!(duration(7_500.0), "2 h 05 min");
    }

    #[test]
    fn non_finite_durations_do_not_panic() {
        assert_eq!(duration(f64::INFINITY), "—");
        assert_eq!(duration(f64::NAN), "—");
        assert_eq!(duration(-1.0), "—");
    }

    #[test]
    fn percentages_are_clamped_to_a_sane_range() {
        assert_eq!(percent(0.0), "0.0%");
        assert_eq!(percent(1.0), "100.0%");
        assert_eq!(percent(1.5), "100.0%");
        assert_eq!(percent(-0.2), "0.0%");
    }
}
