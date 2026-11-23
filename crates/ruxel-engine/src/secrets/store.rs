//! The store write: the value staged in a file, the store process, and
//! the removal of the staging file on every path out of the write — the
//! success, the failure, and the store that never started.

use super::backend::ProcessBackend;
use super::staging::StagedValue;

impl ProcessBackend {
    /// The store write, with the process spawn behind a seam: a spawn
    /// that fails on its own — the store never starts — is testable, and
    /// the staging file it leaves behind is removed on that path too.
    ///
    /// # Errors
    /// Returns an error if the key is malformed, the value cannot be
    /// staged, the store cannot be started or finished, or the staging
    /// file cannot be removed.
    pub(super) fn fnox_set_with(
        key: &str,
        value: &str,
        spawn: impl FnOnce(&StagedValue) -> std::io::Result<std::process::Child>,
    ) -> Result<(), String> {
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
        let Ok(mut child) = spawn(&staged) else {
            // The store never started and the staging file holds the
            // plaintext: the removal is part of the failed store, and a
            // removal that cannot be confirmed fails the store rather
            // than leave the plaintext behind under a generic process
            // error. The drop stays the best-effort fallback for a
            // removal that failed here.
            staged.cleanup()?;
            return Err("process could not start".to_owned());
        };
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
}
