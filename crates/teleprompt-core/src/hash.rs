use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn short(&self) -> String {
        self.to_string()[..6].to_string()
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.short())
    }
}

impl serde::Serialize for Hash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Hash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // Guard on `is_ascii()` before any byte-index slicing: a non-ASCII
        // string can be 64 *bytes* long without being 64 *characters* long,
        // and slicing at a non-char-boundary byte offset panics rather than
        // returning a `Result`. A corrupted committed timeline must produce
        // a readable deserialize error, not a panic.
        if s.len() != 64 || !s.is_ascii() {
            return Err(serde::de::Error::custom("hash must be 64 hex characters"));
        }
        let raw = s.as_bytes();
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| {
                serde::de::Error::custom(format!(
                    "hash must be lowercase hex, found {:?} at byte {}",
                    raw[i * 2] as char,
                    i * 2
                ))
            })?;
        }
        Ok(Hash(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(Hash::of(b"welcome"), Hash::of(b"welcome"));
    }

    #[test]
    fn hash_distinguishes_content() {
        assert_ne!(Hash::of(b"welcome"), Hash::of(b"welcome "));
    }

    #[test]
    fn short_form_is_six_hex_chars() {
        let s = Hash::of(b"welcome").short();
        assert_eq!(s.len(), 6);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
