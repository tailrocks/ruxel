use super::*;

#[test]
fn fetches_are_field_scoped_and_select_ids_or_labels() {
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
    // The fetch is field-scoped: one op call per named field, cached after.
    assert_eq!(backend.items.load(Ordering::SeqCst), 2);
}

#[test]
fn a_label_shared_by_sections_fails_naming_them() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    let lookup = Lookup::OnePassword {
        vault: "vault".into(),
        item: "item".into(),
        field: "Password".into(),
        section: None,
    };
    let error = resolver.resolve(&lookup).unwrap_err();
    assert_eq!(
        error,
        "ambiguous 1Password field \"Password\" in item \"item\": it matches sections \"Login\", \"API\"; name the section"
    );
    assert!(!error.contains("fake-password"));
    // With the section named, the same label resolves.
    assert_eq!(
        resolver
            .resolve(&Lookup::OnePassword {
                vault: "vault".into(),
                item: "item".into(),
                field: "Password".into(),
                section: Some("API".into()),
            })
            .unwrap(),
        "fake-password-api"
    );
}

#[test]
fn fields_in_different_sections_never_share_a_fetch() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    for (section, expected) in [("API", "fake-password-api"), ("Login", "fake-password")] {
        let lookup = Lookup::OnePassword {
            vault: "vault".into(),
            item: "item".into(),
            field: "Password".into(),
            section: Some(section.into()),
        };
        assert_eq!(resolver.resolve(&lookup).unwrap(), expected);
    }
    assert_eq!(backend.items.load(Ordering::SeqCst), 2);
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
