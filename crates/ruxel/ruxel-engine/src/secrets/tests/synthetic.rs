use super::*;

#[test]
fn synthetic_mode_never_calls_backend_and_is_stable() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::synthetic_with(backend.clone()).unwrap();
    let lookup = Lookup::OnePasswordUri {
        reference: "op://vault/item/username".into(),
    };
    let value = resolver.resolve(&lookup).unwrap();
    assert!(value.starts_with("dry-secret-"));
    assert_eq!(
        value,
        Resolver::synthetic().unwrap().resolve(&lookup).unwrap()
    );
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

/// Synthetic mode never writes: a `FetchSecret` under the synthetic
/// resolver reports unchanged, with its fingerprint, and contacts no
/// backend method at all — the fake counts zero `set`, `get` and write
/// lock calls.
#[test]
fn synthetic_mode_fetch_secret_reports_unchanged_and_never_writes() {
    #[derive(Default)]
    struct Counting {
        calls: AtomicUsize,
    }
    impl Backend for Counting {
        fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn fnox_write_lock(&self, _: &str) -> Result<FnoxLock, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            FnoxLock::unheld()
        }
        fn onepassword_fields(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> Result<serde_json::Value, String> {
            unreachable!()
        }
        fn onepassword_reference(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn command(&self, _: &[String]) -> Result<String, String> {
            unreachable!()
        }
    }
    let backend = Arc::new(Counting::default());
    let resolver = Resolver::synthetic_with(backend.clone()).unwrap();
    let (changed, fingerprint) = resolver
        .fetch_secret("K8S_ADMIN_KUBECONFIG", "kubeconfig-fixture\n")
        .unwrap();
    assert!(!changed, "nothing is written in synthetic mode");
    assert_eq!(fingerprint.len(), 8, "the report still fingerprints");
    assert_eq!(
        backend.calls.load(Ordering::SeqCst),
        0,
        "the synthetic resolver contacted the backend"
    );
}
