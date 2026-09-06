#[cfg(any(
    feature = "raster-fallback",
    feature = "gpr-fallback",
    feature = "dcraw-fallback"
))]
use anyhow::Context;
use anyhow::Result;
#[cfg(not(feature = "dcraw-fallback"))]
use anyhow::bail;
use memmap2::Mmap;
use std::path::Path;

#[cfg(feature = "dcraw-fallback")]
pub mod dcraw;
#[cfg(feature = "raster-fallback")]
pub mod generic;
#[cfg(feature = "gpr-fallback")]
pub mod gpr;

#[cfg(any(
    feature = "raster-fallback",
    feature = "tiff-preview",
    feature = "dcraw-fallback"
))]
pub(crate) fn jpeg_output_capacity(input_len: usize) -> usize {
    input_len.saturating_div(3).max(1024)
}

pub(crate) async fn decode_raw_preview_fallback(
    entry_path: &Path,
    mapped: Option<Mmap>,
) -> Result<Vec<u8>> {
    let mapped = mapped;
    #[cfg(feature = "raster-fallback")]
    let mut mapped = mapped;
    #[cfg(feature = "raster-fallback")]
    if generic::is_generic_extension(entry_path) {
        let generic_path = entry_path.to_path_buf();
        let generic_mapped = mapped.take();
        if let Some(jpeg) = run_blocking_fallback("generic raster", move || {
            if let Some(mapped) = generic_mapped {
                generic::decode_generic_raster_preview(&generic_path, &mapped)
                    .map(|jpeg| jpeg.map(std::borrow::Cow::into_owned))
            } else {
                let bytes = std::fs::read(&generic_path).with_context(|| {
                    format!(
                        "failed to read generic raster source {}",
                        generic_path.display()
                    )
                })?;
                generic::decode_generic_raster_preview(&generic_path, &bytes)
                    .map(|jpeg| jpeg.map(std::borrow::Cow::into_owned))
            }
        })
        .await?
        {
            return Ok(jpeg);
        }
    }

    drop(mapped);
    #[cfg(feature = "gpr-fallback")]
    let extension = entry_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    let entry_path = entry_path.to_path_buf();

    #[cfg(feature = "gpr-fallback")]
    if extension.as_deref() == Some("gpr") {
        return run_blocking_fallback("gpr", move || gpr::decode_raw_preview(&entry_path)).await;
    }

    #[cfg(feature = "dcraw-fallback")]
    return run_blocking_fallback("dcraw", move || dcraw::decode_raw_preview(&entry_path)).await;

    #[cfg(not(feature = "dcraw-fallback"))]
    bail!(
        "no preview found and no fallback is enabled for {}",
        entry_path.display()
    )
}

#[cfg(any(
    feature = "raster-fallback",
    feature = "gpr-fallback",
    feature = "dcraw-fallback"
))]
pub(crate) async fn run_blocking_fallback<T, F>(label: &str, decode: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(decode)
            .await
            .with_context(|| format!("{label} fallback task failed"))?,
        Err(_) => decode(),
    }
}
