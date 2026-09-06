# jpgfromraw-lib

Library fork of Chris Down's [`jpgfromraw`](https://github.com/cdown/jpgfromraw),
published under a separate crate name because `jpgfromraw` already belongs to
upstream on crates.io.

This fork keeps the fast embedded JPEG extraction code and exposes it as a Rust
library for apps that need RAW previews without shelling out to the CLI.

## Usage

```rust
use jpgfromraw_lib::{process_file_bytes, FindJpegType};

let jpeg = process_file_bytes(path, FindJpegType::Largest).await?;
```

`FindJpegType::Largest` usually returns the full-size embedded preview.
`FindJpegType::Smallest` usually returns the thumbnail.

The crate also exports:

- `process_file` to write an extracted JPEG to disk.
- `SUPPORTED_EXTENSIONS` for the supported RAW/camera extension list.

Embedded JPEG discovery is mmap-first and runs for every input extension. The
library checks structured previews, ordinary JPEG markers, and missing-FF JPEG
repair before invoking any decoder or external tool. The public async API keeps
file I/O asynchronous; parsing and enabled fallback decoders run as synchronous
work, with fallback work moved to Tokio's blocking pool.

The default feature set is intentionally empty. Enable only the fallback
behavior an application needs:

| Feature | Enables |
| --- | --- |
| `cli` | `jpgfromraw` and `perf_probe` binaries, Clap, and Indicatif |
| `tiff-preview` | RGB TIFF preview conversion |
| `raster-fallback` | Base raster conversion formats |
| `generic-raster` | Base raster formats plus AVIF, DDS, and EXR |
| `dcraw-fallback` | Vendored dcraw fallback |
| `gpr-fallback` | Vendored GPR fallback |
| `full` | All fallback features; add `cli` for the binaries |

Build the command-line tools with `cargo build --features cli`. Native fallback
tools are built only when their feature is enabled. At runtime, set
`JPGFROMRAW_DCRAW` or `JPGFROMRAW_GPR_TOOLS` to an explicit executable path;
otherwise the library uses a valid build-produced development path and then
looks up `jpgfromraw-dcraw` or `gpr_tools` on `PATH`. The explicit environment
variables have highest precedence, and tool paths are passed as process
arguments. This lets deployed applications bundle the tools outside the
original Cargo build directory.

## Notes

- License: MIT, preserving upstream attribution.
- Repository: <https://github.com/ioma8/jpgfromrawlib>
- `dcraw-fallback` requires a C compiler; `gpr-fallback` requires CMake and a
  C++ toolchain during builds.
- Mapped input assumes the backing file is not concurrently truncated or
  mutated while processing. Bounds checks do not make concurrent file mutation
  safe.
