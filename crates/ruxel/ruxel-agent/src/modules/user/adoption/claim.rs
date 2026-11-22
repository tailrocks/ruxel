//! Whether the passwd claim hands an existing home to the managed
//! account: name and number both, read from the live entry. A
//! directory another principal owns is refused whatever its mode, so
//! the account never executes a stranger's contents at login.

use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;

use super::{own_directory, resolution::planted_in};

/// The account a home operation manages: the adoption admits an
/// existing directory only where the passwd home field names this
/// account — name and number both. The number is `None` where the
/// step states none and the allocator draws it.
pub(crate) struct ManagedAccount<'a> {
    pub(crate) name: &'a str,
    pub(crate) uid: Option<u32>,
}

#[cfg(test)]
impl<'a> ManagedAccount<'a> {
    /// The managed account where the plan states no number.
    pub(crate) fn unnumbered(name: &'a str) -> Self {
        ManagedAccount { name, uid: None }
    }
}

/// Whether the passwd claim hands the leaf to the managed account —
/// name and number both — with the leaf carrying the claimant's
/// number, so the entry read is the live one, not a stale line.
pub(crate) fn managed_claim(
    claimant: &str,
    uid: u32,
    leaf_uid: u32,
    managed: &ManagedAccount<'_>,
) -> bool {
    claimant == managed.name && managed.uid == Some(uid) && uid == leaf_uid
}

/// An existing home directory the account adopts, only under trusted
/// ancestors — where it is root's or the operator's own, or the
/// managed account's whose passwd home field names the path. A
/// directory another principal owns may have been pre-created by that
/// principal with contents the new account would then execute at login,
/// so it is refused whatever its mode: a 0700 leaf under trusted
/// ancestors is still another principal's. The ancestors weigh even
/// the operator's own staging — a leftover `/home/alice`, a
/// same-stage file's directory — since a renamed-aside leaf replanted
/// under a writable ancestor must never become the account's. A
/// creation whose account is not in the database yet misses the
/// intended lookup and still takes the operator's rule, so the miss
/// never fails a valid creation.
pub(crate) fn adoption_trusted(
    home: &str,
    leaf: &std::fs::Metadata,
    managed: &ManagedAccount<'_>,
) -> Result<(), String> {
    // The operator's own directory is adopted under trusted ancestors
    // too: only the ancestors weigh a leaf renamed aside and replanted.
    if own_directory(leaf.uid(), leaf.mode()) {
        return trusted_ancestors(home);
    }
    // The managed account's own leaf is adopted under trusted
    // ancestors — and only where it is itself writable by no one
    // else, or anyone's contents in it would become the account's.
    // The claim must name the managed account itself, so adopting
    // `/home/bob` for Alice is refused while Bob still points there.
    if let Some((claimant, uid)) = intended_claim(home)
        && managed_claim(&claimant, uid, leaf.uid(), managed)
    {
        if leaf.mode() & 0o022 != 0 {
            return Err(format!(
                "user: refusing to adopt the existing home {home}: the directory is writable by others, so its contents may be a non-root principal's"
            ));
        }
        return trusted_ancestors(home);
    }
    Err(format!(
        "user: refusing to adopt the existing home {home}: the directory belongs to another principal, so its contents may be theirs, not the account's"
    ))
}

/// The account the passwd database intends the home for: the name and
/// number from the single line whose home field names the path — or
/// `None` where none names it, or where two do.
pub(crate) fn intended_claim(home: &str) -> Option<(String, u32)> {
    sole_claimant(&std::fs::read_to_string("/etc/passwd").ok()?, home)
}

/// The sole claimant in the passwd text, voided by a second claimant.
pub(crate) fn sole_claimant(passwd: &str, home: &str) -> Option<(String, u32)> {
    let mut found: Option<(String, u32)> = None;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.get(5) != Some(&home) {
            continue;
        }
        let uid: u32 = fields.get(2)?.parse().ok()?;
        if found.replace((fields.first()?.to_string(), uid)).is_some() {
            return None;
        }
    }
    found
}

/// Every existing ancestor of the home path, trusted the way the
/// symlinked-ancestor rule trusts a parent: root's — or the
/// operator's own — and writable by no one else. A directory a
/// non-root principal can write is one that principal can plant the
/// home under, so the adoption is refused there. A missing component
/// ends the walk: the rest does not exist yet.
pub(crate) fn trusted_ancestors(home: &str) -> Result<(), String> {
    let components: Vec<std::path::Component<'_>> =
        std::path::Path::new(home).components().collect();
    let mut prefix = std::path::PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        prefix.push(component);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.is_symlink() => planted_in(&prefix, home)?,
            Ok(meta) if meta.is_dir() => {
                if !own_directory(meta.uid(), meta.mode()) {
                    return Err(format!(
                        "user: refusing to adopt the existing home {home}: {} is writable by others or not the operator's, so a non-root principal may have placed the home",
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

/// The adoption the opened descriptors pin, re-verified after the
/// open: the path the pre-open check inspected may have been replaced
/// in between, so the trust decision is re-made on the inode the
/// operation will own, never on the path.
pub(crate) fn adoption_pinned(
    home: &str,
    fd: &OwnedFd,
    managed: &ManagedAccount<'_>,
) -> Result<(), String> {
    let duplicate = fd
        .try_clone()
        .map_err(|error| format!("user: own home {home}: {error}"))?;
    let meta = std::fs::File::from(duplicate)
        .metadata()
        .map_err(|error| format!("user: own home {home}: {error}"))?;
    adoption_trusted(home, &meta, managed)
}
