use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    /// A hash over several fields, length-prefixed so the encoding is
    /// injective.
    ///
    /// Joining user-controlled fields with a separator is not: `locale =
    /// "en/US"` with `voice = "af_heart"` and `locale = "en"` with `voice =
    /// "US/af_heart"` produce the same string, and a collision in a cache
    /// key silently serves one voice's audio for another. Every key in
    /// teleprompt goes through here, so that argument is made once.
    pub fn of_fields(fields: &[&str]) -> Self {
        let mut canonical = String::new();
        for field in fields {
            canonical.push_str(&field.len().to_string());
            canonical.push(':');
            canonical.push_str(field);
        }
        Hash::of(canonical.as_bytes())
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
        // `from_str_radix(_, 16)` accepts both cases, and `Hash`'s only
        // `Serialize` impl always emits lowercase, so there is nothing to
        // gain from rejecting uppercase input here — accept either case
        // rather than pretend to be stricter than we are.
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| serde::de::Error::custom("hash must be 64 hex characters"))?;
        }
        Ok(Hash(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::IntoDeserializer;
    use serde::Deserialize;

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

    /// Runs `Hash::deserialize` against a plain `&str`, the same shape a
    /// JSON/YAML string field decodes to, without depending on any
    /// particular serde data format crate.
    fn parse(s: &str) -> Result<Hash, serde::de::value::Error> {
        Hash::deserialize(s.into_deserializer())
    }

    #[test]
    fn a_63_character_string_is_a_readable_error() {
        let s = "a".repeat(63);
        let err = parse(&s).unwrap_err();
        assert!(err.to_string().contains("64 hex characters"));
    }

    #[test]
    fn a_65_character_string_is_a_readable_error() {
        let s = "a".repeat(65);
        let err = parse(&s).unwrap_err();
        assert!(err.to_string().contains("64 hex characters"));
    }

    #[test]
    fn non_hex_ascii_is_a_readable_error() {
        // 64 ASCII characters, none of them valid hex digits.
        let s = "z".repeat(64);
        let err = parse(&s).unwrap_err();
        assert!(err.to_string().contains("64 hex characters"));
    }

    #[test]
    fn a_64_byte_multi_byte_string_is_a_readable_error_not_a_panic() {
        // 'é' (U+00E9) is 2 bytes in UTF-8, so 32 of them is 64 *bytes*
        // but only 32 *characters*. Byte-index slicing at an odd offset
        // into this string would land mid-character and panic; the
        // `is_ascii()` guard must reject it before any slicing happens.
        let s = "é".repeat(32);
        assert_eq!(s.len(), 64, "test fixture must be exactly 64 bytes");
        let err = parse(&s).unwrap_err();
        assert!(err.to_string().contains("64 hex characters"));
    }

    #[test]
    fn uppercase_hex_is_accepted_and_round_trips_to_the_same_hash() {
        let original = Hash::of(b"welcome");
        let uppercase = original.to_string().to_uppercase();
        assert_eq!(parse(&uppercase).unwrap(), original);
    }
}
