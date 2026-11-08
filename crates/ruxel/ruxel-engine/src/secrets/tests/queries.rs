use super::*;

#[test]
fn query_references_preserve_cli_semantics_and_cache_full_reference() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
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
fn quoted_op_read_queries_use_complete_reference_instead_of_cached_field() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    for quote in ['\'', '"'] {
        let lookup = Lookup::Pipe {
            command: format!("op read {quote}op://vault/item/username?attribute=otp{quote}"),
        };
        assert_eq!(resolver.resolve(&lookup).unwrap(), "synthetic-otp");
        assert_eq!(resolver.resolve(&lookup).unwrap(), "synthetic-otp");
    }
    assert_eq!(backend.references.lock().unwrap().len(), 2);
    assert_eq!(backend.items.load(Ordering::SeqCst), 0);
}

#[test]
fn query_failures_remain_redacted_and_cached() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    let lookup = Lookup::OnePasswordUri {
        reference: "op://vault/item/username?attribute=synthetic-invalid".into(),
    };
    for _ in 0..2 {
        assert_eq!(
            resolver.resolve(&lookup).unwrap_err(),
            "secret lookup failed"
        );
    }
    assert_eq!(backend.references.lock().unwrap().len(), 1);
}

#[test]
fn query_pipes_with_shell_operations_keep_their_original_execution() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    for command in [
        "op read 'op://vault/item/username?attribute=otp' | base64",
        "op read \"op://vault/item/$field?attribute=otp\"",
        "op read op://vault/item/username?attribute=otp",
    ] {
        assert_eq!(
            resolver
                .resolve(&Lookup::Pipe {
                    command: command.into()
                })
                .unwrap(),
            "fake-pipe"
        );
    }
    assert!(backend.references.lock().unwrap().is_empty());
    assert_eq!(backend.items.load(Ordering::SeqCst), 0);
}
