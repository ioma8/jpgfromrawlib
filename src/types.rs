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
