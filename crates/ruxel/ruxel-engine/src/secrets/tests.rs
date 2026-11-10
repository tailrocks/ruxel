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
    fn onepassword_fields(
        &self,
        _vault: &str,
        _item: &str,
        _section: Option<&str>,
        field: &str,
    ) -> Result<serde_json::Value, String> {
        self.items.fetch_add(1, Ordering::SeqCst);
        // The item's fields, pre-filtered the way `op item get --fields`
        // filters: by label or id. "password" matches two sections.
        let fields = [
            r#"{"id":"username","label":"Username","value":"alice"}"#,
            r#"{"id":"password","label":"Password","section":{"id":"login","label":"Login"},"value":"fake-password"}"#,
            r#"{"id":"password","label":"Password","section":{"id":"api","label":"API"},"value":"fake-password-api"}"#,
        ];
        Ok(serde_json::json!(
            fields
                .iter()
                .filter(|json| json.contains(&format!("\"{field}\"")))
                .map(|json| serde_json::from_str(json).unwrap())
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

mod queries;
mod synthetic;

mod lookups;
mod retry;
