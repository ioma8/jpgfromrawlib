use anyhow::{Context, Result};
use std::path::Path;

pub mod dcraw;
pub mod generic;
pub mod gpr;
mod temp_dir;

pub(crate) fn jpeg_output_capacity(input_len: usize) -> usize {
    input_len.saturating_div(3).max(1024)
}

pub(crate) async fn decode_raw_preview_fallback(entry_path: &Path) -> Result<Vec<u8>> {
    if generic::is_generic_extension(entry_path) {
        let generic_path = entry_path.to_path_buf();
        if let Some(jpeg) = run_blocking_fallback("generic raster", move || {
            let bytes = std::fs::read(&generic_path).with_context(|| {
                format!(
                    "failed to read generic raster source {}",
                    generic_path.display()
                )
            })?;
            generic::decode_generic_raster_preview(&generic_path, &bytes)
                .map(|jpeg| jpeg.map(std::borrow::Cow::into_owned))
        })
        .await?
        {
            return Ok(jpeg);
        }
    }

    let extension = entry_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    let entry_path = entry_path.to_path_buf();

    if extension.as_deref() == Some("gpr") {
        return run_blocking_fallback("gpr", move || gpr::decode_raw_preview(&entry_path)).await;
    }

    run_blocking_fallback("dcraw", move || dcraw::decode_raw_preview(&entry_path)).await
}

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
