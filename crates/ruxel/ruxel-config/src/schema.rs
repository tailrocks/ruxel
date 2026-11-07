#[cfg(test)]
mod tests;

pub const FILES: &[(&str, &str)] = &[("Ruxel.pkl", include_str!("../schema/Ruxel.pkl"))];

pub fn blake3_hex(contents: &str) -> String {
    blake3::hash(contents.as_bytes()).to_hex().to_string()
}
