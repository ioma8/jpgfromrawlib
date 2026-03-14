use anyhow::{ensure, Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageFormat};
use std::ffi::{c_char, c_int, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static DCRAW_LOCK: Mutex<()> = Mutex::new(());
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" {
    fn jpgfromraw_dcraw_main(argc: c_int, argv: *mut *const c_char) -> c_int;
}

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let _guard = DCRAW_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("dcraw lock poisoned"))?;
    let temp_dir = make_temp_dir()?;
    let temp_input = temp_dir.join(raw_name(entry_path)?);
    std::fs::copy(entry_path, &temp_input).with_context(|| {
        format!(
            "Failed to copy RAW file for dcraw fallback: {}",
            entry_path.display()
        )
    })?;

    let output_path = temp_input.with_extension("ppm");
    run_dcraw(&temp_input)?;
    let ppm = std::fs::read(&output_path)
        .with_context(|| format!("Failed to read dcraw output: {}", output_path.display()))?;
    let jpeg = ppm_to_jpeg(&ppm)?;
    let _ = std::fs::remove_file(&output_path);
    let _ = std::fs::remove_file(&temp_input);
    let _ = std::fs::remove_dir(&temp_dir);
    Ok(jpeg)
}

fn run_dcraw(input_path: &Path) -> Result<()> {
    let arg0 = CString::new("dcraw")?;
    let arg1 = CString::new("-h")?;
    let arg2 = CString::new("-W")?;
    let arg3 = CString::new("-o")?;
    let arg4 = CString::new("1")?;
    let arg5 = CString::new(input_path.as_os_str().to_string_lossy().as_bytes())?;
    let empty = CString::new("")?;
    let mut argv = vec![
        arg0.as_ptr(),
        arg1.as_ptr(),
        arg2.as_ptr(),
        arg3.as_ptr(),
        arg4.as_ptr(),
        arg5.as_ptr(),
        empty.as_ptr(),
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

fn make_temp_dir() -> Result<PathBuf> {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.push(format!("jpgfromraw-dcraw-{nanos}-{counter}"));
    std::fs::create_dir(&dir)
        .with_context(|| format!("Failed to create dcraw temp dir: {}", dir.display()))?;
    Ok(dir)
}

fn raw_name(entry_path: &Path) -> Result<&std::ffi::OsStr> {
    entry_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("RAW path has no file name: {}", entry_path.display()))
}
