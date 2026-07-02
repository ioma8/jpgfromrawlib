use anyhow::{Context, Result, ensure};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;
use memchr::memmem;

use crate::fallback;

use super::FindJpegType;
use super::is_better;
use super::jpeg::{EmbeddedJpegInfo, get_jpeg_data};

pub(crate) const TIFF_HEADER: &[u8; 4] = b"II*\0";
const TIFF_HEADER_RW2: &[u8; 4] = b"IIU\0";
pub(crate) const TIFF_HEADERMM: &[u8; 4] = b"MM\0*";
pub(crate) const EXIF_HEADER: &[u8; 6] = b"Exif\0\0";
pub(crate) const EXIF_HEADER_SIZE: usize = 6;
const EXIF_TIFF_SEARCH_WINDOW: usize = 10;
const EXIF_SEARCH_CAP: usize = 256 * 1024;
const IFD_ENTRY_SIZE: usize = 12;
const MAX_IFD_CHAIN_LEN: usize = 64;
const JPEG_TAG: u16 = 0x201;
const JPEG_LENGTH_TAG: u16 = 0x202;
const TAG_JPG_FROM_RAW: u16 = 0x2E;
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
const TAG_SUB_IFDS: u16 = 0x14A;

#[derive(Debug, Default)]
pub(crate) struct TiffPreviewInfo {
    strip_offsets: Vec<usize>,
    strip_byte_counts: Vec<usize>,
    encoded_len: usize,
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
        self.encoded_len
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
        ensure!(!self.visited.contains(&offset), "TIFF IFD cycle detected");
        self.visited.push(offset);
        Ok(())
    }
}

pub(crate) fn find_tiff_header_offset(raw_buf: &[u8]) -> Result<usize> {
    ensure!(
        raw_buf.len() >= TIFF_HEADER.len(),
        "Not enough data for TIFF header"
    );

    if raw_buf.starts_with(TIFF_HEADER)
        || raw_buf.starts_with(TIFF_HEADER_RW2)
        || raw_buf.starts_with(TIFF_HEADERMM)
    {
        return Ok(0);
    }

    let head = &raw_buf[..raw_buf.len().min(EXIF_SEARCH_CAP)];
    let found = memmem::find(head, EXIF_HEADER);
    if let Some(pos) = found {
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
pub(crate) fn find_embedded_jpeg_in_tiff(
    raw_buf: &[u8],
    tiff_offset: usize,
    find_type: FindJpegType,
) -> Result<Option<EmbeddedJpegInfo>> {
    let raw_buf = &raw_buf[tiff_offset..];
    let reader = tiff_reader(raw_buf)?;
    let mut pending = vec![usize::try_from(reader.read_u32(&raw_buf[4..8]))?];
    let mut best: Option<EmbeddedJpegInfo> = None;
    let mut chain_guard = IfdChainGuard::new();

    while let Some(ifd_offset) = pending.pop() {
        if ifd_offset == 0 {
            continue;
        }
        chain_guard.visit(ifd_offset)?;
        ensure!(ifd_offset + 2 <= raw_buf.len(), "Invalid IFD offset");

        let cursor = &raw_buf[ifd_offset..];
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
            let type_id = reader.read_u16(&entry[2..4]);
            let count = reader.read_u32(&entry[4..8]);

            match tag {
                JPEG_TAG => {
                    cur_offset = Some(reader.read_u32(&entry[8..12]).try_into()?);
                    cur_offset_is_strip = false;
                }
                TAG_JPG_FROM_RAW => {
                    let offset = usize::try_from(reader.read_u32(&entry[8..12]))?;
                    let length = usize::try_from(count)?;
                    if raw_buf.get(offset..offset + 3) == Some(b"\xFF\xD8\xFF") {
                        cur_offset = Some(offset);
                        cur_length = Some(length);
                        cur_offset_is_strip = false;
                        cur_length_is_strip = false;
                    }
                }
                JPEG_LENGTH_TAG => {
                    cur_length = Some(reader.read_u32(&entry[8..12]).try_into()?);
                    cur_length_is_strip = false;
                }
                TAG_SUB_IFDS => {
                    if let Ok(values) = reader.read_u32_values(type_id, count, &entry[8..12]) {
                        for sub in values {
                            if let Ok(sub) = usize::try_from(sub) {
                                pending.push(sub);
                            }
                        }
                    }
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
                    offset + 3 <= raw_buf.len() && raw_buf[offset..].starts_with(b"\xFF\xD8\xFF")
                } else {
                    true
                };
                if !is_jpeg_candidate {
                    continue;
                }
                if is_better(length, best.map_or(0, |jpeg| jpeg.length), find_type) {
                    best = Some(EmbeddedJpegInfo::new(offset, length, cur_orientation));
                }
            }
        }

        let next_ifd_offset_offset = 2 + entries_len;
        ensure!(
            cursor.len() >= next_ifd_offset_offset + 4,
            "Invalid next IFD offset"
        );
        pending.push(
            reader
                .read_u32(&cursor[next_ifd_offset_offset..][..4])
                .try_into()?,
        );
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

pub(crate) fn find_tiff_preview(
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
                    candidate.width =
                        first_u32(&reader, type_id, count, value_field, "ImageWidth")?;
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
                    candidate.encoded_len = candidate.strip_byte_counts.iter().sum();
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

    let mut jpeg = Vec::with_capacity(fallback::jpeg_output_capacity(rgb.len()));
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 95);
    encoder
        .encode(&rgb, preview.width, preview.height, ExtendedColorType::Rgb8)
        .context("Failed to encode TIFF preview as JPEG")?;
    Ok(jpeg)
}

pub(crate) fn try_tiff_preview(
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
        &EmbeddedJpegInfo::new(0, jpeg_buf.len(), preview.orientation),
    )))
}

fn tiff_reader(raw_buf: &[u8]) -> Result<TiffReader<'_>> {
    ensure!(raw_buf.len() >= 8, "Not enough data for TIFF header");
    let is_le = raw_buf.starts_with(TIFF_HEADER) || raw_buf.starts_with(TIFF_HEADER_RW2);
    ensure!(
        is_le || raw_buf.starts_with(TIFF_HEADERMM),
        "Not a valid TIFF file"
    );
    Ok(TiffReader {
        buf: raw_buf,
        is_le,
    })
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
