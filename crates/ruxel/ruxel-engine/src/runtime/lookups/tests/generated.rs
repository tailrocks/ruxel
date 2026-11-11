use super::*;

/// A generated value joins the redaction registry the moment it exists:
/// a later task that echoes one — the exact value or its base64 or hex
/// form — is still censored, and the report names the key and the
/// format, never the value.
#[test]
fn generated_values_are_registered_with_the_redaction_registry() -> Result<()> {
    #[derive(Default)]
    struct Capture {
        value: Mutex<Option<String>>,
    }
    impl Backend for Capture {
        fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
            Ok(self.value.lock().map_err(|_| "poisoned store")?.clone())
        }
        fn fnox_set(&self, _: &str, value: &str) -> Result<(), String> {
            *self.value.lock().map_err(|_| "poisoned store")? = Some(value.to_owned());
            Ok(())
        }
        fn onepassword_fields(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> Result<Value, String> {
            Err("unexpected 1Password fetch".into())
        }
        fn onepassword_reference(&self, _: &str) -> Result<String, String> {
            Err("unexpected 1Password reference".into())
        }
        fn command(&self, _: &[String]) -> Result<String, String> {
            Err("unexpected command lookup".into())
        }
    }
    let store = Arc::new(Capture::default());
    let lookups = Lookups::new(Resolver::new(store.clone()).expect("resolver"));
    let mut reported = Vec::new();
    lookups
        .ensure_generated(
            &[GeneratedRequest {
                key: "PROCESSOR_DB_PASSWORD".into(),
                format: "hex32".into(),
            }],
            &mut |key, format| reported.push(format!("{key} ({format})")),
        )
        .expect("generation");
    assert_eq!(reported, ["PROCESSOR_DB_PASSWORD (hex32)"]);
    let generated = store
        .value
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned store"))?
        .clone()
        .context("the generated value was stored")?;
    assert_eq!(generated.len(), 32, "hex32");
    let hex = generated
        .bytes()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| char::from_digit(u32::from(nibble), 16).expect("hex digit"))
        .collect::<String>();
    let echoed = lookups.redact(&json!({
        "msg": format!("echo {generated} and {hex}"),
    }));
    let echoed = serde_json::to_string(&echoed)?;
    assert!(!echoed.contains(&generated), "the echo carries the value");
    assert!(!echoed.contains(&hex), "the echo carries the hex form");
    assert!(echoed.contains("[REDACTED]"));
    Ok(())
}
