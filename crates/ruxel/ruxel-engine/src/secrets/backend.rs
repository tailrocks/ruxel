use super::Backend;
use super::staging::StagedValue;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct ProcessBackend {
    fnox: PathBuf,
    op: PathBuf,
    lock_dir: Option<PathBuf>,
}

impl Default for ProcessBackend {
    fn default() -> Self {
        Self::with_programs("fnox", "op")
    }
}

impl ProcessBackend {
    #[must_use]
    pub fn with_programs(fnox: impl Into<PathBuf>, op: impl Into<PathBuf>) -> Self {
        Self {
            fnox: fnox.into(),
            op: op.into(),
            lock_dir: None,
        }
    }

    /// Pins the directory the write lock lives in. The store fixture tests
    /// pin their own directory; unset, the lock is searched from the fnox
    /// configuration's directory, like the fnox CLI resolves its config,
    /// falling back to the working directory.
    #[must_use]
    pub fn with_lock_dir(mut self, lock_dir: impl Into<PathBuf>) -> Self {
        self.lock_dir = Some(lock_dir.into());
        self
    }

    /// Whether the store keeps this key's value in a file.
    ///
    /// `fnox get` prints the value of a secret; a secret configured
    /// `as_file = true` prints the path of a temporary file that holds the
    /// value instead. The store says which of the two a key is: `fnox env
    /// --json --describe` — the unfiltered query — lists every key of the
    /// active profile with its `env` and `as_file` flags, and resolves
    /// nothing. The answer is the store's own contract, never a guess
    /// about what the printed text looks like. The query runs before any
    /// `fnox get`, so a file-backed key is known without `get` ever
    /// creating its persistent file.
    ///
    /// Anything that is not a complete report of the key's flag fails
    /// closed: an error document, or one too short to name the flag, means
    /// the store cannot report whether the key is file-backed, and the
    /// read fails naming `as_file` as unsupported. The alternative is
    /// taking the pathname `fnox get` printed for the secret.
    ///
    /// A key the store does not know is not file-backed: the read that
    /// follows fails with the store's own missing-key error.
    fn fnox_file_backed(&self, key: &str) -> Result<bool, String> {
        let mut cmd = Command::new(&self.fnox);
        cmd.args(["env", "--json", "--describe"]);
        let output = cmd
            .output()
            .map_err(|_| "process could not start".to_owned())?;
        let cannot_report = || {
            format!(
                "fnox cannot report whether \"{key}\" is a file-backed secret: \
                     a secret configured with `as_file = true` is not supported"
            )
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let document: serde_json::Value =
            serde_json::from_str(stdout.trim()).map_err(|_| cannot_report())?;
        let Some(keys) = document.get("keys").and_then(serde_json::Value::as_array) else {
            // The store answered with an error document instead of the
            // key list: it cannot report the flag, so the read fails
            // closed rather than taking a pathname for a secret.
            return Err(cannot_report());
        };
        let Some(entry) = keys
            .iter()
            .find(|entry| entry.get("key").and_then(serde_json::Value::as_str) == Some(key))
        else {
            // The key is not among the profile's keys: not file-backed,
            // and the read that follows fails with the store's own error.
            return Ok(false);
        };
        // An entry too short to name the flag cannot report it either.
        match entry.get("as_file").and_then(serde_json::Value::as_bool) {
            Some(as_file) => Ok(as_file),
            None => Err(cannot_report()),
        }
    }
}

fn process_output(mut command: Command) -> Result<String, String> {
    let result = command
        .output()
        .map_err(|_| "process could not start".to_owned())?;
    if !result.status.success() {
        return Err("process failed".into());
    }
    Ok(with_record_terminator(&result.stdout))
}

/// The directory holding `fnox.toml`, searched upward from the working
/// directory, like the fnox CLI resolves its configuration. Without one, the
/// working directory itself.
fn fnox_config_dir() -> Option<std::path::PathBuf> {
    let mut directory = std::env::current_dir().ok()?;
    loop {
        if directory.join("fnox.toml").is_file() {
            return Some(directory);
        }
        if !directory.pop() {
            return None;
        }
    }
}

impl ProcessBackend {
    /// Reads one key through `fnox get --if-missing error`: a key configured
    /// `env = false` is never injected into `fnox exec`'s environment, so an
    /// exec+printenv read always fails for it; `get` prints the resolved
    /// value whatever the key's env setting. For a key configured
    /// `as_file = true`, the resolved value `get` prints is the path of the
    /// persistent file it created to hold the decrypted content. `--if-missing
    /// error` keeps a missing key distinct from an empty one: with the
    /// config's `warn` or `ignore`, `get` prints nothing and exits 0, which
    /// would silently resolve to an empty value.
    fn fnox_read(&self, key: &str) -> Result<Result<String, ReadFailure>, String> {
        let mut cmd = Command::new(&self.fnox);
        cmd.args(["get", "--if-missing", "error", key]);
        let result = cmd
            .stdin(Stdio::null())
            .output()
            .map_err(|_| "process could not start".to_owned())?;
        if result.status.success() {
            return Ok(Ok(with_record_terminator(&result.stdout)));
        }
        // "Missing" maps from the pinned fnox version's exit code and
        // stderr, so a missing key is distinct from a failed store: a
        // generated key's absence means create it; a store failure fails
        // the run. The pinned CLI exits 1 and says `not found`.
        if result.status.code() == Some(1)
            && String::from_utf8_lossy(&result.stderr)
                .to_lowercase()
                .contains("not found")
        {
            return Ok(Err(ReadFailure::Missing));
        }
        Ok(Err(ReadFailure::Failed))
    }

    /// The raw contents of a file-backed key from the `files` entry of
    /// `fnox env --json --keys <key>`.
    ///
    /// `as_file` secrets come back as raw contents in `files`, so fnox
    /// leaves no files behind: the caller writes them. This is the preferred
    /// read of a file-backed key over `fnox get`, whose persistent file
    /// holds the decrypted value on disk.
    ///
    /// `Ok(None)` — the preferred contract cannot serve the key: `--keys`
    /// refuses a key configured `env = false` as not injectable, and a
    /// document without the key's entry is the same answer. The caller then
    /// falls back to `fnox get`, whose file it removes.
    fn fnox_env_files(&self, key: &str) -> Result<Option<String>, String> {
        let mut cmd = Command::new(&self.fnox);
        cmd.args(["env", "--json", "--keys", key]);
        let output = cmd
            .output()
            .map_err(|_| "process could not start".to_owned())?;
        let Ok(document) = serde_json::from_str::<serde_json::Value>(
            String::from_utf8_lossy(&output.stdout).trim(),
        ) else {
            // An error document, or no document at all: the contract cannot
            // serve this key.
            return Ok(None);
        };
        Ok(document
            .get("files")
            .and_then(serde_json::Value::as_object)
            .and_then(|files| files.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned))
    }

    /// The fallback read of a file-backed key: `fnox get` prints the path of
    /// the persistent file it created, and the value is that file's content.
    ///
    /// The file is removed before this returns on every path — read success,
    /// read error, and any early return after the path is known — because the
    /// file holds the decrypted value: plaintext must not outlive the
    /// lookup. A removal failure fails the read closed even when the content
    /// read fine; a file already gone counts as removed, which is the
    /// guarantee: no plaintext left behind.
    fn fnox_read_persistent_file(&self, key: &str) -> Result<Option<String>, String> {
        let path = match self.fnox_read(key)? {
            Ok(path) => path,
            Err(ReadFailure::Missing) => return Ok(None),
            Err(ReadFailure::Failed) => return Err("process failed".to_owned()),
        };
        let content = std::fs::read_to_string(&path);
        let removal_failed = std::fs::remove_file(&path)
            .err()
            .filter(|error| error.kind() != std::io::ErrorKind::NotFound);
        if let Some(error) = removal_failed {
            return Err(format!(
                "the file of the file-backed secret \"{key}\" could not be removed: {error}"
            ));
        }
        content
            .map(Some)
            .map_err(|error| format!("the file-backed secret \"{key}\" could not be read: {error}"))
    }

    /// The store's write lock: an exclusive `flock` on
    /// `<fnox config dir>/.ruxel-fnox-write.lock`, held while a run creates
    /// a missing generated key or stores a fetched secret. Runs that honor
    /// the lock serialize; the guard releases on drop.
    fn write_lock(&self) -> Result<super::FnoxLock, String> {
        let directory = self
            .lock_dir
            .clone()
            .or_else(fnox_config_dir)
            .or_else(|| std::env::current_dir().ok())
            .ok_or("fnox write lock unavailable")?;
        let path = directory.join(".ruxel-fnox-write.lock");
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| "fnox write lock unavailable".to_owned())?
        };
        #[cfg(not(unix))]
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|_| "fnox write lock unavailable".to_owned())?;
        super::FnoxLock::exclusive(file)
    }
}

/// Why a fnox read failed. A missing key is not a failed store.
enum ReadFailure {
    Missing,
    Failed,
}

/// The CLIs behind these lookups append exactly one record terminator:
/// `fnox get`, and `op read`. A value that legitimately ends in a
/// newline — PEM keys do — must keep it, so at most one trailing `\n` goes,
/// plus the `\r` only when it is part of that terminator. This is the
/// terminator the CLI adds, never one the value carries: a value that
/// ends in `\n` is read back as the value with its `\n`.
fn with_record_terminator(stdout: &[u8]) -> String {
    let mut value = String::from_utf8_lossy(stdout).into_owned();
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    value
}

impl Backend for ProcessBackend {
    fn fnox_get(&self, key: &str) -> Result<Option<String>, String> {
        // The same grammar the Engine schema and model validation enforce;
        // the message names it with the sentence Core and Lowering use.
        if !super::is_fnox_key(key) {
            return Err(format!(
                "fnox key \"{key}\" is malformed: {}",
                super::FNOX_KEY_GRAMMAR
            ));
        }
        // The store's report of the key's flags comes first: a file-backed
        // key is known before `fnox get` ever runs. `fnox get` creates a
        // persistent temporary file that holds the decrypted value of an
        // `as_file = true` key and prints its path — plaintext left on disk —
        // so a key known file-backed up front is served without ever running
        // `get`: no `get`, no file.
        if !self.fnox_file_backed(key)? {
            return match self.fnox_read(key)? {
                Ok(value) => Ok(Some(value)),
                Err(ReadFailure::Missing) => Ok(None),
                Err(ReadFailure::Failed) => Err("process failed".to_owned()),
            };
        }
        // A file-backed secret: `fnox env --json --keys` returns the raw
        // contents in its `files` entry and explicitly leaves no files
        // behind — the caller writes them. That contract cannot serve a key
        // configured `env = false`: `--keys` refuses it as not injectable,
        // and the read falls back to `get`.
        if let Some(contents) = self.fnox_env_files(key)? {
            return Ok(Some(contents));
        }
        // The fallback: `fnox get` prints the path of the persistent file it
        // created, and that file is removed on every path after. If it
        // cannot be removed the read fails closed: decrypted plaintext must
        // never stay on disk after Ruxel finishes, and never silently.
        self.fnox_read_persistent_file(key)
    }

    fn fnox_set(&self, key: &str, value: &str) -> Result<(), String> {
        // The value reaches `fnox set` in a staging file through
        // `--from-file`, never in a command argument and never on stdin:
        // the pinned fnox trims the whitespace around a value it reads from
        // stdin, and `--from-file` keeps the file's exact bytes, so the
        // read-back comparison sees the value that was meant, byte for
        // byte. The staging file is removed when the store call finishes,
        // on success and failure alike.
        if !super::is_fnox_key(key) {
            return Err(format!(
                "fnox key \"{key}\" is malformed: {}",
                super::FNOX_KEY_GRAMMAR
            ));
        }
        let mut staged = StagedValue::new(value)?;
        let mut child = Command::new(&self.fnox)
            .args(["set", key, "--from-file"])
            .arg(staged.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "process could not start".to_owned())?;
        let waited = child.wait().map_err(|_| "store did not finish".to_owned());
        // The staging file holds the plaintext, so its removal is part of
        // the store, not a cleanup after it: a removal that fails fails
        // the store rather than report success with the complete secret
        // left behind. `Drop` stays the best-effort fallback for the paths
        // that never reach this line.
        staged.cleanup()?;
        if !waited?.success() {
            return Err("process failed".into());
        }
        Ok(())
    }

    fn fnox_write_lock(&self, _key: &str) -> Result<super::FnoxLock, String> {
        self.write_lock()
    }

    fn onepassword_fields(
        &self,
        vault: &str,
        item: &str,
        section: Option<&str>,
        field: &str,
    ) -> Result<serde_json::Value, String> {
        let selector =
            section.map_or_else(|| field.to_owned(), |section| format!("{section}.{field}"));
        let mut cmd = Command::new(&self.op);
        cmd.args(["item", "get", "--vault"])
            .arg(vault)
            .args(["--fields"])
            .arg(selector)
            .args(["--format", "json", "--"])
            .arg(item);
        match serde_json::from_str(&process_output(cmd)?) {
            Ok(value @ serde_json::Value::Array(_)) => Ok(value),
            // A unique match may come back as a single field object.
            Ok(value @ serde_json::Value::Object(_)) => Ok(serde_json::json!([value])),
            _ => Err("invalid item response".into()),
        }
    }
    fn onepassword_reference(&self, reference: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.op);
        cmd.args(["read", "--", reference]);
        process_output(cmd)
    }
    fn command(&self, argv: &[String]) -> Result<String, String> {
        let (program, arguments) = argv.split_first().ok_or("empty command")?;
        let mut cmd = Command::new(program);
        cmd.args(arguments);
        process_output(cmd)
    }
}
