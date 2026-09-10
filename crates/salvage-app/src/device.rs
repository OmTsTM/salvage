//! Device descriptions and the ports used to reach them.
//!
//! The ports are deliberately minimal: read sectors, write sectors, flush.
//! Everything operating-system specific sits behind these traits, which is what
//! makes it possible to exercise the entire program against a simulated card
//! before touching real hardware.

use std::fmt;

use salvage_core::DeviceGeometry;
use serde::{Deserialize, Serialize};

/// Bus the device is attached to.
///
/// This is the strongest single signal for telling a memory card apart from the
/// disk the operating system is running on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusType {
    /// Native SD card reader.
    Sd,
    /// MMC controller.
    Mmc,
    /// USB attachment, including external card readers.
    Usb,
    /// NVMe: always an internal SSD.
    Nvme,
    /// SATA or ATA: internal disk.
    Sata,
    /// SCSI or SAS.
    Scsi,
    /// RAID controller.
    Raid,
    /// Unrecognised bus.
    Unknown,
}

impl BusType {
    /// Buses a memory card can appear on.
    ///
    /// USB is on the list because nearly every external microSD reader presents
    /// itself that way.
    #[inline]
    pub const fn can_carry_removable_card(&self) -> bool {
        matches!(self, Self::Sd | Self::Mmc | Self::Usb)
    }

    /// Buses that exist only for fixed internal storage.
    #[inline]
    pub const fn is_internal_only(&self) -> bool {
        matches!(self, Self::Nvme | Self::Sata | Self::Raid)
    }

    /// Stable identifier for display.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Sd => "SD",
            Self::Mmc => "MMC",
            Self::Usb => "USB",
            Self::Nvme => "NVMe",
            Self::Sata => "SATA",
            Self::Scsi => "SCSI",
            Self::Raid => "RAID",
            Self::Unknown => "unknown",
        }
    }
}

/// A mounted volume belonging to a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfo {
    /// Drive letter, when one is assigned.
    pub drive_letter: Option<char>,
    /// Volume label.
    pub label: Option<String>,
    /// Reported filesystem.
    pub filesystem: Option<String>,
    /// Whether the volume hosts the Windows directory.
    pub is_system_volume: bool,
    /// Whether the volume hosts the boot partition.
    pub is_boot_volume: bool,
}

/// Everything known about a device before opening it for writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Path used to open it, e.g. `\\.\PhysicalDrive3`.
    pub path: String,
    /// Physical index assigned by the system.
    pub index: u32,
    /// Reported model.
    pub model: String,
    /// Reported vendor.
    pub vendor: String,
    /// Serial number, when available.
    pub serial: Option<String>,
    /// Bus type.
    pub bus_type: BusType,
    /// Whether the media is declared removable.
    ///
    /// Not trustworthy on its own: many USB microSD adapters report `false`
    /// even while carrying a card.
    pub removable_media: bool,
    /// Reported geometry.
    pub geometry: DeviceGeometry,
    /// Mounted volumes belonging to this device.
    pub volumes: Vec<VolumeInfo>,
}

impl DeviceInfo {
    /// Readable name, used for display and for typed confirmation.
    pub fn display_name(&self) -> String {
        let m = self.model.trim();
        let v = self.vendor.trim();
        if v.is_empty() || m.starts_with(v) {
            m.to_string()
        } else {
            format!("{v} {m}")
        }
    }

    /// Whether any volume on this device hosts the system or the boot partition.
    pub fn hosts_system(&self) -> bool {
        self.volumes.iter().any(|v| v.is_system_volume || v.is_boot_volume)
    }

    /// Capacity in bytes, as reported.
    pub const fn capacity_bytes(&self) -> u64 {
        self.geometry.total_bytes()
    }

    /// Stable fingerprint, used to guarantee the device approved in a plan is
    /// the same one being written.
    ///
    /// The physical index alone will not do: it is recycled when media is
    /// swapped, so a plan approved for one card could end up applied to another
    /// that inherited the same index.
    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.model.trim(),
            self.vendor.trim(),
            self.serial.as_deref().unwrap_or("-").trim(),
            self.geometry.total_sectors(),
            self.geometry.sector_size()
        )
    }
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}, {}, {})",
            self.display_name(),
            self.path,
            salvage_core::format::bytes(self.capacity_bytes()),
            self.bus_type.as_str()
        )
    }
}

/// Errors reaching a block device.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// I/O failure reported by the system.
    #[error("I/O error at LBA {lba}: {source}")]
    Io {
        /// LBA where the operation failed.
        lba: u64,
        /// Underlying cause.
        #[source]
        source: std::io::Error,
    },
    /// Buffer size incompatible with the sector size.
    #[error("buffer of {len} bytes is not a multiple of the {sector_size} byte sector")]
    UnalignedBuffer {
        /// Buffer length.
        len: usize,
        /// Sector size.
        sector_size: u32,
    },
    /// Access past the end of the device.
    #[error("access to {lba}..{end} exceeds the device's {total} sectors")]
    OutOfBounds {
        /// First LBA requested.
        lba: u64,
        /// End of the access.
        end: u64,
        /// Device size.
        total: u64,
    },
    /// Exclusive access could not be obtained.
    #[error("could not lock the device: {0}")]
    LockFailed(String),
    /// Write attempted on a device opened read-only.
    #[error("device was opened read-only")]
    ReadOnly,
    /// Any other failure.
    #[error("{0}")]
    Other(String),
}

/// Raw, sector-level access to a block device.
///
/// Implementations must treat `read_at` and `write_at` as exact operations: a
/// short transfer is an error, not a success.
pub trait BlockDevice: Send {
    /// Device geometry.
    fn geometry(&self) -> DeviceGeometry;

    /// Reads consecutive sectors from `lba`, filling `buf` completely.
    fn read_at(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DeviceError>;

    /// Writes all of `buf` starting at `lba`.
    fn write_at(&mut self, lba: u64, buf: &[u8]) -> Result<(), DeviceError>;

    /// Ensures writes have reached the media.
    fn flush(&mut self) -> Result<(), DeviceError>;

    /// Whether the device accepts writes.
    fn is_writable(&self) -> bool;

    /// Validates an access against the geometry. Helper for implementers.
    fn check_access(&self, lba: u64, len: usize) -> Result<u64, DeviceError> {
        let g = self.geometry();
        let ss = g.sector_size();
        if len == 0 || len % ss as usize != 0 {
            return Err(DeviceError::UnalignedBuffer { len, sector_size: ss });
        }
        let sectors = (len / ss as usize) as u64;
        let end = lba.saturating_add(sectors);
        if end > g.total_sectors() {
            return Err(DeviceError::OutOfBounds { lba, end, total: g.total_sectors() });
        }
        Ok(sectors)
    }
}

/// Discovery of the devices present on the machine.
pub trait DeviceEnumerator {
    /// Lists every visible storage device.
    fn enumerate(&self) -> Result<Vec<DeviceInfo>, DeviceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DeviceInfo {
        DeviceInfo {
            path: "\\\\.\\PhysicalDrive3".into(),
            index: 3,
            model: "SDXC Card".into(),
            vendor: "Generic".into(),
            serial: Some("ABC123".into()),
            bus_type: BusType::Sd,
            removable_media: true,
            geometry: DeviceGeometry::new(512, 125_000_000).unwrap(),
            volumes: vec![],
        }
    }

    #[test]
    fn usb_counts_as_a_possible_card_carrier() {
        assert!(BusType::Usb.can_carry_removable_card());
        assert!(BusType::Sd.can_carry_removable_card());
        assert!(!BusType::Nvme.can_carry_removable_card());
    }

    #[test]
    fn internal_buses_are_recognised() {
        assert!(BusType::Nvme.is_internal_only());
        assert!(BusType::Sata.is_internal_only());
        assert!(!BusType::Usb.is_internal_only());
    }

    #[test]
    fn display_name_avoids_repeating_the_vendor() {
        let mut d = info();
        d.vendor = "Generic".into();
        d.model = "Generic SDXC".into();
        assert_eq!(d.display_name(), "Generic SDXC");

        d.model = "SDXC Card".into();
        assert_eq!(d.display_name(), "Generic SDXC Card");
    }

    #[test]
    fn fingerprint_changes_when_the_media_changes() {
        let a = info();
        let mut b = info();
        b.serial = Some("XYZ789".into());
        assert_ne!(a.fingerprint(), b.fingerprint());

        let mut c = info();
        c.geometry = DeviceGeometry::new(512, 62_000_000).unwrap();
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    /// The physical index is recycled when a card is swapped, so it must not be
    /// part of the device's identity.
    #[test]
    fn fingerprint_ignores_the_recyclable_physical_index() {
        let a = info();
        let mut b = info();
        b.index = 9;
        b.path = "\\\\.\\PhysicalDrive9".into();
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn hosting_a_system_or_boot_volume_is_detected() {
        let mut d = info();
        assert!(!d.hosts_system());
        d.volumes.push(VolumeInfo {
            drive_letter: Some('C'),
            label: None,
            filesystem: Some("NTFS".into()),
            is_system_volume: true,
            is_boot_volume: false,
        });
        assert!(d.hosts_system());
    }
}
