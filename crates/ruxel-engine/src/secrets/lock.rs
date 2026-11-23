//! The fnox write lock: one `flock` held for a create-or-compare sequence.

use std::fmt;
use std::sync::Mutex;

/// The fnox write lock, held for the length of one create-or-compare
/// sequence. Dropping the guard releases it.
pub struct FnoxLock {
    file: Mutex<std::fs::File>,
}

impl FnoxLock {
    /// Locks an open file exclusively; the guard unlocks and closes it.
    ///
    /// # Errors
    /// Returns an error if the lock cannot be taken.
    pub fn exclusive(file: std::fs::File) -> Result<Self, String> {
        use std::os::unix::io::AsFd;
        rustix::fs::flock(file.as_fd(), rustix::fs::FlockOperation::LockExclusive)
            .map_err(|_| "fnox write lock unavailable".to_owned())?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    /// A lock that guards nothing: backends without a store-side lock, and
    /// test doubles.
    ///
    /// # Errors
    /// Returns an error if the placeholder file cannot be opened.
    pub fn unheld() -> Result<Self, String> {
        Self::exclusive(
            std::fs::File::open("/dev/null")
                .map_err(|_| "fnox write lock unavailable".to_owned())?,
        )
    }
}

impl fmt::Debug for FnoxLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FnoxLock")
    }
}

impl Drop for FnoxLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsFd;
        if let Ok(file) = self.file.lock() {
            let _result = rustix::fs::flock(file.as_fd(), rustix::fs::FlockOperation::Unlock);
        }
    }
}
