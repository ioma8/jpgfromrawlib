use anyhow::Result;
use memmap2::Mmap;
use std::fs::File;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows::Win32::Storage::FileSystem::FILE_FLAG_RANDOM_ACCESS;

use super::EmbeddedJpegInfo;

pub fn mmap_raw(file: &File) -> Result<Mmap> {
    // SAFETY: see comment in unix.rs
    let raw_buf = unsafe { Mmap::map(file.as_raw_handle())? };
    Ok(raw_buf)
}

pub fn open_raw(path: &Path) -> Result<File> {
    // There's no MADV_RANDOM equivalent, we have to do it at open time. See unix.rs for why we do
    // this in general.
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_RANDOM_ACCESS.0)
        .open(path)?)
}

pub fn prefetch_jpeg(raw_buf: &Mmap, jpeg: &EmbeddedJpegInfo) -> Result<()> {
    let _ = (raw_buf, jpeg);
    Ok(())
}
