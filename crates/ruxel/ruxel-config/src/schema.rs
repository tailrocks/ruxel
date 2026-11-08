//! The canonical executable Pkl schema, embedded for distribution.

/// Schema files and their contents.
pub const FILES: &[(&str, &str)] = &[("Ruxel.pkl", include_str!("../../../../schema/Ruxel.pkl"))];

/// Return the lowercase BLAKE3 digest of schema contents.
#[must_use]
pub fn blake3_hex(contents: &str) -> String {
    blake3::hash(contents.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests;
