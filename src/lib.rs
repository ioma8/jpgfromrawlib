mod fallback;
#[path = "parser/mod.rs"]
mod parser_impl;
mod platform;

mod extract;
mod types;

pub use extract::{process_file, process_file_bytes};
pub use types::{FindJpegType, SUPPORTED_EXTENSIONS};

#[deprecated(note = "use crate-root exports instead of jpgfromraw::parser")]
pub mod parser {
    pub use crate::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};
}
