use anyhow::{Context, Result, ensure};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageFormat};
use std::path::Path;
use std::process::Command;

use super::temp_dir::TempDirGuard;

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let temp_dir = TempDirGuard::new("jpgfromraw-dcraw")?;
    let temp_input = temp_dir.path().join(raw_name(entry_path)?);
    stage_input_file(entry_path, &temp_input)?;

    let output_path = temp_input.with_extension("ppm");
    run_dcraw(&temp_input)?;
    let ppm = std::fs::read(&output_path)
        .with_context(|| format!("Failed to read dcraw output: {}", output_path.display()))?;
    let jpeg = ppm_to_jpeg(&ppm)?;
    Ok(jpeg)
}

fn run_dcraw(input_path: &Path) -> Result<()> {
    let status = Command::new(env!("DCRAW_BIN"))
        .arg("-h")
        .arg("-W")
        .arg("-o")
        .arg("1")
        .arg(input_path)
        .status()
        .context("Failed to launch dcraw fallback")?;
    ensure!(status.success(), "dcraw failed with status {status}");
    Ok(())
}

fn ppm_to_jpeg(ppm: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory_with_format(ppm, ImageFormat::Pnm)
        .context("Failed to decode dcraw PPM output")?;
    let rgb = image.into_rgb8();
    let mut jpeg = Vec::with_capacity(super::jpeg_output_capacity(rgb.as_raw().len()));
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 95);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .context("Failed to encode dcraw preview as JPEG")?;
    Ok(jpeg)
}

fn stage_input_file(entry_path: &Path, temp_input: &Path) -> Result<()> {
    match std::fs::hard_link(entry_path, temp_input) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::copy(entry_path, temp_input)
            .map(|_| ())
            .with_context(|| {
                format!(
                    "Failed to stage RAW file for dcraw fallback: {}",
                    entry_path.display()
                )
            }),
    }
}

fn raw_name(entry_path: &Path) -> Result<&std::ffi::OsStr> {
    entry_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("RAW path has no file name: {}", entry_path.display()))
}
