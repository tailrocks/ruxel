#[test]
fn blake3_hex_matches_the_empty_string_vector() {
    assert_eq!(
        super::blake3_hex(""),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
}
