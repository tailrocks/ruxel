use super::{FILES, blake3_hex};

#[test]
fn embedded_schema_and_digest() {
    assert_eq!(FILES[0].0, "Ruxel.pkl");
    assert_eq!(FILES[0].1, include_str!("../../../../../schema/Ruxel.pkl"));
    assert_eq!(
        blake3_hex(""),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
}
