use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::types::FindJpegType;

pub async fn process_file(
    entry_path: &Path,
    out_dir: &Path,
    relative_path: &Path,
    find_type: FindJpegType,
) -> Result<()> {
    let output_file = output_path_for_relative_path(out_dir, relative_path);
    if let Some(parent) = output_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let jpeg_data = process_file_bytes(entry_path, find_type).await?;
    tokio::fs::write(&output_file, &jpeg_data).await?;
    Ok(())
}

pub async fn process_file_bytes(entry_path: &Path, find_type: FindJpegType) -> Result<Vec<u8>> {
    crate::parser_impl::process_file_bytes(entry_path, find_type).await
}

fn output_path_for_relative_path(out_dir: &Path, relative_path: &Path) -> PathBuf {
    let mut output_file = out_dir.join(relative_path);
    output_file.set_extension("jpg");
    output_file
}

#[cfg(test)]
mod tests {
    use super::output_path_for_relative_path;
    use std::path::Path;

    #[test]
    fn output_path_replaces_the_input_extension_with_jpg() {
        let out =
            output_path_for_relative_path(Path::new("/tmp/out"), Path::new("nested/file.arw"));
        assert_eq!(out, Path::new("/tmp/out/nested/file.jpg"));
    }
}
