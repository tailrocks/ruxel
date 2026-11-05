use super::super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ruxel-ledger-test-{}-{name}-{id}",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("operation succeeds");
        Self(path)
    }

    fn file(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, content).expect("operation succeeds");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn record_file(ledger: &mut Ledger, key: &str, path: &Path) {
    ledger.record(
        key,
        "file",
        &json!({"path": path}),
        "ok",
        &json!({"changed": true, "marker": "kept"}),
    );
}

mod group_1;
mod group_2;
mod group_3;
