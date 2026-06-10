use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, ImageFormat};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::Path;

const JPEG_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jpe", "jfif", "jps"];
const GENERIC_EXTENSIONS: &[&str] = &[
    "apng", "bmp", "exif", "gif", "hdr", "ico", "jfif", "jpe", "jpeg", "jpg", "jps", "pam", "pbm",
    "pgm", "png", "pnm", "ppm", "qoi", "rgbe", "tga", "tif", "tiff", "webp",
];
// Decoders for these formats are only compiled in with the generic-raster feature, so without it
// the extensions must not be claimed: matching them would fail at decode time instead of falling
// through to the RAW fallbacks.
#[cfg(feature = "generic-raster")]
const FEATURE_GATED_EXTENSIONS: &[&str] = &["avif", "dds", "exr"];
#[cfg(not(feature = "generic-raster"))]
const FEATURE_GATED_EXTENSIONS: &[&str] = &[];

pub fn decode_generic_raster_preview<'a>(
    entry_path: &Path,
    bytes: &'a [u8],
) -> Result<Option<Cow<'a, [u8]>>> {
    if bytes.is_empty() {
        return Ok(None);
    }

    if !is_generic_extension(entry_path) {
        return Ok(None);
    }

    if is_jpeg_extension(entry_path) || matches!(guess_format(bytes), Some(ImageFormat::Jpeg)) {
        return Ok(Some(Cow::Borrowed(bytes)));
    }

    let Some(format) = detect_format(entry_path, bytes) else {
        return Ok(None);
    };

    let image = image::load_from_memory_with_format(bytes, format)
        .with_context(|| format!("failed to decode generic raster as {format:?}"))?;
    let jpeg = encode_as_jpeg(&image)?;
    Ok(Some(Cow::Owned(jpeg)))
}

pub fn is_generic_extension(entry_path: &Path) -> bool {
    has_extension(entry_path, GENERIC_EXTENSIONS)
        || has_extension(entry_path, FEATURE_GATED_EXTENSIONS)
}

fn detect_format(entry_path: &Path, bytes: &[u8]) -> Option<ImageFormat> {
    image_format_from_extension(entry_path).or_else(|| guess_format(bytes))
}

fn image_format_from_extension(entry_path: &Path) -> Option<ImageFormat> {
    let extension = entry_path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "apng" => Some(ImageFormat::Png),
        #[cfg(feature = "generic-raster")]
        "avif" => Some(ImageFormat::Avif),
        "bmp" => Some(ImageFormat::Bmp),
        #[cfg(feature = "generic-raster")]
        "dds" => Some(ImageFormat::Dds),
        "exif" => Some(ImageFormat::Jpeg),
        #[cfg(feature = "generic-raster")]
        "exr" => Some(ImageFormat::OpenExr),
        "gif" => Some(ImageFormat::Gif),
        "hdr" | "rgbe" => Some(ImageFormat::Hdr),
        "ico" => Some(ImageFormat::Ico),
        "jfif" | "jpe" | "jpeg" | "jpg" | "jps" => Some(ImageFormat::Jpeg),
        "pam" => Some(ImageFormat::Pnm),
        "pbm" => Some(ImageFormat::Pnm),
        "pgm" => Some(ImageFormat::Pnm),
        "pnm" => Some(ImageFormat::Pnm),
        "ppm" => Some(ImageFormat::Pnm),
        "png" => Some(ImageFormat::Png),
        "qoi" => Some(ImageFormat::Qoi),
        "tga" => Some(ImageFormat::Tga),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn guess_format(bytes: &[u8]) -> Option<ImageFormat> {
    image::guess_format(bytes).ok()
}

fn is_jpeg_extension(entry_path: &Path) -> bool {
    has_extension(entry_path, JPEG_EXTENSIONS)
}

fn has_extension(entry_path: &Path, known_extensions: &[&str]) -> bool {
    entry_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| known_extensions.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn encode_as_jpeg(image: &DynamicImage) -> Result<Vec<u8>> {
    let rgb = image.to_rgb8();
    let mut output = Vec::new();
    let mut cursor = Cursor::new(&mut output);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 95);
    encoder.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        ExtendedColorType::Rgb8,
    )?;
    Ok(output)
}
