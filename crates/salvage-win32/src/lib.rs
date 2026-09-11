//! Windows infrastructure for the Salvage toolchain.
//!
//! This is the only crate in the project containing `unsafe`. The others forbid
//! it at compiler level, which concentrates the memory-safety audit in one
//! place: the boundary with the system API.
//!
//! - [`sys`] — thin wrappers over system calls.
//! - [`raw_device`] — raw, unbuffered sector access.
//! - [`enumerate`] — disk and volume discovery.
//! - [`apply`] — partition table writing and formatting.
//! - [`text`] — English wording for the command-line tools.

#![warn(missing_docs)]

pub mod apply;
pub mod enumerate;
pub mod history;
pub mod raw_device;
pub mod sys;
pub mod text;

pub use apply::{apply_plan, prepare_card, release_card, ApplyError, ApplyOutcome};
pub use enumerate::WindowsDeviceEnumerator;
pub use history::FileHistory;
pub use raw_device::RawBlockDevice;
pub use sys::{windows_executable, Access};
