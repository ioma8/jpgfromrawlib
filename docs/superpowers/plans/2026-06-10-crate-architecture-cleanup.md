# Crate Architecture Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the crate surface extractor-oriented, reduce coupling to `parser`, and split `src/parser/mod.rs` into focused internal modules without changing extraction behavior or performance characteristics.

**Architecture:** First lock down the public API at the crate root and stop tests from depending on parser internals. Then extract public types and orchestration into semantic modules, followed by moving TIFF logic, JPEG logic, fallback code, and platform code into private modules with clear responsibilities. Keep the documented root API stable and use the existing release audits to guard performance.

**Tech Stack:** Rust 2024, Tokio, `memmap2`, `memchr`, `byteorder`, `image`, `tempfile`, `trybuild`

---

## File Structure

- Create: `src/types.rs`
  Responsibility: public library enums/constants such as `FindJpegType` and `SUPPORTED_EXTENSIONS`.
- Create: `src/extract.rs`
  Responsibility: public async entrypoints, output-path helper, and orchestration of fast path vs fallback.
- Create: `src/jpeg.rs`
  Responsibility: embedded JPEG metadata, marker scanning, missing-`0xFF` recovery, JPEG validation, EXIF header assembly.
- Create: `src/tiff.rs`
  Responsibility: TIFF header detection, TIFF reader, IFD walking, embedded JPEG/TIFF preview discovery.
- Create: `src/fallback/mod.rs`
  Responsibility: fallback orchestration and the shared blocking-runtime bridge.
- Create: `src/fallback/generic.rs`
  Responsibility: generic raster decode/re-encode fallback.
- Create: `src/fallback/dcraw.rs`
  Responsibility: dcraw subprocess fallback.
- Create: `src/fallback/gpr.rs`
  Responsibility: gpr_tools subprocess fallback.
- Create: `src/platform/mod.rs`
  Responsibility: platform dispatch for file open, mmap, and prefetch helpers.
- Create: `src/platform/unix.rs`
  Responsibility: Unix mmap/open/prefetch implementation.
- Create: `src/platform/windows.rs`
  Responsibility: Windows mmap/open/prefetch implementation.
- Create: `tests/public_api_contract_tests.rs`
  Responsibility: integration tests for crate-root exports and parser-internal compile-fail coverage.
- Create: `tests/ui/parser_internal_api_is_hidden.rs`
  Responsibility: compile-fail fixture proving parser internals are not public API.
- Modify: `src/lib.rs`
  Responsibility: semantic crate root exports plus a narrow compatibility shim for `jpgfromraw::parser`.
- Modify: `tests/process_file_bytes_tests.rs`
  Responsibility: stop importing through `jpgfromraw::parser`.
- Modify: `tests/sample_corpus_tests.rs`
  Responsibility: replace `jpgfromraw::parser::is_sof_marker` with a file-local helper.
- Modify: `README.md`
  Responsibility: describe the crate-root API and stop implying parser internals are a supported integration surface.
- Modify: `Cargo.toml`
  Responsibility: add `trybuild` under `[dev-dependencies]` for API boundary compile tests.
- Delete: `src/parser/mod.rs`
  Responsibility: remove the monolithic implementation file once all code has moved.
- Move: `src/parser/generic.rs` -> `src/fallback/generic.rs`
- Move: `src/parser/dcraw.rs` -> `src/fallback/dcraw.rs`
- Move: `src/parser/gpr.rs` -> `src/fallback/gpr.rs`
- Move: `src/parser/unix.rs` -> `src/platform/unix.rs`
- Move: `src/parser/windows.rs` -> `src/platform/windows.rs`

### Task 1: Lock The Public API Boundary

**Files:**
- Create: `tests/public_api_contract_tests.rs`
- Create: `tests/ui/parser_internal_api_is_hidden.rs`
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Modify: `tests/process_file_bytes_tests.rs`
- Modify: `tests/sample_corpus_tests.rs`

- [ ] **Step 1: Write the failing API-boundary tests**

```rust
// tests/public_api_contract_tests.rs
#[test]
fn crate_root_exports_cover_the_supported_library_surface() {
    use jpgfromraw::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};

    let _ = FindJpegType::Largest;
    let _ = FindJpegType::Smallest;
    let _ = process_file;
    let _ = process_file_bytes;
    assert!(SUPPORTED_EXTENSIONS.contains(&"arw"));
    assert!(SUPPORTED_EXTENSIONS.contains(&"gpr"));
}

#[test]
fn parser_internal_helpers_are_not_public_api() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/parser_internal_api_is_hidden.rs");
}
```

```rust
// tests/ui/parser_internal_api_is_hidden.rs
use jpgfromraw::parser::is_sof_marker;

fn main() {
    let _ = is_sof_marker(0xC0);
}
```

```toml
# Cargo.toml
[dev-dependencies]
trybuild = "1.0.114"
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test --test public_api_contract_tests -- --nocapture`

Expected: FAIL because `tests/ui/parser_internal_api_is_hidden.rs` currently compiles, which means parser internals are still part of the public API.

- [ ] **Step 3: Implement the crate-root API and narrow parser compatibility shim**

```rust
// src/lib.rs
#[path = "parser/mod.rs"]
mod parser_impl;

pub use parser_impl::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};

#[deprecated(note = "use crate-root exports instead of jpgfromraw::parser")]
pub mod parser {
    pub use crate::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};
}
```

```rust
// tests/process_file_bytes_tests.rs
use anyhow::Result;
use jpgfromraw::{FindJpegType, process_file_bytes};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs;
```

```rust
// tests/sample_corpus_tests.rs
use anyhow::{Context, Result, ensure};
use jpgfromraw::{FindJpegType, process_file_bytes};
use std::path::{Path, PathBuf};
use std::time::Instant;

const fn is_sof_marker(marker: u8) -> bool {
    marker >= 0xC0 && marker <= 0xCF && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}
```

- [ ] **Step 4: Run the targeted tests to verify they pass**

Run: `cargo test --test public_api_contract_tests -- --nocapture`
Expected: PASS for `crate_root_exports_cover_the_supported_library_surface`; PASS for the `trybuild` compile-fail test because `jpgfromraw::parser::is_sof_marker` is no longer reachable.

Run: `cargo test --test process_file_bytes_tests -- --nocapture`
Expected: PASS for the updated process-file-bytes integration tests.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs tests/public_api_contract_tests.rs tests/ui/parser_internal_api_is_hidden.rs tests/process_file_bytes_tests.rs tests/sample_corpus_tests.rs
git commit -m "refactor: lock crate public api at root"
```

### Task 2: Extract Public Types And Orchestration

**Files:**
- Create: `src/types.rs`
- Create: `src/extract.rs`
- Modify: `src/lib.rs`
- Test: `src/extract.rs`

- [ ] **Step 1: Write the failing orchestration test in the new extract module**

```rust
// add to src/extract.rs first, before implementation
#[cfg(test)]
mod tests {
    use super::output_path_for_relative_path;
    use std::path::Path;

    #[test]
    fn output_path_replaces_the_input_extension_with_jpg() {
        let out = output_path_for_relative_path(Path::new("/tmp/out"), Path::new("nested/file.arw"));
        assert_eq!(out, Path::new("/tmp/out/nested/file.jpg"));
    }
}
```

- [ ] **Step 2: Run the library tests to verify they fail**

Run: `cargo test --lib extract::tests::output_path_replaces_the_input_extension_with_jpg -- --nocapture`

Expected: FAIL with a compile error because `src/extract.rs` and `output_path_for_relative_path` do not exist yet.

- [ ] **Step 3: Create semantic root modules and move the public entrypoints there**

```rust
// src/types.rs
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
```

```rust
// src/extract.rs
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::parser_impl;
use crate::types::FindJpegType;

pub async fn process_file(
    entry_path: &Path,
    out_dir: &Path,
    relative_path: &Path,
    find_type: FindJpegType,
) -> Result<()> {
    let output_file = output_path_for_relative_path(out_dir, relative_path);
    if let Some(parent) = output_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let jpeg_data = process_file_bytes(entry_path, find_type).await?;
    tokio::fs::write(&output_file, &jpeg_data).await?;
    Ok(())
}

pub async fn process_file_bytes(entry_path: &Path, find_type: FindJpegType) -> Result<Vec<u8>> {
    parser_impl::process_file_bytes(entry_path, find_type).await
}

fn output_path_for_relative_path(out_dir: &Path, relative_path: &Path) -> PathBuf {
    let mut output_file = out_dir.join(relative_path);
    output_file.set_extension("jpg");
    output_file
}
```

```rust
// src/lib.rs
#[path = "parser/mod.rs"]
mod parser_impl;

mod extract;
mod types;

pub use extract::{process_file, process_file_bytes};
pub use types::{FindJpegType, SUPPORTED_EXTENSIONS};

#[deprecated(note = "use crate-root exports instead of jpgfromraw::parser")]
pub mod parser {
    pub use crate::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};
}
```

- [ ] **Step 4: Run focused tests to verify the semantic modules pass**

Run: `cargo test --lib extract::tests::output_path_replaces_the_input_extension_with_jpg -- --nocapture`

Expected: PASS for the new extract-path helper test.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/types.rs src/extract.rs
git commit -m "refactor: extract semantic root modules"
```

### Task 3: Split TIFF And JPEG Logic Out Of The Monolith

**Files:**
- Create: `src/tiff.rs`
- Create: `src/jpeg.rs`
- Modify: `src/extract.rs`
- Modify: `src/lib.rs`
- Modify: `src/parser/mod.rs`
- Test: `src/tiff.rs`
- Test: `src/jpeg.rs`

- [ ] **Step 1: Write the failing unit tests in the new TIFF and JPEG modules**

```rust
// src/tiff.rs
#[cfg(test)]
mod tests {
    use super::{find_tiff_header_offset, find_tiff_preview};
    use crate::types::FindJpegType;

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
}
```

```rust
// src/jpeg.rs
#[cfg(test)]
mod tests {
    use super::{find_embedded_jpeg_by_markers, find_embedded_jpeg_missing_ff_prefix, is_valid_jpeg};
    use crate::types::FindJpegType;

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
        let invalid = [0xD8, 0xFF, 0xDB, 0x00, 0x04, 0, 0, 0xFF, 0xD9];
        let valid = valid_missing_ff_candidate();
        let mut buf = Vec::new();
        buf.extend_from_slice(&invalid);
        buf.extend_from_slice(&valid);

        let jpeg = find_embedded_jpeg_missing_ff_prefix(&buf, FindJpegType::Largest).unwrap();
        assert!(is_valid_jpeg(&jpeg));
    }
}
```

- [ ] **Step 2: Run the library tests to verify they fail**

Run: `cargo test --lib -- --nocapture`

Expected: FAIL with compile errors because `src/tiff.rs` and `src/jpeg.rs` do not exist yet.

- [ ] **Step 3: Move TIFF and JPEG responsibilities into dedicated modules**

```rust
// src/lib.rs
#[path = "parser/mod.rs"]
mod parser_impl;

mod extract;
mod jpeg;
mod tiff;
mod types;

pub use extract::{process_file, process_file_bytes};
pub use types::{FindJpegType, SUPPORTED_EXTENSIONS};

#[deprecated(note = "use crate-root exports instead of jpgfromraw::parser")]
pub mod parser {
    pub use crate::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};
}
```

```rust
// src/extract.rs
use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::jpeg::{extract_jpeg, get_jpeg_data, is_valid_jpeg, try_missing_ff};
use crate::parser_impl::{decode_raw_preview_fallback, mmap_raw, open_raw};
use crate::tiff::{find_embedded_jpeg_in_tiff, find_tiff_header_offset, try_tiff_preview};
use crate::types::FindJpegType;

pub async fn process_file_bytes(entry_path: &Path, find_type: FindJpegType) -> Result<Vec<u8>> {
    let in_file = open_raw(entry_path)?;
    let raw_buf = mmap_raw(&in_file)?;
    let tiff_offset = find_tiff_header_offset(&raw_buf).ok();
    let jpeg_info = match tiff_offset {
        Some(tiff_offset) => find_embedded_jpeg_in_tiff(&raw_buf, tiff_offset, find_type)
            .ok()
            .flatten()
            .or_else(|| crate::jpeg::find_embedded_jpeg_by_markers(&raw_buf, find_type).ok()),
        None => crate::jpeg::find_embedded_jpeg_by_markers(&raw_buf, find_type).ok(),
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

    decode_raw_preview_fallback(entry_path).await
}
```

```rust
// src/parser/mod.rs
use anyhow::{Context, Result};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

mod dcraw;
mod generic;
mod gpr;
mod temp_dir;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

pub(crate) use platform::{mmap_raw, open_raw, prefetch_jpeg};

pub(crate) async fn decode_raw_preview_fallback(entry_path: &Path) -> Result<Vec<u8>> {
    if generic::is_generic_extension(entry_path) {
        let generic_path = entry_path.to_path_buf();
        if let Some(jpeg) = run_blocking_fallback("generic raster", move || {
            let bytes = std::fs::read(&generic_path).with_context(|| {
                format!("failed to read generic raster source {}", generic_path.display())
            })?;
            generic::decode_generic_raster_preview(&generic_path, &bytes)
                .map(|jpeg| jpeg.map(std::borrow::Cow::into_owned))
        })
        .await?
        {
            return Ok(jpeg);
        }
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

pub(crate) async fn run_blocking_fallback<T, F>(label: &str, decode: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(decode)
            .await
            .with_context(|| format!("{label} fallback task failed"))?,
        Err(_) => decode(),
    }
}
```

- [ ] **Step 4: Run the library tests to verify they pass**

Run: `cargo test --lib -- --nocapture`

Expected: PASS for all moved TIFF/JPEG tests; no behavior changes in the existing extraction flow.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/extract.rs src/tiff.rs src/jpeg.rs src/parser/mod.rs
git commit -m "refactor: split tiff and jpeg internals"
```

### Task 4: Split Fallback And Platform Code, Then Remove The Monolith

**Files:**
- Create: `src/fallback/mod.rs`
- Create: `src/fallback/generic.rs`
- Create: `src/fallback/dcraw.rs`
- Create: `src/fallback/gpr.rs`
- Create: `src/platform/mod.rs`
- Create: `src/platform/unix.rs`
- Create: `src/platform/windows.rs`
- Modify: `src/extract.rs`
- Modify: `README.md`
- Modify: `tests/sample_corpus_tests.rs`
- Delete: `src/parser/mod.rs`

- [ ] **Step 1: Write the failing tests for the final module split**

```rust
// add to src/fallback/mod.rs
#[cfg(test)]
mod tests {
    use super::run_blocking_fallback;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    fn block_on_without_tokio<F: Future>(fut: F) -> F::Output {
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

    #[test]
    fn blocking_fallback_runs_inline_without_a_tokio_runtime() {
        let result = block_on_without_tokio(run_blocking_fallback("unit", || Ok::<_, anyhow::Error>(41usize)))
            .unwrap();
        assert_eq!(result, 41);
    }
}
```

```rust
// tests/sample_corpus_tests.rs
#[test]
fn local_sof_helper_matches_the_jpeg_marker_table() {
    assert!(is_sof_marker(0xC0));
    assert!(is_sof_marker(0xC2));
    assert!(!is_sof_marker(0xC4));
    assert!(!is_sof_marker(0xCC));
}
```

- [ ] **Step 2: Run the test suite to verify it fails**

Run: `cargo test --lib --tests -- --nocapture`

Expected: FAIL with compile errors because `src/fallback/mod.rs` and `src/platform/mod.rs` do not exist yet.

- [ ] **Step 3: Move fallback/platform code into private modules and update docs**

```rust
// src/fallback/mod.rs
use anyhow::{Context, Result};
use std::path::Path;

pub mod dcraw;
pub mod generic;
pub mod gpr;

pub async fn decode_raw_preview_fallback(entry_path: &Path) -> Result<Vec<u8>> {
    if generic::is_generic_extension(entry_path) {
        let generic_path = entry_path.to_path_buf();
        if let Some(jpeg) = run_blocking_fallback("generic raster", move || {
            let bytes = std::fs::read(&generic_path).with_context(|| {
                format!("failed to read generic raster source {}", generic_path.display())
            })?;
            generic::decode_generic_raster_preview(&generic_path, &bytes)
                .map(|jpeg| jpeg.map(std::borrow::Cow::into_owned))
        })
        .await?
        {
            return Ok(jpeg);
        }
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

pub async fn run_blocking_fallback<T, F>(label: &str, decode: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(decode)
            .await
            .with_context(|| format!("{label} fallback task failed"))?,
        Err(_) => decode(),
    }
}
```

```rust
// src/platform/mod.rs
use anyhow::Result;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

use crate::jpeg::EmbeddedJpegInfo;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{mmap_raw, open_raw, prefetch_jpeg};
#[cfg(windows)]
pub use windows::{mmap_raw, open_raw, prefetch_jpeg};
```

```markdown
<!-- README.md -->
The supported library surface is the crate root:

```rust
use jpgfromraw::{process_file_bytes, FindJpegType};
```

`jpgfromraw::parser` remains a compatibility shim for older callers, but new code should import from the crate root.
```

- [ ] **Step 4: Run the full verification suite**

Run: `cargo test --lib --tests`
Expected: PASS

Run: `cargo test --release --lib parsing_performance_audit -- --ignored --nocapture`
Expected: PASS, with TIFF-path timings staying in the same rough range as the current baseline and marker fallback remaining roughly three orders of magnitude slower than the happy path.

Run: `cargo test --release --lib parallel_scaling_audit -- --ignored --nocapture`
Expected: PASS, with throughput still scaling materially from 1 worker to 4 workers.

- [ ] **Step 5: Commit**

```bash
git add README.md src/extract.rs src/fallback src/platform src/jpeg.rs src/tiff.rs tests/sample_corpus_tests.rs
git rm src/parser/mod.rs
git commit -m "refactor: split fallback and platform modules"
```
