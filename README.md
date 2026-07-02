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

## Notes

- License: MIT, preserving upstream attribution.
- Repository: <https://github.com/ioma8/jpgfromrawlib>
- Some RAW fallbacks build vendored C/C++ tools, so consumers may need a C
  compiler and CMake available during builds.
