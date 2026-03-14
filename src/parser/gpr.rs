use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static GPR_LOCK: Mutex<()> = Mutex::new(());
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let _guard = GPR_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("gpr lock poisoned"))?;

    let temp_dir = make_temp_dir()?;
    let output_path = temp_dir.join("preview.jpg");
    let status = Command::new(env!("GPR_TOOLS_BIN"))
        .arg("-i")
        .arg(entry_path)
        .arg("-o")
        .arg(&output_path)
        .status()
        .context("Failed to launch gpr_tools")?;
    ensure!(status.success(), "gpr_tools failed with status {status}");

    let jpeg = std::fs::read(&output_path)
        .with_context(|| format!("Failed to read gpr_tools output: {}", output_path.display()))?;
    let _ = std::fs::remove_file(&output_path);
    let _ = std::fs::remove_dir(&temp_dir);
    Ok(jpeg)
}

fn make_temp_dir() -> Result<std::path::PathBuf> {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.push(format!("jpgfromraw-gpr-{nanos}-{counter}"));
    std::fs::create_dir(&dir)
        .with_context(|| format!("Failed to create gpr temp dir: {}", dir.display()))?;
    Ok(dir)
}
