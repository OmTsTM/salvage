//! Acesso bruto, por setor, a um dispositivo fisico do Windows.

use salvage_app::device::{BlockDevice, DeviceError};
use salvage_core::DeviceGeometry;

use crate::sys::{
    self, Access, AlignedBuffer, OwnedHandle, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME,
};

/// Alignment required by unbuffered I/O. Covers 512- and 4096-byte sectors.
const IO_ALIGNMENT: usize = 4096;

/// Default bounce buffer size: 4 MiB.
const DEFAULT_BOUNCE_BYTES: usize = 4 * 1024 * 1024;

/// A physical device opened for direct read or write.
///
/// # The bounce buffer
///
/// The [`BlockDevice`] port works with ordinary slices, but unbuffered I/O
/// demands aligned buffers. Rather than contaminate the rest of the program
/// with that constraint, the device keeps an aligned buffer of its own and
/// copies through it. The extra copy costs fractions of a millisecond per
/// block, against tens of milliseconds of card latency: irrelevant overall.
pub struct RawBlockDevice {
    handle: OwnedHandle,
    geometry: DeviceGeometry,
    bounce: AlignedBuffer,
    writable: bool,
    /// Volume handles kept open. The lock lasts as long as the handle does, so
    /// holding them here is what stops Windows from remounting the volumes in
    /// the middle of a write.
    locked_volumes: Vec<OwnedHandle>,
}

impl RawBlockDevice {
    /// Opens a device by path, for example `\\.\PhysicalDrive3`.
    pub fn open(path: &str, access: Access) -> Result<Self, DeviceError> {
        let handle = sys::open_device(path, access)
            .map_err(|e| DeviceError::Other(format!("could not open {path}: {e}")))?;

        let (total_bytes, sector_size) = sys::query_geometry(&handle)
            .map_err(|e| DeviceError::Other(format!("geometria indisponivel: {e}")))?;

        let sector_size = if sector_size == 0 { 512 } else { sector_size };
        let geometry = DeviceGeometry::new(sector_size, total_bytes / sector_size as u64)
            .map_err(|e| DeviceError::Other(format!("geometria invalida: {e}")))?;

        Ok(Self {
            handle,
            geometry,
            bounce: AlignedBuffer::new(DEFAULT_BOUNCE_BYTES, IO_ALIGNMENT),
            writable: access == Access::ReadWrite,
            locked_volumes: Vec::new(),
        })
    }

    /// Locks and dismounts every volume on the given disk.
    ///
    /// Without this, Windows keeps writing metadata to the card during the
    /// operation, which would corrupt the scan result and could undo the
    /// partition table just written.
    ///
    /// Volumes that refuse the lock do not abort the operation: the usual cause
    /// is some program holding a file open, and the subsequent dismount
    /// normally resolves it. The failure is returned in the list so the layer
    /// above can report it.
    pub fn lock_volumes_of_disk(&mut self, disk_index: u32) -> Vec<String> {
        let mut problems = Vec::new();
        let guids = match sys::enumerate_volume_guids() {
            Ok(g) => g,
            Err(e) => {
                problems.push(format!("could not enumerate volumes: {e}"));
                return problems;
            }
        };

        for guid in guids {
            // `CreateFileW` rejects the GUID with its trailing backslash.
            let path = guid.trim_end_matches('\\');

            // Disk membership is determined with read access. Requesting write
            // at this stage would make volumes on *other* disks fail for lack of
            // permission, and the volume that matters would be discarded along
            // with them, silently.
            let belongs = match sys::open_device(path, Access::Read) {
                Ok(h) => sys::volume_disk_numbers(&h)
                    .map(|disks| disks.contains(&disk_index))
                    .unwrap_or(false),
                Err(_) => false,
            };
            if !belongs {
                continue;
            }

            match sys::open_device(path, Access::ReadWrite) {
                Ok(vol) => {
                    if let Err(e) = sys::control_simple(&vol, FSCTL_LOCK_VOLUME) {
                        problems.push(format!("volume {path} could not be locked: {e}"));
                    }
                    if let Err(e) = sys::control_simple(&vol, FSCTL_DISMOUNT_VOLUME) {
                        problems.push(format!("volume {path} could not be dismounted: {e}"));
                    }
                    self.locked_volumes.push(vol);
                }
                // Without this report, the write would be denied later with
                // nothing explaining why.
                Err(e) => problems.push(format!(
                    "volume {path} could not be opened for writing: {e}. \
                     A escrita direta no cartao sera recusada enquanto ele estiver montado."
                )),
            }
        }
        problems
    }

    /// Returns dismounted volumes to the system.
    ///
    /// Dismounting without remounting leaves the card inaccessible until it is
    /// physically unplugged and reconnected. Since an inspection can be
    /// cancelled or fail at any moment, the hand-back must happen on every exit
    /// path — which is why it lives in `Drop` rather than in a call someone
    /// could forget.
    fn release_volumes(&mut self) {
        if self.locked_volumes.is_empty() {
            return;
        }
        // Closing the handles releases the lock; notifying the system brings
        // the volume back.
        self.locked_volumes.clear();
        let _ = sys::update_disk_properties(&self.handle);
    }

    /// Makes the system re-read the partition table.
    pub fn refresh_partition_table(&mut self) -> Result<(), DeviceError> {
        // Releasing the locked volumes lets the system mount whatever comes
        // into existence after the re-read.
        self.locked_volumes.clear();
        sys::update_disk_properties(&self.handle)
            .map_err(|e| DeviceError::Other(format!("failed to refresh the partition table: {e}")))
    }

    /// Largest transfer that fits the bounce buffer, in bytes, rounded down to
    /// a sector multiple.
    fn max_transfer(&self) -> usize {
        let ss = self.geometry.sector_size() as usize;
        let len = self.bounce.len();
        len - (len % ss)
    }

    /// Positions the file pointer at the start of an LBA.
    fn seek_to(&self, lba: u64) -> Result<(), DeviceError> {
        let offset = lba.saturating_mul(self.geometry.sector_size() as u64);
        sys::seek(&self.handle, offset).map_err(|e| DeviceError::Io { lba, source: e })
    }
}

impl Drop for RawBlockDevice {
    fn drop(&mut self) {
        self.release_volumes();
    }
}

impl BlockDevice for RawBlockDevice {
    fn geometry(&self) -> DeviceGeometry {
        self.geometry
    }

    fn read_at(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DeviceError> {
        self.check_access(lba, buf.len())?;
        let ss = self.geometry.sector_size() as usize;
        let max = self.max_transfer();
        let mut offset = 0usize;

        while offset < buf.len() {
            let take = (buf.len() - offset).min(max);
            let current = lba + (offset / ss) as u64;

            self.seek_to(current)?;
            sys::read_exact(&self.handle, &mut self.bounce.as_mut_slice()[..take])
                .map_err(|e| DeviceError::Io { lba: current, source: e })?;
            buf[offset..offset + take].copy_from_slice(&self.bounce.as_slice()[..take]);

            offset += take;
        }
        Ok(())
    }

    fn write_at(&mut self, lba: u64, buf: &[u8]) -> Result<(), DeviceError> {
        if !self.writable {
            return Err(DeviceError::ReadOnly);
        }
        self.check_access(lba, buf.len())?;
        let ss = self.geometry.sector_size() as usize;
        let max = self.max_transfer();
        let mut offset = 0usize;

        while offset < buf.len() {
            let take = (buf.len() - offset).min(max);
            let current = lba + (offset / ss) as u64;

            self.bounce.as_mut_slice()[..take].copy_from_slice(&buf[offset..offset + take]);
            self.seek_to(current)?;
            sys::write_exact(&self.handle, &self.bounce.as_slice()[..take])
                .map_err(|e| DeviceError::Io { lba: current, source: e })?;

            offset += take;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        // The device is opened with `FILE_FLAG_WRITE_THROUGH`: every write
        // reaches the media before returning, so nothing is pending.
        Ok(())
    }

    fn is_writable(&self) -> bool {
        self.writable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_nonexistent_drive_reports_an_error() {
        let e = RawBlockDevice::open("\\\\.\\PhysicalDrive250", Access::Read);
        assert!(e.is_err());
    }

    /// Opening the system disk read-only must work (the session runs
    /// elevated), and the geometry must be coherent. Nothing is written: the
    /// test only confirms geometry reads work against real hardware.
    #[test]
    fn reading_geometry_from_a_real_disk_works_when_elevated() {
        let Ok(dev) = RawBlockDevice::open("\\\\.\\PhysicalDrive0", Access::Read) else {
            // No privilege, or no disk 0: nothing to verify.
            return;
        };
        let g = dev.geometry();
        assert!(matches!(g.sector_size(), 512 | 1024 | 2048 | 4096));
        assert!(g.total_sectors() > 0);
        assert!(!dev.is_writable(), "a read-only handle must not report itself writable");
    }
}
