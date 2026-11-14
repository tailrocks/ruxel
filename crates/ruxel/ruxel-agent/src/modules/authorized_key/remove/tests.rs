#[path = "tests/concurrent.rs"]
mod concurrent;
#[path = "tests/detached.rs"]
mod detached;
#[path = "tests/keys.rs"]
mod keys;

use std::os::unix::fs::MetadataExt;

/// The armed hook is process-wide, so the tests that arm it take
/// turns: without this, one test's arming overwrites the other's
/// before its removal runs, the swap never happens and the test
/// fails against the very race it pins down.
static ARMING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scratch(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("ruxel-authkey-remove-{tag}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}
