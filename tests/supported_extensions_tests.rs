use anyhow::{Result, ensure};
use jpgfromraw_lib::SUPPORTED_EXTENSIONS;
use std::collections::HashSet;

#[test]
fn supported_extensions_include_requested_formats() -> Result<()> {
    let supported = SUPPORTED_EXTENSIONS.iter().copied().collect::<HashSet<_>>();
    let expected = [
        "3fr", "ari", "arw", "bay", "braw", "cap", "cr2", "cr3", "cri", "crw", "dcr", "dcs", "dng",
        "drf", "eip", "erf", "fff", "gpr", "iiq", "jxs", "k25", "kdc", "mdc", "mef", "mos", "mrw",
        "nef", "nrw", "orf", "pef", "ptx", "pxn", "r3d", "raf", "raw", "rw2", "rwl", "rwz", "sr2",
        "srf", "srw", "tco", "x3f",
    ];

    for extension in expected {
        ensure!(
            supported.contains(extension),
            "expected default support for .{extension}"
        );
    }

    Ok(())
}
