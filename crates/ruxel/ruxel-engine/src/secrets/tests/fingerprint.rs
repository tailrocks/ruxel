use crate::secrets::fingerprint::Fingerprints;

/// The fingerprint key fails closed: when the operating system's random
/// source is unavailable there is no safe key, so construction errors
/// rather than falling back to a weak or constant key an attacker could
/// enumerate.
#[test]
fn the_fingerprint_key_fails_closed_without_a_random_source() {
    let failed = Fingerprints::from_random(|_| Err("no random source available".to_owned()));
    assert!(failed.is_err(), "no random source must fail closed");
    let drawn = Fingerprints::from_random(|buffer| {
        buffer.fill(0x5a);
        Ok(())
    })
    .expect("a working draw");
    assert_eq!(drawn.of("selector", "value").len(), 8);
}
