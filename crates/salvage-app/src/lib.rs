//! Use cases and ports for the Salvage toolchain.
//!
//! The layer between the pure domain (`salvage-core`) and the operating
//! system. It defines **what** the program does without deciding **how** the
//! system does it: disk access, enumeration and formatting enter through
//! traits, which allows the full flow to be exercised against the
//! [`simulator`] before touching hardware.
//!
//! # Modules
//!
//! - [`device`] — device descriptions and access ports.
//! - [`safety`] — guards deciding whether a device may be written to.
//! - [`scan`] — scanning, classification and bisection refinement.
//! - [`simulator`] — in-memory card with programmable defects.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod device;
pub mod safety;
pub mod scan;
pub mod simulator;

pub use device::{BlockDevice, BusType, DeviceEnumerator, DeviceError, DeviceInfo, VolumeInfo};
pub use safety::{
    evaluate, ConfirmationError, DestructiveConsent, SafetyBlock, SafetyPolicy, SafetyVerdict,
    SafetyWarning,
};
pub use scan::{
    CancellationToken, ScanConfig, ScanError, ScanObserver, ScanOutcome, ScanPhase, ScanProgress,
    Scanner, SilentObserver,
};
pub use simulator::{SimulatedCard, SimulatedFault};
