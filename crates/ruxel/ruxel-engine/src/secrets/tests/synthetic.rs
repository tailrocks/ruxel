use super::*;

#[test]
fn synthetic_mode_never_calls_backend_and_is_stable() {
    let backend = Arc::new(Fake::default());
    let mut resolver = Resolver::new(backend.clone());
    resolver.synthetic = true;
    let lookup = Lookup::Pipe {
        command: "op read 'op://vault/item/username'".into(),
    };
    let value = resolver.resolve(&lookup).unwrap();
    assert!(value.starts_with("dry-secret-"));
    assert_eq!(value, Resolver::synthetic().resolve(&lookup).unwrap());
    for lookup in [
        Lookup::Fnox { key: "KEY".into() },
        Lookup::OnePasswordUri {
            reference: "op://vault/item/username".into(),
        },
        Lookup::Command {
            argv: vec!["program".into()],
        },
    ] {
        assert!(
            resolver
                .resolve(&lookup)
                .unwrap()
                .starts_with("dry-secret-")
        );
    }
    assert_eq!(backend.items.load(Ordering::SeqCst), 0);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}
