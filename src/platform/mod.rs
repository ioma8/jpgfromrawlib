#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::{mmap_raw, open_raw, prefetch_jpeg};
#[cfg(windows)]
pub(crate) use windows::{mmap_raw, open_raw, prefetch_jpeg};
