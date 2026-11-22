//! Symlink aliases of the managed boot file.

use std::path::{Path, PathBuf};

/// Whether `candidate` is a symlink alias of the managed file: the
/// entry resolves to the file itself, so it carries the managed bytes
/// at boot rather than shadowing them. Only a link that targets the
/// managed file is exempt — a regular file, a mask, a broken link, or
/// a link anywhere else still holds the name over the lower-precedence
/// copy. The comparison is path-level (`read_link` with
/// `canonicalize`), never a raw device-and-inode stat conversion.
pub(super) fn is_self_alias(candidate: &str, file: &str) -> bool {
    let Ok(target) = std::fs::read_link(candidate) else {
        return false;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        Path::new(candidate)
            .parent()
            .map_or(target.clone(), |parent| parent.join(&target))
    };
    match (
        std::fs::canonicalize(&resolved),
        std::fs::canonicalize(file),
    ) {
        (Ok(into), Ok(managed)) => into == managed,
        // Neither side resolves — the link dangles at the managed
        // path the publish has yet to create — so the names compare
        // lexically after resolving `.` and `..` without touching the
        // filesystem: a relative alias such as
        // `/etc/sysctl.d/../../usr/lib/sysctl.d/x.conf` is a genuine
        // self-alias once the managed file is published, while a
        // broken link anywhere else still shadows.
        (Err(_), Err(_)) => lexical_normal(&resolved) == lexical_normal(Path::new(file)),
        _ => false,
    }
}

/// The path with `.` dropped and `..` applied lexically, without
/// touching the filesystem: both sides dangle, so no component may be
/// resolved through a link — the comparison is spelling, already
/// proven to name the same file once the publish creates it.
fn lexical_normal(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(component),
        }
    }
    out
}

pub(crate) fn shadow_candidates_in(file: &str, dirs: &[(&str, u8)]) -> Vec<String> {
    let Some((dir, base)) = file.rsplit_once('/') else {
        return Vec::new();
    };
    let dir = format!("{dir}/");
    let Some((_, rank)) = dirs.iter().find(|(known, _)| *known == dir) else {
        return Vec::new();
    };
    dirs.iter()
        .filter(|(_, higher)| higher > rank)
        .map(|(known, _)| format!("{known}{base}"))
        .collect()
}
