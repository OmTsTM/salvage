//! Domain core for inspecting flash media and salvaging the space that still
//! works.
//!
//! This layer is pure. It opens no devices, touches no filesystem, and depends
//! on no operating system. Every function maps data to data, which is what
//! makes it possible to test the logic that decides where a user's files may
//! live — exhaustively, and without risking real hardware.
//!
//! # Modules
//!
//! - [`geometry`] — device geometry and LBA interval algebra.
//! - [`pattern`] — self-identifying test pattern that exposes corruption and
//!   address aliasing.
//! - [`sector_map`] — run-length map of per-sector state.
//! - [`health`] — failure classification and the assurance level it warrants.
//! - [`planner`] — partition layouts that fence defects away from user data.
//! - [`mbr`] — partition table serialization.
//! - [`format`] — human-readable rendering of sizes and durations.
//!
//! # What this tool can honestly promise
//!
//! An SD card does not expose physical memory. Between the LBA the host sees
//! and the NAND cell holding the data sits a controller that remaps blocks on
//! every write, performs wear levelling, and retires failing blocks silently.
//! "Isolating bad sectors" therefore cannot be sold as a guarantee: the mapping
//! that holds today is not the mapping that will hold tomorrow.
//!
//! What the domain does instead is more useful, and more honest. It identifies
//! **why** a card fails, and only then states how far the result can be
//! trusted. A card whose capacity is a firmware lie has a fixed boundary that
//! fencing resolves permanently; a card whose controller is actively
//! deteriorating cannot be helped by any partition layout. These are different
//! situations with different answers, and conflating them is the one way this
//! tool could cause data loss. See [`health::Assurance`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod fat32;
pub mod format;
pub mod geometry;
pub mod health;
pub mod mbr;
pub mod pattern;
pub mod planner;
pub mod sector_map;

pub use fat32::{Fat32Error, Fat32Image, Fat32Layout};
pub use geometry::{DeviceGeometry, GeometryError, LbaRange};
pub use health::{diagnose, Assurance, FailureScenario, HealthReport};
pub use mbr::{MasterBootRecord, MbrError, PartitionEntry, PriorLayout};
pub use pattern::{PatternGenerator, PatternKind, SectorVerdict};
pub use planner::{
    layout_requirements, plan_layouts, FileSystem, LayoutRequirements, PartitionPlan,
    PartitionRole, PlanError, PlannedPartition, PlanningPolicy, Strategy,
};
pub use sector_map::{AliasObservation, SectorCounts, SectorMap, SectorState};
