//! English wording for the command-line tools.
//!
//! The domain and application layers return classifications, never sentences.
//! This module is one presentation layer among several: the graphical interface
//! has its own, in its own language. Keeping the wording out of the core is
//! what allows both to exist without either dictating the other's voice.

use salvage_app::safety::{SafetyBlock, SafetyWarning};
use salvage_core::format;

/// Explains why a device was refused outright.
pub fn block_message(block: &SafetyBlock) -> String {
    match block {
        SafetyBlock::HostsOperatingSystem { volumes } => format!(
            "This disk holds the operating system (volumes: {}). Writing to it would \
             leave the machine unable to boot.",
            if volumes.is_empty() { "system".into() } else { volumes.join(", ") }
        ),
        SafetyBlock::InternalBus { bus } => format!(
            "Attached over {bus}, a bus used only by internal disks. Memory cards do \
             not appear this way."
        ),
        SafetyBlock::ExceedsAddressableCapacity { capacity_bytes, limit_bytes } => format!(
            "Capacity of {} exceeds the {} limit. No memory card is that large.",
            format::bytes(*capacity_bytes),
            format::bytes(*limit_bytes)
        ),
        SafetyBlock::ZeroCapacity => {
            "The device reports zero capacity. No media is inserted, or it is not \
             responding."
                .into()
        }
    }
}

/// Explains a risk the user may knowingly accept.
pub fn warning_message(warning: &SafetyWarning) -> String {
    match warning {
        SafetyWarning::NotDeclaredRemovable => {
            "Windows does not mark this media as removable. That is normal for USB \
             microSD adapters, but confirm the target is really the card."
                .into()
        }
        SafetyWarning::HasMountedVolumes { volumes } => {
            format!("Mounted volumes present ({}). All contents will be lost.", volumes.join(", "))
        }
        SafetyWarning::UnusuallyLarge { capacity_bytes } => format!(
            "A capacity of {} is large for a card. Confirm the target.",
            format::bytes(*capacity_bytes)
        ),
        SafetyWarning::UnknownBus => "The device's bus could not be identified.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_block_produces_a_non_empty_explanation() {
        let blocks = [
            SafetyBlock::HostsOperatingSystem { volumes: vec!["C:".into()] },
            SafetyBlock::InternalBus { bus: "NVMe".into() },
            SafetyBlock::ExceedsAddressableCapacity {
                capacity_bytes: 4_000_000_000_000,
                limit_bytes: 2_199_023_255_552,
            },
            SafetyBlock::ZeroCapacity,
        ];
        for b in &blocks {
            assert!(!block_message(b).trim().is_empty(), "{b:?} has no message");
        }
    }

    #[test]
    fn the_system_disk_message_names_the_volume() {
        let m = block_message(&SafetyBlock::HostsOperatingSystem { volumes: vec!["C:".into()] });
        assert!(m.contains("C:"));
    }

    #[test]
    fn every_warning_produces_a_non_empty_explanation() {
        let warnings = [
            SafetyWarning::NotDeclaredRemovable,
            SafetyWarning::HasMountedVolumes { volumes: vec!["E:".into()] },
            SafetyWarning::UnusuallyLarge { capacity_bytes: 2_000_000_000_000 },
            SafetyWarning::UnknownBus,
        ];
        for w in &warnings {
            assert!(!warning_message(w).trim().is_empty(), "{w:?} has no message");
        }
    }
}
