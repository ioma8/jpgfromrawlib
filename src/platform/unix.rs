use anyhow::Result;
use memmap2::Advice;
use memmap2::Mmap;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use crate::parser_impl::jpeg::EmbeddedJpegInfo;

pub fn mmap_raw(file: &File) -> Result<Mmap> {
    // SAFETY: mmap in general is unsafe because the lifecycle of the backing bytes are mutable
    // from outside the program.
    //
    // The mapping remains valid while the returned Mmap is alive, but concurrent truncation or
    // mutation of the backing file is outside this API's safety contract. On Unix, truncation can
    // cause SIGBUS when a later access faults in an invalidated page; callers must process stable
    // input files and must not mutate or replace them while extraction is running.
    let raw_buf = unsafe { Mmap::map(file.as_raw_fd())? };
    // We only touch the IFD chain and the embedded JPEG range, so disabling readahead avoids
    // paging in megabytes we never read. Advice is an optimization only; failure is harmless.
    let _ = raw_buf.advise(Advice::Random);
    Ok(raw_buf)
}

pub fn open_raw(path: &Path) -> Result<File> {
    Ok(File::open(path)?)
}

pub fn prefetch_jpeg(raw_buf: &Mmap, jpeg: &EmbeddedJpegInfo) {
    // Advice is an optimization only; failure is harmless.
    let _ = raw_buf.advise_range(Advice::WillNeed, jpeg.offset, jpeg.length);
}
