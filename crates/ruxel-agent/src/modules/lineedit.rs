//! Shared guards for the line-editing operations (`lineinfile`,
//! `replace`): editing through a link keeps the link, and patterns
//! matching empty spans never converge.

/// A line-editing target that stays a link: `read_to_string` follows the
/// link into its target, but an atomic publish would replace the link's
/// own inode, silently converting a linked configuration path into a
/// regular file while leaving the intended target unchanged. Edit the
/// resolved target instead, so the link survives. A path that resolves to
/// nothing keeps its spelling and fails where it always did.
pub(crate) fn resolve_edit_target(path: &str) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path))
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
