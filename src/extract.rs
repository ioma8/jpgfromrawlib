use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

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
    if output_aliases_input(&output_file, entry_path) {
        let jpeg_data = process_file_bytes(entry_path, find_type).await?;
        tokio::fs::write(&output_file, &jpeg_data).await?;
        return Ok(());
    }
    let (raw_buf, jpeg_info) = crate::parser_impl::map_embedded_jpeg(entry_path, find_type)?;
    if let Some(jpeg_info) = jpeg_info {
        let jpeg_buf = crate::parser_impl::jpeg::extract_jpeg(&raw_buf, &jpeg_info);
        let mut output = tokio::fs::File::create(&output_file).await?;
        output
            .write_all(&crate::parser_impl::jpeg::get_header_bytes(
                jpeg_info.orientation.unwrap_or(1),
            ))
            .await?;
        output.write_all(&jpeg_buf[2..]).await?;
        return Ok(());
    }
    let jpeg_data =
        crate::parser_impl::process_mapped_fallback(entry_path, find_type, raw_buf).await?;
    tokio::fs::write(&output_file, &jpeg_data).await?;
    Ok(())
}

fn output_aliases_input(output_file: &Path, input_file: &Path) -> bool {
    output_file == input_file
        || (output_file.exists()
            && std::fs::canonicalize(output_file).ok() == std::fs::canonicalize(input_file).ok())
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
