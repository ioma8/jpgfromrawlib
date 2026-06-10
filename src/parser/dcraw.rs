use anyhow::{ensure, Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageFormat};
use std::ffi::{c_char, c_int, CString};
use std::path::Path;
use std::sync::Mutex;

use super::temp_dir::TempDirGuard;

static DCRAW_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn jpgfromraw_dcraw_main(argc: c_int, argv: *mut *const c_char) -> c_int;
}

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let _guard = DCRAW_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("dcraw lock poisoned"))?;
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
    let arg0 = CString::new("dcraw")?;
    let arg1 = CString::new("-h")?;
    let arg2 = CString::new("-W")?;
    let arg3 = CString::new("-o")?;
    let arg4 = CString::new("1")?;
    let arg5 = CString::new(input_path.as_os_str().to_string_lossy().as_bytes())?;
    let mut argv = vec![
        arg0.as_ptr(),
        arg1.as_ptr(),
        arg2.as_ptr(),
        arg3.as_ptr(),
        arg4.as_ptr(),
        arg5.as_ptr(),
        std::ptr::null(),
    ];

    let status = unsafe { jpgfromraw_dcraw_main(6, argv.as_mut_ptr()) };
    ensure!(status == 0, "dcraw failed with status {status}");
    Ok(())
}

fn ppm_to_jpeg(ppm: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory_with_format(ppm, ImageFormat::Pnm)
        .context("Failed to decode dcraw PPM output")?;
    let rgb = image.into_rgb8();
    let mut jpeg = Vec::new();
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
