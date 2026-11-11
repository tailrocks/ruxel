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
    pub(super) fn new() -> Self {
        let mut key = [0_u8; 32];
        if random_bytes(&mut key).is_err() {
            // Without the kernel source, fall back to process-unique
            // mixing; fingerprints only ever compare within one process.
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let mut hasher = blake3::Hasher::new();
            hasher.update(&time.to_le_bytes());
            hasher.update(&std::process::id().to_le_bytes());
            hasher.update(&(std::ptr::from_ref(&key) as usize).to_le_bytes());
            key = *hasher.finalize().as_bytes();
        }
        Self { key }
    }

    pub(super) fn of(&self, selector: &str, value: &str) -> String {
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(selector.as_bytes());
        hasher.update(&[31]);
        hasher.update(value.as_bytes());
        hasher.finalize().to_hex()[..8].to_owned()
    }
}

impl Default for Fingerprints {
    fn default() -> Self {
        Self::new()
    }
}
