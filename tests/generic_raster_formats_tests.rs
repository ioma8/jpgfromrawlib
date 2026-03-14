use anyhow::{ensure, Context, Result};
use jpgfromraw::{process_file_bytes, FindJpegType};
use std::path::{Path, PathBuf};

struct GenericSampleCase {
    format_name: &'static str,
    file_name: &'static str,
}

const GENERIC_SAMPLE_CASES: &[GenericSampleCase] = &[
    GenericSampleCase { format_name: "AVIF", file_name: "generic.avif" },
    GenericSampleCase { format_name: "BMP", file_name: "generic.bmp" },
    GenericSampleCase { format_name: "DDS", file_name: "generic.dds" },
    GenericSampleCase { format_name: "EXR", file_name: "generic.exr" },
    GenericSampleCase { format_name: "GIF", file_name: "generic.gif" },
    GenericSampleCase { format_name: "HDRi", file_name: "generic.hdr" },
    GenericSampleCase { format_name: "ICO", file_name: "generic.ico" },
    GenericSampleCase { format_name: "JFIF", file_name: "generic.jfif" },
    GenericSampleCase { format_name: "JPEG", file_name: "generic.jpg" },
    GenericSampleCase { format_name: "PAM", file_name: "generic.pam" },
    GenericSampleCase { format_name: "PBM", file_name: "generic.pbm" },
    GenericSampleCase { format_name: "PGM", file_name: "generic.pgm" },
    GenericSampleCase { format_name: "PNG", file_name: "generic.png" },
    GenericSampleCase { format_name: "PNM", file_name: "generic.pnm" },
    GenericSampleCase { format_name: "PPM", file_name: "generic.ppm" },
    GenericSampleCase { format_name: "PSD", file_name: "generic_embedded_thumb.psd" },
    GenericSampleCase { format_name: "QOI", file_name: "generic.qoi" },
    GenericSampleCase { format_name: "RGBE", file_name: "generic.rgbe" },
    GenericSampleCase { format_name: "TGA", file_name: "generic.tga" },
    GenericSampleCase { format_name: "TIFF", file_name: "generic.tiff" },
    GenericSampleCase { format_name: "TIFF/EP", file_name: "generic_tiff_ep.tif" },
    GenericSampleCase { format_name: "TIFF/IT", file_name: "generic_tiff_it.tif" },
    GenericSampleCase { format_name: "WebP", file_name: "generic.webp" },
];

fn sample_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("sample_generic")
}

fn sample_path(case: &GenericSampleCase) -> PathBuf {
    sample_root().join(case.file_name)
}

fn validate_jpeg(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.starts_with(&[0xFF, 0xD8]), "missing JPEG SOI");
    ensure!(bytes.ends_with(&[0xFF, 0xD9]), "missing JPEG EOI");
    Ok(())
}

#[test]
fn generic_samples_exist() -> Result<()> {
    for case in GENERIC_SAMPLE_CASES {
        let path = sample_path(case);
        ensure!(
            path.exists(),
            "missing generic sample for {}: {}",
            case.format_name,
            path.display()
        );
    }
    Ok(())
}

#[tokio::test]
async fn generic_samples_produce_valid_jpegs() -> Result<()> {
    for case in GENERIC_SAMPLE_CASES {
        let path = sample_path(case);
        let jpeg = process_file_bytes(&path, FindJpegType::Largest)
            .await
            .with_context(|| format!("failed to process {}", path.display()))?;
        validate_jpeg(&jpeg).with_context(|| format!("invalid JPEG for {}", path.display()))?;
    }
    Ok(())
}
