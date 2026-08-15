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
