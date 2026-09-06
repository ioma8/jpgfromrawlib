use anyhow::{Context, Result, ensure};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageFormat};
use std::path::Path;
use std::process::Command;

use tempfile::Builder;

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let temp_dir = Builder::new().prefix("jpgfromraw-dcraw").tempdir()?;
    let temp_input = temp_dir.path().join(raw_name(entry_path)?);
    stage_input_file(entry_path, &temp_input)?;

    if let Some(jpeg) = try_dcraw_thumbnail(&temp_input)? {
        return Ok(jpeg);
    }

    let output_path = temp_input.with_extension("ppm");
    run_dcraw(&temp_input)?;
    let ppm = std::fs::read(&output_path)
        .with_context(|| format!("Failed to read dcraw output: {}", output_path.display()))?;
    let jpeg = ppm_to_jpeg(&ppm)?;
    Ok(jpeg)
}

fn try_dcraw_thumbnail(temp_input: &Path) -> Result<Option<Vec<u8>>> {
    let status = Command::new(tool_path("JPGFROMRAW_DCRAW", "jpgfromraw-dcraw"))
        .arg("-e")
        .arg(temp_input)
        .status();
    if !matches!(status, Ok(status) if status.success()) {
        return Ok(None);
    }

    for (ext, is_jpeg) in [("thumb.jpg", true), ("thumb.ppm", false)] {
        let path = temp_input.with_extension(ext);
        if let Ok(bytes) = std::fs::read(&path) {
            return if is_jpeg {
                Ok(is_complete_jpeg(&bytes).then_some(bytes))
            } else {
                Ok(Some(ppm_to_jpeg(&bytes)?))
            };
        }
    }

    Ok(None)
}

fn is_complete_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8]) && bytes.ends_with(&[0xFF, 0xD9])
}

fn run_dcraw(input_path: &Path) -> Result<()> {
    let status = Command::new(tool_path("JPGFROMRAW_DCRAW", "jpgfromraw-dcraw"))
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

fn tool_path(variable: &str, executable: &str) -> std::path::PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        return path.into();
    }
    if let Some(path) = option_env!("DCRAW_BIN")
        && std::path::Path::new(path).exists()
    {
        return path.into();
    }
    executable.into()
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
