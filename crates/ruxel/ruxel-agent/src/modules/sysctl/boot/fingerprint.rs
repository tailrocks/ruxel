//! The fingerprint of the boot surface: the digest the ledger weighs
//! on the cached results of a task that judged it.

use std::path::PathBuf;

use super::{boot_bytes, system_files};

/// The fingerprint of the effective boot surface: the files that load
/// at boot and what they assign. A change of it — a new file, another
/// assignment — moves the value the host comes up with, so a cached
/// task that judged the host before the change must not be served.
pub(crate) fn boot_fingerprint() -> Option<String> {
    fingerprint_of(&system_files().ok()?)
}

/// The fingerprint of the boot files `files` names: their raw name
/// bytes and contents, so a file whose name is not text moves the
/// fingerprint like any other. The bytes come from the same
/// file-type-safe classification the override probe reads: an entry
/// that loads as nothing — a directory, a FIFO, a mask, a broken
/// link, a file that vanished — contributes no assignments and is
/// never opened (a FIFO with no writer would block the open the way
/// it blocks procps), while a regular file that cannot be read
/// refuses the fingerprint — its assignments are unknown. Every field
/// is length-framed: boot contents are arbitrary bytes and may hold a
/// NUL, so bare separators are ambiguous — one file holding
/// `X\0B\0Y` hashes like two files holding `X` and `Y` — and the
/// framing keeps them apart. The format differs from the bare one, so
/// a fingerprint the old code wrote misses under the new one; the
/// verify already treats a mismatch as a moved surface and recomputes,
/// so the invalidation only reruns the task, never serves a stale one.
pub(crate) fn fingerprint_of(files: &[PathBuf]) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    let mut hasher = Sha256::new();
    for path in files {
        let content = match boot_bytes(path) {
            Ok(Some(content)) => content,
            Ok(None) => continue,
            Err(_) => return None,
        };
        for field in [std::ffi::OsStr::new(path).as_bytes(), content.as_slice()] {
            let len = u64::try_from(field.len()).ok()?;
            hasher.update(len.to_le_bytes());
            hasher.update(field);
        }
    }
    Some(crate::ledger::hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::super::scratch::Scratch;
    use super::fingerprint_of;
    use std::path::PathBuf;

    /// The old framing, for the invalidation test: `path\0content\0`
    /// per file, with no lengths — the ambiguity the framing removes.
    fn legacy_fingerprint(files: &[PathBuf]) -> Option<String> {
        use sha2::Digest;
        use std::os::unix::ffi::OsStrExt;
        let mut hasher = sha2::Sha256::new();
        for path in files {
            let content = match super::super::boot_bytes(path) {
                Ok(Some(content)) => content,
                Ok(None) => continue,
                Err(_) => return None,
            };
            hasher.update(std::ffi::OsStr::new(path).as_bytes());
            hasher.update([0u8]);
            hasher.update(&content);
            hasher.update([0u8]);
        }
        Some(crate::ledger::hex(&hasher.finalize()))
    }

    #[test]
    fn nul_bearing_contents_no_longer_collide() {
        // One file holding `X\0<other>\0Y` hashed exactly like two
        // files holding `X` and `Y`: a removed file plus a changed one
        // kept the fingerprint while the boot surface moved. The framed
        // fields keep them apart.
        use std::os::unix::ffi::OsStrExt;
        let farm = Scratch::new("framing-collision");
        let first = farm.0.join("first.conf");
        let second = farm.0.join("second.conf");
        std::fs::write(&first, b"X").expect("operation succeeds");
        std::fs::write(&second, b"Y").expect("operation succeeds");
        let pair = vec![first.clone(), second.clone()];
        let pair_legacy = legacy_fingerprint(&pair);
        let pair_new = fingerprint_of(&pair);
        // The single file carries the pair's own bytes, so the old
        // stream is one record either way.
        let mut ambiguous = b"X\x00".to_vec();
        ambiguous.extend_from_slice(second.as_os_str().as_bytes());
        ambiguous.extend_from_slice(b"\x00Y");
        std::fs::write(&first, &ambiguous).expect("operation succeeds");
        let single = vec![first];
        assert_eq!(
            legacy_fingerprint(&single),
            pair_legacy,
            "the old framing collides: the test's premise"
        );
        assert_ne!(
            fingerprint_of(&single),
            pair_new,
            "the framed fields keep the file-sets apart"
        );
    }

    #[test]
    fn the_new_framing_invalidates_the_old_format() {
        // A fingerprint the old code wrote must miss, not hit: the
        // verify already treats a mismatch as a moved surface and
        // recomputes, so the format change only reruns the task, never
        // serves a stale one.
        let farm = Scratch::new("framing-invalidation");
        let file = farm.0.join("managed.conf");
        std::fs::write(&file, b"net.test = 1\n").expect("operation succeeds");
        let files = vec![file];
        assert_ne!(
            fingerprint_of(&files),
            legacy_fingerprint(&files),
            "the old format misses under the new framing"
        );
    }

    #[test]
    fn distinct_file_sets_have_distinct_fingerprints() {
        // The property over the shapes the old separators blurred: a
        // name, a member, and a NUL-bearing edit each move it — and an
        // empty file still weighs, while a vanished one loads as
        // nothing.
        let farm = Scratch::new("framing-distinct");
        let left = farm.0.join("left.conf");
        let right = farm.0.join("right.conf");
        let probe = |files: &[PathBuf]| fingerprint_of(files).expect("the files read");
        std::fs::write(&left, b"1").expect("operation succeeds");
        std::fs::write(&right, b"1").expect("operation succeeds");
        assert_ne!(
            probe(std::slice::from_ref(&left)),
            probe(std::slice::from_ref(&right)),
            "the name weighs, not just the content"
        );
        assert_ne!(
            probe(std::slice::from_ref(&left)),
            probe(&[left.clone(), right.clone()]),
            "an added file moves it"
        );
        std::fs::write(&left, b"1\x00").expect("operation succeeds");
        assert_ne!(
            probe(std::slice::from_ref(&left)),
            probe(std::slice::from_ref(&right)),
            "a NUL-bearing edit moves it"
        );
        std::fs::write(&left, b"").expect("operation succeeds");
        assert_ne!(
            probe(std::slice::from_ref(&left)),
            probe(&[]),
            "an empty file still weighs"
        );
    }
}
