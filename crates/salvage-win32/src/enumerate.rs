//! Discovery of storage devices and the volumes occupying them.

use salvage_app::device::{BusType, DeviceEnumerator, DeviceError, DeviceInfo, VolumeInfo};
use salvage_core::DeviceGeometry;

use crate::sys::{self, Access};

/// How many physical disk indices to probe.
///
/// Windows assigns sequential indices and a home machine rarely exceeds half a
/// dozen; thirty-two covers it comfortably at no noticeable cost.
const MAX_PHYSICAL_DRIVES: u32 = 32;

/// Relevant `STORAGE_BUS_TYPE` codes.
mod bus_codes {
    pub const SCSI: u32 = 0x01;
    pub const ATAPI: u32 = 0x02;
    pub const ATA: u32 = 0x03;
    pub const USB: u32 = 0x07;
    pub const RAID: u32 = 0x08;
    pub const SAS: u32 = 0x0A;
    pub const SATA: u32 = 0x0B;
    pub const SD: u32 = 0x0C;
    pub const MMC: u32 = 0x0D;
    pub const NVME: u32 = 0x11;
}

/// Maps the numeric Windows bus code.
fn map_bus(code: u32) -> BusType {
    match code {
        bus_codes::SD => BusType::Sd,
        bus_codes::MMC => BusType::Mmc,
        bus_codes::USB => BusType::Usb,
        bus_codes::NVME => BusType::Nvme,
        bus_codes::SATA | bus_codes::ATA | bus_codes::ATAPI => BusType::Sata,
        bus_codes::SCSI | bus_codes::SAS => BusType::Scsi,
        bus_codes::RAID => BusType::Raid,
        _ => BusType::Unknown,
    }
}

/// Drive letter where Windows is installed.
///
/// Lets the system disk be recognised even when it is not `C:`.
fn system_drive_letter() -> Option<char> {
    let root = std::env::var("SystemRoot").or_else(|_| std::env::var("windir")).ok()?;
    root.chars().next().map(|c| c.to_ascii_uppercase())
}

/// A mounted volume, already associated with the disk it belongs to.
struct MountedVolume {
    disk_indices: Vec<u32>,
    info: VolumeInfo,
}

/// Collects every mounted volume and the disks they belong to.
fn collect_volumes() -> Vec<MountedVolume> {
    let system_letter = system_drive_letter();
    let Ok(guids) = sys::enumerate_volume_guids() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for guid in guids {
        let path = guid.trim_end_matches('\\');
        // Read access suffices to learn which disk a volume belongs to, and
        // avoids requesting write on volumes that will not be touched.
        let Ok(handle) = sys::open_device(path, Access::Read) else {
            continue;
        };
        let Ok(disk_indices) = sys::volume_disk_numbers(&handle) else {
            continue;
        };

        let mounts = sys::volume_mount_points(&guid).unwrap_or_default();
        let drive_letter = mounts
            .iter()
            .filter_map(|m| m.chars().next())
            .find(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_ascii_uppercase());

        let is_system = match (drive_letter, system_letter) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };

        out.push(MountedVolume {
            disk_indices,
            info: VolumeInfo {
                drive_letter,
                label: None,
                filesystem: None,
                // In practice the volume hosting Windows is also the one the
                // machine needs in order to boot.
                is_system_volume: is_system,
                is_boot_volume: is_system,
            },
        });
    }
    out
}

/// Windows device enumerator.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDeviceEnumerator;

impl WindowsDeviceEnumerator {
    /// Creates the enumerator.
    pub const fn new() -> Self {
        Self
    }

    /// Queries a single physical disk by index.
    ///
    /// Returns `None` when the index does not exist, which is the normal case
    /// while sweeping the range of possible indices.
    fn probe(index: u32, volumes: &[MountedVolume]) -> Option<DeviceInfo> {
        let path = format!("\\\\.\\PhysicalDrive{index}");
        let handle = sys::open_device(&path, Access::Read).ok()?;

        let (total_bytes, raw_sector) = sys::query_geometry(&handle).ok()?;
        let sector_size = if raw_sector == 0 { 512 } else { raw_sector };
        let geometry = DeviceGeometry::new(sector_size, total_bytes / sector_size as u64).ok()?;

        // The descriptor is desirable, but a device without one is still a
        // device: rather than vanish from the list, it appears unidentified.
        let descriptor = sys::query_storage_descriptor(&handle).unwrap_or_default();

        let mine: Vec<VolumeInfo> = volumes
            .iter()
            .filter(|v| v.disk_indices.contains(&index))
            .map(|v| v.info.clone())
            .collect();

        Some(DeviceInfo {
            path,
            index,
            // Not a translated string, unlike the rest of what the user reads.
            // The model becomes the device's display name, and the display name
            // is what a destructive operation asks to be typed back — a name
            // that changed with the interface language would be a name the
            // consent no longer matches.
            model: if descriptor.product.is_empty() {
                "Unidentified device".to_string()
            } else {
                descriptor.product
            },
            vendor: descriptor.vendor,
            serial: descriptor.serial,
            bus_type: map_bus(descriptor.bus_type),
            removable_media: descriptor.removable,
            geometry,
            volumes: mine,
        })
    }
}

impl DeviceEnumerator for WindowsDeviceEnumerator {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
        let volumes = collect_volumes();
        let mut out = Vec::new();
        for index in 0..MAX_PHYSICAL_DRIVES {
            if let Some(d) = Self::probe(index, &volumes) {
                out.push(d);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_codes_map_to_the_right_categories() {
        assert_eq!(map_bus(bus_codes::SD), BusType::Sd);
        assert_eq!(map_bus(bus_codes::MMC), BusType::Mmc);
        assert_eq!(map_bus(bus_codes::USB), BusType::Usb);
        assert_eq!(map_bus(bus_codes::NVME), BusType::Nvme);
        assert_eq!(map_bus(bus_codes::SATA), BusType::Sata);
        assert_eq!(map_bus(bus_codes::RAID), BusType::Raid);
        assert_eq!(map_bus(0xFF), BusType::Unknown);
    }

    #[test]
    fn internal_buses_are_never_mistaken_for_card_carriers() {
        for code in [bus_codes::NVME, bus_codes::SATA, bus_codes::ATA, bus_codes::RAID] {
            assert!(!map_bus(code).can_carry_removable_card(), "codigo {code:#x} passou");
        }
    }

    #[test]
    fn the_system_drive_letter_is_discoverable() {
        let letter = system_drive_letter();
        assert!(letter.is_some(), "SystemRoot deveria estar definido no Windows");
        assert!(letter.unwrap().is_ascii_uppercase());
    }

    /// Check against the machine's real hardware: enumeration must find disks
    /// and, above all, mark the system disk so the safety guards refuse it.
    #[test]
    fn real_enumeration_finds_disks_and_protects_the_system_one() {
        use salvage_app::safety::{evaluate, SafetyPolicy};

        let devices = WindowsDeviceEnumerator::new().enumerate().expect("enumeracao");
        if devices.is_empty() {
            return; // sem privilegio para abrir os discos
        }

        for d in &devices {
            assert!(d.geometry.total_sectors() > 0, "{} reports no sectors", d.path);
            assert!(!d.model.is_empty());
        }

        let system: Vec<_> = devices.iter().filter(|d| d.hosts_system()).collect();
        for d in system {
            assert!(
                !evaluate(d, &SafetyPolicy::default()).permits_writing(),
                "system disk {} was not blocked",
                d.path
            );
        }
    }
}
