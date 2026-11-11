use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Fake {
    items: AtomicUsize,
    calls: AtomicUsize,
    references: Mutex<Vec<String>>,
}
impl Backend for Fake {
    fn fnox_get(&self, _: &str) -> Result<Option<String>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some("fake-value".into()))
    }
    fn fnox_set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("unexpected fnox store".into())
    }
    fn onepassword_fields(
        &self,
        _vault: &str,
        _item: &str,
        _section: Option<&str>,
        field: &str,
    ) -> Result<serde_json::Value, String> {
        self.items.fetch_add(1, Ordering::SeqCst);
        // The item's fields, pre-filtered the way `op item get --fields`
        // filters: by label or id, case-insensitively the way 1Password
        // matches reference parts. "password" matches two sections; "pin"
        // matches two fields that differ only by case.
        let fields = [
            r#"{"id":"username","label":"Username","value":"alice"}"#,
            r#"{"id":"password","label":"Password","section":{"id":"login","label":"Login"},"value":"fake-password"}"#,
            r#"{"id":"password","label":"Password","section":{"id":"api","label":"API"},"value":"fake-password-api"}"#,
            r#"{"id":"api_key","label":"ApiKey","value":"fake-apikey"}"#,
            r#"{"id":"totp","label":"TOTP","section":{"id":"extra","label":"Extra Codes"},"value":"fake-totp"}"#,
            r#"{"id":"pin","label":"PIN","value":"fake-pin"}"#,
            r#"{"id":"PIN2","label":"pin","value":"fake-pin-2"}"#,
        ];
        let wanted = field.to_lowercase();
        Ok(serde_json::json!(
            fields
                .iter()
                .map(|json| serde_json::from_str(json).unwrap())
                .filter(|candidate: &serde_json::Value| {
                    ["label", "id"].iter().any(|key| {
                        candidate
                            .get(*key)
                            .and_then(|v| v.as_str())
                            .is_some_and(|have| have.to_lowercase() == wanted)
                    })
                })
                .collect::<Vec<serde_json::Value>>()
        ))
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
}

#[cfg(unix)]
mod process;

mod generation;
mod queries;
mod synthetic;

mod lookups;
mod retry;
