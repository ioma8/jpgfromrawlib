use anyhow::Result;
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
pub(crate) mod jpeg;
pub(crate) mod tiff;

pub(crate) async fn process_file_bytes(
    entry_path: &Path,
    find_type: FindJpegType,
) -> Result<Vec<u8>> {
    let in_file = platform::open_raw(entry_path)?;
    let raw_buf = platform::mmap_raw(&in_file)?;
    let tiff_offset = tiff::find_tiff_header_offset(&raw_buf).ok();
    let jpeg_info = match tiff_offset {
        Some(tiff_offset) => tiff::find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, find_type)
            .ok()
            .flatten()
            .or_else(|| jpeg::find_embedded_jpeg_by_markers(&raw_buf, find_type).ok()),
        None => jpeg::find_embedded_jpeg_by_markers(&raw_buf, find_type).ok(),
    };

    if let Some(jpeg_info) = jpeg_info {
        let jpeg_buf = jpeg::extract_jpeg(&raw_buf, &jpeg_info);
        if jpeg::is_valid_jpeg(jpeg_buf) {
            return Ok(jpeg::get_jpeg_data(jpeg_buf, &jpeg_info));
        }
    }

    if let Some(jpeg_data) = jpeg::try_missing_ff(&raw_buf, find_type) {
        return Ok(jpeg_data);
    }
    if let Some(jpeg_data) = tiff::try_tiff_preview(&raw_buf, tiff_offset, find_type)? {
        return Ok(jpeg_data);
    }

    fallback::decode_raw_preview_fallback(entry_path).await
}

#[cfg(test)]
mod tests {
    use super::jpeg::{
        JPEG_EOI, JPEG_SOI_MISSING_PREFIX, JPEG_SOI_PREFIX, extract_jpeg,
        find_embedded_jpeg_by_markers, find_embedded_jpeg_missing_ff_prefix, get_jpeg_data,
        is_valid_jpeg,
    };
    use super::tiff::{
        EXIF_HEADER, EXIF_HEADER_SIZE, TIFF_HEADER, find_embedded_jpeg_in_tiff,
        find_tiff_header_offset, find_tiff_preview,
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
                std::hint::black_box(get_jpeg_data(jpeg_buf, &info));
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
