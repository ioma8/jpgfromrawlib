# jpgfromraw

jpgfromraw provides a much faster way to extract embedded JPEGs from RAW files
than exiftool's `-JpgFromRaw`. In a directory with 4000 files, jpgfromraw
extracts JPEGs about 15 times faster than exiftool:

    % rm -rf ~/jfr && mkdir -p ~/jfr/{jpgfromraw,exiftool}
    % sudo sh -c 'sync; echo 3 > /proc/sys/vm/drop_caches'
    % \time -v jpgfromraw /mnt/sdcard/DCIM/101MSDCF ~/jfr/jpgfromraw >/dev/null
        User time (seconds): 2.18
        System time (seconds): 20.99
        Elapsed (wall clock) time (h:mm:ss or m:ss): 0:29.72
        File system inputs: 22956551
        File system outputs: 22855504


    % sudo sh -c 'sync; echo 3 > /proc/sys/vm/drop_caches'
    % \time -v exiftool -b -JpgFromRaw -ext arw -r /mnt/sdcard/DCIM/101MSDCF -w ~/jfr/exiftool/%f.jpg >/dev/null
        User time (seconds): 113.21
        System time (seconds): 58.15
        Elapsed (wall clock) time (h:mm:ss or m:ss): 7:52.87
        File system inputs: 316175244
        File system outputs: 22864400

The total size of the output JPEGs is 11.5GiB, so in terms of throughput,
jpgfromraw does ~386MiB/s, and exiftool does ~24MiB/s.

The key reason jpgfromraw is so much faster is because it very carefully avoids
overreading into the entire RAW file. exiftool does not do that and suffers
quite greatly in (useful) throughput as a result. This is achieved through
judicious use of `madvise` (and similar strategies on other platforms).

Other than that, jpgfromraw also processes multiple files concurrently, which
can help a lot on faster devices like CFexpress cards.

## CLI usage

    jpgfromraw <input_dir> [output_dir] [-t <transfers>] [-e <extra_extension>]

The input directory is searched recursively; the relative directory structure
is recreated in the output directory. Extensions are matched
case-insensitively against the supported list (`-e` adds one more):

3fr, ari, arw, bay, braw, cap, cr2, cr3, cri, crw, dcr, dcs, dng, drf, eip,
erf, fff, gpr, iiq, jxs, k25, kdc, mdc, mef, mos, mrw, nef, nrw, orf, pef,
ptx, pxn, r3d, raf, raw, rw2, rwl, rwz, sr2, srf, srw, tco, x3f

`-t/--transfers` controls how many files are processed concurrently. The
permit is acquired before each file is opened, so file descriptors and mapped
memory stay bounded by that value. On warm-cache happy-path workloads the
scaling is sub-linear but real; in the included audit, throughput improved
from roughly 22k files/sec at `-t 1` to 59k files/sec at `-t 4`.

## Library usage

The crate also exposes the extraction pipeline as a library:

```rust
use jpgfromraw::{process_file_bytes, FindJpegType};

let jpeg: Vec<u8> = process_file_bytes(path, FindJpegType::Largest).await?;
```

`FindJpegType::Largest` picks the biggest embedded preview (usually
full-size), `FindJpegType::Smallest` the smallest (usually a thumbnail).
`process_file` additionally writes the result to an output directory.

The functions are async and run heavy fallback decodes on a blocking thread
when driven by a tokio runtime; under any other executor they still work, with
the fallback decode running inline.

For parallel library use, spawn tasks yourself and bound concurrency with a
`tokio::sync::Semaphore`, ideally acquiring the permit before calling
`process_file_bytes` so open files and mapped memory stay capped:

```rust
use std::sync::Arc;

use jpgfromraw::{process_file_bytes, FindJpegType};
use tokio::sync::Semaphore;

async fn extract_many(paths: Vec<std::path::PathBuf>, concurrency: usize) -> anyhow::Result<Vec<Vec<u8>>> {
    let paths = Arc::new(paths);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = Vec::with_capacity(paths.len());

    for index in 0..paths.len() {
        let paths = Arc::clone(&paths);
        let semaphore = Arc::clone(&semaphore);
        tasks.push(tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await?;
            process_file_bytes(&paths[index], FindJpegType::Largest).await
        }));
    }

    let mut outputs = Vec::with_capacity(tasks.len());
    for task in tasks {
        outputs.push(task.await??);
    }

    Ok(outputs)
}
```

This is the same pattern used by the CLI and the scaling audit.

## How it works

For each file, jpgfromraw tries a chain of strategies, cheapest first:

1. Parse the TIFF/EXIF IFD chain and extract the referenced embedded JPEG
   (memory-mapped, touching only the bytes needed).
2. Scan for raw JPEG SOI/EOI markers, including JPEGs with a missing leading
   `0xFF` byte, validating candidates before accepting them.
3. Re-encode an uncompressed RGB TIFF preview as JPEG, if the file has one.
4. Decode generic raster formats (PNG, TIFF, WebP, BMP, ...) via the `image`
   crate and re-encode as JPEG.
5. Fall back to the vendored gpr_tools (for GPR) or dcraw (everything else)
   to develop a preview from the raw sensor data.

The embedded JPEG is emitted with a minimal EXIF header carrying over the
orientation tag, so images display the right way up.

## Building

The vendored fallbacks need a C toolchain, and gpr_tools is built with CMake:

- a C compiler (used by `cc` to build the bundled dcraw)
- `cmake` on the PATH (used to build `vendor/gpr`)

Then a regular `cargo build --release` works.

### Cargo features

- `generic-raster` (default): enables the heavier image decoders (AVIF, DDS,
  EXR) for the generic raster fallback. Building with
  `--no-default-features` drops those decoders and their native
  dependencies; files with those extensions then go through the RAW
  fallbacks instead.

## Testing

`cargo test` runs the unit tests everywhere. The integration suites need
fixtures that are too large to commit and skip themselves when absent:

- `tests/sample_corpus_tests.rs` expects RAW samples in `sample_raws/`
  (source URLs are listed in the test file).
- `tests/generic_raster_formats_tests.rs` expects fixtures in
  `sample_generic/`.
- `tests/process_file_bytes_tests.rs` runs against a local directory of RAW
  files given via the `JPGFROMRAW_TEST_DIR` environment variable.
