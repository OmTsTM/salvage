//! Card records kept on disk.
//!
//! The filesystem half of [`salvage_app::history`]. One JSON file per card,
//! under the same directory as the diagnostic log — the user's own data
//! directory, chosen for the same reason: an elevated process writing to a path
//! named by an environment variable can be induced to write elsewhere.
//!
//! # Why the filename is a hash
//!
//! A fingerprint contains the card's serial number. Putting it in a filename
//! would scatter hardware identifiers across a directory anyone can list, and
//! would break on the punctuation a serial is allowed to contain. The hash is
//! not a security measure — the fingerprint is inside the file — it is a way to
//! get a short, valid, stable name.
//!
//! # Why a temporary file
//!
//! A record is written beside its destination and renamed over it. A card map
//! is the result of hours of work; a crash midway through writing it should
//! leave the previous record intact rather than a truncated file that parses as
//! nothing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use salvage_app::history::{CardHistory, CardRecord, HistoryError};

/// Records live beside the diagnostic log, in a directory of their own.
const DIRECTORY: &str = "cards";

/// A history stored as one file per card.
#[derive(Debug, Clone)]
pub struct FileHistory {
    root: PathBuf,
}

impl FileHistory {
    /// A history rooted at the given directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The default location: `%LOCALAPPDATA%\Salvage\cards`.
    pub fn in_user_data() -> Self {
        let base =
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("Salvage").join(DIRECTORY))
    }

    fn path_for(&self, fingerprint: &str) -> PathBuf {
        self.root.join(format!("{}.json", digest(fingerprint)))
    }
}

/// A short, stable, filename-safe digest of a fingerprint.
///
/// FNV-1a, chosen because it is four lines rather than a dependency, and
/// nothing here rests on it being hard to reverse: the fingerprint is stored in
/// the file, and a collision is caught by the fingerprint check on load.
fn digest(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn unavailable(e: io::Error) -> HistoryError {
    HistoryError::Unavailable(e.to_string())
}

impl CardHistory for FileHistory {
    fn load(&self, fingerprint: &str) -> Result<Option<CardRecord>, HistoryError> {
        let path = self.path_for(fingerprint);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            // Nothing remembered is a normal answer, not a failure.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(unavailable(e)),
        };

        let record: CardRecord =
            serde_json::from_str(&text).map_err(|e| HistoryError::Corrupt(e.to_string()))?;

        // The digest collided, or the file was edited. Either way this record
        // describes a different card, and using it would describe defects that
        // are not there and miss the ones that are.
        if !record.matches(fingerprint) {
            return Err(HistoryError::WrongCard);
        }
        Ok(Some(record))
    }

    fn save(&self, record: &CardRecord) -> Result<(), HistoryError> {
        fs::create_dir_all(&self.root).map_err(unavailable)?;
        let path = self.path_for(&record.fingerprint);
        let text =
            serde_json::to_string(record).map_err(|e| HistoryError::Corrupt(e.to_string()))?;

        // Written beside the destination and renamed over it, so a crash leaves
        // the previous record rather than half of this one.
        let temporary = path.with_extension("json.partial");
        fs::write(&temporary, text).map_err(unavailable)?;
        fs::rename(&temporary, &path).map_err(unavailable)
    }

    fn forget(&self, fingerprint: &str) -> Result<(), HistoryError> {
        match fs::remove_file(self.path_for(fingerprint)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(unavailable(e)),
        }
    }
}

impl AsRef<Path> for FileHistory {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use salvage_app::history::TABLE_BYTES;
    use salvage_core::pattern::PatternKind;
    use salvage_core::sector_map::{SectorMap, SectorState};
    use salvage_core::{DeviceGeometry, LbaRange};

    const FP: &str = "SDXC|Generic|S1|4096|512";

    fn temp_root(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("salvage-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    fn map() -> SectorMap {
        let mut m = SectorMap::new(DeviceGeometry::new(512, 4096).unwrap());
        m.mark(LbaRange::from_bounds(0, 4096), SectorState::Good);
        m.mark(LbaRange::from_bounds(500, 700), SectorState::Corrupt);
        m
    }

    #[test]
    fn a_record_survives_a_round_trip_through_the_filesystem() {
        let store = FileHistory::new(temp_root("roundtrip"));
        let mut record = CardRecord::new(FP, map());
        record.remember_table(&vec![0xAA; TABLE_BYTES]);
        store.save(&record).unwrap();

        let back = store.load(FP).unwrap().expect("the record should be on disk");
        assert_eq!(back.map, record.map);
        assert_eq!(back.table_before, record.table_before);
        assert_eq!(back.scanned_at, record.scanned_at);
        let _ = fs::remove_dir_all(store.root);
    }

    /// Records written before the pattern was stored must keep loading. They
    /// simply cannot be re-checked, which is the honest outcome: there is
    /// nothing left to compare their card against.
    #[test]
    fn a_record_from_before_the_pattern_was_stored_still_loads() {
        let root = temp_root("older");
        let store = FileHistory::new(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            store.path_for(FP),
            r#"{"fingerprint":"SDXC|Generic|S1|4096|512","scanned_at":1,
               "map":{"geometry":{"sector_size":512,"total_sectors":4096},
               "runs":[{"range":{"start":0,"end":4096},"state":"good"}],"aliases":[]},
               "table_before":null}"#,
        )
        .unwrap();

        let record = store.load(FP).unwrap().expect("an older record must still load");
        assert!(record.pattern.is_none(), "and it offers nothing to compare against");
        let _ = fs::remove_dir_all(&root);
    }

    /// Without the seed there is no expected content for a sector, so a record
    /// that lost it could only be re-checked against a guess.
    #[test]
    fn the_pattern_survives_the_round_trip_so_the_area_can_be_read_back() {
        let root = temp_root("pattern");
        let store = FileHistory::new(&root);
        store
            .save(&CardRecord::new(FP, map()).wrote(0xDEAD_BEEF, PatternKind::Pseudorandom))
            .unwrap();

        let pattern = store.load(FP).unwrap().unwrap().pattern.expect("the seed should survive");
        assert_eq!(pattern.nonce, 0xDEAD_BEEF);
        assert_eq!(pattern.kind, PatternKind::Pseudorandom);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_card_is_not_an_error() {
        let store = FileHistory::new(temp_root("unknown"));
        assert!(store.load(FP).unwrap().is_none());
    }

    /// The check that stops one card's map being applied to another, whether
    /// through a digest collision or an edited file.
    #[test]
    fn a_record_naming_another_card_is_refused() {
        let root = temp_root("wrong");
        let store = FileHistory::new(&root);
        store.save(&CardRecord::new(FP, map())).unwrap();

        // Rewrite the file under the digest of a different fingerprint.
        let other = "OTHER|Generic|S9|4096|512";
        let text = fs::read_to_string(store.path_for(FP)).unwrap();
        fs::write(store.path_for(other), text).unwrap();

        assert!(matches!(store.load(other), Err(HistoryError::WrongCard)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_file_is_reported_rather_than_ignored() {
        let root = temp_root("corrupt");
        let store = FileHistory::new(&root);
        store.save(&CardRecord::new(FP, map())).unwrap();
        fs::write(store.path_for(FP), "{ not json").unwrap();

        assert!(matches!(store.load(FP), Err(HistoryError::Corrupt(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn forgetting_an_unknown_card_succeeds_quietly() {
        let store = FileHistory::new(temp_root("forget"));
        store.forget(FP).unwrap();
    }

    /// Nothing rests on the digest being irreversible, but everything rests on
    /// it being stable: a different name each run would lose every record.
    #[test]
    fn the_digest_is_stable_and_filename_safe() {
        let a = digest(FP);
        assert_eq!(a, digest(FP));
        assert_ne!(a, digest("OTHER|Generic|S9|4096|512"));
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// A serial can contain punctuation a filename cannot, and should not be
    /// scattered across a directory anyone can list.
    #[test]
    fn a_serial_never_reaches_the_filename() {
        let store = FileHistory::new(temp_root("serial"));
        let awkward = r"Card|Vendor|A/B:C\D*E|4096|512";
        let name = store.path_for(awkward).file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains("A/B"), "the serial leaked into {name}");
        assert!(name.ends_with(".json"));
    }
}
