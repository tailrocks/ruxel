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

#[cfg(unix)]
mod process;

mod queries;
mod synthetic;

mod lookups;
mod retry;
