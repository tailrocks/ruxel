use super::*;

#[test]
fn prefetched_fields_share_item_and_select_ids_or_labels() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    let user = Lookup::OnePasswordUri {
        reference: "op://vault/item/username".into(),
    };
    let password = Lookup::OnePasswordUri {
        reference: "op://vault/item/Login/Password".into(),
    };
    resolver
        .prefetch(&[user.clone(), password.clone(), user.clone()])
        .unwrap();
    assert_eq!(resolver.resolve(&user).unwrap(), "alice");
    assert_eq!(resolver.resolve(&password).unwrap(), "fake-password");
    assert_eq!(backend.items.load(Ordering::SeqCst), 1);
}

#[test]
fn other_lookup_types_cache_and_redact_errors() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    let key = Lookup::Fnox { key: "KEY".into() };
    resolver.prefetch(&[key.clone(), key.clone()]).unwrap();
    assert_eq!(resolver.resolve(&key).unwrap(), "fake-value");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    let command = Lookup::Command {
        argv: vec!["fake-sensitive-command".into()],
    };
    assert_eq!(
        resolver.resolve(&command).unwrap_err(),
        "secret lookup failed"
    );
    assert!(!format!("{command:?}").contains("fake-sensitive"));
    assert_eq!(
        resolver
            .resolve(&Lookup::Synthetic {
                value: "synthetic".into()
            })
            .unwrap(),
        "synthetic"
    );
}
