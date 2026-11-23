use super::*;

#[test]
fn query_references_preserve_cli_semantics_and_cache_full_reference() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone()).unwrap();
    for (query, expected) in [("otp", "synthetic-otp"), ("type", "synthetic-type")] {
        let lookup = Lookup::OnePasswordUri {
            reference: format!("op://vault/item/username?attribute={query}"),
        };
        resolver
            .prefetch(&[lookup.clone(), lookup.clone()])
            .unwrap();
        assert_eq!(resolver.resolve(&lookup).unwrap(), expected);
        assert_eq!(resolver.resolve(&lookup).unwrap(), expected);
        assert!(!format!("{lookup:?}").contains(query));
    }
    assert_eq!(backend.references.lock().unwrap().len(), 2);
    assert_eq!(backend.items.load(Ordering::SeqCst), 0);
}

#[test]
fn query_failures_remain_redacted_and_are_retried() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone()).unwrap();
    let lookup = Lookup::OnePasswordUri {
        reference: "op://vault/item/username?attribute=synthetic-invalid".into(),
    };
    for _ in 0..2 {
        assert_eq!(
            resolver.resolve(&lookup).unwrap_err(),
            "secret lookup failed"
        );
    }
    assert_eq!(backend.references.lock().unwrap().len(), 2);
}
