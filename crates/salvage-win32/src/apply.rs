//! Applying a partition plan to a real device.
//!
//! This is the only code path in the program that changes a card permanently,
//! so it revalidates everything before writing a byte: that the consent still
//! covers this device, that the plan is still safe against the sector map that
//! produced it, and that the new table fits the format. Any disagreement aborts
//! the operation.

use std::process::Command;
use std::time::Duration;

use salvage_app::device::{BlockDevice, DeviceInfo};
use salvage_app::safety::DestructiveConsent;
use salvage_core::fat32::Fat32Image;
use salvage_core::geometry::LbaRange;
use salvage_core::mbr::{describe_change, MasterBootRecord, MbrError, TableChange, MBR_SIZE};
use salvage_core::planner::{Containment, FileSystem, PartitionPlan, PlanError};
use salvage_core::sector_map::SectorMap;

use crate::raw_device::RawBlockDevice;
use crate::sys::{self, Access};

/// Bytes zeroed at the start of each data partition.
///
/// Erases any lingering superblock so Windows does not recognise a stale
/// filesystem over the new layout.
/// Largest FAT32 volume the Windows formatter will create.
///
/// `format.com` refuses FAT32 above this, and says so only after being asked.
/// Past it the filesystem is written here instead, with the same code the
/// spliced strategy uses — which has no such limit, because it is not
/// Microsoft's formatter.
const FORMAT_COM_FAT32_LIMIT: u64 = 32 * 1024 * 1024 * 1024;

/// Bytes written per call while laying down a spliced volume. The table for a
/// 128 GB card runs to tens of megabytes, and one sector at a time would spend
/// the whole operation in call overhead.
const SPLICED_WRITE_BYTES: usize = 1024 * 1024;

const WIPE_HEAD_BYTES: u64 = 1024 * 1024;

/// Attempts to find the freshly created volume before giving up.
const MOUNT_ATTEMPTS: u32 = 20;

/// Delay between mount attempts.
const MOUNT_RETRY_DELAY: Duration = Duration::from_millis(500);

/// A step performed while applying a plan.
///
/// Structured rather than pre-formatted: the presentation layer chooses the
/// wording, and an automated report can consume the same data without parsing
/// prose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum ApplyStep {
    /// Existing volumes were locked and unmounted.
    VolumesDismounted,
    /// A volume could not be locked or unmounted; the text names the cause.
    VolumeWarning(String),
    /// The partition table changed in the described way.
    TableChanged(TableChange),
    /// The head of a data partition was zeroed.
    PartitionHeadWiped {
        /// Label of the affected partition.
        label: String,
    },
    /// The new partition table was written.
    TableWritten,
    /// The system was notified to re-read the layout.
    SystemNotified,
    /// The data partition was mounted at the given drive letter.
    VolumeMounted {
        /// Drive letter assigned by Windows.
        letter: char,
    },
    /// The volume was written directly, with its defective clusters withheld
    /// in its own allocation table.
    ClusterMapWritten {
        /// Clusters marked defective and never handed out.
        withheld_clusters: u32,
        /// Bytes the volume will actually offer.
        usable_bytes: u64,
    },
    /// A table was stored for this card, and it is not a partition table.
    ///
    /// Sector 0 was read on the way past an earlier operation, and what was in
    /// it then was the inspection's own pattern. Releasing the card still
    /// works — it gets a full-capacity partition, the same as a card with
    /// nothing stored — but the way back does not lead where it promised.
    StoredTableUnusable,
    /// The partition table the card arrived with was put back.
    TableRestored {
        /// Whether it was the card's own table, or a full-capacity one made
        /// because none had been kept.
        from_card: bool,
    },
    /// The data partition was formatted.
    Formatted {
        /// Drive letter formatted.
        letter: char,
        /// Filesystem written.
        filesystem: FileSystem,
    },
    /// The partition exists but Windows did not mount it in time.
    MountTimedOut,
}

/// Failures while applying a plan.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    /// The consent does not match the device present.
    #[error("the device changed since authorisation; operation aborted")]
    ConsentMismatch,
    /// The plan is no longer safe for the given map.
    #[error("the plan failed revalidation: {0}")]
    UnsafePlan(#[from] PlanError),
    /// The partition table could not be assembled.
    #[error(transparent)]
    Mbr(#[from] MbrError),
    /// Device access failed.
    #[error("device access failed: {0}")]
    Device(String),
    /// Formatting failed.
    #[error("failed to format {letter}: {detail}")]
    FormatFailed {
        /// Drive letter.
        letter: char,
        /// Formatter output.
        detail: String,
    },
}

/// Result of applying a plan.
#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    /// Ordered record of what was done.
    pub steps: Vec<ApplyStep>,
    /// Drive letter assigned to the data partition, when Windows mounted it.
    pub data_volume_letter: Option<char>,
    /// The table the card carried before this operation overwrote it.
    ///
    /// Returned so the caller can keep it: it is what makes the fencing
    /// reversible, and the only moment it can be read is just before it stops
    /// existing.
    pub table_before: Option<Vec<u8>>,
}

/// Derives a disk signature from the device's identity.
///
/// Must be stable for the same card and different between cards, so Windows
/// does not confuse two devices sharing a signature.
fn disk_signature_for(device: &DeviceInfo) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in device.fingerprint().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    // Zero is reserved by convention: Windows treats it as "no signature".
    ((h ^ (h >> 32)) as u32).max(1)
}

/// Finds the drive letter assigned to this disk's data partition.
fn find_data_volume_letter(disk_index: u32) -> Option<char> {
    let guids = sys::enumerate_volume_guids().ok()?;
    for guid in guids {
        let path = guid.trim_end_matches('\\');
        let Ok(handle) = sys::open_device(path, Access::Read) else {
            continue;
        };
        let Ok(disks) = sys::volume_disk_numbers(&handle) else {
            continue;
        };
        if !disks.contains(&disk_index) {
            continue;
        }
        let mounts = sys::volume_mount_points(&guid).unwrap_or_default();
        if let Some(letter) =
            mounts.iter().filter_map(|m| m.chars().next()).find(|c| c.is_ascii_alphabetic())
        {
            return Some(letter.to_ascii_uppercase());
        }
    }
    None
}

/// Formats an already-mounted volume.
///
/// Formatting is delegated to the Windows formatter rather than writing the
/// filesystem directly. Building a valid exFAT by hand is possible, but it
/// would be new, untested code at exactly the point where a mistake costs the
/// user's data; the system formatter is the same one Explorer uses.
///
/// The executable is invoked by absolute path from the system directory, and
/// without going through a command interpreter. Two reasons. First, privilege:
/// this process runs elevated, and resolving a bare name through `PATH` would
/// let a planted executable run as Administrator. Second, surface area: a shell
/// reinterprets its command line, and none of the quoting below would be
/// necessary if the arguments never reached one.
fn format_volume(
    letter: char,
    filesystem: FileSystem,
    label: &str,
) -> Result<ApplyStep, ApplyError> {
    // The letter comes from Windows' own enumeration, but is validated anyway
    // before entering a command line.
    if !letter.is_ascii_alphabetic() {
        return Err(ApplyError::FormatFailed { letter, detail: "invalid drive letter".into() });
    }
    // The same mapping the FAT32 writer applies, and for the same reason: a
    // volume name has to survive to the card unchanged by which code path
    // happened to write it. Characters outside the safe set become an
    // underscore rather than vanishing — dropping the space in "CARTAO OK"
    // yields "CARTAOOK", which is a different word.
    let safe_label: String = label.chars().take(11).map(sanitise_label_char).collect();

    let format_exe = sys::system_executable("format.com")
        .map_err(|e| ApplyError::FormatFailed { letter, detail: e.to_string() })?;

    let mut child = Command::new(&format_exe)
        .args([
            format!("{letter}:"),
            format!("/FS:{}", filesystem.format_name()),
            "/Q".to_string(),
            "/Y".to_string(),
            format!("/V:{safe_label}"),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ApplyError::FormatFailed { letter, detail: e.to_string() })?;

    // Some Windows editions still prompt for confirmation even with /Y.
    // Answering and closing the pipe prevents an indefinite wait.
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(b"Y\r\n");
    }

    let output = child
        .wait_with_output()
        .map_err(|e| ApplyError::FormatFailed { letter, detail: e.to_string() })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(ApplyError::FormatFailed { letter, detail: detail.trim().to_string() });
    }
    Ok(ApplyStep::Formatted { letter, filesystem })
}

/// Applies the plan to the device.
///
/// Step order matters: the table is only written after the old volumes are
/// locked and unmounted, and formatting only happens after Windows has
/// acknowledged the new layout.
pub fn apply_plan(
    device: &DeviceInfo,
    plan: &PartitionPlan,
    map: &SectorMap,
    consent: &DestructiveConsent,
    filesystem: FileSystem,
    label: &str,
) -> Result<ApplyOutcome, ApplyError> {
    // 1. The consent covers this device, here and now.
    if !consent.matches(device) {
        return Err(ApplyError::ConsentMismatch);
    }

    // 2. The plan is still safe against the map that produced it.
    plan.validate(map, 4)?;

    let mut steps = Vec::new();
    let signature = disk_signature_for(device);
    let new_mbr = MasterBootRecord::from_plan(plan, signature)?;

    let mut dev = RawBlockDevice::open(&device.path, Access::ReadWrite)
        .map_err(|e| ApplyError::Device(e.to_string()))?;

    // 3. Get the old volumes out of the way.
    let problems = dev.lock_volumes_of_disk(device.index);
    if problems.is_empty() {
        steps.push(ApplyStep::VolumesDismounted);
    } else {
        steps.extend(problems.into_iter().map(ApplyStep::VolumeWarning));
    }

    // 4. Record the previous table so the user can see what changed.
    //
    // The read must span a whole sector: unbuffered access rejects transfers
    // that are not a sector multiple, and media with 4096-byte sectors would
    // reject a 512-byte request.
    let sector_size = device.geometry.sector_size() as usize;
    let mut first_sector = vec![0u8; sector_size];
    let previous = dev
        .read_at(0, &mut first_sector)
        .ok()
        .and_then(|()| MasterBootRecord::from_bytes(&first_sector[..MBR_SIZE]).ok());
    steps.extend(
        describe_change(previous.as_ref(), &new_mbr).into_iter().map(ApplyStep::TableChanged),
    );
    // Handed back so the caller can remember it. Without this, fencing is a
    // one-way door: the card can be given a layout but never returned to the
    // one it arrived with.
    let table_before = first_sector[..MBR_SIZE].to_vec();

    // 5. Wipe the head of each data partition so no stale filesystem is
    //    recognised over the new layout.
    let sector_size = sector_size as u64;
    let wipe_sectors = (WIPE_HEAD_BYTES / sector_size).max(1);
    let zeros = vec![0u8; (wipe_sectors * sector_size) as usize];
    for p in plan.data_partitions() {
        let count = wipe_sectors.min(p.range.len());
        let bytes = (count * sector_size) as usize;
        if dev.write_at(p.range.start(), &zeros[..bytes]).is_ok() {
            steps.push(ApplyStep::PartitionHeadWiped { label: p.label.clone() });
        }
    }

    // 6. Write the spliced volume, if that is the mechanism this plan uses.
    //
    // It goes down before the partition table so that nothing is ever published
    // pointing at an area that does not yet hold a filesystem: if this fails,
    // the old table is still in place and the disk is no worse off than after
    // the head wipe.
    if plan.containment == Containment::FilesystemClusterMap {
        for p in plan.data_partitions() {
            steps.push(write_spliced_volume(&mut dev, map, &p.range, signature, label)?);
        }
    }

    // 7. Write the partition table.
    //
    // The MBR occupies 512 bytes, but the write must cover a whole sector. The
    // remainder is zeroed: nothing valid lives there, and leaving residue would
    // risk some tool interpreting it.
    let mut boot_sector = vec![0u8; sector_size as usize];
    boot_sector[..MBR_SIZE].copy_from_slice(&new_mbr.to_bytes());
    dev.write_at(0, &boot_sector)
        .map_err(|e| ApplyError::Device(format!("failed to write the partition table: {e}")))?;
    dev.flush().map_err(|e| ApplyError::Device(e.to_string()))?;
    steps.push(ApplyStep::TableWritten);

    // 8. Have Windows re-read the disk.
    dev.refresh_partition_table().map_err(|e| ApplyError::Device(e.to_string()))?;
    steps.push(ApplyStep::SystemNotified);
    drop(dev);

    // 9. Wait for the mount. A spliced volume is already formatted; a fenced
    //    one is handed to the system formatter.
    let mut letter = None;
    for _ in 0..MOUNT_ATTEMPTS {
        if let Some(l) = find_data_volume_letter(device.index) {
            letter = Some(l);
            break;
        }
        std::thread::sleep(MOUNT_RETRY_DELAY);
    }

    let Some(letter) = letter else {
        steps.push(ApplyStep::MountTimedOut);
        return Ok(ApplyOutcome {
            steps,
            data_volume_letter: None,
            table_before: Some(table_before),
        });
    };

    steps.push(ApplyStep::VolumeMounted { letter });
    if plan.containment == Containment::PartitionBoundary {
        // Formatting a spliced volume here would replace the very table that
        // withholds its defective clusters with one that knows nothing about
        // them, and hand every dead cluster back to the user.
        steps.push(format_volume(letter, filesystem, label)?);
    }

    Ok(ApplyOutcome { steps, data_volume_letter: Some(letter), table_before: Some(table_before) })
}

/// Writes one full-capacity partition and formats it.
///
/// For a card the inspection found intact. Until this existed the healthy path
/// was the only one with no way out: a card with defects got a layout and a
/// formatted volume, while a card with none was left erased, unpartitioned, and
/// unmounted — the better result leading to the worse state.
///
/// It is offered only where nothing was condemned. On a card with defects, one
/// partition over everything is [`release_card`], which says what it is giving
/// back and why that is a decision rather than a convenience.
pub fn prepare_card(
    device: &DeviceInfo,
    map: &SectorMap,
    consent: &DestructiveConsent,
    filesystem: FileSystem,
    label: &str,
) -> Result<ApplyOutcome, ApplyError> {
    if !consent.matches(device) {
        return Err(ApplyError::ConsentMismatch);
    }

    let mut steps = Vec::new();
    let sector_size = device.geometry.sector_size();
    let signature = disk_signature_for(device);
    let new_mbr = MasterBootRecord::single_partition(&device.geometry, filesystem, signature)?;

    let area = new_mbr
        .used_partitions()
        .next()
        .map(|(_, p)| p.range())
        .ok_or(MbrError::EmptyPartition(0))?;

    let mut dev = RawBlockDevice::open(&device.path, Access::ReadWrite)
        .map_err(|e| ApplyError::Device(e.to_string()))?;

    let problems = dev.lock_volumes_of_disk(device.index);
    if problems.is_empty() {
        steps.push(ApplyStep::VolumesDismounted);
    } else {
        steps.extend(problems.into_iter().map(ApplyStep::VolumeWarning));
    }

    // Deliberately nothing. Fencing keeps the table it overwrites, because that
    // table is the way back to the card's own layout — but there is no such
    // table here. The inspection wrote its pattern over every sector including
    // sector 0, so what sits there now is pattern, not a partition table.
    //
    // Keeping it would do worse than nothing: a record holds the first table it
    // is given and never replaces it, so 512 bytes of pattern would take the
    // slot that a later fencing needs, and releasing the card afterwards would
    // restore garbage instead of the volume this function just created.
    let table_before = None;

    // Windows will not make a FAT32 volume this large, so it is made here.
    let own_filesystem = writes_own_filesystem(filesystem, &area, sector_size);
    if own_filesystem {
        steps.push(write_spliced_volume(&mut dev, map, &area, signature, label)?);
    }

    let mut boot_sector = vec![0u8; sector_size as usize];
    boot_sector[..MBR_SIZE].copy_from_slice(&new_mbr.to_bytes());
    dev.write_at(0, &boot_sector)
        .map_err(|e| ApplyError::Device(format!("failed to write the partition table: {e}")))?;
    dev.flush().map_err(|e| ApplyError::Device(e.to_string()))?;
    steps.push(ApplyStep::TableWritten);

    dev.refresh_partition_table().map_err(|e| ApplyError::Device(e.to_string()))?;
    steps.push(ApplyStep::SystemNotified);
    drop(dev);

    let mut letter = None;
    for _ in 0..MOUNT_ATTEMPTS {
        if let Some(l) = find_data_volume_letter(device.index) {
            letter = Some(l);
            break;
        }
        std::thread::sleep(MOUNT_RETRY_DELAY);
    }

    match letter {
        Some(l) => {
            steps.push(ApplyStep::VolumeMounted { letter: l });
            // Formatting a volume this program already wrote would replace it.
            if !own_filesystem {
                steps.push(format_volume(l, filesystem, label)?);
            }
        }
        None => steps.push(ApplyStep::MountTimedOut),
    }

    Ok(ApplyOutcome { steps, data_volume_letter: letter, table_before })
}

/// Whether what is about to be written came off the card rather than from here.
///
/// Compared rather than assumed: a stored table that would not parse is
/// replaced by an invented one, and reporting that as the card's own would be
/// the wrong half of the only sentence the user gets about it.
fn from_card_used(restored: &MasterBootRecord, table_before: Option<&[u8]>) -> bool {
    match table_before {
        Some(bytes) if bytes.len() >= MBR_SIZE => {
            restored.to_bytes()[..MBR_SIZE] == bytes[..MBR_SIZE]
        }
        _ => false,
    }
}

/// Removes this program's layout and gives the card back.
///
/// # What this does and does not undo
///
/// It restores a partition table. It does not restore data: the inspection
/// wrote a pattern over every sector long before any layout existed, and the
/// card's original contents went with it.
///
/// More importantly, it removes the protection rather than the damage. A card
/// fenced because 99.9% of it is dead comes back as one full-capacity volume
/// that will accept files and lose them. That is a legitimate thing to want —
/// to re-inspect the whole card differently, to try another tool, to be rid of
/// it — but it is what the caller is choosing, and the wording around this
/// function should say so.
///
/// `table_before` is the table the card carried when this program first wrote
/// one, if it was kept. Without it the card is given a single partition
/// spanning its full capacity, which is what almost every card ships with.
pub fn release_card(
    device: &DeviceInfo,
    consent: &DestructiveConsent,
    table_before: Option<&[u8]>,
    filesystem: FileSystem,
    label: &str,
) -> Result<ApplyOutcome, ApplyError> {
    if !consent.matches(device) {
        return Err(ApplyError::ConsentMismatch);
    }

    let mut steps = Vec::new();
    let sector_size = device.geometry.sector_size() as usize;

    // Either the table the card arrived with, or a single partition over
    // everything it reports.
    //
    // A stored table that will not parse is treated as no table at all rather
    // than as a failure. Records written before the capture was checked can
    // hold 512 bytes of inspection pattern instead of a partition table, and
    // refusing to release the card over that would strand it: the way out
    // would be barred by the one thing meant to make the way out possible.
    let from_card = table_before
        .filter(|bytes| bytes.len() >= MBR_SIZE)
        .and_then(|bytes| MasterBootRecord::from_bytes(&bytes[..MBR_SIZE]).ok());

    // Kept apart from "nothing was stored", because they are different facts
    // and the second would be a lie about the first.
    if from_card.is_none() && table_before.is_some() {
        steps.push(ApplyStep::StoredTableUnusable);
    }

    let restored = match from_card {
        Some(mbr) => mbr,
        None => MasterBootRecord::single_partition(
            &device.geometry,
            filesystem,
            disk_signature_for(device),
        )?,
    };
    steps.push(ApplyStep::TableRestored { from_card: from_card_used(&restored, table_before) });

    let mut dev = RawBlockDevice::open(&device.path, Access::ReadWrite)
        .map_err(|e| ApplyError::Device(e.to_string()))?;

    let problems = dev.lock_volumes_of_disk(device.index);
    if problems.is_empty() {
        steps.push(ApplyStep::VolumesDismounted);
    } else {
        steps.extend(problems.into_iter().map(ApplyStep::VolumeWarning));
    }

    let mut boot_sector = vec![0u8; sector_size];
    boot_sector[..MBR_SIZE].copy_from_slice(&restored.to_bytes());
    dev.write_at(0, &boot_sector)
        .map_err(|e| ApplyError::Device(format!("failed to write the partition table: {e}")))?;
    dev.flush().map_err(|e| ApplyError::Device(e.to_string()))?;
    steps.push(ApplyStep::TableWritten);

    dev.refresh_partition_table().map_err(|e| ApplyError::Device(e.to_string()))?;
    steps.push(ApplyStep::SystemNotified);
    drop(dev);

    // The restored volume is formatted only when the table was ours to invent.
    // Reinstating the card's own table and then formatting over it would
    // destroy whatever that table described.
    let mut letter = None;
    if table_before.is_none() {
        for _ in 0..MOUNT_ATTEMPTS {
            if let Some(l) = find_data_volume_letter(device.index) {
                letter = Some(l);
                break;
            }
            std::thread::sleep(MOUNT_RETRY_DELAY);
        }
        match letter {
            Some(l) => {
                steps.push(ApplyStep::VolumeMounted { letter: l });
                steps.push(format_volume(l, filesystem, label)?);
            }
            None => steps.push(ApplyStep::MountTimedOut),
        }
    }

    Ok(ApplyOutcome { steps, data_volume_letter: letter, table_before: None })
}

/// One character of a volume name, as both filesystems will accept it.
///
/// Shared with [`salvage_core::fat32`], which applies the same mapping when it
/// writes the label into the boot sector itself. Two rules would mean the same
/// typed name reaching the card differently depending on the card's size.
fn sanitise_label_char(c: char) -> char {
    match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' => c,
        _ => '_',
    }
}

/// Whether this program writes the filesystem itself rather than asking Windows.
///
/// Only where Windows would refuse. `format.com` is the better-travelled path
/// for everything it accepts, and the point of the exception is to stop a
/// legitimate choice from failing, not to replace a working tool.
fn writes_own_filesystem(filesystem: FileSystem, area: &LbaRange, sector_size: u32) -> bool {
    filesystem == FileSystem::Fat32
        && area.len().saturating_mul(sector_size as u64) > FORMAT_COM_FAT32_LIMIT
}

/// Writes a FAT32 volume whose allocation table withholds every cluster the
/// scan did not approve.
///
/// Only the metadata is written — reserved sectors, both copies of the table
/// and the root directory. The data region needs no initialization, because a
/// cluster nobody allocates is never read.
fn write_spliced_volume(
    dev: &mut RawBlockDevice,
    map: &SectorMap,
    area: &LbaRange,
    volume_id: u32,
    label: &str,
) -> Result<ApplyStep, ApplyError> {
    let image = Fat32Image::new(map, area, volume_id, label)
        .map_err(|e| ApplyError::Device(format!("could not lay out the volume: {e}")))?;

    let sector_size = image.layout().bytes_per_sector() as usize;
    let per_batch = (SPLICED_WRITE_BYTES / sector_size).max(1);
    let total = image.sectors_to_write();
    let mut buf = vec![0u8; per_batch * sector_size];

    let mut written = 0u64;
    while written < total {
        let count = per_batch.min((total - written) as usize);
        for i in 0..count {
            let at = i * sector_size;
            image.render_sector(written + i as u64, &mut buf[at..at + sector_size]);
        }
        dev.write_at(area.start() + written, &buf[..count * sector_size])
            .map_err(|e| ApplyError::Device(format!("failed to write the volume: {e}")))?;
        written += count as u64;
    }
    dev.flush().map_err(|e| ApplyError::Device(e.to_string()))?;

    Ok(ApplyStep::ClusterMapWritten {
        withheld_clusters: image.condemned_clusters(),
        usable_bytes: image.usable_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use salvage_app::device::BusType;
    use salvage_core::DeviceGeometry;

    fn device() -> DeviceInfo {
        DeviceInfo {
            path: "\\\\.\\PhysicalDrive9".into(),
            index: 9,
            model: "SDXC".into(),
            vendor: "Generic".into(),
            serial: Some("S1".into()),
            bus_type: BusType::Sd,
            removable_media: true,
            geometry: DeviceGeometry::new(512, 1_000_000).unwrap(),
            volumes: vec![],
        }
    }

    /// The two code paths that write a volume name must spell it the same way.
    ///
    /// `salvage_core::fat32::encode_label` applies this mapping when this
    /// program writes the filesystem itself; this side applies it when Windows
    /// does. They diverged once — one dropped the offending character and the
    /// other replaced it — so the same typed name produced "CARTAOOK" on a
    /// small card and "CARTAO_OK" on a large one.
    #[test]
    fn a_space_becomes_an_underscore_rather_than_disappearing() {
        let mapped: String = "CARTAO OK".chars().map(sanitise_label_char).collect();
        assert_eq!(mapped, "CARTAO_OK");
    }

    #[test]
    fn a_label_keeps_only_what_both_filesystems_accept() {
        let mapped: String = r"a/b:c\d*e".chars().map(sanitise_label_char).collect();
        assert_eq!(mapped, "a_b_c_d_e", "punctuation a filesystem refuses must not reach it");
        assert_eq!("Ok-9_".chars().map(sanitise_label_char).collect::<String>(), "Ok-9_");
    }

    /// The boundary this exists for. `format.com` refuses FAT32 above 32 GB,
    /// and a 64 GB card asked for FAT32 would otherwise fail at the last step,
    /// after the partition table had already been written.
    #[test]
    fn fat32_past_the_formatter_limit_is_written_here_instead() {
        let sectors_32g = FORMAT_COM_FAT32_LIMIT / 512;
        let at = LbaRange::from_bounds(0, sectors_32g);
        let past = LbaRange::from_bounds(0, sectors_32g + 1);

        assert!(!writes_own_filesystem(FileSystem::Fat32, &at, 512), "32 GB is accepted");
        assert!(writes_own_filesystem(FileSystem::Fat32, &past, 512), "past it is not");
    }

    /// Windows makes exFAT volumes of any size, so nothing is gained by
    /// replacing a working path with one of our own.
    #[test]
    fn exfat_is_always_left_to_windows() {
        let huge = LbaRange::from_bounds(0, u32::MAX as u64);
        assert!(!writes_own_filesystem(FileSystem::ExFat, &huge, 512));
    }

    /// The limit is a size in bytes, not a sector count: the same number of
    /// sectors is twice the volume on a 4 KiB card.
    #[test]
    fn the_limit_is_measured_in_bytes_rather_than_sectors() {
        let sectors = (FORMAT_COM_FAT32_LIMIT / 512) - 1;
        let area = LbaRange::from_bounds(0, sectors);
        assert!(!writes_own_filesystem(FileSystem::Fat32, &area, 512));
        assert!(
            writes_own_filesystem(FileSystem::Fat32, &area, 4096),
            "the same sectors at 4 KiB are eight times the bytes"
        );
    }

    #[test]
    fn disk_signature_is_stable_per_device_and_never_zero() {
        let d = device();
        assert_eq!(disk_signature_for(&d), disk_signature_for(&d));
        assert_ne!(disk_signature_for(&d), 0);

        let mut other = device();
        other.serial = Some("S2".into());
        assert_ne!(disk_signature_for(&d), disk_signature_for(&other));
    }

    /// Applying with consent issued for a different card must abort before any
    /// write takes place.
    #[test]
    fn applying_with_foreign_consent_aborts() {
        use salvage_app::safety::SafetyPolicy;
        use salvage_core::planner::{plan_layouts, PlanningPolicy};
        use salvage_core::sector_map::SectorState;
        use salvage_core::LbaRange;

        let d = device();
        let consent =
            DestructiveConsent::issue(&d, &d.display_name(), &SafetyPolicy::default()).unwrap();

        let mut map = SectorMap::new(d.geometry);
        map.mark(LbaRange::from_bounds(0, 1_000_000), SectorState::Good);
        let plans =
            plan_layouts(&map, FileSystem::ExFat, &PlanningPolicy::recommended_for(&d.geometry));

        let mut swapped = device();
        swapped.serial = Some("ANOTHER CARD".into());

        let err = apply_plan(&swapped, &plans[0], &map, &consent, FileSystem::ExFat, "SALVAGE")
            .unwrap_err();
        assert!(matches!(err, ApplyError::ConsentMismatch));
    }
}
