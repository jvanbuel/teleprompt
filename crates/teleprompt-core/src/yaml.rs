//! YAML as teleprompt reads and writes it, in one place so the library
//! behind it can change without any caller noticing.
//!
//! An untyped YAML value is a [`serde_json::Value`]: settings are keyed by
//! text, which is all JSON allows, and every crate already speaks it. Only
//! `true` and `false` are booleans, so `no` and `on` stay the words an
//! author wrote.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Why YAML could not be read or written, as the YAML library words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl Error {
    /// An error in what was read, worded by whoever read it.
    pub fn from_message(message: String) -> Self {
        Self(message)
    }
}

/// How teleprompt reads YAML: only `true` and `false` are booleans, and a
/// merge key `<<` is an ordinary key.
fn options() -> serde_saphyr::Options {
    let mut options = serde_saphyr::Options::default();
    options.strict_booleans = true;
    options.merge_keys = serde_saphyr::MergeKeyPolicy::AsOrdinary;
    options
}

/// `text` read as YAML into a `T`.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_saphyr::from_str_with_options(text, options()).map_err(|e| Error(e.to_string()))
}

/// `value` written as YAML.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_saphyr::to_string(value).map_err(|e| Error(e.to_string()))
}
