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
        // Both sides fail to resolve. Only a missing file counts as
        // dangling — the link aims at the managed path the publish has
        // yet to create — so any other failure (a link loop, an
        // unreachable component, a permission) refuses outright. And
        // even two missing files never compare as raw spellings: a
        // `..` past a link resolves from the link's target, not from
        // the spelling, so collapsing it lexically across a symlink
        // names a file the entry never reaches. Each side is resolved
        // the way the kernel resolves it — through every link that is
        // still on the filesystem, failing closed where one cannot be
        // judged — and only the names that comparison leaves behind
        // are weighed. A relative alias such as
        // `/etc/sysctl.d/../../usr/lib/sysctl.d/x.conf` over plain
        // directories still resolves to the genuine self-alias the
        // publish creates, while a broken link anywhere else still
        // shadows.
        (Err(into), Err(managed))
            if into.kind() == std::io::ErrorKind::NotFound
                && managed.kind() == std::io::ErrorKind::NotFound =>
        {
            match (pinned_lexical(&resolved), pinned_lexical(Path::new(file))) {
                (Some(into), Some(managed)) => into == managed,
                _ => false,
            }
        }
        _ => false,
    }
}

/// The file the dangling path names once the publish creates it,
/// resolved the way the kernel resolves it: every component that is
/// still on the filesystem is followed — a link through its target,
/// a `..` from the directory the walk actually stands in — so a
/// spelling whose `..` crosses a link lands where the loader lands,
/// not where the letters collapse. Only the tail past the first
/// missing component is spelling, and even that spelling fails
/// closed where a `..` would cancel a name that was never on the
/// filesystem. `None` is fail-closed: a link the walk cannot judge —
/// dangling, looping, permissioned — or any other refusal names no
/// file, so the candidate shadows.
fn pinned_lexical(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut resolved = match path.components().next() {
        Some(Component::Prefix(_)) => return None,
        Some(Component::RootDir) => PathBuf::from("/"),
        // A relative spelling resolves against the working directory
        // the loader runs in; an unjudgeable one names nothing.
        _ => std::env::current_dir().ok()?,
    };
    let mut missing: Option<Vec<Component>> = None;
    for component in path.components() {
        if let Some(tail) = missing.as_mut() {
            tail.push(component);
            continue;
        }
        match component {
            Component::Prefix(_) => return None,
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                let next = resolved.join(name);
                match std::fs::symlink_metadata(&next) {
                    Ok(meta) if meta.is_symlink() => {
                        // Through the link the way the loader goes: a
                        // target the walk cannot reach dangles here,
                        // not at the managed file.
                        resolved = std::fs::canonicalize(&next).ok()?;
                    }
                    Ok(_) => resolved = next,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing = Some(vec![component]);
                    }
                    Err(_) => return None,
                }
            }
        }
    }
    let tail = missing.unwrap_or_default();
    // The resolved base holds no link, so a `..` that climbs it climbs
    // the way the loader climbs; a `..` that would cancel a name the
    // walk never found on the filesystem compares two dangles at two
    // different missing names, so it fails closed instead.
    let mut base_len = resolved.components().count();
    for component in tail {
        match component {
            Component::Prefix(_) => return None,
            Component::RootDir => {
                resolved = PathBuf::from("/");
                base_len = resolved.components().count();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if resolved.components().count() > base_len {
                    return None;
                }
                if resolved.pop() {
                    base_len = base_len.saturating_sub(1);
                }
            }
            Component::Normal(_) => resolved.push(component),
        }
    }
    Some(resolved)
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
