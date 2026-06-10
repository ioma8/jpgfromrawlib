use anyhow::Result;
use std::path::Path;

pub struct TempDirGuard {
    inner: tempfile::TempDir,
}

impl TempDirGuard {
    pub fn new(prefix: &str) -> Result<Self> {
        let inner = tempfile::Builder::new().prefix(prefix).tempdir()?;
        Ok(Self { inner })
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}
