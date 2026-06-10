use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::process::Command;

use super::temp_dir::TempDirGuard;

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let temp_dir = TempDirGuard::new("jpgfromraw-gpr")?;
    let output_path = temp_dir.path().join("preview.jpg");
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
    Ok(jpeg)
}
