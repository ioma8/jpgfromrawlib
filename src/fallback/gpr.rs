use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::process::Command;

use tempfile::Builder;

pub fn decode_raw_preview(entry_path: &Path) -> Result<Vec<u8>> {
    let temp_dir = Builder::new().prefix("jpgfromraw-gpr").tempdir()?;
    let output_path = temp_dir.path().join("preview.jpg");
    let status = Command::new(tool_path("JPGFROMRAW_GPR_TOOLS", "gpr_tools"))
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

fn tool_path(variable: &str, executable: &str) -> std::path::PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        return path.into();
    }
    if let Some(path) = option_env!("GPR_TOOLS_BIN")
        && std::path::Path::new(path).exists()
    {
        return path.into();
    }
    executable.into()
}
