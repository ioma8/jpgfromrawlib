use anyhow::Result;
use byteorder::{BigEndian, ByteOrder};
use memchr::memmem;
use memmap2::Mmap;

use crate::platform;
use crate::types::FindJpegType;

use super::is_better;

/// An embedded JPEG in a RAW file.
#[derive(Clone, Copy)]
pub(crate) struct EmbeddedJpegInfo {
    pub(crate) offset: usize,
    pub(crate) length: usize,
    pub(crate) orientation: Option<u16>,
}

pub(crate) const JPEG_SOI_PREFIX: &[u8; 3] = b"\xFF\xD8\xFF";
pub(crate) const JPEG_SOI_MISSING_PREFIX: &[u8; 3] = b"\xD8\xFF\xDB";
pub(crate) const JPEG_EOI: &[u8; 2] = b"\xFF\xD9";

impl EmbeddedJpegInfo {
    pub(crate) fn new(offset: usize, length: usize, orientation: Option<u16>) -> Self {
        Self {
            offset,
            length,
            orientation,
        }
    }
}

/// Extract the JPEG bytes from the memory-mapped RAW buffer.
pub(crate) fn extract_jpeg<'raw>(raw_buf: &'raw Mmap, jpeg: &EmbeddedJpegInfo) -> &'raw [u8] {
    platform::prefetch_jpeg(raw_buf, jpeg);
    &raw_buf[jpeg.offset..jpeg.offset + jpeg.length]
}

/// Finds the exclusive end of the first JPEG EOI marker at or after a position, memoizing the
/// last hit so overlapping candidate scans do not rescan the same tail.
struct EoiScanner {
    finder: memmem::Finder<'static>,
    cached_end: Option<usize>,
}

impl EoiScanner {
    fn new() -> Self {
        Self {
            finder: memmem::Finder::new(JPEG_EOI),
            cached_end: None,
        }
    }

    /// `search_from` must be non-decreasing across calls on the same buffer for the memoized
    /// result to remain the first EOI at or after it.
    fn end_at_or_after(&mut self, buf: &[u8], search_from: usize) -> Option<usize> {
        if let Some(end) = self
            .cached_end
            .filter(|end| *end >= search_from + JPEG_EOI.len())
        {
            return Some(end);
        }
        let relative_end = self.finder.find(&buf[search_from..])?;
        let end = search_from + relative_end + JPEG_EOI.len();
        self.cached_end = Some(end);
        Some(end)
    }
}

pub(crate) fn find_embedded_jpeg_by_markers(
    raw_buf: &[u8],
    find_type: FindJpegType,
) -> Result<EmbeddedJpegInfo> {
    let soi_finder = memmem::Finder::new(JPEG_SOI_PREFIX);
    let mut eoi_scanner = EoiScanner::new();
    let mut best: Option<EmbeddedJpegInfo> = None;
    let mut search_start = 0;

    while let Some(relative_start) = soi_finder.find(&raw_buf[search_start..]) {
        let jpeg_start = search_start + relative_start;
        search_start = jpeg_start + JPEG_SOI_PREFIX.len();

        let Some(jpeg_end) = eoi_scanner.end_at_or_after(raw_buf, search_start) else {
            break;
        };
        let candidate_len = jpeg_end - jpeg_start;
        if !is_better(candidate_len, best.map_or(0, |jpeg| jpeg.length), find_type) {
            continue;
        }
        if is_valid_jpeg(&raw_buf[jpeg_start..jpeg_end]) {
            best = Some(EmbeddedJpegInfo::new(jpeg_start, candidate_len, None));
        }
    }

    best.ok_or_else(|| anyhow::anyhow!("No JPEG marker data found"))
}

pub(crate) fn find_embedded_jpeg_missing_ff_prefix(
    raw_buf: &[u8],
    find_type: FindJpegType,
) -> Result<Vec<u8>> {
    let soi_finder = memmem::Finder::new(JPEG_SOI_MISSING_PREFIX);
    let mut eoi_scanner = EoiScanner::new();
    let mut best: Option<Vec<u8>> = None;
    let mut search_start = 0;

    while let Some(relative_start) = soi_finder.find(&raw_buf[search_start..]) {
        let jpeg_start = search_start + relative_start;
        search_start = jpeg_start + 1;
        if jpeg_start > 0 && raw_buf[jpeg_start - 1] == 0xFF {
            continue;
        }

        let Some(jpeg_end) =
            eoi_scanner.end_at_or_after(raw_buf, jpeg_start + JPEG_SOI_MISSING_PREFIX.len())
        else {
            break;
        };
        let candidate_len = jpeg_end + 1 - jpeg_start;
        if !is_better(candidate_len, best.as_ref().map_or(0, Vec::len), find_type) {
            continue;
        }
        let mut jpeg = Vec::with_capacity(candidate_len);
        jpeg.push(0xFF);
        jpeg.extend_from_slice(&raw_buf[jpeg_start..jpeg_end]);
        if is_valid_jpeg(&jpeg) {
            best = Some(jpeg);
        }
    }

    best.ok_or_else(|| anyhow::anyhow!("No valid JPEG marker data found"))
}

pub(crate) fn is_valid_jpeg(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xFF, 0xD8]) || !bytes.ends_with(&[0xFF, 0xD9]) {
        return false;
    }

    let mut cursor = 2;
    let mut found_sof = false;

    while cursor + 3 < bytes.len() {
        if bytes[cursor] != 0xFF {
            return false;
        }
        while cursor < bytes.len() && bytes[cursor] == 0xFF {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return false;
        }

        let marker = bytes[cursor];
        cursor += 1;

        if marker == 0xD9 {
            break;
        }
        if marker == 0xDA {
            return found_sof;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if cursor + 1 >= bytes.len() {
            return false;
        }

        let segment_len = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
        if segment_len < 2 {
            return false;
        }
        let segment_start = cursor + 2;
        let segment_end = cursor + segment_len;
        if segment_end > bytes.len() {
            return false;
        }

        if is_sof_marker(marker) {
            if segment_end - segment_start < 6 {
                return false;
            }
            let height = u16::from_be_bytes([bytes[segment_start + 1], bytes[segment_start + 2]]);
            let width = u16::from_be_bytes([bytes[segment_start + 3], bytes[segment_start + 4]]);
            if width == 0 || height == 0 || bytes[segment_start] == 0 {
                return false;
            }
            found_sof = true;
        }

        cursor = segment_end;
    }

    found_sof
}

pub const fn is_sof_marker(marker: u8) -> bool {
    marker >= 0xC0 && marker <= 0xCF && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// The embedded JPEG comes with no EXIF data. While most of it is outside of the scope of this
/// application, it's pretty vexing to have the wrong orientation, so copy that over.
#[rustfmt::skip]
pub(crate) const fn get_header_bytes(orientation: u16) -> [u8; 34] {
    let orientation_bytes = orientation.to_le_bytes();
    [
        0xff, 0xd8, // SOI
        0xff, 0xe1, // APP1
        0x00, 0x1e, // 30 bytes including this length
        0x45, 0x78, 0x69, 0x66, 0x00, 0x00, // Exif\0\0
        0x49, 0x49, 0x2A, 0x00, // TIFF LE
        0x08, 0x00, 0x00, 0x00, // Offset to IFD
        0x01, 0x00, // One entry in IFD
        0x12, 0x01, // Tag for orientation
        0x03, 0x00, // Type: SHORT
        0x01, 0x00, 0x00, 0x00, // Count: 1
        orientation_bytes[0], orientation_bytes[1], // Orientation
        0x00, 0x00, // Next IFD
    ]
}

pub(crate) fn get_jpeg_data(jpeg_buf: &[u8], jpeg_info: &EmbeddedJpegInfo) -> Vec<u8> {
    let mut jpeg_data = Vec::with_capacity(jpeg_buf.len() + 34);
    jpeg_data.extend_from_slice(&get_header_bytes(jpeg_info.orientation.unwrap_or(1)));
    jpeg_data.extend_from_slice(&jpeg_buf[2..]);
    jpeg_data
}

pub(crate) fn try_missing_ff(raw_buf: &[u8], find_type: FindJpegType) -> Option<Vec<u8>> {
    let jpeg_buf = find_embedded_jpeg_missing_ff_prefix(raw_buf, find_type).ok()?;
    Some(get_jpeg_data(
        &jpeg_buf,
        &EmbeddedJpegInfo::new(0, jpeg_buf.len(), None),
    ))
}

pub(crate) fn find_raf_jpeg(raw_buf: &[u8]) -> Option<EmbeddedJpegInfo> {
    if !raw_buf.starts_with(b"FUJIFILMCCD-RAW") || raw_buf.len() < 92 {
        return None;
    }
    let offset = usize::try_from(BigEndian::read_u32(&raw_buf[84..88])).ok()?;
    let length = usize::try_from(BigEndian::read_u32(&raw_buf[88..92])).ok()?;
    let end = offset.checked_add(length)?;
    if end > raw_buf.len() || !raw_buf[offset..].starts_with(JPEG_SOI_PREFIX) {
        return None;
    }
    Some(EmbeddedJpegInfo::new(offset, length, None))
}
