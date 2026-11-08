use super::*;

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
