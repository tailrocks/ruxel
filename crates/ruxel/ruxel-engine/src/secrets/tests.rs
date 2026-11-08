use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Fake {
    items: AtomicUsize,
    calls: AtomicUsize,
    references: Mutex<Vec<String>>,
}
impl Backend for Fake {
    fn fnox(&self, _: &str) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("fake-value".into())
    }
    fn onepassword_item(&self, _: &str, _: &str) -> Result<serde_json::Value, String> {
        self.items.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"fields": [
            {"id":"username", "label":"Username", "value":"alice"},
            {"id":"password", "label":"Password", "section":{"id":"login", "label":"Login"}, "value":"fake-password"}
        ]}))
    }
    fn onepassword_reference(&self, reference: &str) -> Result<String, String> {
        self.references.lock().unwrap().push(reference.into());
        match reference {
            "op://vault/item/username?attribute=otp" => Ok("synthetic-otp".into()),
            "op://vault/item/username?attribute=type" => Ok("synthetic-type".into()),
            _ => Err("synthetic-sensitive-query-error".into()),
        }
    }
    fn command(&self, _: &[String]) -> Result<String, String> {
        Err("fake-sensitive-error".into())
    }
    fn pipe(&self, _: &str) -> Result<String, String> {
        Ok("fake-pipe".into())
    }
}

#[test]
fn prefetched_fields_share_item_and_select_ids_or_labels() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    let user = Lookup::OnePasswordUri {
        reference: "op://vault/item/username".into(),
    };
    let password = Lookup::Pipe {
        command: "op read 'op://vault/item/Login/Password'".into(),
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
    assert_eq!(
        resolver
            .resolve(&Lookup::Pipe {
                command: "fake".into()
            })
            .unwrap(),
        "fake-pipe"
    );
}
#[test]
fn only_plain_op_read_is_optimized() {
    let backend = Arc::new(Fake::default());
    let resolver = Resolver::new(backend.clone());
    for command in [
        "printf '%s' 'op://vault/item/username'",
        "op read 'op://vault/item/username' | base64",
        "op read \"op://vault/item/$name\"",
        "op\nread 'op://vault/item/username'",
        "op read\n'op://vault/item/username'",
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
    assert_eq!(backend.items.load(Ordering::SeqCst), 0);
    assert_eq!(
        resolver
            .resolve(&Lookup::Pipe {
                command: "op read 'op://vault/item/username'".into()
            })
            .unwrap(),
        "alice"
    );
}

#[cfg(unix)]
mod process;

mod queries;
mod synthetic;

#[test]
fn transient_failures_retry_and_successes_are_shared() {
    struct Flaky {
        values: AtomicUsize,
        items: AtomicUsize,
    }
    impl Backend for Flaky {
        fn fnox(&self, _: &str) -> Result<String, String> {
            if self.values.fetch_add(1, Ordering::SeqCst) == 0 {
                Err("sensitive failure".into())
            } else {
                Ok("recovered".into())
            }
        }
        fn onepassword_item(&self, _: &str, _: &str) -> Result<serde_json::Value, String> {
            if self.items.fetch_add(1, Ordering::SeqCst) == 0 {
                Err("sensitive failure".into())
            } else {
                Ok(
                    serde_json::json!({"fields": [{"id": "a", "value": "recovered"}, {"id": "b", "value": "second"}]}),
                )
            }
        }
        fn onepassword_reference(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn command(&self, _: &[String]) -> Result<String, String> {
            unreachable!()
        }
        fn pipe(&self, _: &str) -> Result<String, String> {
            unreachable!()
        }
    }
    let backend = Arc::new(Flaky {
        values: AtomicUsize::new(0),
        items: AtomicUsize::new(0),
    });
    let resolver = Resolver::new(backend.clone());
    let key = Lookup::Fnox { key: "KEY".into() };
    assert_eq!(resolver.resolve(&key).unwrap_err(), "secret lookup failed");
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| assert_eq!(resolver.resolve(&key).unwrap(), "recovered"));
        }
    });
    assert_eq!(backend.values.load(Ordering::SeqCst), 2);
    let field = |field: &str| Lookup::OnePassword {
        vault: "v".into(),
        item: "i".into(),
        field: field.into(),
        section: None,
    };
    assert_eq!(
        resolver.resolve(&field("a")).unwrap_err(),
        "secret lookup failed"
    );
    assert_eq!(resolver.resolve(&field("a")).unwrap(), "recovered");
    assert_eq!(resolver.resolve(&field("b")).unwrap(), "second");
    assert_eq!(backend.items.load(Ordering::SeqCst), 2);
}
