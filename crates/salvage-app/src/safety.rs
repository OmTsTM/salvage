//! Guards deciding whether a device may be written to.
//!
//! This tool writes to raw disks. A wrong target does not corrupt a file: it
//! destroys an entire drive. All of the "may I" logic lives here, isolated,
//! free of I/O, and exhaustively tested.
//!
//! # Why the removable-media flag is not enough
//!
//! The obvious criterion — accept only devices marked removable — rejects the
//! primary use case. Many USB microSD adapters present themselves to Windows as
//! fixed disks, and the user's card would be turned away. The criterion adopted
//! here is composite instead: the bus says what the device *can* be, the
//! mounted volumes say what it *cannot* be, and whatever remains depends on
//! explicit confirmation.
//!
//! # Two levels, with different meanings
//!
//! A [`SafetyBlock`] is final and has no override anywhere in the interface. A
//! [`SafetyWarning`] describes risk the user may knowingly accept by typing the
//! device's name.
//!
//! Like the domain layer, this module returns classifications rather than
//! sentences; each front end supplies its own wording.

use serde::{Deserialize, Serialize};

use crate::device::{BusType, DeviceInfo};

/// Capacity ceiling accepted: 2 TiB, the MBR addressing limit. No SD card in
/// existence comes close.
pub const MAX_CARD_CAPACITY_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;

/// Above this size, a "card" warrants explicit suspicion.
pub const SUSPICIOUS_CAPACITY_BYTES: u64 = 1024 * 1024 * 1024 * 1024;

/// An absolute impediment. No confirmation unlocks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SafetyBlock {
    /// The device hosts the operating system or the boot partition.
    HostsOperatingSystem {
        /// Drive letters of the system volumes found.
        volumes: Vec<String>,
    },
    /// A bus used exclusively by fixed internal storage.
    InternalBus {
        /// Bus detected.
        bus: String,
    },
    /// Capacity beyond what the partition format can address.
    ExceedsAddressableCapacity {
        /// Reported capacity, in bytes.
        capacity_bytes: u64,
        /// Ceiling accepted.
        limit_bytes: u64,
    },
    /// Device reports no usable capacity.
    ZeroCapacity,
}

/// Risk the user may knowingly accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SafetyWarning {
    /// Media not declared removable — common on USB card adapters.
    NotDeclaredRemovable,
    /// Mounted volumes exist and will be unmounted and erased.
    HasMountedVolumes {
        /// Description of the affected volumes.
        volumes: Vec<String>,
    },
    /// Capacity unusually large for a memory card.
    UnusuallyLarge {
        /// Reported capacity, in bytes.
        capacity_bytes: u64,
    },
    /// Bus could not be identified.
    UnknownBus,
}

/// Outcome of a safety evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum SafetyVerdict {
    /// Cleared without reservation. A destructive operation still requires
    /// typed confirmation.
    Allowed,
    /// Cleared only on conscious acknowledgement of the listed risks.
    NeedsConfirmation {
        /// Risks identified.
        warnings: Vec<SafetyWarning>,
    },
    /// Refused. No path in the interface unlocks this.
    Blocked {
        /// Impediments found.
        reasons: Vec<SafetyBlock>,
    },
}

impl SafetyVerdict {
    /// Whether the device may be written to, given adequate confirmation.
    #[inline]
    pub const fn permits_writing(&self) -> bool {
        !matches!(self, Self::Blocked { .. })
    }

    /// Whether there is any risk to communicate.
    #[inline]
    pub const fn has_warnings(&self) -> bool {
        matches!(self, Self::NeedsConfirmation { .. })
    }
}

/// Evaluation parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyPolicy {
    /// Maximum capacity accepted, in bytes.
    pub max_capacity_bytes: u64,
    /// Capacity above which a warning is raised.
    pub suspicious_capacity_bytes: u64,
    /// Whether devices not declared removable may proceed on confirmation.
    ///
    /// Turning this off rejects most USB adapters.
    pub allow_non_removable: bool,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            max_capacity_bytes: MAX_CARD_CAPACITY_BYTES,
            suspicious_capacity_bytes: SUSPICIOUS_CAPACITY_BYTES,
            allow_non_removable: true,
        }
    }
}

/// Evaluates whether a device may be the target of a destructive operation.
///
/// Order matters: absolute impediments are checked first and, if any exist, the
/// verdict is refusal — no warning ever softens a block.
pub fn evaluate(device: &DeviceInfo, policy: &SafetyPolicy) -> SafetyVerdict {
    let mut blocks = Vec::new();

    if device.hosts_system() {
        let volumes = device
            .volumes
            .iter()
            .filter(|v| v.is_system_volume || v.is_boot_volume)
            .map(|v| v.drive_letter.map_or_else(|| "unlettered".into(), |c| format!("{c}:")))
            .collect();
        blocks.push(SafetyBlock::HostsOperatingSystem { volumes });
    }

    if device.bus_type.is_internal_only() {
        blocks.push(SafetyBlock::InternalBus { bus: device.bus_type.as_str().to_string() });
    }

    let capacity = device.capacity_bytes();
    if capacity == 0 {
        blocks.push(SafetyBlock::ZeroCapacity);
    } else if capacity > policy.max_capacity_bytes {
        blocks.push(SafetyBlock::ExceedsAddressableCapacity {
            capacity_bytes: capacity,
            limit_bytes: policy.max_capacity_bytes,
        });
    }

    if !device.removable_media && !policy.allow_non_removable {
        blocks.push(SafetyBlock::InternalBus { bus: device.bus_type.as_str().to_string() });
    }

    if !blocks.is_empty() {
        return SafetyVerdict::Blocked { reasons: blocks };
    }

    let mut warnings = Vec::new();

    if !device.removable_media {
        warnings.push(SafetyWarning::NotDeclaredRemovable);
    }
    if device.bus_type == BusType::Unknown {
        warnings.push(SafetyWarning::UnknownBus);
    }
    if capacity > policy.suspicious_capacity_bytes {
        warnings.push(SafetyWarning::UnusuallyLarge { capacity_bytes: capacity });
    }
    if !device.volumes.is_empty() {
        let volumes = device
            .volumes
            .iter()
            .map(|v| match (v.drive_letter, v.label.as_deref()) {
                (Some(c), Some(l)) if !l.is_empty() => format!("{c}: \"{l}\""),
                (Some(c), _) => format!("{c}:"),
                (None, Some(l)) => l.to_string(),
                (None, None) => "unlettered volume".into(),
            })
            .collect();
        warnings.push(SafetyWarning::HasMountedVolumes { volumes });
    }

    if warnings.is_empty() {
        SafetyVerdict::Allowed
    } else {
        SafetyVerdict::NeedsConfirmation { warnings }
    }
}

/// Failures while issuing consent for a destructive operation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfirmationError {
    /// The device is blocked outright.
    #[error("device refused by the safety guards")]
    DeviceBlocked,
    /// The typed text does not match the device name.
    #[error("invalid confirmation: typed {typed:?}, expected {expected:?}")]
    NameMismatch {
        /// What the user typed.
        typed: String,
        /// What was expected.
        expected: String,
    },
}

/// Proof that the user authorised a destructive operation on this specific
/// device.
///
/// The only way to obtain one is [`DestructiveConsent::issue`], and destructive
/// operations take it as a parameter. The result is that no code path can write
/// without passing through the safety evaluation: the guarantee stops depending
/// on discipline and becomes structural.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestructiveConsent {
    fingerprint: String,
    device_path: String,
}

impl DestructiveConsent {
    /// Issues consent if the device is eligible and the typed name matches.
    ///
    /// Requiring the name is not ceremony: it separates "clicked OK without
    /// reading" from "recognised the device".
    pub fn issue(
        device: &DeviceInfo,
        typed_name: &str,
        policy: &SafetyPolicy,
    ) -> Result<Self, ConfirmationError> {
        if !evaluate(device, policy).permits_writing() {
            return Err(ConfirmationError::DeviceBlocked);
        }
        let expected = device.display_name();
        if !typed_name.trim().eq_ignore_ascii_case(expected.trim()) {
            return Err(ConfirmationError::NameMismatch {
                typed: typed_name.trim().to_string(),
                expected,
            });
        }
        Ok(Self { fingerprint: device.fingerprint(), device_path: device.path.clone() })
    }

    /// Checks that the consent applies to this device.
    ///
    /// Called again immediately before writing. If the user swapped the card
    /// between approval and application, the fingerprint changes and the
    /// operation aborts.
    pub fn matches(&self, device: &DeviceInfo) -> bool {
        self.fingerprint == device.fingerprint() && self.device_path == device.path
    }

    /// The authorised fingerprint.
    #[inline]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::VolumeInfo;
    use salvage_core::DeviceGeometry;

    fn card() -> DeviceInfo {
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

    fn system_volume() -> VolumeInfo {
        VolumeInfo {
            drive_letter: Some('C'),
            label: Some("Windows".into()),
            filesystem: Some("NTFS".into()),
            is_system_volume: true,
            is_boot_volume: true,
        }
    }

    #[test]
    fn a_plain_sd_card_is_allowed() {
        assert_eq!(evaluate(&card(), &SafetyPolicy::default()), SafetyVerdict::Allowed);
    }

    /// The single most important guard in the program.
    #[test]
    fn the_system_disk_is_always_blocked() {
        let mut d = card();
        d.volumes.push(system_volume());
        let v = evaluate(&d, &SafetyPolicy::default());
        assert!(!v.permits_writing());
        match v {
            SafetyVerdict::Blocked { reasons } => {
                assert!(reasons
                    .iter()
                    .any(|r| matches!(r, SafetyBlock::HostsOperatingSystem { .. })));
            }
            other => panic!("expected a block, got {other:?}"),
        }
    }

    #[test]
    fn an_nvme_ssd_is_blocked_even_with_no_volumes() {
        let mut d = card();
        d.bus_type = BusType::Nvme;
        d.removable_media = false;
        d.volumes.clear();
        assert!(!evaluate(&d, &SafetyPolicy::default()).permits_writing());
    }

    #[test]
    fn sata_and_raid_are_blocked() {
        for bus in [BusType::Sata, BusType::Raid] {
            let mut d = card();
            d.bus_type = bus;
            assert!(
                !evaluate(&d, &SafetyPolicy::default()).permits_writing(),
                "{bus:?} slipped through"
            );
        }
    }

    /// The case a naive guard gets wrong: a USB adapter reporting fixed media.
    #[test]
    fn a_usb_card_reader_reporting_fixed_media_is_allowed_with_a_warning() {
        let mut d = card();
        d.bus_type = BusType::Usb;
        d.removable_media = false;
        match evaluate(&d, &SafetyPolicy::default()) {
            SafetyVerdict::NeedsConfirmation { warnings } => {
                assert!(warnings.contains(&SafetyWarning::NotDeclaredRemovable));
            }
            other => panic!("the USB adapter should pass with a warning, got {other:?}"),
        }
    }

    #[test]
    fn a_strict_policy_can_refuse_non_removable_media() {
        let mut d = card();
        d.bus_type = BusType::Usb;
        d.removable_media = false;
        let strict = SafetyPolicy { allow_non_removable: false, ..Default::default() };
        assert!(!evaluate(&d, &strict).permits_writing());
    }

    #[test]
    fn mounted_volumes_produce_a_warning_not_a_block() {
        let mut d = card();
        d.volumes.push(VolumeInfo {
            drive_letter: Some('E'),
            label: Some("PHOTOS".into()),
            filesystem: Some("exFAT".into()),
            is_system_volume: false,
            is_boot_volume: false,
        });
        match evaluate(&d, &SafetyPolicy::default()) {
            SafetyVerdict::NeedsConfirmation { warnings } => {
                let named = warnings.iter().any(|w| matches!(
                    w,
                    SafetyWarning::HasMountedVolumes { volumes } if volumes.iter().any(|v| v.starts_with("E:"))
                ));
                assert!(named, "the warning should name the volume");
            }
            other => panic!("expected a warning, got {other:?}"),
        }
    }

    #[test]
    fn zero_and_oversized_capacity_are_blocked() {
        let mut d = card();
        d.geometry = DeviceGeometry::new(4096, u64::MAX / 4096).unwrap();
        assert!(!evaluate(&d, &SafetyPolicy::default()).permits_writing());
    }

    #[test]
    fn a_block_always_beats_a_warning() {
        let mut d = card();
        d.bus_type = BusType::Nvme; // block
        d.removable_media = false; // warning
        d.volumes.push(system_volume()); // block
        match evaluate(&d, &SafetyPolicy::default()) {
            SafetyVerdict::Blocked { reasons } => assert!(reasons.len() >= 2),
            other => panic!("expected a block, got {other:?}"),
        }
    }

    #[test]
    fn consent_requires_the_exact_device_name() {
        let d = card();
        let p = SafetyPolicy::default();
        assert!(DestructiveConsent::issue(&d, "Generic SDXC Card", &p).is_ok());
        // Whitespace and case do not matter; the name does.
        assert!(DestructiveConsent::issue(&d, "  generic sdxc card  ", &p).is_ok());
        assert!(matches!(
            DestructiveConsent::issue(&d, "yes", &p),
            Err(ConfirmationError::NameMismatch { .. })
        ));
        assert!(matches!(
            DestructiveConsent::issue(&d, "", &p),
            Err(ConfirmationError::NameMismatch { .. })
        ));
    }

    #[test]
    fn consent_cannot_be_issued_for_a_blocked_device() {
        let mut d = card();
        d.volumes.push(system_volume());
        assert_eq!(
            DestructiveConsent::issue(&d, &d.display_name(), &SafetyPolicy::default()),
            Err(ConfirmationError::DeviceBlocked)
        );
    }

    /// The user approves, swaps the card, then applies. The operation must
    /// abort rather than write to the new card.
    #[test]
    fn consent_does_not_transfer_to_a_swapped_card() {
        let original = card();
        let consent = DestructiveConsent::issue(
            &original,
            &original.display_name(),
            &SafetyPolicy::default(),
        )
        .unwrap();
        assert!(consent.matches(&original));

        let mut swapped = card();
        swapped.serial = Some("OTHER".into());
        assert!(!consent.matches(&swapped), "consent leaked to another card");

        let mut resized = card();
        resized.geometry = DeviceGeometry::new(512, 60_000_000).unwrap();
        assert!(!consent.matches(&resized));
    }

    #[test]
    fn consent_does_not_transfer_to_another_slot() {
        let original = card();
        let consent = DestructiveConsent::issue(
            &original,
            &original.display_name(),
            &SafetyPolicy::default(),
        )
        .unwrap();
        let mut moved = card();
        moved.path = "\\\\.\\PhysicalDrive7".into();
        assert!(!consent.matches(&moved));
    }

    /// Sweep over combinations: no system disk may ever escape.
    #[test]
    fn no_combination_ever_unblocks_a_system_disk() {
        for bus in [BusType::Sd, BusType::Mmc, BusType::Usb, BusType::Unknown] {
            for removable in [true, false] {
                let mut d = card();
                d.bus_type = bus;
                d.removable_media = removable;
                d.volumes.push(system_volume());
                assert!(
                    !evaluate(&d, &SafetyPolicy::default()).permits_writing(),
                    "bus={bus:?} removable={removable} unblocked the system disk"
                );
                assert!(DestructiveConsent::issue(&d, &d.display_name(), &SafetyPolicy::default())
                    .is_err());
            }
        }
    }
}
