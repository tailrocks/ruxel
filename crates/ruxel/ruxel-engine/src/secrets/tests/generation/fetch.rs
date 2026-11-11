//! The fetch path of the resolver: the comparison against the stored
//! value, the store of a change, and the fingerprint the controller
//! reports.

use super::Fixture;

/// The fetched secret is stored only when it differs from the stored value,
/// compared on the controller; the fingerprint it reports is eight hex
/// characters and keyed, never an unsalted full hash.
#[test]
fn a_fetched_secret_stores_only_a_change_and_fingerprints_are_keyed_and_truncated() {
    let fixture = Fixture::new();
    fixture.put("K8S_ADMIN_KUBECONFIG", "stored kubeconfig content\n");
    let resolver = fixture.resolver();
    // Same content: no store, no change.
    let (changed, fingerprint) = resolver
        .fetch_secret("K8S_ADMIN_KUBECONFIG", "stored kubeconfig content\n")
        .expect("compare");
    assert!(!changed);
    assert_eq!(
        fingerprint.len(),
        8,
        "the unchanged store reports a fingerprint too"
    );
    assert_eq!(fixture.set_calls(), 0);
    // Different content: one store, a change, an eight-hex fingerprint.
    let (changed, fingerprint) = resolver
        .fetch_secret("K8S_ADMIN_KUBECONFIG", "new kubeconfig content\n")
        .expect("store");
    assert!(changed);
    assert_eq!(fixture.set_calls(), 1);
    assert_eq!(
        fixture.value("K8S_ADMIN_KUBECONFIG").as_deref(),
        Some("new kubeconfig content\n")
    );
    assert_eq!(fingerprint.len(), 8);
    assert!(
        fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    // The fingerprint is keyed per process: another resolver's digest of the
    // same value differs, so no offline oracle can test guesses against it.
    let other = fixture
        .resolver()
        .fingerprint("K8S_ADMIN_KUBECONFIG", "new kubeconfig content\n");
    assert_ne!(other, fingerprint);
    // A malformed key is refused before any store contact.
    assert!(resolver.fetch_secret("k8s_admin_kubeconfig", "x").is_err());
    assert_eq!(fixture.set_calls(), 1);
}
