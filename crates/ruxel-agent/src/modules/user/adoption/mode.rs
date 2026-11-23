//! The modes the home operation enforces and trusts: the mode a
//! created directory takes, and whether an existing directory is the
//! operator's own or a chain intermediate no unprivileged principal
//! can plant through.

use rustix::fs::{Mode, fchmod};
use std::os::fd::OwnedFd;

use super::operating_uid;

/// The mode a created directory is given, enforced on the directory
/// itself after it is created: the requested mode is only a request —
/// the process's umask takes bits away, and a default ACL on the
/// parent grants the ones the request does not ask for. Parents are
/// created traversable and never world-writable, and the home takes
/// the policy mode: the trust the adoption checks rely on.
pub(crate) fn enforce_mode(fd: &OwnedFd, mode: Mode, home: &str) -> Result<(), String> {
    fchmod(fd, mode).map_err(|error| format!("user: create home {home}: {error}"))
}

/// Whether the directory is the operator's own and writable by no one
/// else — so its adoption hands the account no principal's contents
/// but the operator's — or a chain intermediate under the exception.
pub(crate) fn own_directory(uid: u32, mode: u32) -> bool {
    (uid == 0 || uid == operating_uid()) && mode & 0o022 == 0
}

/// Whether a chain intermediate is trusted: the full ancestor rule, or
/// a root-owned directory whose group writability belongs to a
/// privileged group — the owner is root, the group holds no
/// unprivileged principal, and others cannot write, so no unprivileged
/// principal can plant through it. The exception is off-Linux only:
/// on Linux a lower-privileged member of the owning group can rename
/// the target aside and plant a replacement home, so even `root:root
/// 0775` refuses there. The other-triplet check is other-write, not
/// other-closed: others traverse every root-owned system directory
/// already (including `/`), and traversal without write cannot rename
/// anything aside. An other-closed check would refuse `root:admin
/// 0775` layouts — macOS routes all of `/home` through one — without
/// buying any privilege boundary.
pub(crate) fn chain_dir_trusted(uid: u32, gid: u32, mode: u32) -> bool {
    chain_dir_trusted_on(uid, gid, mode, cfg!(target_os = "linux"))
}

/// The chain rule for the platform `strict` names: the Linux rule
/// trusts the strict base alone, while other platforms keep the
/// privileged-group exception. Split out so the Linux refusal is
/// provable on any host.
pub(crate) fn chain_dir_trusted_on(uid: u32, gid: u32, mode: u32, strict: bool) -> bool {
    own_directory(uid, mode)
        || (!strict && uid == 0 && mode & 0o002 == 0 && mode & 0o020 != 0 && privileged_gid(gid))
}

/// Whether the group holds no unprivileged principal: root everywhere,
/// and admin on macOS, whose members can already sudo.
pub(crate) fn privileged_gid(gid: u32) -> bool {
    gid == 0 || (cfg!(target_os = "macos") && gid == 80)
}
