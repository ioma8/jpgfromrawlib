use anyhow::{ensure, Context, Result};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use image::codecs::jpeg::JpegEncoder;
use image::ExtendedColorType;
use memchr::memmem;
use memmap2::Mmap;
use std::path::Path;

#[cfg(unix)]
mod unix;

mod dcraw;
mod generic;
mod gpr;
mod temp_dir;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// An embedded JPEG in a RAW file.
#[derive(Clone, Copy)]
pub struct EmbeddedJpegInfo {
    offset: usize,
    length: usize,
    orientation: Option<u16>,
}

#[derive(Clone, Copy)]
pub enum FindJpegType {
    Largest,
    Smallest,
}

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "3fr", "ari", "arw", "bay", "braw", "cap", "cr2", "cr3", "cri", "crw", "dcr", "dcs", "dng",
    "drf", "eip", "erf", "fff", "gpr", "iiq", "jxs", "k25", "kdc", "mdc", "mef", "mos", "mrw",
    "nef", "nrw", "orf", "pef", "ptx", "pxn", "r3d", "raf", "raw", "rw2", "rwl", "rwz", "sr2",
    "srf", "srw", "tco", "x3f",
];

const TIFF_HEADER: &[u8; 4] = b"II*\0";
const TIFF_HEADERMM: &[u8; 4] = b"MM\0*";
const EXIF_HEADER: &[u8; 6] = b"Exif\0\0";
const EXIF_HEADER_SIZE: usize = 6;
const EXIF_TIFF_SEARCH_WINDOW: usize = 10;
const JPEG_SOI_PREFIX: &[u8; 3] = b"\xFF\xD8\xFF";
const JPEG_SOI_MISSING_PREFIX: &[u8; 3] = b"\xD8\xFF\xDB";
const JPEG_EOI: &[u8; 2] = b"\xFF\xD9";
const IFD_ENTRY_SIZE: usize = 12;
const MAX_IFD_CHAIN_LEN: usize = 64;
const JPEG_TAG: u16 = 0x201;
const JPEG_LENGTH_TAG: u16 = 0x202;
const TAG_IMAGE_WIDTH: u16 = 0x100;
const TAG_IMAGE_LENGTH: u16 = 0x101;
const TAG_BITS_PER_SAMPLE: u16 = 0x102;
const TAG_COMPRESSION: u16 = 0x103;
const TAG_PHOTOMETRIC_INTERPRETATION: u16 = 0x106;
const TAG_STRIP_OFFSETS: u16 = 0x111;
const TAG_ORIENTATION: u16 = 0x112;
const TAG_SAMPLES_PER_PIXEL: u16 = 0x115;
const TAG_ROWS_PER_STRIP: u16 = 0x116;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x117;

#[derive(Debug, Default)]
struct TiffPreviewInfo {
    strip_offsets: Vec<usize>,
    strip_byte_counts: Vec<usize>,
    width: u32,
    height: u32,
    bits_per_sample: Vec<u16>,
    compression: Option<u16>,
    photometric_interpretation: Option<u16>,
    orientation: Option<u16>,
    samples_per_pixel: Option<u16>,
    rows_per_strip: Option<u32>,
}

impl TiffPreviewInfo {
    fn encoded_len(&self) -> usize {
        self.strip_byte_counts.iter().sum()
    }
}

impl EmbeddedJpegInfo {
    fn new(offset: usize, length: usize, orientation: Option<u16>) -> Self {
        Self {
            offset,
            length,
            orientation,
        }
    }
}

struct TiffReader<'a> {
    buf: &'a [u8],
    is_le: bool,
}

impl<'a> TiffReader<'a> {
    fn read_u16(&self, bytes: &[u8]) -> u16 {
        if self.is_le {
            LittleEndian::read_u16(bytes)
        } else {
            BigEndian::read_u16(bytes)
        }
    }

    fn read_u32(&self, bytes: &[u8]) -> u32 {
        if self.is_le {
            LittleEndian::read_u32(bytes)
        } else {
            BigEndian::read_u32(bytes)
        }
    }

    fn field_bytes(&self, type_id: u16, count: u32, value_field: &'a [u8]) -> Result<&'a [u8]> {
        let unit_size = match type_id {
            3 => 2usize,
            4 => 4usize,
            _ => return Err(anyhow::anyhow!("Unsupported TIFF field type: {type_id}")),
        };
        let count = usize::try_from(count)?;
        let total_size = unit_size
            .checked_mul(count)
            .ok_or_else(|| anyhow::anyhow!("TIFF field size overflow"))?;

        if total_size <= value_field.len() {
            return Ok(&value_field[..total_size]);
        }

        let offset = usize::try_from(self.read_u32(value_field))?;
        let end = offset
            .checked_add(total_size)
            .ok_or_else(|| anyhow::anyhow!("TIFF field range overflow"))?;
        ensure!(end <= self.buf.len(), "Invalid TIFF field range");
        Ok(&self.buf[offset..end])
    }

    fn read_u16_values(&self, type_id: u16, count: u32, value_field: &'a [u8]) -> Result<Vec<u16>> {
        ensure!(type_id == 3, "Unsupported TIFF u16 field type: {type_id}");
        let bytes = self.field_bytes(type_id, count, value_field)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|chunk| self.read_u16(chunk))
            .collect())
    }

    fn read_u32_values(&self, type_id: u16, count: u32, value_field: &'a [u8]) -> Result<Vec<u32>> {
        let bytes = self.field_bytes(type_id, count, value_field)?;
        match type_id {
            3 => Ok(bytes
                .chunks_exact(2)
                .map(|chunk| u32::from(self.read_u16(chunk)))
                .collect()),
            4 => Ok(bytes
                .chunks_exact(4)
                .map(|chunk| self.read_u32(chunk))
                .collect()),
            _ => Err(anyhow::anyhow!(
                "Unsupported TIFF u32 field type: {type_id}"
            )),
        }
    }
}

fn find_tiff_header_offset(raw_buf: &[u8]) -> Result<usize> {
    ensure!(raw_buf.len() >= TIFF_HEADER.len(), "Not enough data for TIFF header");

    if raw_buf.starts_with(TIFF_HEADER) || raw_buf.starts_with(TIFF_HEADERMM) {
        return Ok(0);
    }

    let found = memmem::find(raw_buf, EXIF_HEADER);
    if let Some(pos) = found {
        // APP1 stores Exif\0\0 after the marker and length bytes, so the TIFF header should land
        // within the next 10 bytes.
        if pos < 10 {
            return Err(anyhow::anyhow!("Invalid Exif header position: {}", pos));
        }
        let slice_start = pos + EXIF_HEADER_SIZE;
        ensure!(slice_start < raw_buf.len(), "Invalid Exif header position");
        let slice_end = slice_start + EXIF_TIFF_SEARCH_WINDOW;
        ensure!(slice_end <= raw_buf.len(), "Invalid Exif header position");
        let slice = &raw_buf[slice_start..slice_end];
        let tiff_found = memmem::find(slice, TIFF_HEADER);
        if let Some(tiff_pos) = tiff_found {
            return Ok(slice_start + tiff_pos);
        }
        let tiff_found = memmem::find(slice, TIFF_HEADERMM);
        if let Some(tiff_pos) = tiff_found {
            return Ok(slice_start + tiff_pos);
        }
        return Ok(slice_start);
    }
    Err(anyhow::anyhow!(
        "No Exif APP1 segment with TIFF header found"
    ))
}
/// Find the largest embedded JPEG data in a memory-mapped RAW buffer.
///
/// This function parses the IFDs in the TIFF structure of the RAW file to find the largest JPEG
/// thumbnail embedded in the file.
///
/// We hand roll the IFD parsing because libraries do not fit requirements. For example:
///
/// - kamadak-exif: Reads into a big `Vec<u8>`, which is huge for our big RAW.
/// - quickexif: Cannot iterate over IFDs.
fn find_embedded_jpeg_in_tiff(
    raw_buf: &[u8],
    tiff_offset: usize,
    find_type: FindJpegType,
) -> Result<Option<EmbeddedJpegInfo>> {
    let raw_buf = &raw_buf[tiff_offset..];
    let reader = tiff_reader(raw_buf)?;
    let mut next_ifd_offset = usize::try_from(reader.read_u32(&raw_buf[4..8]))?;
    let mut best: Option<EmbeddedJpegInfo> = None;
    let mut chain_guard = IfdChainGuard::new();

    while next_ifd_offset != 0 {
        chain_guard.visit(next_ifd_offset)?;
        ensure!(next_ifd_offset + 2 <= raw_buf.len(), "Invalid IFD offset");

        let cursor = &raw_buf[next_ifd_offset..];
        let num_entries = usize::from(reader.read_u16(&cursor[..2]));
        let entries_cursor = &cursor[2..];
        let entries_len = num_entries * IFD_ENTRY_SIZE;
        ensure!(
            entries_cursor.len() >= entries_len,
            "Invalid number of IFD entries"
        );

        let mut cur_offset = None;
        let mut cur_length = None;
        let mut cur_orientation = None;
        let mut cur_offset_is_strip = false;
        let mut cur_length_is_strip = false;

        for entry in entries_cursor
            .chunks_exact(IFD_ENTRY_SIZE)
            .take(num_entries)
        {
            let tag = reader.read_u16(&entry[..2]);
            let count = reader.read_u32(&entry[4..8]);

            match tag {
                JPEG_TAG => {
                    cur_offset = Some(reader.read_u32(&entry[8..12]).try_into()?);
                    cur_offset_is_strip = false;
                }
                JPEG_LENGTH_TAG => {
                    cur_length = Some(reader.read_u32(&entry[8..12]).try_into()?);
                    cur_length_is_strip = false;
                }
                TAG_STRIP_OFFSETS if count == 1 => {
                    cur_offset = Some(reader.read_u32(&entry[8..12]).try_into()?);
                    cur_offset_is_strip = true;
                }
                TAG_STRIP_BYTE_COUNTS if count == 1 => {
                    cur_length = Some(reader.read_u32(&entry[8..12]).try_into()?);
                    cur_length_is_strip = true;
                }
                TAG_ORIENTATION => cur_orientation = Some(reader.read_u16(&entry[8..10])),
                _ => {}
            }

            if let (Some(offset), Some(length)) = (cur_offset, cur_length) {
                let is_jpeg_candidate = if cur_offset_is_strip || cur_length_is_strip {
                    offset + JPEG_SOI_PREFIX.len() <= raw_buf.len()
                        && raw_buf[offset..].starts_with(JPEG_SOI_PREFIX)
                } else {
                    true
                };
                if !is_jpeg_candidate {
                    continue;
                }
                if is_better(length, best.map_or(0, |jpeg| jpeg.length), find_type) {
                    best = Some(EmbeddedJpegInfo::new(offset, length, cur_orientation));
                }
                break;
            }
        }

        let next_ifd_offset_offset = 2 + entries_len;
        ensure!(
            cursor.len() >= next_ifd_offset_offset + 4,
            "Invalid next IFD offset"
        );
        next_ifd_offset = reader
            .read_u32(&cursor[next_ifd_offset_offset..][..4])
            .try_into()?;
    }

    if let Some(best) = best {
        ensure!(
            best.offset + best.length <= raw_buf.len(),
            "JPEG data exceeds file size"
        );
        return Ok(Some(EmbeddedJpegInfo::new(
            best.offset + tiff_offset,
            best.length,
            best.orientation,
        )));
    }

    Ok(None)
}

/// Extract the JPEG bytes from the memory-mapped RAW buffer.
fn extract_jpeg<'raw>(raw_buf: &'raw Mmap, jpeg: &EmbeddedJpegInfo) -> &'raw [u8] {
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
        // The cached end is only reusable when the EOI it points at starts at or after
        // `search_from`; an EOI overlapping or preceding the new candidate must not be reused.
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

fn find_embedded_jpeg_by_markers(
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
            continue;
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

fn find_tiff_preview(
    raw_buf: &[u8],
    tiff_offset: usize,
    find_type: FindJpegType,
) -> Result<TiffPreviewInfo> {
    let raw_buf = &raw_buf[tiff_offset..];
    let reader = tiff_reader(raw_buf)?;
    let mut next_ifd_offset = usize::try_from(reader.read_u32(&raw_buf[4..8]))?;
    let mut best = TiffPreviewInfo::default();
    let mut chain_guard = IfdChainGuard::new();

    while next_ifd_offset != 0 {
        chain_guard.visit(next_ifd_offset)?;
        ensure!(next_ifd_offset + 2 <= raw_buf.len(), "Invalid IFD offset");
        let cursor = &raw_buf[next_ifd_offset..];
        let num_entries = usize::from(reader.read_u16(&cursor[..2]));
        let entries_cursor = &cursor[2..];
        let entries_len = num_entries * IFD_ENTRY_SIZE;
        ensure!(
            entries_cursor.len() >= entries_len,
            "Invalid number of TIFF IFD entries"
        );

        let mut candidate = TiffPreviewInfo::default();

        for entry in entries_cursor
            .chunks_exact(IFD_ENTRY_SIZE)
            .take(num_entries)
        {
            let tag = reader.read_u16(&entry[..2]);
            let type_id = reader.read_u16(&entry[2..4]);
            let count = reader.read_u32(&entry[4..8]);
            let value_field = &entry[8..12];

            match tag {
                TAG_IMAGE_WIDTH => {
                    candidate.width = first_u32(&reader, type_id, count, value_field, "ImageWidth")?;
                }
                TAG_IMAGE_LENGTH => {
                    candidate.height =
                        first_u32(&reader, type_id, count, value_field, "ImageLength")?;
                }
                TAG_BITS_PER_SAMPLE => {
                    candidate.bits_per_sample =
                        reader.read_u16_values(type_id, count, value_field)?;
                }
                TAG_COMPRESSION => {
                    candidate.compression = Some(u16::try_from(first_u32(
                        &reader,
                        type_id,
                        count,
                        value_field,
                        "Compression",
                    )?)?);
                }
                TAG_PHOTOMETRIC_INTERPRETATION => {
                    candidate.photometric_interpretation = Some(u16::try_from(first_u32(
                        &reader,
                        type_id,
                        count,
                        value_field,
                        "PhotometricInterpretation",
                    )?)?);
                }
                TAG_STRIP_OFFSETS => {
                    candidate.strip_offsets = reader
                        .read_u32_values(type_id, count, value_field)?
                        .into_iter()
                        .map(usize::try_from)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                }
                TAG_ORIENTATION => {
                    candidate.orientation = Some(u16::try_from(first_u32(
                        &reader,
                        type_id,
                        count,
                        value_field,
                        "Orientation",
                    )?)?);
                }
                TAG_SAMPLES_PER_PIXEL => {
                    candidate.samples_per_pixel = Some(u16::try_from(first_u32(
                        &reader,
                        type_id,
                        count,
                        value_field,
                        "SamplesPerPixel",
                    )?)?);
                }
                TAG_ROWS_PER_STRIP => {
                    candidate.rows_per_strip = Some(first_u32(
                        &reader,
                        type_id,
                        count,
                        value_field,
                        "RowsPerStrip",
                    )?);
                }
                TAG_STRIP_BYTE_COUNTS => {
                    candidate.strip_byte_counts = reader
                        .read_u32_values(type_id, count, value_field)?
                        .into_iter()
                        .map(usize::try_from)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                }
                _ => {}
            }
        }

        if is_supported_tiff_preview(&candidate)
            && is_better(candidate.encoded_len(), best.encoded_len(), find_type)
        {
            best = candidate;
        }

        let next_ifd_offset_offset = 2 + entries_len;
        ensure!(
            cursor.len() >= next_ifd_offset_offset + 4,
            "Invalid next TIFF IFD offset"
        );
        next_ifd_offset = usize::try_from(reader.read_u32(&cursor[next_ifd_offset_offset..][..4]))?;
    }

    ensure!(best.encoded_len() > 0, "No supported TIFF preview found");
    Ok(best)
}

fn is_supported_tiff_preview(preview: &TiffPreviewInfo) -> bool {
    preview.width > 0
        && preview.height > 0
        && preview.compression == Some(1)
        && preview.photometric_interpretation == Some(2)
        && preview.samples_per_pixel == Some(3)
        && !preview.strip_offsets.is_empty()
        && preview.strip_offsets.len() == preview.strip_byte_counts.len()
        && !preview.bits_per_sample.is_empty()
        && preview.bits_per_sample.iter().all(|bits| *bits == 8)
        && preview.encoded_len() == preview.width as usize * preview.height as usize * 3
}

fn encode_tiff_preview_as_jpeg(raw_buf: &[u8], preview: &TiffPreviewInfo) -> Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity(preview.encoded_len());
    for (offset, len) in preview
        .strip_offsets
        .iter()
        .copied()
        .zip(preview.strip_byte_counts.iter().copied())
    {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| anyhow::anyhow!("TIFF preview range overflow"))?;
        ensure!(end <= raw_buf.len(), "TIFF preview range exceeds file size");
        rgb.extend_from_slice(&raw_buf[offset..end]);
    }

    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 95);
    encoder
        .encode(&rgb, preview.width, preview.height, ExtendedColorType::Rgb8)
        .context("Failed to encode TIFF preview as JPEG")?;
    Ok(jpeg)
}

fn find_embedded_jpeg_missing_ff_prefix(
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
            continue;
        };
        // Prepending the missing 0xFF makes the candidate one byte longer than the raw range.
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

fn is_valid_jpeg(bytes: &[u8]) -> bool {
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
const fn get_header_bytes(orientation: u16) -> [u8; 34] {
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

fn get_jpeg_data(jpeg_buf: &[u8], jpeg_info: &EmbeddedJpegInfo) -> Vec<u8> {
    let mut jpeg_data = Vec::with_capacity(jpeg_buf.len() + 34);
    jpeg_data.extend_from_slice(&get_header_bytes(jpeg_info.orientation.unwrap_or(1)));
    jpeg_data.extend_from_slice(&jpeg_buf[2..]);
    jpeg_data
}

/// Bounds the IFD chain walk so malformed files with cyclic or unbounded next-IFD offsets fail
/// instead of looping forever.
struct IfdChainGuard {
    visited: Vec<usize>,
}

impl IfdChainGuard {
    fn new() -> Self {
        Self {
            visited: Vec::new(),
        }
    }

    fn visit(&mut self, offset: usize) -> Result<()> {
        ensure!(
            self.visited.len() < MAX_IFD_CHAIN_LEN,
            "TIFF IFD chain exceeds limit"
        );
        ensure!(
            !self.visited.contains(&offset),
            "TIFF IFD cycle detected"
        );
        self.visited.push(offset);
        Ok(())
    }
}

fn tiff_reader(raw_buf: &[u8]) -> Result<TiffReader<'_>> {
    ensure!(raw_buf.len() >= 8, "Not enough data for TIFF header");
    let is_le = raw_buf.starts_with(TIFF_HEADER);
    ensure!(
        is_le || raw_buf.starts_with(TIFF_HEADERMM),
        "Not a valid TIFF file"
    );
    Ok(TiffReader { buf: raw_buf, is_le })
}

fn first_u32(
    reader: &TiffReader<'_>,
    type_id: u16,
    count: u32,
    value_field: &[u8],
    field_name: &str,
) -> Result<u32> {
    reader
        .read_u32_values(type_id, count, value_field)?
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("TIFF field {field_name} is empty"))
}

fn is_better(candidate: usize, best: usize, find_type: FindJpegType) -> bool {
    match find_type {
        FindJpegType::Largest => candidate > best,
        FindJpegType::Smallest => best == 0 || candidate < best,
    }
}

fn jpeg_info_for_bytes(jpeg_buf: &[u8], orientation: Option<u16>) -> EmbeddedJpegInfo {
    EmbeddedJpegInfo::new(0, jpeg_buf.len(), orientation)
}

fn try_missing_ff(raw_buf: &[u8], find_type: FindJpegType) -> Option<Vec<u8>> {
    let jpeg_buf = find_embedded_jpeg_missing_ff_prefix(raw_buf, find_type).ok()?;
    Some(get_jpeg_data(
        &jpeg_buf,
        &jpeg_info_for_bytes(&jpeg_buf, None),
    ))
}

fn try_tiff_preview(
    raw_buf: &[u8],
    tiff_offset: Option<usize>,
    find_type: FindJpegType,
) -> Result<Option<Vec<u8>>> {
    let Some(tiff_offset) = tiff_offset else {
        return Ok(None);
    };
    let Ok(preview) = find_tiff_preview(raw_buf, tiff_offset, find_type) else {
        return Ok(None);
    };
    let jpeg_buf = encode_tiff_preview_as_jpeg(&raw_buf[tiff_offset..], &preview)?;
    Ok(Some(get_jpeg_data(
        &jpeg_buf,
        &jpeg_info_for_bytes(&jpeg_buf, preview.orientation),
    )))
}

/// Process a single RAW file to extract the embedded JPEG, and then write the extracted JPEG to
/// the output directory.
///
/// # Errors
///
/// Returns an error if the RAW file cannot be parsed or if writing the JPEG output fails.
pub async fn process_file(
    entry_path: &Path,
    out_dir: &Path,
    relative_path: &Path,
    find_type: FindJpegType,
) -> Result<()> {
    let mut output_file = out_dir.join(relative_path);
    output_file.set_extension("jpg");
    if let Some(parent) = output_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let jpeg_data = process_file_bytes(entry_path, find_type).await?;
    tokio::fs::write(&output_file, &jpeg_data).await?;
    Ok(())
}

/// Process a single RAW file to extract the embedded JPEG and return the JPEG bytes.
///
/// # Errors
///
/// Returns an error if the RAW file cannot be opened, mapped, or parsed.
pub async fn process_file_bytes(entry_path: &Path, find_type: FindJpegType) -> Result<Vec<u8>> {
    let in_file = platform::open_raw(entry_path)?;
    let raw_buf = platform::mmap_raw(&in_file)?;
    let tiff_offset = find_tiff_header_offset(&raw_buf).ok();
    let jpeg_info = match tiff_offset {
        Some(tiff_offset) => find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, find_type)
            .ok()
            .flatten()
            .or_else(|| find_embedded_jpeg_by_markers(&raw_buf, find_type).ok()),
        None => find_embedded_jpeg_by_markers(&raw_buf, find_type).ok(),
    };

    if let Some(jpeg_info) = jpeg_info {
        let jpeg_buf = extract_jpeg(&raw_buf, &jpeg_info);
        if is_valid_jpeg(jpeg_buf) {
            return Ok(get_jpeg_data(jpeg_buf, &jpeg_info));
        }
    }

    if let Some(jpeg_data) = try_missing_ff(&raw_buf, find_type) {
        return Ok(jpeg_data);
    }
    if let Some(jpeg_data) = try_tiff_preview(&raw_buf, tiff_offset, find_type)? {
        return Ok(jpeg_data);
    }

    decode_raw_preview_fallback(entry_path, &raw_buf).await
}

async fn decode_raw_preview_fallback(entry_path: &Path, raw_buf: &[u8]) -> Result<Vec<u8>> {
    let generic_path = entry_path.to_path_buf();
    let generic_bytes = raw_buf.to_vec();
    if let Some(jpeg) = run_optional_blocking_fallback("generic raster", move || {
        generic::decode_generic_raster_preview(&generic_path, &generic_bytes)
    })
    .await?
    {
        return Ok(jpeg);
    }

    let extension = entry_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    let entry_path = entry_path.to_path_buf();

    if extension.as_deref() == Some("gpr") {
        return run_blocking_fallback("gpr", move || gpr::decode_raw_preview(&entry_path)).await;
    }

    run_blocking_fallback("dcraw", move || dcraw::decode_raw_preview(&entry_path)).await
}

/// Run a blocking decode off the async workers when a tokio runtime drives us; non-tokio callers
/// get the decode inline instead of a panic.
async fn run_blocking_fallback<F>(label: &str, decode: F) -> Result<Vec<u8>>
where
    F: FnOnce() -> Result<Vec<u8>> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(decode)
            .await
            .with_context(|| format!("{label} fallback task failed"))?,
        Err(_) => decode(),
    }
}

async fn run_optional_blocking_fallback<F>(label: &str, decode: F) -> Result<Option<Vec<u8>>>
where
    F: FnOnce() -> Result<Option<Vec<u8>>> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(decode)
            .await
            .with_context(|| format!("{label} fallback task failed"))?,
        Err(_) => decode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_buffers_do_not_panic_when_searching_for_tiff_header() {
        for len in 0..4 {
            let buf = vec![0u8; len];
            assert!(find_tiff_header_offset(&buf).is_err());
        }
    }

    #[test]
    fn zero_count_tiff_fields_error_instead_of_panicking() {
        let buf = [
            b'I', b'I', b'*', 0, 8, 0, 0, 0, 1, 0, 0x00, 0x01, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0,
        ];
        let err = find_tiff_preview(&buf, 0, FindJpegType::Largest).unwrap_err();
        assert!(err.to_string().contains("ImageWidth is empty"));
    }

    #[test]
    fn tiff_ifd_cycles_error_out() {
        let buf = [
            b'I', b'I', b'*', 0, 8, 0, 0, 0, 0, 0, 8, 0, 0, 0,
        ];
        let err = find_tiff_preview(&buf, 0, FindJpegType::Largest).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn missing_ff_search_keeps_valid_smaller_candidate() {
        let valid = valid_missing_ff_candidate();
        let invalid = [0xD8, 0xFF, 0xDB, 0x00, 0x04, 0, 0, 0xFF, 0xD9];
        let mut buf = Vec::new();
        buf.extend_from_slice(&valid);
        buf.extend_from_slice(&[0, 0, 0]);
        buf.extend_from_slice(&invalid);

        let jpeg = find_embedded_jpeg_missing_ff_prefix(&buf, FindJpegType::Largest).unwrap();
        assert!(is_valid_jpeg(&jpeg));
    }

    #[test]
    fn back_to_back_jpegs_largest_finds_second() {
        let small = make_test_jpeg(16);
        let large = make_test_jpeg(256);
        let mut buf = Vec::new();
        buf.extend_from_slice(&small);
        buf.extend_from_slice(&large);

        let found = find_embedded_jpeg_by_markers(&buf, FindJpegType::Largest).unwrap();
        assert_eq!(found.offset, small.len());
        assert_eq!(found.length, large.len());
    }

    #[test]
    fn missing_ff_candidate_adjacent_to_prior_eoi_is_found() {
        // An invalid missing-FF candidate whose EOI ends exactly where the valid candidate
        // starts, so a stale cached EOI position must not be reused for the valid one.
        let invalid = [0xD8, 0xFF, 0xDB, 0x00, 0x04, 0, 0, 0xFF, 0xD9];
        let valid = valid_missing_ff_candidate();
        let mut buf = Vec::new();
        buf.extend_from_slice(&invalid);
        buf.extend_from_slice(&valid);

        let jpeg = find_embedded_jpeg_missing_ff_prefix(&buf, FindJpegType::Largest).unwrap();
        assert!(is_valid_jpeg(&jpeg));
        assert_eq!(jpeg.len(), valid.len() + 1);
    }

    #[test]
    fn process_file_bytes_returns_error_outside_tokio_runtime() {
        let temp_dir = temp_dir::TempDirGuard::new("jpgfromraw-test").unwrap();
        let path = temp_dir.path().join("garbage.arw");
        std::fs::write(&path, [0u8; 64]).unwrap();

        let result =
            block_on_without_tokio(process_file_bytes(&path, FindJpegType::Largest));
        assert!(result.is_err());
    }

    /// Polls a future on the current thread with no tokio runtime, like a non-tokio caller
    /// driving the library with another executor's `block_on`.
    fn block_on_without_tokio<F: std::future::Future>(fut: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct ThreadWaker(std::thread::Thread);
        impl Wake for ThreadWaker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    /// Builds a minimal JPEG that passes `is_valid_jpeg`, sized via DQT padding so tests can
    /// construct candidates of different lengths.
    fn make_test_jpeg(dqt_padding: usize) -> Vec<u8> {
        let dqt_len = u16::try_from(2 + 1 + dqt_padding).unwrap();
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xDB]);
        jpeg.extend_from_slice(&dqt_len.to_be_bytes());
        jpeg.extend(std::iter::repeat(0).take(1 + dqt_padding));
        // SOF0: precision 8, 1x1, one component.
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert!(is_valid_jpeg(&jpeg));
        jpeg
    }

    fn valid_missing_ff_candidate() -> Vec<u8> {
        let candidate = make_test_jpeg(64)[1..].to_vec();
        assert!(candidate.starts_with(JPEG_SOI_MISSING_PREFIX));
        candidate
    }

    /// Straightforward per-SOI forward search; the production scanner must select the same
    /// candidate.
    fn find_by_markers_reference(
        raw_buf: &[u8],
        find_type: FindJpegType,
    ) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        let mut search_start = 0;
        while let Some(rel) = memmem::find(&raw_buf[search_start..], JPEG_SOI_PREFIX) {
            let jpeg_start = search_start + rel;
            search_start = jpeg_start + JPEG_SOI_PREFIX.len();
            let Some(rel_end) = memmem::find(&raw_buf[search_start..], JPEG_EOI) else {
                continue;
            };
            let jpeg_end = search_start + rel_end + JPEG_EOI.len();
            let len = jpeg_end - jpeg_start;
            if is_better(len, best.map_or(0, |(_, len)| len), find_type)
                && is_valid_jpeg(&raw_buf[jpeg_start..jpeg_end])
            {
                best = Some((jpeg_start, len));
            }
        }
        best
    }

    #[test]
    fn marker_scan_matches_reference_on_randomized_buffers() {
        let mut seed = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };

        for case in 0..200 {
            // Noise with a high density of 0xFF/0xD8/0xD9 bytes to stress marker handling.
            let len = 200 + (next() % 2000) as usize;
            let mut buf: Vec<u8> = (0..len)
                .map(|_| match next() % 8 {
                    0 => 0xFF,
                    1 => 0xD8,
                    2 => 0xD9,
                    _ => (next() % 256) as u8,
                })
                .collect();
            // Embed up to two valid JPEGs at random offsets.
            for _ in 0..(next() % 3) {
                let jpeg = make_test_jpeg(8 + (next() % 64) as usize);
                if buf.len() > jpeg.len() {
                    let at = (next() as usize) % (buf.len() - jpeg.len());
                    buf[at..at + jpeg.len()].copy_from_slice(&jpeg);
                }
            }

            for find_type in [FindJpegType::Largest, FindJpegType::Smallest] {
                let expected = find_by_markers_reference(&buf, find_type);
                let actual = find_embedded_jpeg_by_markers(&buf, find_type)
                    .ok()
                    .map(|info| (info.offset, info.length));
                assert_eq!(actual, expected, "case {case} diverged");
            }
        }
    }

    /// Times each parsing stage on the real RAW corpus in test_images. Run with:
    /// cargo test --release --lib -- --ignored parsing_performance_audit --nocapture
    #[test]
    #[ignore = "benchmark-style audit; run in release mode with --nocapture"]
    fn parsing_performance_audit() {
        let files = bench_raw_files();
        assert!(!files.is_empty(), "no .arw files in test_images");

        let buffers: Vec<Vec<u8>> = files
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();

        report_stage("find_tiff_header_offset (TIFF at 0)", &buffers, 200, |buf| {
            std::hint::black_box(find_tiff_header_offset(buf).unwrap());
        });

        report_stage("find_embedded_jpeg_in_tiff Smallest", &buffers, 200, |buf| {
            std::hint::black_box(
                find_embedded_jpeg_in_tiff(buf, 0, FindJpegType::Smallest).unwrap(),
            );
        });

        report_stage("find_embedded_jpeg_in_tiff Largest", &buffers, 200, |buf| {
            std::hint::black_box(
                find_embedded_jpeg_in_tiff(buf, 0, FindJpegType::Largest).unwrap(),
            );
        });

        report_stage("validate + assemble Largest", &buffers, 50, |buf| {
            let info = find_embedded_jpeg_in_tiff(buf, 0, FindJpegType::Largest)
                .unwrap()
                .unwrap();
            let jpeg_buf = &buf[info.offset..info.offset + info.length];
            assert!(is_valid_jpeg(jpeg_buf));
            std::hint::black_box(get_jpeg_data(jpeg_buf, &info));
        });

        let scan_buffers = &buffers[..buffers.len().min(3)];
        report_stage("marker scan Smallest (fallback)", scan_buffers, 5, |buf| {
            std::hint::black_box(
                find_embedded_jpeg_by_markers(buf, FindJpegType::Smallest).unwrap(),
            );
        });
        report_stage("marker scan Largest (fallback)", scan_buffers, 5, |buf| {
            std::hint::black_box(
                find_embedded_jpeg_by_markers(buf, FindJpegType::Largest).unwrap(),
            );
        });
        report_stage("missing-FF scan Largest (fallback)", scan_buffers, 5, |buf| {
            std::hint::black_box(find_embedded_jpeg_missing_ff_prefix(
                buf,
                FindJpegType::Largest,
            ))
            .ok();
        });

        // RAF-like layout: no TIFF magic at offset 0, Exif APP1 near the start of the file.
        let mut raf_like = vec![0u8; scan_buffers[0].len()];
        raf_like[..8].copy_from_slice(b"FUJIFILM");
        let exif_at = 1024;
        raf_like[exif_at - 10..exif_at].copy_from_slice(&[0xFF, 0xE1, 0, 100, 0, 0, 0, 0, 0, 0]);
        raf_like[exif_at..exif_at + EXIF_HEADER_SIZE].copy_from_slice(EXIF_HEADER);
        raf_like[exif_at + EXIF_HEADER_SIZE..exif_at + EXIF_HEADER_SIZE + 4]
            .copy_from_slice(TIFF_HEADER);
        let raf_buffers = vec![raf_like];
        report_stage(
            "find_tiff_header_offset (Exif at 1KB, no TIFF at 0)",
            &raf_buffers,
            20,
            |buf| {
                std::hint::black_box(find_tiff_header_offset(buf).unwrap());
            },
        );

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        report_stage("process_file_bytes Smallest (end-to-end)", &files, 50, |path| {
            let jpeg = runtime
                .block_on(process_file_bytes(path, FindJpegType::Smallest))
                .unwrap();
            std::hint::black_box(jpeg);
        });
        report_stage("process_file_bytes Largest (end-to-end)", &files, 50, |path| {
            let jpeg = runtime
                .block_on(process_file_bytes(path, FindJpegType::Largest))
                .unwrap();
            std::hint::black_box(jpeg);
        });

        report_stage("open only", &files, 50, |path| {
            std::hint::black_box(platform::open_raw(path).unwrap());
        });
        report_stage("open + mmap + advise + drop", &files, 50, |path| {
            let file = platform::open_raw(path).unwrap();
            std::hint::black_box(platform::mmap_raw(&file).unwrap());
        });
        report_stage("open + mmap + parse (no extract)", &files, 50, |path| {
            let file = platform::open_raw(path).unwrap();
            let raw_buf = platform::mmap_raw(&file).unwrap();
            let tiff_offset = find_tiff_header_offset(&raw_buf).unwrap();
            std::hint::black_box(
                find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, FindJpegType::Smallest)
                    .unwrap(),
            );
        });
        report_stage("open + mmap + parse + extract/validate", &files, 50, |path| {
            let file = platform::open_raw(path).unwrap();
            let raw_buf = platform::mmap_raw(&file).unwrap();
            let tiff_offset = find_tiff_header_offset(&raw_buf).unwrap();
            let info = find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, FindJpegType::Smallest)
                .unwrap()
                .unwrap();
            let jpeg_buf = extract_jpeg(&raw_buf, &info);
            assert!(is_valid_jpeg(jpeg_buf));
            std::hint::black_box(get_jpeg_data(jpeg_buf, &info));
        });

    }

    /// Measures how end-to-end throughput scales with concurrency. Run with:
    /// cargo test --release --lib -- --ignored parallel_scaling_audit --nocapture
    #[test]
    #[ignore = "benchmark-style audit; run in release mode with --nocapture"]
    fn parallel_scaling_audit() {
        use std::sync::Arc;

        let files = Arc::new(bench_raw_files());
        assert!(!files.is_empty(), "no .arw files in test_images");
        const ROUNDS: usize = 100;

        let mut single_threaded_ops_per_sec = 0.0;
        for workers in [1usize, 2, 4, 8] {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers)
                .enable_all()
                .build()
                .unwrap();
            let total_ops = ROUNDS * files.len();

            // Warmup round.
            runtime.block_on(run_rounds(Arc::clone(&files), 5, workers));
            let start = std::time::Instant::now();
            runtime.block_on(run_rounds(Arc::clone(&files), ROUNDS, workers));
            let elapsed = start.elapsed().as_secs_f64();

            let ops_per_sec = total_ops as f64 / elapsed;
            if workers == 1 {
                single_threaded_ops_per_sec = ops_per_sec;
            }
            println!(
                "workers {workers:>2}: {ops_per_sec:>10.0} files/sec  (scaling x{:.2})",
                ops_per_sec / single_threaded_ops_per_sec
            );
        }

        async fn run_rounds(
            files: Arc<Vec<std::path::PathBuf>>,
            rounds: usize,
            concurrency: usize,
        ) {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
            let mut tasks = Vec::new();
            for _ in 0..rounds {
                for index in 0..files.len() {
                    let files = Arc::clone(&files);
                    let semaphore = Arc::clone(&semaphore);
                    tasks.push(tokio::spawn(async move {
                        let _permit = semaphore.acquire_owned().await.unwrap();
                        let jpeg = process_file_bytes(&files[index], FindJpegType::Largest)
                            .await
                            .unwrap();
                        std::hint::black_box(jpeg.len())
                    }));
                }
            }
            for task in tasks {
                task.await.unwrap();
            }
        }
    }

    fn bench_raw_files() -> Vec<std::path::PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_images");
        let mut files: Vec<_> = std::fs::read_dir(root)
            .map(|dir| {
                dir.filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| {
                        path.extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("arw"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    }

    fn report_stage<T>(label: &str, inputs: &[T], iterations: usize, mut op: impl FnMut(&T)) {
        // Warmup.
        for input in inputs {
            op(input);
        }
        let mut per_iteration = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = std::time::Instant::now();
            for input in inputs {
                op(input);
            }
            per_iteration.push(start.elapsed().as_secs_f64() / inputs.len() as f64);
        }
        per_iteration.sort_by(f64::total_cmp);
        let median = per_iteration[per_iteration.len() / 2];
        let min = per_iteration[0];
        println!("{label:<55} median {:>12.3} us  min {:>12.3} us", median * 1e6, min * 1e6);
    }
}
