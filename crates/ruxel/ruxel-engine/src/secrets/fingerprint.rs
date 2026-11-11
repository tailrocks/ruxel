//! The keyed, truncated secret fingerprint and its per-process key.

/// Cryptographically random bytes from the operating system.
pub(crate) fn random_bytes(buffer: &mut [u8]) -> Result<(), String> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(buffer))
        .map_err(|_| "no random source available".to_owned())
}

/// A keyed, truncated secret fingerprint. The key is drawn once per process,
/// so a fingerprint compares only within one run and is never an offline
/// oracle: an attacker without the key cannot test guesses against it, and
/// at most eight hex characters are shown.
pub(super) struct Fingerprints {
    key: [u8; 32],
}

impl Fingerprints {
    /// Draws the per-process key from the operating system's random source.
    /// This fails closed: with no random source there is no safe key, so
    /// construction errors and the run stops rather than falling back to a
    /// weak or constant key an attacker could enumerate.
    ///
    /// # Errors
    /// Returns an error if the operating system's random source is
    /// unavailable.
    pub(super) fn new() -> Result<Self, String> {
        Self::from_random(random_bytes)
    }

    /// The same construction over a named entropy draw, so the fail-closed
    /// path is testable: any draw that errors aborts construction.
    ///
    /// # Errors
    /// Returns an error if the draw fails.
    pub(super) fn from_random(
        draw: impl FnOnce(&mut [u8]) -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut key = [0_u8; 32];
        draw(&mut key)?;
        Ok(Self { key })
    }

    pub(super) fn of(&self, selector: &str, value: &str) -> String {
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(selector.as_bytes());
        hasher.update(&[31]);
        hasher.update(value.as_bytes());
        hasher.finalize().to_hex()[..8].to_owned()
    }
}
