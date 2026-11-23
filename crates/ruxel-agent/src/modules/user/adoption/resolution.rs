//! The trust walk over the resolved path: a symlink is refused
//! wherever a non-root principal could plant or replace it, and every
//! directory the target leads through is trusted the way the ancestors
//! are. The kernel resolves `..` after following links, so the walk
//! follows each component the same way instead of popping lexically.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use super::{chain_dir_trusted, own_directory};

/// Whether a symlinked ancestor stands where a non-root principal
/// could plant or replace it: a link only root can replace is system
/// configuration the operator chose, and the path resolves through it
/// as it always did — but only through directories the target leads
/// through that are trusted the same way, so a link in a trusted
/// directory cannot point at a home a non-root principal pre-created
/// (`/srv/homes -> /tmp/homes`, `/home -> /tmp/attacker`).
pub(crate) fn planted_in(link: &Path, home: &str) -> Result<(), String> {
    let parent = standing_parent(link, home)?;
    let target = std::fs::read_link(link)
        .map_err(|error| format!("user: home {home}: {}: {error}", link.display()))?;
    symlink_target_trusted(&parent, &target, home)
}

/// The directory a symlinked ancestor stands in: root's and writable
/// by no one else — the operator's configuration.
pub(crate) fn standing_parent(link: &Path, home: &str) -> Result<PathBuf, String> {
    let parent = match link.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let meta = std::fs::metadata(&parent)
        .map_err(|error| format!("user: home {home}: {}: {error}", parent.display()))?;
    if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        return Err(format!(
            "user: home {home}: {} is a symlink in {}, which a non-root principal can rewrite",
            link.display(),
            parent.display()
        ));
    }
    Ok(parent)
}

/// A symlink target resolved the way the kernel resolves it, then
/// trusted the way trusted ancestors are: every component is
/// followed against the already-resolved prefix, so a `..` climbs
/// out of the link's own target instead of cancelling the link
/// lexically — `link/../homes` with `link -> /tmp/evil` resolves
/// under `/tmp`, and a lexical `<base>/homes` approval can never
/// cover it. An intermediate admits a root-owned directory whose
/// group writability belongs to a privileged group; the resolved
/// target itself always takes the full ancestor rule. A missing
/// component ends the walk, for nothing could have been planted
/// under it.
pub(crate) fn symlink_target_trusted(base: &Path, target: &Path, home: &str) -> Result<(), String> {
    target_dirs_trusted(&resolve_target(base, target, home, 8)?, home, 8)
}

/// A symlink target resolved the way the kernel resolves it: each
/// component is followed against the already-resolved prefix, an
/// absolute target restarts at the root, and a nested link expands
/// in place against the same budget. A `..` the walk cannot place
/// — past the root of a relative base — is refused, never guessed;
/// a loop of links is refused within the budget; a missing
/// component ends the walk with the rest appended lexically, for
/// the trust walk below stops at the same missing component.
fn resolve_target(base: &Path, target: &Path, home: &str, budget: u8) -> Result<PathBuf, String> {
    let mut resolved = if target.is_absolute() {
        PathBuf::from("/")
    } else {
        base.to_path_buf()
    };
    let mut floor = resolved.clone();
    let mut pending: VecDeque<OsString> = target.components().filter_map(split_component).collect();
    let mut budget = budget;
    while let Some(name) = pending.pop_front() {
        if name.to_str() == Some("..") {
            step_above(&mut resolved, &floor, home)?;
            continue;
        }
        resolved.push(&name);
        match std::fs::symlink_metadata(&resolved) {
            Ok(meta) if meta.is_symlink() => {
                if budget == 0 {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} leads through too many symlinks",
                        resolved.display()
                    ));
                }
                budget -= 1;
                let nested = std::fs::read_link(&resolved).map_err(|error| {
                    format!("user: home {home}: {}: {error}", resolved.display())
                })?;
                resolved.pop();
                if nested.is_absolute() {
                    resolved = PathBuf::from("/");
                    floor = resolved.clone();
                }
                let mut front: VecDeque<OsString> =
                    nested.components().filter_map(split_component).collect();
                front.extend(pending);
                pending = front;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for rest in pending {
                    resolved.push(rest);
                }
                return Ok(resolved);
            }
            Err(error) => {
                return Err(format!(
                    "user: home {home}: {}: {error}",
                    resolved.display()
                ));
            }
        }
    }
    Ok(resolved)
}

/// A path component as the pending queue holds it: a name to enter,
/// or the marker for a parent step. Anchors and current-directory
/// steps resolve to nothing, the way the kernel skips them.
fn split_component(part: Component<'_>) -> Option<OsString> {
    match part {
        Component::Normal(name) => Some(name.to_os_string()),
        Component::ParentDir => Some(OsString::from("..")),
        Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
    }
}

/// A `..` applied to the already-resolved prefix the way the kernel
/// applies it: climbing out of a link target the walk already
/// followed. Past the root of a relative base there is nothing the
/// walk placed, so the target is refused instead of resolved
/// against an unknown parent.
fn step_above(resolved: &mut PathBuf, floor: &Path, home: &str) -> Result<(), String> {
    if *resolved == *floor && floor.is_relative() {
        return Err(format!(
            "user: refusing to adopt the existing home {home}: .. escapes the trusted base"
        ));
    }
    resolved.pop();
    Ok(())
}

/// Every existing directory of a resolved symlink target, trusted the
/// way trusted ancestors are. Resolution already expanded every
/// link it met, so a link here appeared mid-check: it is admitted
/// where it stands and the walk continues under its own resolved
/// target, within the expansion budget — a loop of links is refused
/// instead of followed forever.
fn target_dirs_trusted(resolved: &Path, home: &str, budget: u8) -> Result<(), String> {
    let mut prefix = PathBuf::new();
    for component in resolved.components() {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => {
                if budget == 0 {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} leads through too many symlinks",
                        prefix.display()
                    ));
                }
                let parent = standing_parent(&prefix, home)?;
                let nested = std::fs::read_link(&prefix)
                    .map_err(|error| format!("user: home {home}: {}: {error}", prefix.display()))?;
                let mut spliced = resolve_target(&parent, &nested, home, budget)?;
                let rest = resolved
                    .strip_prefix(&prefix)
                    .unwrap_or_else(|_| Path::new(""))
                    .to_path_buf();
                for part in rest.components() {
                    spliced.push(part);
                }
                return target_dirs_trusted(&spliced, home, budget - 1);
            }
            Ok(meta) if meta.is_dir() => {
                // The resolved target takes the full ancestor rule; an
                // intermediate takes the chain rule: a principal that
                // can write the target can redirect the home, while an
                // intermediate admits root-owned group writability by a
                // privileged group.
                let trusted = if prefix == *resolved {
                    own_directory(meta.uid(), meta.mode())
                } else {
                    chain_dir_trusted(meta.uid(), meta.gid(), meta.mode())
                };
                if !trusted {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} is writable by others or not the operator's, so a non-root principal may have placed the home beneath it",
                        prefix.display()
                    ));
                }
            }
            Ok(_) => {
                return Err(format!(
                    "user: home {home}: {} is not a directory",
                    prefix.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("user: home {home}: {error}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod target_resolution_tests {
    use super::super::operating_uid;
    use super::{resolve_target, symlink_target_trusted};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn stage(name: &str) -> PathBuf {
        // Canonicalized: the walk follows every component the way the
        // kernel does, including a symlinked TMPDIR prefix itself
        // (`/var -> private/var` on macOS), so the expected paths
        // must use the same physical spelling.
        let dir = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("ruxel-adopt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("base")).unwrap();
        std::fs::create_dir_all(dir.join("evil")).unwrap();
        std::fs::create_dir_all(dir.join("sibling")).unwrap();
        std::os::unix::fs::symlink(dir.join("evil"), dir.join("base").join("link")).unwrap();
        dir
    }

    #[test]
    fn dotdot_applies_after_the_link_target() {
        let stage = stage("dotdot");
        let resolved =
            resolve_target(&stage.join("base"), Path::new("link/../homes"), "test", 8).unwrap();
        assert_eq!(
            resolved,
            stage.join("homes"),
            "the kernel follows the link first, so .. climbs out of its target"
        );
        std::fs::remove_dir_all(&stage).unwrap();
    }

    #[test]
    fn plain_dotdot_above_a_trusted_base_resolves() {
        let stage = stage("plain");
        let resolved =
            resolve_target(&stage.join("base"), Path::new("../sibling"), "test", 8).unwrap();
        assert_eq!(resolved, stage.join("sibling"));
        std::fs::remove_dir_all(&stage).unwrap();
    }

    #[test]
    fn absolute_link_target_resets_resolution_to_root() {
        let stage = stage("absolute");
        std::os::unix::fs::symlink(stage.join("evil"), stage.join("base").join("abslink")).unwrap();
        let resolved =
            resolve_target(&stage.join("base"), Path::new("abslink/x"), "test", 8).unwrap();
        assert_eq!(
            resolved,
            stage.join("evil").join("x"),
            "an absolute link target restarts resolution at the root"
        );
        std::fs::remove_dir_all(&stage).unwrap();
    }

    #[test]
    fn nested_link_loop_is_refused_within_budget() {
        let stage = stage("loop");
        let base = stage.join("base");
        std::os::unix::fs::symlink("loopy2", base.join("loopy1")).unwrap();
        std::os::unix::fs::symlink("loopy1", base.join("loopy2")).unwrap();
        let error = resolve_target(&base, Path::new("loopy1/x"), "test", 8)
            .expect_err("a loop of links must be refused, not followed forever");
        assert!(error.contains("too many symlinks"), "{error}");
        std::fs::remove_dir_all(&stage).unwrap();
    }

    #[test]
    fn dotdot_past_a_relative_base_is_refused() {
        let error = resolve_target(Path::new("."), Path::new("../escape"), "test", 8)
            .expect_err("a .. past a relative base cannot resolve safely");
        assert!(error.contains("escapes"), "{error}");
    }

    #[test]
    fn dotdot_after_a_link_into_an_untrusted_target_is_refused() {
        if operating_uid() != 0 {
            return;
        }
        let root = Path::new("/").join(format!("ruxel-adopt-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        let evil = root.join("evil");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&evil).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&evil, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::os::unix::fs::symlink(&evil, base.join("link")).unwrap();
        let home = base.join("homes").join("alice");
        let error =
            symlink_target_trusted(&base, Path::new("link/../homes"), home.to_str().unwrap())
                .expect_err("a .. that escapes through an untrusted link target must be refused");
        assert!(error.contains("evil"), "{error}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
