//! The in-memory gate that serializes one fnox key's reads with its store
//! attempts, so a read never answers a value the store has already replaced.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The in-memory gate every fnox key has: a resolution holds it across the
/// read, a store attempt holds it across the attempt and the cache
/// invalidation that follows. Cloned resolvers share the registry, so a
/// read that races a store in this process waits for the invalidation
/// instead of answering the value cached before it. The gate is memory,
/// not the store's write lock: a run that only reads keys never opens,
/// creates, or writes a lock file beside `fnox.toml`, so a read-only
/// checkout or configuration mount reads as it always did — the filesystem
/// lock stays with the store attempts that may really write. The caller
/// holds the gate next to its guard, which is what keeps the guard alive.
pub(super) fn key_gate(key: &str) -> Arc<Mutex<()>> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    match GATES.get_or_init(Mutex::default).lock() {
        Ok(mut gates) => gates.entry(key.to_owned()).or_default().clone(),
        // A poisoned registry still serializes what it can: a fresh gate
        // for this key is the conservative answer, never an open one.
        Err(_) => Arc::new(Mutex::new(())),
    }
}
