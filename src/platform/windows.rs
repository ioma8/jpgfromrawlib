use anyhow::Result;
use memmap2::Mmap;
use std::fs::File;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows::Win32::Storage::FileSystem::FILE_FLAG_RANDOM_ACCESS;

use crate::parser_impl::jpeg::EmbeddedJpegInfo;

pub fn mmap_raw(file: &File) -> Result<Mmap> {
    // SAFETY: the mapping requires a stable backing file for its lifetime; callers must not
    // concurrently mutate, truncate, or replace the input while extraction is running.
    let raw_buf = unsafe { Mmap::map(file.as_raw_handle())? };
    Ok(raw_buf)
}

pub fn open_raw(path: &Path) -> Result<File> {
    // There's no MADV_RANDOM equivalent we can apply to the mapping, so request random access at
    // open time instead. See mmap_raw in unix.rs for why we advise random access in general.
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_RANDOM_ACCESS.0)
        .open(path)?)
}

pub fn prefetch_jpeg(raw_buf: &Mmap, jpeg: &EmbeddedJpegInfo) {
    use windows::Win32::System::Memory::{PrefetchVirtualMemory, WIN32_MEMORY_RANGE_ENTRY};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let range = WIN32_MEMORY_RANGE_ENTRY {
        VirtualAddress: raw_buf.as_ptr().wrapping_add(jpeg.offset) as *mut core::ffi::c_void,
        NumberOfBytes: jpeg.length,
    };
    // SAFETY: the range lies within the live mapping; callers validate offset and length against
    // the map bounds before extraction. Prefetch is an optimization only; failure is harmless.
    unsafe {
        let _ = PrefetchVirtualMemory(GetCurrentProcess(), &[range], 0);
    }
}
