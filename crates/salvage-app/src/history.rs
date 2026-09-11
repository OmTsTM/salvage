//! What is remembered about a card between sessions.
//!
//! # Why remember anything
//!
//! A full inspection of a 128 GB card takes hours. Having paid that once, a user
//! should not pay it again to try a different layout on the same findings, and
//! should not be told nothing is known about a card this program has already
//! measured.
//!
//! # What a record is, and what it is not
//!
//! A record is **a map of where to look**. It is never a certificate.
//!
//! The distinction is the whole design. Flash degrades, and the flash
//! translation layer remaps addresses on every write, so a map from three months
//! ago is a claim about hardware that has moved on. Letting it approve area
//! would reintroduce exactly the failure the rest of this program refuses:
//! certifying without proof.
//!
//! So a loaded record restores the picture and the planning, and nothing more.
//! Before any data partition is written from one, the area it covers is
//! verified again, now. That re-verification is not a weaker form of the
//! original scan — it is the same test, aimed only at the sectors that are
//! about to be used:
//!
//! - a defect that appeared outside the approved area changes nothing, because
//!   that area was already condemned;
//! - a defect that appeared inside it is precisely what the re-verification
//!   finds.
//!
//! # What else it carries
//!
//! The partition table that was on the card before this program wrote one. It
//! is 512 bytes, and keeping it is what makes fencing reversible: a user who
//! wants the whole card back — to re-inspect it differently, to try another
//! tool, or to be rid of it — gets the table they started with rather than a
//! guess at it.

use std::time::{SystemTime, UNIX_EPOCH};

use salvage_core::sector_map::SectorMap;
use serde::{Deserialize, Serialize};

/// Bytes of a master boot record.
pub const TABLE_BYTES: usize = 512;

/// What is known about one card, from one earlier session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardRecord {
    /// Identity of the card this describes: model, vendor, serial, geometry.
    ///
    /// Checked on load. Windows recycles disk indices, and a record applied to
    /// the wrong card would describe defects that are not there and miss the
    /// ones that are.
    pub fingerprint: String,
    /// When the inspection finished, in seconds since the Unix epoch.
    ///
    /// Shown beside the record rather than used to expire it. The age is
    /// information the user should weigh; it is not a threshold, because the
    /// re-verification covers the risk either way.
    pub scanned_at: u64,
    /// The measured state of every sector.
    pub map: SectorMap,
    /// The partition table found before this program first wrote one, if it was
    /// ever captured.
    pub table_before: Option<Vec<u8>>,
}

impl CardRecord {
    /// Builds a record for a map just measured.
    pub fn new(fingerprint: impl Into<String>, map: SectorMap) -> Self {
        Self { fingerprint: fingerprint.into(), scanned_at: now(), map, table_before: None }
    }

    /// Attaches the table that was on the card before this program wrote one.
    ///
    /// Kept only the first time. A second fencing would otherwise record the
    /// first one's table as "what was there before", and the way back would
    /// lead to a layout this program wrote rather than to the card's own.
    pub fn remember_table(&mut self, bytes: &[u8]) {
        if self.table_before.is_none() && bytes.len() >= TABLE_BYTES {
            self.table_before = Some(bytes[..TABLE_BYTES].to_vec());
        }
    }

    /// Seconds elapsed since the inspection, or `None` if the clock disagrees.
    ///
    /// A record from the future means the system clock moved, not that the card
    /// was scanned tomorrow. Reporting `None` lets the window say "unknown"
    /// instead of a negative age.
    pub fn age_seconds(&self) -> Option<u64> {
        now().checked_sub(self.scanned_at)
    }

    /// Whether this record describes the given card.
    pub fn matches(&self, fingerprint: &str) -> bool {
        self.fingerprint == fingerprint
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Reasons a record could not be stored or retrieved.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// The underlying storage refused the operation.
    #[error("card history is unavailable: {0}")]
    Unavailable(String),
    /// A stored record could not be read back as one.
    #[error("the stored record could not be read: {0}")]
    Corrupt(String),
    /// The record on file describes a different card.
    #[error("the stored record belongs to another card")]
    WrongCard,
}

/// Where records are kept.
///
/// A port, so the domain of "remember this card" stays free of any particular
/// storage, and so the rules above can be tested without a filesystem.
pub trait CardHistory {
    /// The record for a card, if one was kept and still matches it.
    fn load(&self, fingerprint: &str) -> Result<Option<CardRecord>, HistoryError>;

    /// Stores a record, replacing any earlier one for the same card.
    fn save(&self, record: &CardRecord) -> Result<(), HistoryError>;

    /// Discards what is known about a card.
    fn forget(&self, fingerprint: &str) -> Result<(), HistoryError>;
}

/// An in-memory history, for tests and for a session that must not persist.
#[derive(Debug, Default)]
pub struct MemoryHistory {
    records: std::sync::Mutex<Vec<CardRecord>>,
}

impl CardHistory for MemoryHistory {
    fn load(&self, fingerprint: &str) -> Result<Option<CardRecord>, HistoryError> {
        let guard =
            self.records.lock().map_err(|_| HistoryError::Unavailable("poisoned".into()))?;
        Ok(guard.iter().find(|r| r.matches(fingerprint)).cloned())
    }

    fn save(&self, record: &CardRecord) -> Result<(), HistoryError> {
        let mut guard =
            self.records.lock().map_err(|_| HistoryError::Unavailable("poisoned".into()))?;
        guard.retain(|r| !r.matches(&record.fingerprint));
        guard.push(record.clone());
        Ok(())
    }

    fn forget(&self, fingerprint: &str) -> Result<(), HistoryError> {
        let mut guard =
            self.records.lock().map_err(|_| HistoryError::Unavailable("poisoned".into()))?;
        guard.retain(|r| !r.matches(fingerprint));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use salvage_core::sector_map::SectorState;
    use salvage_core::{DeviceGeometry, LbaRange};

    const FP: &str = "SDXC|Generic|S1|4096|512";

    fn map() -> SectorMap {
        let mut m = SectorMap::new(DeviceGeometry::new(512, 4096).unwrap());
        m.mark(LbaRange::from_bounds(0, 4096), SectorState::Good);
        m.mark(LbaRange::from_bounds(1000, 1100), SectorState::Corrupt);
        m
    }

    #[test]
    fn a_record_round_trips_through_the_store() {
        let store = MemoryHistory::default();
        let record = CardRecord::new(FP, map());
        store.save(&record).unwrap();

        let back = store.load(FP).unwrap().expect("the record should be found");
        assert_eq!(back.map, record.map);
        assert_eq!(back.fingerprint, FP);
    }

    /// Windows recycles disk indices, so identity has to come from the card
    /// rather than from where it happens to be plugged in.
    #[test]
    fn a_record_is_not_returned_for_a_different_card() {
        let store = MemoryHistory::default();
        store.save(&CardRecord::new(FP, map())).unwrap();
        assert!(store.load("OTHER|Generic|S9|4096|512").unwrap().is_none());
    }

    #[test]
    fn saving_again_replaces_rather_than_accumulates() {
        let store = MemoryHistory::default();
        store.save(&CardRecord::new(FP, map())).unwrap();

        let mut second = CardRecord::new(FP, map());
        second.map.mark(LbaRange::from_bounds(2000, 2100), SectorState::BadRead);
        store.save(&second).unwrap();

        let back = store.load(FP).unwrap().unwrap();
        assert_eq!(back.map.state_at(2000), Some(SectorState::BadRead));
        assert!(store.load(FP).unwrap().is_some());
        store.forget(FP).unwrap();
        assert!(store.load(FP).unwrap().is_none());
    }

    /// The way back has to lead to the card's own table, not to one this
    /// program wrote. A second fencing must not overwrite the first capture.
    #[test]
    fn the_original_table_is_captured_once_and_never_replaced() {
        let mut record = CardRecord::new(FP, map());
        let original = vec![0xAA; TABLE_BYTES];
        let ours = vec![0xBB; TABLE_BYTES];

        record.remember_table(&original);
        record.remember_table(&ours);

        assert_eq!(record.table_before.as_deref(), Some(&original[..]));
    }

    #[test]
    fn a_short_buffer_is_not_mistaken_for_a_table() {
        let mut record = CardRecord::new(FP, map());
        record.remember_table(&[0xAA; 16]);
        assert!(record.table_before.is_none(), "16 bytes is not a partition table");
    }

    /// A clock that moved backwards must not produce a negative age, which
    /// would be shown to the user as a card scanned in the future.
    #[test]
    fn an_age_from_the_future_is_reported_as_unknown() {
        let mut record = CardRecord::new(FP, map());
        record.scanned_at = now() + 86_400;
        assert!(record.age_seconds().is_none());
    }

    #[test]
    fn a_record_just_made_is_essentially_new() {
        let record = CardRecord::new(FP, map());
        assert!(record.age_seconds().unwrap_or(u64::MAX) < 5);
    }
}
