//! Helpers for the workspace's tests. A dev-dependency only.

use std::ops::Deref;
use std::path::Path;

/// A fresh directory that is deleted when dropped, even when the test
/// panics. Derefs to [`Path`], so it stands wherever a test used a
/// `PathBuf` it made by hand under the system temp directory and never
/// removed.
#[derive(Debug)]
pub struct TestDir(tempfile::TempDir);

/// A new, empty directory whose name starts with `tp-{prefix}-`, so a
/// leftover from a killed run still says which test made it.
pub fn test_dir(prefix: &str) -> TestDir {
    let dir = tempfile::Builder::new()
        .prefix(&format!("tp-{prefix}-"))
        .tempdir()
        .expect("create a test directory");
    TestDir(dir)
}

impl TestDir {
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}
