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
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// An embedded JPEG in a RAW file.
#[derive(Default, Eq, PartialEq)]
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
const JPEG_SOI_PREFIX: &[u8; 3] = b"\xFF\xD8\xFF";
const JPEG_SOI_MISSING_PREFIX: &[u8; 3] = b"\xD8\xFF\xDB";
const JPEG_EOI: &[u8; 2] = b"\xFF\xD9";
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

#[derive(Default)]
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
    if &raw_buf[0..4] == TIFF_HEADER || &raw_buf[0..4] == TIFF_HEADERMM {
        return Ok(0);
    }

    let found = memmem::find_iter(raw_buf, EXIF_HEADER).next();
    if let Some(pos) = found {
        if pos < 10 {
            return Err(anyhow::anyhow!("Invalid Exif header position: {}", pos));
        }
        let slice_start = pos + EXIF_HEADER_SIZE;
        ensure!(slice_start < raw_buf.len(), "Invalid Exif header position");
        let slice_end = slice_start + 10;
        ensure!(slice_end <= raw_buf.len(), "Invalid Exif header position");
        let slice = &raw_buf[slice_start..slice_end];
        let tiff_found = memmem::find_iter(slice, TIFF_HEADER).next();
        if let Some(tiff_pos) = tiff_found {
            return Ok(slice_start + tiff_pos);
        }
        let tiff_found = memmem::find_iter(slice, TIFF_HEADERMM).next();
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
fn find_largest_embedded_jpeg(
    raw_buf: &[u8],
    tiff_offset: usize,
    find_type: FindJpegType,
) -> Result<EmbeddedJpegInfo> {
    const IFD_ENTRY_SIZE: usize = 12;
    const TIFF_MAGIC_LE: &[u8] = b"II*\0";
    const TIFF_MAGIC_BE: &[u8] = b"MM\0*";
    const JPEG_TAG: u16 = 0x201;
    const JPEG_LENGTH_TAG: u16 = 0x202;
    const STRIP_OFFSETS_TAG: u16 = 0x111;
    const STRIP_BYTE_COUNTS_TAG: u16 = 0x117;
    const ORIENTATION_TAG: u16 = 0x112;

    let raw_buf = &raw_buf[tiff_offset..];

    ensure!(raw_buf.len() >= 8, "Not enough data for TIFF header");

    let is_le = &raw_buf[0..4] == TIFF_MAGIC_LE;
    ensure!(
        is_le || &raw_buf[0..4] == TIFF_MAGIC_BE,
        "Not a valid TIFF file"
    );

    let read_u16 = if is_le {
        LittleEndian::read_u16
    } else {
        BigEndian::read_u16
    };

    let read_u32 = if is_le {
        LittleEndian::read_u32
    } else {
        BigEndian::read_u32
    };

    let mut next_ifd_offset = read_u32(&raw_buf[4..8]).try_into()?;
    let mut largest_jpeg = EmbeddedJpegInfo::default();

    while next_ifd_offset != 0 {
        ensure!(next_ifd_offset + 2 <= raw_buf.len(), "Invalid IFD offset");

        let cursor = &raw_buf[next_ifd_offset..];
        let num_entries = read_u16(&cursor[..2]).into();
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
            let tag = read_u16(&entry[..2]);
            let count = read_u32(&entry[4..8]);

            match tag {
                JPEG_TAG => {
                    cur_offset = Some(read_u32(&entry[8..12]).try_into()?);
                    cur_offset_is_strip = false;
                }
                JPEG_LENGTH_TAG => {
                    cur_length = Some(read_u32(&entry[8..12]).try_into()?);
                    cur_length_is_strip = false;
                }
                STRIP_OFFSETS_TAG if count == 1 => {
                    cur_offset = Some(read_u32(&entry[8..12]).try_into()?);
                    cur_offset_is_strip = true;
                }
                STRIP_BYTE_COUNTS_TAG if count == 1 => {
                    cur_length = Some(read_u32(&entry[8..12]).try_into()?);
                    cur_length_is_strip = true;
                }
                ORIENTATION_TAG => cur_orientation = Some(read_u16(&entry[8..10])),
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
                match find_type {
                    FindJpegType::Smallest => {
                        if length < largest_jpeg.length || largest_jpeg.length == 0 {
                            largest_jpeg = EmbeddedJpegInfo {
                                offset,
                                length,
                                orientation: cur_orientation,
                            };
                        }
                    }
                    FindJpegType::Largest => {
                        if length > largest_jpeg.length {
                            largest_jpeg = EmbeddedJpegInfo {
                                offset,
                                length,
                                orientation: cur_orientation,
                            };
                        }
                    }
                }
                break;
            }
        }

        let next_ifd_offset_offset = 2 + entries_len;
        ensure!(
            cursor.len() >= next_ifd_offset_offset + 4,
            "Invalid next IFD offset"
        );
        next_ifd_offset = read_u32(&cursor[next_ifd_offset_offset..][..4]).try_into()?;
    }

    ensure!(
        largest_jpeg != EmbeddedJpegInfo::default(),
        "No JPEG data found"
    );
    ensure!(
        largest_jpeg.offset + largest_jpeg.length <= raw_buf.len(),
        "JPEG data exceeds file size"
    );

    let jpeg_offset = largest_jpeg.offset + tiff_offset;

    Ok(EmbeddedJpegInfo {
        offset: jpeg_offset,
        length: largest_jpeg.length,
        orientation: largest_jpeg.orientation,
    })
}

/// Extract the JPEG bytes from the memory-mapped RAW buffer.
fn extract_jpeg<'raw>(raw_buf: &'raw Mmap, jpeg: &'raw EmbeddedJpegInfo) -> Result<&'raw [u8]> {
    platform::prefetch_jpeg(raw_buf, jpeg)?;
    Ok(&raw_buf[jpeg.offset..jpeg.offset + jpeg.length])
}

fn find_embedded_jpeg_by_markers(
    raw_buf: &[u8],
    find_type: FindJpegType,
) -> Result<EmbeddedJpegInfo> {
    let mut best = EmbeddedJpegInfo::default();
    let mut search_start = 0;

    while let Some(relative_start) = memmem::find(&raw_buf[search_start..], JPEG_SOI_PREFIX) {
        let jpeg_start = search_start + relative_start;
        let jpeg_tail = &raw_buf[jpeg_start + JPEG_SOI_PREFIX.len()..];
        let Some(relative_end) = memmem::find(jpeg_tail, JPEG_EOI) else {
            search_start = jpeg_start + JPEG_SOI_PREFIX.len();
            continue;
        };
        let jpeg_end = jpeg_start + JPEG_SOI_PREFIX.len() + relative_end + JPEG_EOI.len();
        let candidate_len = jpeg_end - jpeg_start;
        let candidate = &raw_buf[jpeg_start..jpeg_end];
        if !is_valid_jpeg(candidate) {
            search_start = jpeg_start + JPEG_SOI_PREFIX.len();
            continue;
        }

        match find_type {
            FindJpegType::Largest => {
                if candidate_len > best.length {
                    best = EmbeddedJpegInfo {
                        offset: jpeg_start,
                        length: candidate_len,
                        orientation: None,
                    };
                }
            }
            FindJpegType::Smallest => {
                if best.length == 0 || candidate_len < best.length {
                    best = EmbeddedJpegInfo {
                        offset: jpeg_start,
                        length: candidate_len,
                        orientation: None,
                    };
                }
            }
        }

        search_start = jpeg_start + JPEG_SOI_PREFIX.len();
    }

    ensure!(
        best != EmbeddedJpegInfo::default(),
        "No JPEG marker data found"
    );
    Ok(best)
}

fn find_tiff_preview(
    raw_buf: &[u8],
    tiff_offset: usize,
    find_type: FindJpegType,
) -> Result<TiffPreviewInfo> {
    const IFD_ENTRY_SIZE: usize = 12;
    const TIFF_MAGIC_LE: &[u8] = b"II*\0";
    const TIFF_MAGIC_BE: &[u8] = b"MM\0*";

    let raw_buf = &raw_buf[tiff_offset..];
    ensure!(raw_buf.len() >= 8, "Not enough data for TIFF header");

    let is_le = &raw_buf[0..4] == TIFF_MAGIC_LE;
    ensure!(
        is_le || &raw_buf[0..4] == TIFF_MAGIC_BE,
        "Not a valid TIFF file"
    );

    let reader = TiffReader {
        buf: raw_buf,
        is_le,
    };
    let mut next_ifd_offset = usize::try_from(reader.read_u32(&raw_buf[4..8]))?;
    let mut best = TiffPreviewInfo::default();

    while next_ifd_offset != 0 {
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
                    candidate.width = reader.read_u32_values(type_id, count, value_field)?[0];
                }
                TAG_IMAGE_LENGTH => {
                    candidate.height = reader.read_u32_values(type_id, count, value_field)?[0];
                }
                TAG_BITS_PER_SAMPLE => {
                    candidate.bits_per_sample =
                        reader.read_u16_values(type_id, count, value_field)?;
                }
                TAG_COMPRESSION => {
                    candidate.compression = Some(u16::try_from(
                        reader.read_u32_values(type_id, count, value_field)?[0],
                    )?);
                }
                TAG_PHOTOMETRIC_INTERPRETATION => {
                    candidate.photometric_interpretation = Some(u16::try_from(
                        reader.read_u32_values(type_id, count, value_field)?[0],
                    )?);
                }
                TAG_STRIP_OFFSETS => {
                    candidate.strip_offsets = reader
                        .read_u32_values(type_id, count, value_field)?
                        .into_iter()
                        .map(usize::try_from)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                }
                TAG_ORIENTATION => {
                    candidate.orientation = Some(u16::try_from(
                        reader.read_u32_values(type_id, count, value_field)?[0],
                    )?);
                }
                TAG_SAMPLES_PER_PIXEL => {
                    candidate.samples_per_pixel = Some(u16::try_from(
                        reader.read_u32_values(type_id, count, value_field)?[0],
                    )?);
                }
                TAG_ROWS_PER_STRIP => {
                    candidate.rows_per_strip =
                        Some(reader.read_u32_values(type_id, count, value_field)?[0]);
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

        if is_supported_tiff_preview(&candidate) {
            match find_type {
                FindJpegType::Largest if candidate.encoded_len() > best.encoded_len() => {
                    best = candidate
                }
                FindJpegType::Smallest
                    if best.encoded_len() == 0 || candidate.encoded_len() < best.encoded_len() =>
                {
                    best = candidate;
                }
                _ => {}
            }
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
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 100);
    encoder
        .encode(&rgb, preview.width, preview.height, ExtendedColorType::Rgb8)
        .context("Failed to encode TIFF preview as JPEG")?;
    Ok(jpeg)
}

fn find_embedded_jpeg_missing_ff_prefix(
    raw_buf: &[u8],
    find_type: FindJpegType,
) -> Result<Vec<u8>> {
    let mut best_start = None;
    let mut best_len = 0usize;
    let mut search_start = 0;

    while let Some(relative_start) = memmem::find(&raw_buf[search_start..], JPEG_SOI_MISSING_PREFIX)
    {
        let jpeg_start = search_start + relative_start;
        if jpeg_start > 0 && raw_buf[jpeg_start - 1] == 0xFF {
            search_start = jpeg_start + 1;
            continue;
        }

        let jpeg_tail = &raw_buf[jpeg_start + JPEG_SOI_MISSING_PREFIX.len()..];
        let Some(relative_end) = memmem::find(jpeg_tail, JPEG_EOI) else {
            search_start = jpeg_start + 1;
            continue;
        };
        let candidate_len = JPEG_SOI_PREFIX.len() + relative_end + JPEG_EOI.len() + 1;

        match find_type {
            FindJpegType::Largest if candidate_len > best_len => {
                best_start = Some(jpeg_start);
                best_len = candidate_len;
            }
            FindJpegType::Smallest if best_len == 0 || candidate_len < best_len => {
                best_start = Some(jpeg_start);
                best_len = candidate_len;
            }
            _ => {}
        }

        search_start = jpeg_start + 1;
    }

    let start = best_start.ok_or_else(|| anyhow::anyhow!("No JPEG marker data found"))?;
    let mut jpeg = Vec::with_capacity(best_len);
    jpeg.push(0xFF);
    jpeg.extend_from_slice(&raw_buf[start..start + best_len - 1]);
    ensure!(is_valid_jpeg(&jpeg), "No valid JPEG marker data found");
    Ok(jpeg)
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

        if matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        ) {
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
        Some(tiff_offset) => find_largest_embedded_jpeg(&raw_buf, tiff_offset, find_type)
            .or_else(|_| find_embedded_jpeg_by_markers(&raw_buf, find_type)),
        None => find_embedded_jpeg_by_markers(&raw_buf, find_type),
    };

    //    Format	Hex Signature (First Bytes)	ASCII (if readable)	Offset	Notes
    //JPEG	FF D8 FF	—	0x00	Followed by E0, E1, etc. (APPx markers)
    //PNG	89 50 4E 47 0D 0A 1A 0A	.PNG....	0x00	8 bytes long, very specific
    //GIF	47 49 46 38 37 61 or 47 49 46 38 39 61	GIF87a or GIF89a	0x00	Indicates version
    //TIFF (little-endian)	49 49 2A 00	II*.	0x00	"Intel" byte order
    //TIFF (big-endian)	4D 4D 00 2A	MM.*	0x00	"Motorola" byte order
    /*
         How to Detect BMP and WebP
    BMP: Always starts with 42 4D (ASCII "BM") — stands for "Bitmap".

    WebP:

    Is stored inside a RIFF container (like WAV/AVI).

    First 4 bytes: 52 49 46 46 (ASCII "RIFF")

    At offset 8: 57 45 42 50 (ASCII "WEBP")
     */

    let jpeg_data = if let Ok(jpeg_info) = jpeg_info {
        let jpeg_buf = extract_jpeg(&raw_buf, &jpeg_info)?;
        if is_valid_jpeg(jpeg_buf) {
            get_jpeg_data(jpeg_buf, &jpeg_info)
        } else if let Ok(jpeg_buf) = find_embedded_jpeg_missing_ff_prefix(&raw_buf, find_type) {
            get_jpeg_data(
                &jpeg_buf,
                &EmbeddedJpegInfo {
                    offset: 0,
                    length: jpeg_buf.len(),
                    orientation: None,
                },
            )
        } else if let Some(tiff_offset) = tiff_offset {
            if let Ok(preview) = find_tiff_preview(&raw_buf, tiff_offset, find_type) {
                let jpeg_buf = encode_tiff_preview_as_jpeg(&raw_buf[tiff_offset..], &preview)?;
                get_jpeg_data(
                    &jpeg_buf,
                    &EmbeddedJpegInfo {
                        offset: 0,
                        length: jpeg_buf.len(),
                        orientation: preview.orientation,
                    },
                )
            } else {
                decode_raw_preview_fallback(entry_path)?
            }
        } else {
            decode_raw_preview_fallback(entry_path)?
        }
    } else if let Ok(jpeg_buf) = find_embedded_jpeg_missing_ff_prefix(&raw_buf, find_type) {
        get_jpeg_data(
            &jpeg_buf,
            &EmbeddedJpegInfo {
                offset: 0,
                length: jpeg_buf.len(),
                orientation: None,
            },
        )
    } else if let Some(tiff_offset) = tiff_offset {
        if let Ok(preview) = find_tiff_preview(&raw_buf, tiff_offset, find_type) {
            let jpeg_buf = encode_tiff_preview_as_jpeg(&raw_buf[tiff_offset..], &preview)?;
            get_jpeg_data(
                &jpeg_buf,
                &EmbeddedJpegInfo {
                    offset: 0,
                    length: jpeg_buf.len(),
                    orientation: preview.orientation,
                },
            )
        } else {
            decode_raw_preview_fallback(entry_path)?
        }
    } else {
        decode_raw_preview_fallback(entry_path)?
    };

    Ok(jpeg_data)
}

fn decode_raw_preview_fallback(entry_path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(entry_path)
        .with_context(|| format!("failed to read generic raster source {}", entry_path.display()))?;
    if let Some(jpeg) = generic::decode_generic_raster_preview(entry_path, &bytes)? {
        return Ok(jpeg);
    }

    let extension = entry_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);

    if extension.as_deref() == Some("gpr") {
        return gpr::decode_raw_preview(entry_path);
    }

    dcraw::decode_raw_preview(entry_path)
}
