use anyhow::Result;
use memmap2::Mmap;
use std::path::Path;

pub(crate) use crate::types::FindJpegType;
fn is_better(candidate: usize, best: usize, find_type: FindJpegType) -> bool {
    match find_type {
        FindJpegType::Largest => candidate > best,
        FindJpegType::Smallest => best == 0 || candidate < best,
    }
}

use crate::fallback;
use crate::platform;
pub(crate) mod bmff;
pub(crate) mod jpeg;
pub(crate) mod tiff;

pub(crate) async fn process_file_bytes(
    entry_path: &Path,
    find_type: FindJpegType,
) -> Result<Vec<u8>> {
    let (raw_buf, jpeg_info) = map_embedded_jpeg(entry_path, find_type)?;
    if let Some(jpeg_info) = jpeg_info {
        let jpeg_buf = jpeg::extract_jpeg(&raw_buf, &jpeg_info);
        return jpeg::get_jpeg_data(jpeg_buf, &jpeg_info);
    }

    process_mapped_fallback(entry_path, find_type, raw_buf).await
}

pub(crate) async fn process_mapped_fallback(
    entry_path: &Path,
    find_type: FindJpegType,
    raw_buf: Mmap,
) -> Result<Vec<u8>> {
    let tiff_offset = tiff::find_tiff_header_offset(&raw_buf).ok();
    if let Some(jpeg_data) = jpeg::try_missing_ff(&raw_buf, find_type) {
        return Ok(jpeg_data);
    }
    if let Some(jpeg_data) = tiff::try_tiff_preview(&raw_buf, tiff_offset, find_type)? {
        return Ok(jpeg_data);
    }

    fallback::decode_raw_preview_fallback(entry_path, Some(raw_buf)).await
}

pub(crate) fn map_embedded_jpeg(
    entry_path: &Path,
    find_type: FindJpegType,
) -> Result<(Mmap, Option<jpeg::EmbeddedJpegInfo>)> {
    let in_file = platform::open_raw(entry_path)?;
    let raw_buf = platform::mmap_raw(&in_file)?;
    let tiff_offset = tiff::find_tiff_header_offset(&raw_buf).ok();
    let mut structured = Vec::new();
    structured.extend(bmff::find_cr3_preview(&raw_buf, find_type));
    structured.extend(jpeg::find_raf_jpeg(&raw_buf));
    if let Some(offset) = tiff_offset
        && let Ok(mut candidates) = tiff::find_embedded_jpegs_in_tiff(&raw_buf, offset)
    {
        candidates.sort_unstable_by(|left, right| match find_type {
            FindJpegType::Largest => right.length.cmp(&left.length),
            FindJpegType::Smallest => left.length.cmp(&right.length),
        });
        structured.extend(candidates);
    }

    for jpeg_info in structured {
        if jpeg_info
            .offset
            .checked_add(jpeg_info.length)
            .is_none_or(|end| end > raw_buf.len())
        {
            continue;
        }
        let jpeg_buf = jpeg::extract_jpeg(&raw_buf, &jpeg_info);
        if jpeg::is_valid_jpeg(jpeg_buf) {
            return Ok((raw_buf, Some(jpeg_info)));
        }
    }

    if let Some(jpeg_info) = jpeg::find_embedded_jpeg_by_markers(&raw_buf, find_type).ok()
        && jpeg_info
            .offset
            .checked_add(jpeg_info.length)
            .is_some_and(|end| end <= raw_buf.len())
    {
        let jpeg_buf = jpeg::extract_jpeg(&raw_buf, &jpeg_info);
        if jpeg::is_valid_jpeg(jpeg_buf) {
            return Ok((raw_buf, Some(jpeg_info)));
        }
    }
    Ok((raw_buf, None))
}

#[cfg(test)]
mod tests {
    use super::jpeg::{
        JPEG_EOI, JPEG_SOI_MISSING_PREFIX, JPEG_SOI_PREFIX, extract_jpeg,
        find_embedded_jpeg_by_markers, find_embedded_jpeg_missing_ff_prefix, find_raf_jpeg,
        get_jpeg_data, is_valid_jpeg,
    };
    use super::tiff::{
        EXIF_HEADER, EXIF_HEADER_SIZE, TIFF_HEADER, find_embedded_jpeg_in_tiff,
        find_embedded_jpegs_in_tiff, find_tiff_header_offset, find_tiff_preview,
    };
    use super::*;
    use memchr::memmem;

    #[test]
    fn short_buffers_do_not_panic_when_searching_for_tiff_header() {
        for len in 0..4 {
            let buf = vec![0u8; len];
            assert!(find_tiff_header_offset(&buf).is_err());
        }
    }

    #[test]
    fn exif_search_is_bounded_to_file_head() {
        let mut buf = vec![0u8; 2 * 1024 * 1024];
        let far = 1024 * 1024;
        buf[far..far + EXIF_HEADER_SIZE].copy_from_slice(EXIF_HEADER);
        assert!(find_tiff_header_offset(&buf).is_err());

        let mut buf = vec![0u8; 2 * 1024 * 1024];
        let near = 4096;
        buf[near..near + EXIF_HEADER_SIZE].copy_from_slice(EXIF_HEADER);
        buf[near + EXIF_HEADER_SIZE..near + EXIF_HEADER_SIZE + 4].copy_from_slice(TIFF_HEADER);
        assert_eq!(
            find_tiff_header_offset(&buf).unwrap(),
            near + EXIF_HEADER_SIZE
        );
    }

    #[test]
    fn tiff_sub_ifd_jpeg_is_found() {
        let jpeg = make_test_jpeg(32);
        let jpeg_offset = 96usize;
        let sub_ifd_offset = 40usize;
        let mut buf = le_tiff(8);
        buf.extend_from_slice(&1u16.to_le_bytes());
        tiff_entry(&mut buf, 0x14A, 4, 1, sub_ifd_offset as u32);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(sub_ifd_offset, 0);
        buf.extend_from_slice(&3u16.to_le_bytes());
        tiff_entry(&mut buf, 0x201, 4, 1, jpeg_offset as u32);
        tiff_entry(&mut buf, 0x202, 4, 1, jpeg.len() as u32);
        tiff_entry(&mut buf, 0x112, 3, 1, 6);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(jpeg_offset, 0);
        buf.extend_from_slice(&jpeg);

        let found = find_embedded_jpeg_in_tiff(&buf, 0, FindJpegType::Largest)
            .unwrap()
            .unwrap();
        assert_eq!(found.offset, jpeg_offset);
        assert_eq!(found.length, jpeg.len());
        assert_eq!(found.orientation, Some(6));
    }

    #[test]
    fn rw2_jpg_from_raw_tag_is_found() {
        let jpeg = make_test_jpeg(32);
        let jpeg_offset = 40usize;
        let mut buf = le_tiff_with_header(b"IIU\0", 8);
        buf.extend_from_slice(&1u16.to_le_bytes());
        tiff_entry(&mut buf, 0x2E, 7, jpeg.len() as u32, jpeg_offset as u32);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(jpeg_offset, 0);
        buf.extend_from_slice(&jpeg);

        let found = find_embedded_jpeg_in_tiff(&buf, 0, FindJpegType::Largest)
            .unwrap()
            .unwrap();
        assert_eq!(found.offset, jpeg_offset);
        assert_eq!(found.length, jpeg.len());
        assert_eq!(find_tiff_header_offset(&buf).unwrap(), 0);
    }

    #[test]
    fn unsupported_sub_ifd_type_does_not_abort_tiff_walk() {
        let jpeg = make_test_jpeg(32);
        let jpeg_offset = 64usize;
        let next_ifd_offset = 32usize;
        let mut buf = le_tiff(8);
        buf.extend_from_slice(&1u16.to_le_bytes());
        tiff_entry(&mut buf, 0x14A, 13, 1, 0);
        buf.extend_from_slice(&(next_ifd_offset as u32).to_le_bytes());
        buf.resize(next_ifd_offset, 0);
        buf.extend_from_slice(&2u16.to_le_bytes());
        tiff_entry(&mut buf, 0x201, 4, 1, jpeg_offset as u32);
        tiff_entry(&mut buf, 0x202, 4, 1, jpeg.len() as u32);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(jpeg_offset, 0);
        buf.extend_from_slice(&jpeg);

        let found = find_embedded_jpeg_in_tiff(&buf, 0, FindJpegType::Largest)
            .unwrap()
            .unwrap();
        assert_eq!(found.offset, jpeg_offset);
        assert_eq!(found.length, jpeg.len());
    }

    #[test]
    fn largest_tiff_preview_still_checks_sub_ifds_after_small_strip_preview() {
        let small = make_test_jpeg(16);
        let large = make_test_jpeg(64);
        let small_offset = 96usize;
        let large_offset = small_offset + small.len();
        let sub_ifd_offset = 56usize;
        let mut buf = le_tiff(8);
        buf.extend_from_slice(&3u16.to_le_bytes());
        tiff_entry(&mut buf, 0x111, 4, 1, small_offset as u32);
        tiff_entry(&mut buf, 0x117, 4, 1, small.len() as u32);
        tiff_entry(&mut buf, 0x14A, 4, 1, sub_ifd_offset as u32);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(sub_ifd_offset, 0);
        buf.extend_from_slice(&2u16.to_le_bytes());
        tiff_entry(&mut buf, 0x201, 4, 1, large_offset as u32);
        tiff_entry(&mut buf, 0x202, 4, 1, large.len() as u32);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(small_offset, 0);
        buf.extend_from_slice(&small);
        buf.extend_from_slice(&large);

        let found = find_embedded_jpeg_in_tiff(&buf, 0, FindJpegType::Largest)
            .unwrap()
            .unwrap();
        assert_eq!(found.offset, large_offset);
        assert_eq!(found.length, large.len());
    }

    #[test]
    fn raf_header_jpeg_is_found() {
        let jpeg = make_test_jpeg(32);
        let offset = 100usize;
        let mut buf = Vec::from(&b"FUJIFILMCCD-RAW"[..]);
        buf.resize(84, 0);
        buf.extend_from_slice(&(offset as u32).to_be_bytes());
        buf.extend_from_slice(&(jpeg.len() as u32).to_be_bytes());
        buf.resize(offset, 0);
        buf.extend_from_slice(&jpeg);

        let found = find_raf_jpeg(&buf).unwrap();
        assert_eq!(found.offset, offset);
        assert_eq!(found.length, jpeg.len());
        assert!(find_raf_jpeg(b"not raf").is_none());

        let mut bad = buf.clone();
        bad[84..88].copy_from_slice(&(999_999u32).to_be_bytes());
        assert!(find_raf_jpeg(&bad).is_none());
    }

    #[test]
    fn invalid_raf_candidate_falls_through_to_marker_recovery() {
        let invalid = [0xff, 0xd8, 0xff, 0xdb, 0, 4, 0, 0, 0xff, 0xd9];
        let valid = make_test_jpeg(32);
        let offset = 100usize;
        let mut buf = Vec::from(&b"FUJIFILMCCD-RAW"[..]);
        buf.resize(84, 0);
        buf.extend_from_slice(&(offset as u32).to_be_bytes());
        buf.extend_from_slice(&(invalid.len() as u32).to_be_bytes());
        buf.resize(offset, 0);
        buf.extend_from_slice(&invalid);
        buf.extend_from_slice(&valid);

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("invalid.raf");
        std::fs::write(&path, buf).unwrap();
        let (mapped, found) = map_embedded_jpeg(&path, FindJpegType::Largest).unwrap();
        let found = found.unwrap();
        assert_eq!(&mapped[found.offset..found.offset + found.length], valid);
    }

    #[test]
    fn invalid_cr3_candidate_falls_through_to_marker_recovery() {
        let invalid = [0xff, 0xd8, 0xff, 0xdb, 0, 4, 0, 0, 0xff, 0xd9];
        let valid = make_test_jpeg(32);
        let mut buf = Vec::new();
        buf.extend_from_slice(&12u32.to_be_bytes());
        buf.extend_from_slice(b"ftypcrx ");
        let mut payload = vec![0u8; 12];
        payload.extend_from_slice(&(invalid.len() as u32).to_be_bytes());
        payload.extend_from_slice(&invalid);
        buf.extend_from_slice(&(8u32 + payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(b"PRVW");
        buf.extend_from_slice(&payload);
        buf.extend_from_slice(&valid);

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("invalid.cr3");
        std::fs::write(&path, buf).unwrap();
        let (mapped, found) = map_embedded_jpeg(&path, FindJpegType::Largest).unwrap();
        let found = found.unwrap();
        assert_eq!(&mapped[found.offset..found.offset + found.length], valid);
    }

    #[test]
    fn invalid_larger_tiff_candidate_does_not_hide_valid_smaller_candidate() {
        let invalid = vec![0xff, 0xd8, 0xff, 0xdb, 0, 4, 0, 0, 0xff, 0xd9];
        let valid = make_test_jpeg(32);
        let bad_offset = 80usize;
        let valid_offset = bad_offset + invalid.len() + 8;
        let first_ifd_offset = 8usize;
        let second_ifd_offset = 40usize;
        let mut buf = le_tiff(first_ifd_offset as u32);

        buf.extend_from_slice(&2u16.to_le_bytes());
        tiff_entry(&mut buf, 0x111, 4, 1, bad_offset as u32);
        tiff_entry(&mut buf, 0x117, 4, 1, (invalid.len() + valid.len()) as u32);
        buf.extend_from_slice(&(second_ifd_offset as u32).to_le_bytes());

        buf.resize(second_ifd_offset, 0);
        buf.extend_from_slice(&2u16.to_le_bytes());
        tiff_entry(&mut buf, 0x201, 4, 1, valid_offset as u32);
        tiff_entry(&mut buf, 0x202, 4, 1, valid.len() as u32);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(bad_offset, 0);
        buf.extend_from_slice(&invalid);
        buf.resize(valid_offset, 0);
        buf.extend_from_slice(&valid);

        let candidates = find_embedded_jpegs_in_tiff(&buf, 0).unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].length > candidates[1].length);

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("invalid.tif");
        std::fs::write(&path, buf).unwrap();
        let (mapped, found) = map_embedded_jpeg(&path, FindJpegType::Largest).unwrap();
        let found = found.unwrap();
        assert_eq!(found.offset, valid_offset);
        assert_eq!(&mapped[found.offset..found.offset + found.length], valid);
    }

    #[test]
    fn zero_count_tiff_fields_error_instead_of_panicking() {
        let buf = [
            b'I', b'I', b'*', 0, 8, 0, 0, 0, 1, 0, 0x00, 0x01, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0,
        ];
        let err = find_tiff_preview(&buf, 0, FindJpegType::Largest).unwrap_err();
        assert!(err.to_string().contains("ImageWidth is empty"));
    }

    #[test]
    fn tiff_ifd_cycles_error_out() {
        let buf = [b'I', b'I', b'*', 0, 8, 0, 0, 0, 0, 0, 8, 0, 0, 0];
        let err = find_tiff_preview(&buf, 0, FindJpegType::Largest).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn orientation_header_tiff_has_next_ifd_terminator() {
        let header = jpeg::get_header_bytes(6);
        assert_eq!(header.len(), 38);
        assert_eq!(&header[4..6], &[0x00, 0x22]);
        assert_eq!(&header[30..34], &[6, 0, 0, 0]);
        assert_eq!(&header[34..38], &[0, 0, 0, 0]);
        let candidates = find_embedded_jpegs_in_tiff(&header[12..], 0).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn tiff_jpeg_shorter_than_two_bytes_is_rejected() {
        let mut buf = le_tiff(8);
        buf.extend_from_slice(&2u16.to_le_bytes());
        tiff_entry(&mut buf, 0x201, 4, 1, 40);
        tiff_entry(&mut buf, 0x202, 4, 1, 1);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.resize(41, 0xFF);
        let candidates = find_embedded_jpegs_in_tiff(&buf, 0).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn assembling_short_jpeg_is_an_error() {
        let err = get_jpeg_data(b"\xFF", &jpeg::EmbeddedJpegInfo::new(0, 1, None)).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn raf_jpeg_shorter_than_two_bytes_is_rejected() {
        let mut buf = Vec::from(&b"FUJIFILMCCD-RAW"[..]);
        buf.resize(84, 0);
        buf.extend_from_slice(&100u32.to_be_bytes());
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.resize(101, 0xFF);
        assert!(find_raf_jpeg(&buf).is_none());
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
    fn marker_scans_stop_after_an_eoi_free_tail() {
        let mut buf = make_test_jpeg(16);
        buf.extend(std::iter::repeat_n(JPEG_SOI_PREFIX.iter().copied(), 128).flatten());

        for find_type in [FindJpegType::Largest, FindJpegType::Smallest] {
            let found = find_embedded_jpeg_by_markers(&buf, find_type).unwrap();
            assert_eq!(found.offset, 0);
        }
    }

    #[test]
    fn valid_marker_jpeg_survives_unterminated_later_candidates() {
        let valid = make_test_jpeg(64);
        let mut buf = valid.clone();
        buf.extend(std::iter::repeat_n(JPEG_SOI_PREFIX.iter().copied(), 128).flatten());

        for find_type in [FindJpegType::Largest, FindJpegType::Smallest] {
            let found = find_embedded_jpeg_by_markers(&buf, find_type).unwrap();
            assert_eq!(found.offset, 0);
            assert_eq!(found.length, valid.len());
        }
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
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("garbage.arw");
        std::fs::write(&path, [0u8; 64]).unwrap();

        let result = block_on_without_tokio(process_file_bytes(&path, FindJpegType::Largest));
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
        jpeg.extend(std::iter::repeat_n(0, 1 + dqt_padding));
        // SOF0: precision 8, 1x1, one component.
        jpeg.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
        ]);
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

    fn le_tiff(first_ifd_offset: u32) -> Vec<u8> {
        le_tiff_with_header(TIFF_HEADER, first_ifd_offset)
    }

    fn le_tiff_with_header(header: &[u8; 4], first_ifd_offset: u32) -> Vec<u8> {
        let mut buf = Vec::from(&header[..]);
        buf.extend_from_slice(&first_ifd_offset.to_le_bytes());
        buf
    }

    fn tiff_entry(buf: &mut Vec<u8>, tag: u16, type_id: u16, count: u32, value: u32) {
        buf.extend_from_slice(&tag.to_le_bytes());
        buf.extend_from_slice(&type_id.to_le_bytes());
        buf.extend_from_slice(&count.to_le_bytes());
        buf.extend_from_slice(&value.to_le_bytes());
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
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
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

        report_stage(
            "find_tiff_header_offset (TIFF at 0)",
            &buffers,
            200,
            |buf| {
                std::hint::black_box(find_tiff_header_offset(buf).unwrap());
            },
        );

        report_stage(
            "find_embedded_jpeg_in_tiff Smallest",
            &buffers,
            200,
            |buf| {
                std::hint::black_box(
                    find_embedded_jpeg_in_tiff(buf, 0, FindJpegType::Smallest).unwrap(),
                );
            },
        );

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
            std::hint::black_box(get_jpeg_data(jpeg_buf, &info).unwrap());
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
        report_stage(
            "missing-FF scan Largest (fallback)",
            scan_buffers,
            5,
            |buf| {
                std::hint::black_box(find_embedded_jpeg_missing_ff_prefix(
                    buf,
                    FindJpegType::Largest,
                ))
                .ok();
            },
        );

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
        report_stage(
            "process_file_bytes Smallest (end-to-end)",
            &files,
            50,
            |path| {
                let jpeg = runtime
                    .block_on(process_file_bytes(path, FindJpegType::Smallest))
                    .unwrap();
                std::hint::black_box(jpeg);
            },
        );
        report_stage(
            "process_file_bytes Largest (end-to-end)",
            &files,
            50,
            |path| {
                let jpeg = runtime
                    .block_on(process_file_bytes(path, FindJpegType::Largest))
                    .unwrap();
                std::hint::black_box(jpeg);
            },
        );

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
                find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, FindJpegType::Smallest).unwrap(),
            );
        });
        report_stage(
            "open + mmap + parse + extract/validate",
            &files,
            50,
            |path| {
                let file = platform::open_raw(path).unwrap();
                let raw_buf = platform::mmap_raw(&file).unwrap();
                let tiff_offset = find_tiff_header_offset(&raw_buf).unwrap();
                let info =
                    find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, FindJpegType::Smallest)
                        .unwrap()
                        .unwrap();
                let jpeg_buf = extract_jpeg(&raw_buf, &info);
                assert!(is_valid_jpeg(jpeg_buf));
                std::hint::black_box(get_jpeg_data(jpeg_buf, &info).unwrap());
            },
        );
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
        println!(
            "{label:<55} median {:>12.3} us  min {:>12.3} us",
            median * 1e6,
            min * 1e6
        );
    }
}
