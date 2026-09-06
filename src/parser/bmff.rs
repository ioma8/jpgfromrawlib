use byteorder::{BigEndian, ByteOrder};
use memchr::memmem;

use super::FindJpegType;
use super::is_better;
use super::jpeg::{EmbeddedJpegInfo, JPEG_SOI_PREFIX};

const CANON_PRVW_UUID: &[u8; 16] =
    b"\xEA\xF4\x2B\x5E\x1C\x98\x4B\x88\xB9\xFB\xB7\xDC\x40\x6E\x4D\x16";
const CANON_THMB_UUID: &[u8; 16] =
    b"\x85\xC0\xB6\x87\x82\x0F\x11\xE0\x81\x11\xF4\xCE\x46\x2B\x6A\x48";
const MAX_BOXES: usize = 4096;

pub(crate) fn find_cr3_preview(
    raw_buf: &[u8],
    find_type: FindJpegType,
) -> Option<EmbeddedJpegInfo> {
    if raw_buf.get(4..8)? != b"ftyp" || raw_buf.get(8..12)? != b"crx " {
        return None;
    }

    let mut pending = vec![(0usize, raw_buf.len())];
    let mut best: Option<EmbeddedJpegInfo> = None;
    let mut seen = 0usize;

    while let Some((mut pos, end)) = pending.pop() {
        while pos
            .checked_add(8)
            .is_some_and(|header_end| header_end <= end)
        {
            seen += 1;
            if seen > MAX_BOXES {
                return best;
            }
            let Some(size_bytes) = raw_buf.get(pos..pos + 4) else {
                break;
            };
            let Ok(size) = usize::try_from(BigEndian::read_u32(size_bytes)) else {
                break;
            };
            if size < 8 {
                break;
            }
            let Some(box_end) = pos.checked_add(size) else {
                break;
            };
            if box_end > end || box_end > raw_buf.len() {
                break;
            }
            let Some(kind) = raw_buf.get(pos + 4..pos + 8) else {
                break;
            };
            let payload = pos + 8;

            match kind {
                b"moov" => pending.push((payload, box_end)),
                b"uuid" => {
                    let Some(uuid) = raw_buf.get(payload..payload + 16) else {
                        break;
                    };
                    let uuid_payload = payload + 16;
                    if uuid == CANON_PRVW_UUID {
                        if let Some(child_start) = uuid_payload.checked_add(8) {
                            pending.push((child_start, box_end));
                        }
                    } else if uuid == CANON_THMB_UUID {
                        pending.push((uuid_payload, box_end));
                    }
                }
                b"THMB" | b"PRVW" => {
                    let size_offset = if kind == b"THMB" { 8 } else { 12 };
                    if let Some(candidate) =
                        preview_candidate(raw_buf, payload, box_end, size_offset)
                        && is_better(
                            candidate.length,
                            best.map_or(0, |jpeg| jpeg.length),
                            find_type,
                        )
                    {
                        best = Some(candidate);
                    }
                }
                _ => {}
            }

            pos = box_end;
        }
    }

    best
}

fn preview_candidate(
    raw_buf: &[u8],
    payload: usize,
    box_end: usize,
    size_offset: usize,
) -> Option<EmbeddedJpegInfo> {
    let jpeg_size = usize::try_from(BigEndian::read_u32(
        raw_buf.get(payload + size_offset..payload + size_offset + 4)?,
    ))
    .ok()?;
    let jpeg_start = payload.checked_add(16)?;
    let jpeg_end = jpeg_start.checked_add(jpeg_size)?;
    if jpeg_end <= box_end && raw_buf.get(jpeg_start..jpeg_start + 3) == Some(JPEG_SOI_PREFIX) {
        return Some(EmbeddedJpegInfo::new(jpeg_start, jpeg_size, None));
    }

    let rel = memmem::find(raw_buf.get(payload..box_end)?, JPEG_SOI_PREFIX)?;
    let jpeg_start = payload + rel;
    Some(EmbeddedJpegInfo::new(
        jpeg_start,
        box_end - jpeg_start,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cr3_preview_picker_walks_prvw_and_thmb_boxes() {
        let small = make_test_jpeg(16);
        let large = make_test_jpeg(64);
        let buf = synthetic_cr3(&small, &large);

        let largest = find_cr3_preview(&buf, FindJpegType::Largest).unwrap();
        assert_eq!(&buf[largest.offset..largest.offset + largest.length], large);

        let smallest = find_cr3_preview(&buf, FindJpegType::Smallest).unwrap();
        assert_eq!(
            &buf[smallest.offset..smallest.offset + smallest.length],
            small
        );
    }

    #[test]
    fn malformed_cr3_boxes_return_none() {
        let mut size_underflow = Vec::new();
        size_underflow.extend_from_slice(&12u32.to_be_bytes());
        size_underflow.extend_from_slice(b"ftypcrx ");
        size_underflow.extend_from_slice(&7u32.to_be_bytes());
        size_underflow.extend_from_slice(b"free");
        assert!(find_cr3_preview(&size_underflow, FindJpegType::Largest).is_none());

        let mut truncated = Vec::new();
        truncated.extend_from_slice(&12u32.to_be_bytes());
        truncated.extend_from_slice(b"ftypcrx ");
        truncated.extend_from_slice(&32u32.to_be_bytes());
        truncated.extend_from_slice(b"free");
        assert!(find_cr3_preview(&truncated, FindJpegType::Largest).is_none());
    }

    #[test]
    fn cr3_returns_found_preview_before_trailing_truncated_box() {
        let large = make_test_jpeg(64);
        let mut buf = Vec::new();
        buf.extend_from_slice(&box_with_payload(b"ftyp", b"crx "));
        let prvw_box = preview_box(b"PRVW", &large);
        let mut prvw_uuid_payload = Vec::new();
        prvw_uuid_payload.extend_from_slice(CANON_PRVW_UUID);
        prvw_uuid_payload.extend_from_slice(&[0u8; 8]);
        prvw_uuid_payload.extend_from_slice(&prvw_box);
        buf.extend_from_slice(&box_with_payload(b"uuid", &prvw_uuid_payload));
        buf.extend_from_slice(&32u32.to_be_bytes());
        buf.extend_from_slice(b"free");

        let found = find_cr3_preview(&buf, FindJpegType::Largest).unwrap();
        assert_eq!(&buf[found.offset..found.offset + found.length], large);
    }

    fn synthetic_cr3(thmb: &[u8], prvw: &[u8]) -> Vec<u8> {
        let thmb_box = thmb_box(thmb);
        let mut thmb_uuid_payload = Vec::new();
        thmb_uuid_payload.extend_from_slice(CANON_THMB_UUID);
        thmb_uuid_payload.extend_from_slice(&thmb_box);

        let mut moov_payload = Vec::new();
        moov_payload.extend_from_slice(&box_with_payload(b"uuid", &thmb_uuid_payload));

        let prvw_box = preview_box(b"PRVW", prvw);
        let mut prvw_uuid_payload = Vec::new();
        prvw_uuid_payload.extend_from_slice(CANON_PRVW_UUID);
        prvw_uuid_payload.extend_from_slice(&[0u8; 8]);
        prvw_uuid_payload.extend_from_slice(&prvw_box);

        let mut out = Vec::new();
        out.extend_from_slice(&box_with_payload(b"ftyp", b"crx "));
        out.extend_from_slice(&box_with_payload(b"moov", &moov_payload));
        out.extend_from_slice(&box_with_payload(b"uuid", &prvw_uuid_payload));
        out
    }

    fn thmb_box(jpeg: &[u8]) -> Vec<u8> {
        let mut payload = vec![0u8; 8];
        payload.extend_from_slice(&(jpeg.len() as u32).to_be_bytes());
        payload.extend_from_slice(&[0u8; 4]);
        payload.extend_from_slice(jpeg);
        box_with_payload(b"THMB", &payload)
    }

    fn preview_box(kind: &[u8; 4], jpeg: &[u8]) -> Vec<u8> {
        let mut payload = vec![0u8; 12];
        payload.extend_from_slice(&(jpeg.len() as u32).to_be_bytes());
        payload.extend_from_slice(jpeg);
        box_with_payload(kind, &payload)
    }

    fn box_with_payload(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(8u32 + payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn make_test_jpeg(dqt_padding: usize) -> Vec<u8> {
        let dqt_len = u16::try_from(2 + 1 + dqt_padding).unwrap();
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xDB];
        jpeg.extend_from_slice(&dqt_len.to_be_bytes());
        jpeg.extend(std::iter::repeat_n(0, 1 + dqt_padding));
        jpeg.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
        ]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        jpeg
    }
}
