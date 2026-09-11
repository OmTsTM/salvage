//! Thin layer over the Windows API.
//!
//! All of the project's `unsafe` lives in this crate, and most of it in this
//! module. Each function here wraps a single system call and returns an
//! idiomatic `Result`, so the modules above stay readable and free of raw
//! pointers.
//!
//! # Aligned buffers are not a performance detail
//!
//! Devices are opened with `FILE_FLAG_NO_BUFFERING`, and that is required for
//! the diagnosis to be correct, not an optimisation. With the Windows cache in
//! the path, a read issued right after a write can be served from system memory
//! without ever reaching the card. The pattern would come back intact, a
//! counterfeit card's aliasing would go unnoticed, and the tool would approve an
//! area that does not exist.
//!
//! The price of bypassing the cache is that buffers must be aligned to the
//! sector size, which an ordinary `Vec<u8>` does not satisfy. Hence
//! [`AlignedBuffer`].

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::io;
use std::ptr::NonNull;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, MAX_PATH,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose,
    GetVolumePathNamesForVolumeNameW, ReadFile, SetFilePointerEx, WriteFile, FILE_BEGIN,
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, DISK_GEOMETRY_EX,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_UPDATE_PROPERTIES, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY, VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// `GENERIC_READ`, declared here because it moves between modules across crate versions.
pub const GENERIC_READ: u32 = 0x8000_0000;
/// `GENERIC_WRITE`.
pub const GENERIC_WRITE: u32 = 0x4000_0000;
/// `FSCTL_LOCK_VOLUME`.
pub const FSCTL_LOCK_VOLUME: u32 = 0x0009_0018;
/// `FSCTL_UNLOCK_VOLUME`.
pub const FSCTL_UNLOCK_VOLUME: u32 = 0x0009_001C;
/// `FSCTL_DISMOUNT_VOLUME`.
pub const FSCTL_DISMOUNT_VOLUME: u32 = 0x0009_0020;
/// `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`.
pub const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

/// Absolute path to an executable in the Windows system directory.
///
/// # Why a program is never invoked by bare name
///
/// `Command::new("cmd")` resolves the executable by walking `PATH`. In an
/// elevated process that is a ready-made privilege ladder: anyone able to place
/// a `cmd.exe` in any `PATH` directory they control — or, depending on
/// configuration, in the working directory — gets their own code running as
/// Administrator.
///
/// The system directory comes from the API rather than an environment
/// variable: `SystemRoot` would be manipulable through the process environment
/// just the same.
pub fn system_executable(name: &str) -> io::Result<std::path::PathBuf> {
    let mut buf = [0u16; MAX_PATH as usize];
    // SAFETY: `buf` holds MAX_PATH elements, exactly the size declared.
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) };
    if len == 0 || len as usize > buf.len() {
        return Err(last_error());
    }
    let dir = String::from_utf16_lossy(&buf[..len as usize]);
    Ok(std::path::PathBuf::from(dir).join(name))
}

/// Absolute path to an executable in the Windows directory.
///
/// The counterpart to [`system_executable`], for the handful of programs that
/// live beside Windows itself rather than in System32 — `explorer.exe` among
/// them. The reason is the same and it is not stylistic: naming a bare
/// executable resolves it by walking `PATH`, and this program runs elevated, so
/// a planted file earlier on `PATH` would be launched as Administrator.
pub fn windows_executable(name: &str) -> io::Result<std::path::PathBuf> {
    let mut buf = [0u16; MAX_PATH as usize];
    // SAFETY: `buf` holds MAX_PATH elements, exactly the size declared.
    let len = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) };
    if len == 0 || len as usize > buf.len() {
        return Err(last_error());
    }
    let dir = String::from_utf16_lossy(&buf[..len as usize]);
    Ok(std::path::PathBuf::from(dir).join(name))
}

/// Converts a Rust string into a null-terminated UTF-16 buffer.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads a null-terminated UTF-16 string from a buffer.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// The calling thread's last system error, as an `io::Error`.
pub fn last_error() -> io::Error {
    // SAFETY: `GetLastError` only reads the calling thread's error code and
    // neither takes nor returns pointers.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

/// Aligned buffer, required for unbuffered I/O.
///
/// Allocates with explicit alignment and frees exactly the same `Layout` in
/// `Drop`, which is the condition the global allocator requires.
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
}

// SAFETY: the struct holds exclusive ownership of a plain byte allocation,
// with no shared state and no interior mutability, so it can safely move
// between threads.
unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    /// Allocates `len` zeroed bytes with the given alignment.
    ///
    /// # Panics
    /// If `len` is zero or the alignment is not a power of two.
    pub fn new(len: usize, alignment: usize) -> Self {
        assert!(len > 0, "buffer vazio");
        assert!(alignment.is_power_of_two(), "alignment {alignment} is not a power of two");
        let layout = Layout::from_size_align(len, alignment).expect("layout valido");
        // SAFETY: `layout` has non-zero size and valid alignment, the two
        // preconditions of `alloc_zeroed`.
        let raw = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).unwrap_or_else(|| std::alloc::handle_alloc_error(layout));
        Self { ptr, len, layout }
    }

    /// Immutable slice over the contents.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` points at `len` initialised bytes that live as long as
        // `self`, and the borrow prevents concurrent mutable access.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// Mutable slice over the contents.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: same conditions as `as_slice`; the exclusive borrow of
        // `self` guarantees no other reference is live.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Length of the buffer, in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer is empty. Never is: the constructor refuses a zero
    /// length, and the field is immutable afterwards.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `alloc_zeroed` with exactly this `layout`
        // and has not been freed before — `Drop` runs once.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

/// Self-closing Windows object handle.
pub struct OwnedHandle(HANDLE);

// SAFETY: a Windows file handle may be used from any thread; the struct holds
// exclusive ownership of it.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    /// Raw handle, for the system calls in this module.
    ///
    /// Kept crate-private on purpose: holding an `&OwnedHandle` is what proves
    /// a handle is open, and handing out the bare pointer would let a caller
    /// outlive that proof.
    #[inline]
    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: the handle is valid, came from `CreateFileW`, and has
            // not been closed yet.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Device open mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Read-only access.
    Read,
    /// Read and write.
    ReadWrite,
}

/// Opens a device or volume bypassing the system cache.
///
/// Sharing is permissive because Windows keeps the device's volumes mounted
/// until they are explicitly dismounted; denying sharing here would make the
/// open fail before there was any opportunity to dismount them.
pub fn open_device(path: &str, access: Access) -> io::Result<OwnedHandle> {
    let wide = to_wide(path);
    let desired = match access {
        Access::Read => GENERIC_READ,
        Access::ReadWrite => GENERIC_READ | GENERIC_WRITE,
    };

    // SAFETY: `wide` is a valid null-terminated UTF-16 string that lives for
    // the duration of the call; the remaining arguments are documented
    // constants.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            desired,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH,
            std::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(last_error());
    }
    Ok(OwnedHandle(handle))
}

/// Sends a control code with no input data and returns the raw output.
///
/// # Safety
/// `out` must be a valid buffer of at least `out_len` bytes, and the caller
/// must know the control code fills exactly that type — otherwise the returned
/// structure will be read incorrectly.
unsafe fn control(
    handle: HANDLE,
    code: u32,
    input: *const std::ffi::c_void,
    input_len: u32,
    out: *mut std::ffi::c_void,
    out_len: u32,
) -> io::Result<u32> {
    let mut returned: u32 = 0;
    // SAFETY: delegated to the caller as documented above. The body of an
    // `unsafe` function is already an unsafe context, so no block is needed.
    let ok = DeviceIoControl(
        handle,
        code,
        input,
        input_len,
        out,
        out_len,
        &mut returned,
        std::ptr::null_mut(),
    );
    if ok == 0 {
        return Err(last_error());
    }
    Ok(returned)
}

/// Sends a control code that exchanges no data.
pub fn control_simple(handle: &OwnedHandle, code: u32) -> io::Result<()> {
    // SAFETY: with neither input nor output buffers, null pointers and zero
    // lengths are the documented way to invoke these codes.
    unsafe { control(handle.raw(), code, std::ptr::null(), 0, std::ptr::null_mut(), 0) }.map(|_| ())
}

/// Geometry as reported by the device: total size and sector size.
pub fn query_geometry(handle: &OwnedHandle) -> io::Result<(u64, u32)> {
    // SAFETY: `DISK_GEOMETRY_EX` is a plain-old-data struct of integers and
    // nested PODs, so an all-zero bit pattern is a valid value for it. The
    // control code below overwrites every field it defines.
    let mut geo: DISK_GEOMETRY_EX = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<DISK_GEOMETRY_EX>() as u32;

    // SAFETY: `geo` is a valid `DISK_GEOMETRY_EX` and `size` is exactly its
    // size, which is what this control code fills.
    unsafe {
        control(
            handle.raw(),
            IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            std::ptr::null(),
            0,
            std::ptr::addr_of_mut!(geo).cast(),
            size,
        )?
    };

    let total = geo.DiskSize as u64;
    let sector = geo.Geometry.BytesPerSector;
    Ok((total, sector))
}

/// Storage device description.
#[derive(Debug, Clone, Default)]
pub struct StorageDescriptor {
    /// Fabricante.
    pub vendor: String,
    /// Modelo.
    pub product: String,
    /// Serial number.
    pub serial: Option<String>,
    /// Whether the media is declared removable.
    pub removable: bool,
    /// Codigo numerico do barramento.
    pub bus_type: u32,
}

/// Reads a null-terminated ANSI string at an offset within a buffer.
fn ansi_at(buf: &[u8], offset: u32) -> Option<String> {
    let at = offset as usize;
    if offset == 0 || at >= buf.len() {
        return None;
    }
    let rest = &buf[at..];
    let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
    let s = String::from_utf8_lossy(&rest[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Queries vendor, model, serial, removability and bus type.
pub fn query_storage_descriptor(handle: &OwnedHandle) -> io::Result<StorageDescriptor> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0; 1],
    };

    // The descriptor is variable length: strings sit after the struct, pointed
    // to by offsets. A generous buffer avoids a second call.
    let mut buf = vec![0u8; 1024];

    // SAFETY: `query` is a valid `STORAGE_PROPERTY_QUERY` and `buf` has room
    // for the descriptor plus the strings following it.
    let written = unsafe {
        control(
            handle.raw(),
            IOCTL_STORAGE_QUERY_PROPERTY,
            std::ptr::addr_of!(query).cast(),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
        )?
    } as usize;

    if written < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "descritor truncado"));
    }

    // SAFETY: the control code filled at least one `STORAGE_DEVICE_DESCRIPTOR`
    // at the start of the buffer, and the read copies out so no reference into
    // the buffer is retained.
    let d: STORAGE_DEVICE_DESCRIPTOR = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };

    Ok(StorageDescriptor {
        vendor: ansi_at(&buf, d.VendorIdOffset).unwrap_or_default(),
        product: ansi_at(&buf, d.ProductIdOffset).unwrap_or_default(),
        serial: ansi_at(&buf, d.SerialNumberOffset),
        removable: d.RemovableMedia != 0,
        bus_type: d.BusType as u32,
    })
}

/// Physical disk numbers making up a volume.
pub fn volume_disk_numbers(handle: &OwnedHandle) -> io::Result<Vec<u32>> {
    // A volume may span several disks; the buffer accommodates up to eight.
    let mut buf = vec![0u8; std::mem::size_of::<VOLUME_DISK_EXTENTS>() + 8 * 32];

    // SAFETY: output buffer with room for the struct and its extents.
    unsafe {
        control(
            handle.raw(),
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            std::ptr::null(),
            0,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
        )?
    };

    // SAFETY: the start of the buffer holds a `VOLUME_DISK_EXTENTS`; the
    // unaligned read is safe and avoids assuming the `Vec` is aligned.
    let header: VOLUME_DISK_EXTENTS = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
    let count = header.NumberOfDiskExtents as usize;

    // The first extent is embedded in the struct; the rest follow it.
    let base = std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents);
    let extent_size = 32; // DISK_EXTENT: u32 + padding + i64 + i64

    let mut out = Vec::with_capacity(count);
    for i in 0..count.min(8) {
        let at = base + i * extent_size;
        if at + 4 > buf.len() {
            break;
        }
        // SAFETY: `at` is inside the buffer and points at the `DiskNumber`
        // field, a `u32` at the start of each extent.
        let disk = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(at).cast::<u32>()) };
        out.push(disk);
    }
    Ok(out)
}

/// Moves the file pointer to an absolute offset.
pub fn seek(handle: &OwnedHandle, offset: u64) -> io::Result<()> {
    let mut new_pos: i64 = 0;
    // SAFETY: `handle.raw()` is valid and `new_pos` is an `i64` live for the call.
    let ok = unsafe { SetFilePointerEx(handle.raw(), offset as i64, &mut new_pos, FILE_BEGIN) };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Reads exactly `buf.len()` bytes. A short read is treated as an error.
pub fn read_exact(handle: &OwnedHandle, buf: &mut [u8]) -> io::Result<()> {
    let mut read: u32 = 0;
    // SAFETY: `buf` is a valid mutable slice of `buf.len()` bytes, and the
    // length fits in `u32` because scan blocks are megabytes.
    let ok = unsafe {
        ReadFile(
            handle.raw(),
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut read,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    if read as usize != buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("short read: {read} of {} bytes", buf.len()),
        ));
    }
    Ok(())
}

/// Writes exactly `buf.len()` bytes. A short write is treated as an error.
pub fn write_exact(handle: &OwnedHandle, buf: &[u8]) -> io::Result<()> {
    let mut written: u32 = 0;
    // SAFETY: `buf` is a valid, read-only slice of `buf.len()` bytes.
    let ok = unsafe {
        WriteFile(
            handle.raw(),
            buf.as_ptr().cast(),
            buf.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    if written as usize != buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("short write: {written} of {} bytes", buf.len()),
        ));
    }
    Ok(())
}

/// Asks the system to re-read the device's partition table.
pub fn update_disk_properties(handle: &OwnedHandle) -> io::Result<()> {
    control_simple(handle, IOCTL_DISK_UPDATE_PROPERTIES)
}

/// Enumerates the GUID identifiers of every volume on the machine.
pub fn enumerate_volume_guids() -> io::Result<Vec<String>> {
    let mut name = [0u16; MAX_PATH as usize];
    // SAFETY: `name` holds MAX_PATH elements, exactly the size declared.
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }

    let mut out = vec![from_wide(&name)];
    loop {
        // SAFETY: `find` is a valid search handle.raw() and `name` still holds
        // MAX_PATH elements.
        let more = unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) };
        if more == 0 {
            break;
        }
        out.push(from_wide(&name));
    }
    // SAFETY: encerra a busca aberta por `FindFirstVolumeW`.
    unsafe { FindVolumeClose(find) };
    Ok(out)
}

/// Mount points of a volume, for example `E:\`.
pub fn volume_mount_points(volume_guid: &str) -> io::Result<Vec<String>> {
    let wide = to_wide(volume_guid);
    let mut needed: u32 = 0;
    let mut buf = vec![0u16; 512];

    // SAFETY: `wide` is null-terminated and `buf` holds the reported size;
    // `needed` receives the total character count required.
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(
            wide.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Ok(Vec::new());
    }

    // The output is a sequence of null-terminated strings, closed by one extra
    // null.
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 0..buf.len() {
        if buf[i] == 0 {
            if i == start {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[start..i]));
            start = i + 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_conversion_round_trips() {
        let w = to_wide("PhysicalDrive3");
        assert_eq!(*w.last().unwrap(), 0, "a string precisa terminar em nulo");
        assert_eq!(from_wide(&w), "PhysicalDrive3");
    }

    #[test]
    fn from_wide_stops_at_the_first_nul() {
        assert_eq!(from_wide(&[72, 105, 0, 88, 88]), "Hi");
    }

    #[test]
    fn aligned_buffer_honours_its_alignment_and_zeroing() {
        let b = AlignedBuffer::new(8192, 4096);
        assert_eq!(b.len(), 8192);
        assert_eq!(b.as_slice().as_ptr() as usize % 4096, 0, "buffer desalinhado");
        assert!(b.as_slice().iter().all(|x| *x == 0));
    }

    #[test]
    fn aligned_buffer_is_writable_and_readable() {
        let mut b = AlignedBuffer::new(512, 512);
        b.as_mut_slice()[0] = 0xAB;
        b.as_mut_slice()[511] = 0xCD;
        assert_eq!(b.as_slice()[0], 0xAB);
        assert_eq!(b.as_slice()[511], 0xCD);
    }

    #[test]
    fn ansi_at_reads_embedded_strings() {
        let mut buf = vec![0u8; 32];
        buf[10..15].copy_from_slice(b"SDXC\0");
        assert_eq!(ansi_at(&buf, 10), Some("SDXC".into()));
        assert_eq!(ansi_at(&buf, 0), None, "deslocamento zero significa ausente");
        assert_eq!(ansi_at(&buf, 999), None, "deslocamento fora do buffer");
    }

    /// Locks in the privilege-escalation fix: the executable must come from an
    /// absolute path in the system directory, never from a `PATH` search — this
    /// process runs elevated.
    #[test]
    fn windows_executables_resolve_to_an_absolute_windows_path() {
        let p = windows_executable("explorer.exe").expect("the Windows directory must resolve");
        assert!(p.is_absolute(), "a relative path would still be resolved through PATH: {p:?}");
        let text = p.to_string_lossy().to_ascii_lowercase();
        assert!(text.ends_with(r"\explorer.exe"), "unexpected path: {p:?}");
        assert!(text.contains("windows"), "expected the Windows directory, got {p:?}");
    }

    #[test]
    fn system_executables_resolve_to_an_absolute_system_path() {
        let p = system_executable("format.com").expect("diretorio de sistema");
        assert!(p.is_absolute(), "caminho relativo permitiria substituicao: {p:?}");

        let text = p.to_string_lossy().to_lowercase();
        assert!(text.contains("system32"), "expected System32, got {p:?}");
        assert!(text.ends_with("format.com"));
        assert!(p.exists(), "o formatador do Windows deveria existir em {p:?}");
    }

    #[test]
    fn opening_a_nonexistent_device_fails_cleanly() {
        assert!(open_device("\\\\.\\PhysicalDrive250", Access::Read).is_err());
    }
}
