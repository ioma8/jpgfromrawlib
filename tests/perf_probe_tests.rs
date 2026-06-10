use anyhow::{Result, ensure};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir() -> Result<PathBuf> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("jpgfromraw-perf-probe-{timestamp}"));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn count_files(dir: &Path, extension: &str) -> Result<usize> {
    let count = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == extension))
        .count();
    Ok(count)
}

fn perf_probe_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_perf_probe") {
        return PathBuf::from(path);
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("target/debug/perf_probe")
}

#[test]
fn perf_probe_writes_jpegs_for_test_images() -> Result<()> {
    let input_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_images");
    let output_dir = unique_temp_dir()?;
    let perf_probe = perf_probe_binary();

    let status = Command::new(perf_probe)
        .arg(&input_dir)
        .arg(&output_dir)
        .status()?;

    ensure!(status.success(), "perf_probe binary failed");

    let raw_count = count_files(&input_dir, "arw")?;
    let jpg_count = count_files(&output_dir, "jpg")?;

    ensure!(raw_count > 0, "expected test_images to contain raw files");
    ensure!(
        jpg_count == raw_count,
        "expected {raw_count} jpgs, got {jpg_count}"
    );

    Ok(())
}
