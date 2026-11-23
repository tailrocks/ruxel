#[path = "tests/comments.rs"]
mod comments;
#[path = "tests/identity.rs"]
mod identity;
#[path = "tests/material.rs"]
mod material;
#[path = "tests/options.rs"]
mod options;
#[path = "tests/rollback.rs"]
mod rollback;
#[path = "tests/safety.rs"]
mod safety;

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// The per-process counter keeps two tests that pick the same label from
// sharing one directory: `ruxel-file-swap-<pid>` is also a `file` test's
// scratch base, and a shared path made each test's `remove_dir_all` race
// the other's writes.
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ruxel-{label}-{}-{}",
        std::process::id(),
        NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir(&path).unwrap();
    path
}
