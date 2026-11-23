//! Shared guards for the line-editing operations (`lineinfile`,
//! `replace`): editing through a link keeps the link, and patterns
//! matching empty spans never converge.

/// A line-editing target that stays a link: `read_to_string` follows the
/// link into its target, but an atomic publish would replace the link's
/// own inode, silently converting a linked configuration path into a
/// regular file while leaving the intended target unchanged. Editing
/// through the link instead would let anyone who can plant a link where
/// the entry points redirect a privileged write into an arbitrary file.
/// Neither silent outcome is acceptable, so a final link is refused
/// loudly: point the entry at the target file itself. Links in parent
/// directories keep the kernel's resolution, as before.
pub(crate) fn refuse_final_symlink<'a>(
    module: &str,
    path: &'a str,
) -> Result<&'a std::path::Path, String> {
    let target = std::path::Path::new(path);
    if std::fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "{module}: refusing to edit through the symlink at {path}; point the entry at the target file"
        ));
    }
    Ok(target)
}

/// A regular expression that can match without consuming text: under
/// `replace_all` it leaves the same zero-width match behind on every apply,
/// and under lineinfile's per-line scan it names every line stale, so
/// such a pattern never converges. Both line operations refuse it before
/// touching the file.
pub(crate) fn reject_empty_span(
    module: &str,
    pattern: &str,
    matched_empty: bool,
) -> Result<(), String> {
    if matched_empty {
        return Err(format!(
            "{module}: regexp {pattern:?} matches empty spans; a zero-width match never converges, so the pattern must consume text"
        ));
    }
    Ok(())
}
