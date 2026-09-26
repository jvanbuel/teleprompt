#![allow(dead_code)] // each test uses its own part
//! Shared by the tests: the repository, and the API's examples in it.

use std::path::PathBuf;

pub fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// An example from `docs/api/v1/examples`, which the server is tested
/// against too.
pub fn example(name: &str) -> String {
    let path = repository().join("docs/api/v1/examples").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}
