use anyhow::Result;
use memmap2::Mmap;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use super::EmbeddedJpegInfo;

pub fn mmap_raw(file: &File) -> Result<Mmap> {
    // SAFETY: mmap in general is unsafe because the lifecycle of the backing bytes are mutable
    // from outside the program.
    //
    // This means that, among other things, I/O errors can abort the program (e.g. by SIGBUS). This
    // is not a big problem, since we are just a command line program and have control over the
    // entire execution lifecycle.
    //
    // Also, any guarantees around validation (like taking a string slice from the &[u8]) are also
    // only enforced at creation time, so it's possible for the underlying file to cause corruption
    // (and thus UB). However, in our case, that's not a problem: we don't rely on such
    // enforcement.
    let raw_buf = unsafe { Mmap::map(file.as_raw_fd())? };
    Ok(raw_buf)
}

pub fn open_raw(path: &Path) -> Result<File> {
    Ok(File::open(path)?)
}

pub fn prefetch_jpeg(raw_buf: &Mmap, jpeg: &EmbeddedJpegInfo) -> Result<()> {
    let _ = (raw_buf, jpeg);
    Ok(())
}
