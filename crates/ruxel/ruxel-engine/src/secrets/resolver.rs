//! The lookup resolver: caching, field-scoped 1Password fetches, and the
//! apply-time generation and fetch-store paths.

use super::Cell;
use super::{
    Backend, FNOX_KEY_GRAMMAR, GENERATED_FORMATS, Lookup, ProcessBackend, is_fnox_key,
    is_generated_format, parse_op_reference, select_field, synthetic_value,
};
use crate::secrets::fingerprint::Fingerprints;
use crate::secrets::generation::{GeneratedRequest, generate_value, one_format};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

#[path = "onepassword.rs"]
mod onepassword;

type ItemCache =
    Arc<Mutex<HashMap<(String, String, Option<String>, String), Cell<serde_json::Value>>>>;

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
fn key_gate(key: &str) -> Arc<Mutex<()>> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    match GATES.get_or_init(Mutex::default).lock() {
        Ok(mut gates) => gates.entry(key.to_owned()).or_default().clone(),
        // A poisoned registry still serializes what it can: a fresh gate
        // for this key is the conservative answer, never an open one.
        Err(_) => Arc::new(Mutex::new(())),
    }
}

fn cached<T: Clone>(
    cell: &Cell<T>,
    resolve: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let mut value = cell.lock().map_err(|_| "lookup cache unavailable")?;
    if let Some(value) = value.as_ref() {
        return Ok(value.clone());
    }
    let resolved = resolve()?;
    *value = Some(resolved.clone());
    Ok(resolved)
}

#[derive(Clone)]
pub struct Resolver {
    backend: Arc<dyn Backend>,
    pub(super) synthetic: bool,
    values: Arc<Mutex<HashMap<Lookup, Cell<String>>>>,
    items: ItemCache,
    fingerprints: Arc<Fingerprints>,
}

impl fmt::Debug for Resolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Resolver([REDACTED])")
    }
}

impl Resolver {
    /// # Errors
    /// Returns an error if the per-process fingerprint key cannot be drawn
    /// from the operating system's random source: with no safe key the
    /// resolver fails closed rather than fall back to a weak one.
    pub fn new(backend: Arc<dyn Backend>) -> Result<Self, String> {
        Ok(Self {
            backend,
            synthetic: false,
            values: Arc::default(),
            items: Arc::default(),
            fingerprints: Arc::new(Fingerprints::new()?),
        })
    }

    /// The synthetic resolver: every lookup answers a selector-derived
    /// placeholder and no store is contacted. `plan`, `check` and `diff`
    /// always run this way — the mode is wired to these commands, not an
    /// opt-in flag. The placeholder derives from the selector, not the
    /// value, so synthetic mode is rotation-blind: a rotated value is never
    /// detected there, which the design states rather than promises away.
    /// Nothing is ever generated in this mode.
    /// # Errors
    /// Returns an error if the per-process fingerprint key cannot be drawn
    /// from the operating system's random source.
    pub fn synthetic() -> Result<Self, String> {
        Self::synthetic_with(Arc::new(ProcessBackend::default()))
    }

    /// The synthetic resolver over a named backend, for runs and tests that
    /// must see that the store is never contacted in synthetic mode.
    ///
    /// # Errors
    /// Returns an error if the per-process fingerprint key cannot be drawn
    /// from the operating system's random source.
    pub fn synthetic_with(backend: Arc<dyn Backend>) -> Result<Self, String> {
        let mut resolver = Self::new(backend)?;
        resolver.synthetic = true;
        Ok(resolver)
    }

    /// Whether this resolver runs synthetic: every lookup answers a
    /// selector-derived placeholder and no store is ever contacted.
    #[must_use]
    pub fn is_synthetic(&self) -> bool {
        self.synthetic
    }

    /// The keyed fingerprint of one secret value: a keyed BLAKE3 derivation
    /// over the selector and the value, truncated to at most eight hex
    /// characters. Never an unsalted full SHA-256, which for a human-chosen
    /// password would be an offline dictionary oracle. The key is drawn once
    /// per process, so fingerprints compare only within one run — which is
    /// the only place they compare (check and diff are rotation-blind and
    /// never compare against earlier runs).
    #[must_use]
    pub fn fingerprint(&self, selector: &str, value: &str) -> String {
        self.fingerprints.of(selector, value)
    }

    /// # Errors
    /// Returns a redacted error if the lookup fails.
    pub fn resolve(&self, lookup: &Lookup) -> Result<String, String> {
        // A cached read and a store attempt on the same key serialize on
        // the key's in-memory gate: `fetch_secret` and `ensure_generated`
        // hold it across their store attempts and the cache invalidation
        // that follows, so a read that races a store cannot answer the
        // value cached before the attempt and cannot run before the
        // store's invalidation — it waits, then answers what the store now
        // holds. The gate is memory, not the store's filesystem lock: an
        // ordinary read never opens or creates a lock file beside the
        // fnox configuration, so a read-only checkout still reads. A
        // lookup that reads no fnox key, and a synthetic resolver that
        // never contacts the store, takes no gate.
        let gate = match lookup {
            Lookup::Fnox { key } | Lookup::FnoxGenerated { key, .. } if !self.synthetic => {
                Some(key_gate(key))
            }
            _ => None,
        };
        let _read = gate.as_ref().and_then(|gate| gate.lock().ok());
        let cell = self
            .values
            .lock()
            .map_err(|_| "lookup cache unavailable")?
            .entry(lookup.clone())
            .or_default()
            .clone();
        cached(&cell, || self.resolve_uncached(lookup))
    }
    fn resolve_uncached(&self, lookup: &Lookup) -> Result<String, String> {
        if self.synthetic {
            return Ok(synthetic_value(lookup));
        }
        let generic = || "secret lookup failed".to_owned();
        match lookup {
            Lookup::Fnox { key } => self.read_fnox(key).map_err(|_| generic()),
            Lookup::FnoxGenerated { key, .. } => self.read_fnox(key).map_err(|_| generic()),
            Lookup::OnePassword {
                vault,
                item,
                field,
                section,
            } => self.op_field(vault, item, field, section.as_deref()),
            Lookup::OnePasswordUri { reference } => {
                if reference.contains('?') {
                    return self
                        .backend
                        .onepassword_reference(reference)
                        .map_err(|_| generic());
                }
                let (vault, item, section, field) =
                    parse_op_reference(reference).ok_or_else(generic)?;
                // The reference components are validated before the read
                // path is chosen: a dotted reference goes to `op read` as
                // written, which `op_field`'s guard would otherwise never
                // reach, and a comma or an equals sign would ride along as
                // the CLI's selector punctuation.
                if let Some(section) = section {
                    onepassword::selector_punctuation("section", section)?;
                }
                onepassword::selector_punctuation("field", field)?;
                // A label with a dot in it cannot be spelled as a
                // `--fields` selector: the CLI reads the dot as the
                // section separator and would fetch another field. The
                // parts of a reference are separated by `/`, so the
                // reference names the label as it is written, and the
                // reference path reads it.
                if field.contains('.') || section.is_some_and(|section| section.contains('.')) {
                    return self
                        .backend
                        .onepassword_reference(reference)
                        .map_err(|_| generic());
                }
                self.op_field(vault, item, field, section)
            }
            Lookup::Command { argv } => self.backend.command(argv).map_err(|_| generic()),
            Lookup::Synthetic { value } => Ok(value.clone()),
        }
    }

    fn read_fnox(&self, key: &str) -> Result<String, String> {
        self.backend
            .fnox_get(key)?
            .ok_or_else(|| "secret lookup failed".to_owned())
    }

    /// Whether the value the store holds is the value that was written.
    /// The comparison is exact: the backend removes the one record
    /// terminator the CLI adds when it prints a value, so a read-back is
    /// the stored value byte for byte. A value that ends in a newline is
    /// another value than one that does not: stripping a newline from
    /// both sides would report "value\n" and "value" as equal, and a
    /// fetch whose file lost its final newline would go unnoticed.
    fn read_back_matches(written: &str, read_back: &str) -> bool {
        read_back == written
    }

    /// # Errors
    /// Returns a redacted error if any lookup fails.
    pub fn prefetch(&self, lookups: &[Lookup]) -> Result<(), String> {
        let unique: HashSet<_> = lookups.iter().cloned().collect();
        let queue = Mutex::new(unique.into_iter().collect::<Vec<_>>());
        let failed = Mutex::new(false);
        std::thread::scope(|scope| {
            for _ in 0..lookups.len().min(8) {
                scope.spawn(|| {
                    loop {
                        let next = queue.lock().ok().and_then(|mut q| q.pop());
                        let Some(lookup) = next else { break };
                        if self.resolve(&lookup).is_err()
                            && let Ok(mut value) = failed.lock()
                        {
                            *value = true;
                        }
                    }
                });
            }
        });
        if *failed.lock().map_err(|_| "lookup cache unavailable")? {
            Err("secret lookup failed".into())
        } else {
            Ok(())
        }
    }
}

impl Resolver {
    /// Creates every missing generated key, once per run, on the controller,
    /// before any secret is resolved: under the fnox write lock, check
    /// presence, generate in the named format, store from a staging file,
    /// read back and byte-compare.
    ///
    /// The creation is atomic against the writers that share the fnox write
    /// lock: they wait for it, and the key is created once. A writer that
    /// does not take the lock — another tool, another controller — can
    /// write the key between the presence check and the store, and its
    /// value is overwritten. The presence re-check before the store is the
    /// defence in depth: it catches the writer that arrived before it and
    /// fails the run, and a writer that arrives after it is not caught.
    /// Every key must already be present by the time the run resolves it, so
    /// a rerun after a partial failure presents the value the first run
    /// registered. `plan`, `check` and `diff` never reach this: they run in
    /// synthetic mode, which generates nothing. `emit` is called once per
    /// key this run created, with the key, the format and the generated
    /// value — the caller registers the value with the redaction registry
    /// and never reports it.
    ///
    /// The requests are checked before any key is created: a malformed key,
    /// an unknown format, or one key named with two formats fails the run
    /// without creating anything, whichever order the requests are in.
    ///
    /// # Errors
    /// Returns a redacted error, naming the store and key, never a value.
    pub fn ensure_generated(
        &self,
        requests: &[GeneratedRequest],
        emit: &mut dyn FnMut(&str, &str, &str),
    ) -> Result<(), String> {
        if self.synthetic {
            return Err("generation never runs in synthetic mode".to_owned());
        }
        let mut formats: Vec<(String, String)> = Vec::new();
        for request in requests {
            if !is_fnox_key(&request.key) {
                return Err(format!(
                    "fnox key \"{}\" is malformed: {FNOX_KEY_GRAMMAR}",
                    request.key
                ));
            }
            if !is_generated_format(&request.format) {
                return Err(format!(
                    "fnoxGenerated format \"{}\" is not one of {}",
                    request.format,
                    GENERATED_FORMATS.join(", ")
                ));
            }
            one_format(&request.key, &request.format, &mut formats)?;
        }
        for request in requests {
            // The in-memory gate first, then the store's filesystem lock:
            // cloned resolvers reading the key wait for this attempt and
            // the invalidation that follows it, whatever they hold.
            let store_gate = key_gate(&request.key);
            let _store = store_gate.lock().map_err(|_| "lookup cache unavailable")?;
            let _lock = self.backend.fnox_write_lock(&request.key)?;
            if self.backend.fnox_get(&request.key)?.is_none() {
                let value = generate_value(&request.format)?;
                // The key appeared since the presence check: another writer
                // created it without the lock. The re-check is the defence
                // in depth against a writer that does not take the lock; a
                // writer that arrives after it is not caught.
                if self.backend.fnox_get(&request.key)?.is_some() {
                    return Err(format!(
                        "generated secret {} appeared in fnox while it was being created; the run fails rather than overwrite it",
                        request.key
                    ));
                }
                self.backend.fnox_set(&request.key, &value)?;
                let read_back = self.backend.fnox_get(&request.key)?.ok_or_else(|| {
                    format!(
                        "generated secret {} is missing after it was stored",
                        request.key
                    )
                })?;
                if !Self::read_back_matches(&value, &read_back) {
                    return Err(format!(
                        "generated secret {} read back from fnox differs from what was stored",
                        request.key
                    ));
                }
                emit(&request.key, &request.format, &value);
            }
        }
        Ok(())
    }

    /// The apply-time `FetchSecret` store: the controller compares the fetched
    /// content with the stored value and writes fnox only when they differ.
    /// Returns whether fnox now holds a new value, and the fingerprint of
    /// the fetched content. The comparison is exact, on the controller; the
    /// fingerprint is what the run reports. Under the synthetic resolver
    /// nothing is ever written: the fetch reports unchanged, known after
    /// apply, without contacting the store's read or write path.
    ///
    /// # Errors
    /// Returns a redacted error, naming the store and key, never a value.
    pub fn fetch_secret(&self, key: &str, content: &str) -> Result<(bool, String), String> {
        if !is_fnox_key(key) {
            return Err(format!(
                "fnox key \"{key}\" is malformed: {FNOX_KEY_GRAMMAR}"
            ));
        }
        // The comparison is exact: the store holds the bytes that were
        // written, so a value that ends in a newline is another value than
        // one that does not. A fetch whose file lost its final newline is
        // a change, not an equality.
        let fingerprint = self.fingerprints.of(key, content);
        if self.synthetic {
            return Ok((false, fingerprint));
        }
        let store_gate = key_gate(key);
        let _store = store_gate.lock().map_err(|_| "lookup cache unavailable")?;
        let _lock = self.backend.fnox_write_lock(key)?;
        let stored = self.backend.fnox_get(key)?;
        if stored
            .as_ref()
            .is_some_and(|stored| Self::read_back_matches(content, stored))
        {
            return Ok((false, fingerprint));
        }
        // The store attempt may change the key whatever it reports — a
        // `set` that fails can leave the key half-written — so every
        // cached resolution that reads the key is dropped after the
        // attempt, on its success and its error paths alike. The
        // verification that follows never carries the invalidation: a
        // read-back that fails leaves the cache correct — empty — where
        // it was stale, and a retry that finds the store already holding
        // the content takes the unchanged early return above without ever
        // reaching a verified store.
        let attempted = self.backend.fnox_set(key, content);
        self.forget(key);
        attempted?;
        let read_back = self
            .backend
            .fnox_get(key)?
            .ok_or_else(|| format!("fnox key {key} is missing after it was stored"))?;
        if !Self::read_back_matches(content, &read_back) {
            return Err(format!(
                "fnox key {key} read back from fnox differs from what was stored"
            ));
        }
        Ok((true, fingerprint))
    }

    /// Drops every cached resolution that reads this key. A fetch that
    /// attempted a store may have changed what the key answers — the
    /// attempt's own report says nothing certain about it — so the values
    /// cached before the attempt must not outlive it, and the drop runs
    /// after every attempt, verified or not; a lock failure keeps the
    /// stale entry, which the next store attempt or a fresh run drops.
    fn forget(&self, key: &str) {
        if let Ok(mut values) = self.values.lock() {
            values.retain(|lookup, _| !lookup.reads_fnox_key(key));
        }
    }
}
