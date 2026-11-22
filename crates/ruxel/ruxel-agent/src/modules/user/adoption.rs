//! Whether the account's home is the operator's to give it:
//! the entry checks over the resolved path, the trust the existing
//! directories they lead through must meet, and the modes they carry.
//!
//! The walk itself lives in [`resolution`], the claimant check in
//! [`claim`] and the modes in [`mode`].

mod claim;
mod mode;
mod operator;
mod resolution;

pub(crate) use operator::operating_uid;

pub(super) use claim::{ManagedAccount, adoption_pinned, adoption_trusted, trusted_ancestors};
pub(super) use mode::{chain_dir_trusted, enforce_mode, own_directory};
pub(super) use resolution::symlink_target_trusted;

#[cfg(test)]
pub(super) use claim::{intended_claim, managed_claim, sole_claimant};
#[cfg(test)]
pub(super) use mode::{chain_dir_trusted_on, privileged_gid};

/// The declared home path is usable before the account is touched:
/// `usermod -d` moves the passwd field even when the path is a regular
/// file, so the path is validated before any mutation. A symlink is
/// refused whatever it targets, and every ancestor is inspected the
/// same way. An existing directory is adopted only where
/// [`adoption_trusted`] admits it for the managed account; a missing
/// path is usable only under trusted ancestors.
pub(super) fn home_usable(home: &str, managed: &ManagedAccount<'_>) -> Result<(), String> {
    ancestors_usable(home)?;
    match std::fs::symlink_metadata(home) {
        Ok(meta) if meta.is_symlink() => Err(format!("user: home {home} exists and is a symlink")),
        Ok(meta) if meta.is_dir() => {
            claim::adoption_trusted(home, &meta, managed)?;
            Ok(())
        }
        Ok(_) => Err(format!("user: home {home} exists and is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => trusted_ancestors(home),
        Err(error) => Err(format!("user: home {home}: {error}")),
    }
}

/// Every ancestor of the home path, inspected without following it:
/// path-based creation and ownership would traverse a symlinked
/// component as root. A symlinked ancestor is refused wherever a
/// non-root principal could plant or replace it, and admitted only
/// where the parent is root's and closed. A missing component ends the
/// walk: the rest does not exist yet.
pub(super) fn ancestors_usable(home: &str) -> Result<(), String> {
    let components: Vec<std::path::Component<'_>> =
        std::path::Path::new(home).components().collect();
    let mut prefix = std::path::PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => resolution::planted_in(&prefix, home)?,
            Ok(meta) if meta.is_dir() => {}
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
