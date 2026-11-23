//! The floor a removal never crosses: the roots a removal never takes,
//! however the path reaches them.

use std::path::{Component, Path, PathBuf};

/// The roots a removal never takes: the filesystem root and the
/// directories the system itself lives in.
pub(super) const SYSTEM_ROOTS: [&str; 22] = [
    "/",
    "/etc",
    "/var",
    "/root",
    "/home",
    "/usr",
    "/boot",
    "/dev",
    "/proc",
    "/sys",
    // The system's own links into the merged tree: they are the system's
    // own names, and a removal that takes one breaks the host.
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    // The roots of the platform the agent also runs on: `/private`
    // holds the real `/etc` and `/var` of macOS, `/Users` is the home
    // root of the platform, and the rest hold the system itself.
    "/private",
    "/Users",
    "/Library",
    "/System",
    "/Applications",
    "/Volumes",
];

/// A removal never takes a system root, however the path reaches it.
///
/// The plan holds a removal against the same floor, but the plan reads the
/// path as it is written: a symlinked ancestor (`/tmp/root` -> `/`) carries
/// the removal of `/tmp/root/etc` into the real `/etc`. The ancestors of
/// the path are therefore resolved, and where they land is held against
/// the floor. The last component keeps the name it was given: a symlink
/// that points at a system root is a link, and taking a link takes
/// nothing.
pub(super) fn refuse_system_root(path: &Path) -> Result<(), String> {
    refuse_root(path, &SYSTEM_ROOTS)
}

/// A removal never takes one of the roots, however the path reaches
/// it. The roots are held by spelling and by identity: the spelling
/// says nothing on a case-insensitive filesystem (`/USERS` is
/// `/Users`), while the identity is read without following the final
/// symlink, since taking a symlink takes the link and nothing else.
pub(super) fn refuse_root(path: &Path, roots: &[&str]) -> Result<(), String> {
    let written = written_path(path);
    if roots.contains(&written.as_str()) {
        return Err(format!(
            "file: refusing to remove {written}: a removal never takes a system root"
        ));
    }
    if let Some(root) = root_by_identity(path, roots) {
        return Err(format!(
            "file: refusing to remove {written}: it is the system root {root} under another spelling"
        ));
    }
    // The system's own layout (`/var` -> `/private/var`, `/bin` ->
    // `/usr/bin`) is a first component and is not a crossing.
    if !crosses_symlink(path) {
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let written_parent = parent.to_string_lossy().to_string();
    let Ok(resolved) = std::fs::canonicalize(parent) else {
        return Ok(());
    };
    let resolved = resolved.to_string_lossy().to_string();
    if resolved == "/" {
        return Err(format!(
            "file: refusing to remove {written}: it reaches the filesystem root"
        ));
    }
    for root in roots {
        // A root is held by the name it was written under and by the name
        // it resolves to: `/var` and `/private/var` are one root.
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
        let canonical = canonical.to_string_lossy().to_string();
        let spellings = [root.to_string(), canonical];
        let reached = spellings.iter().any(|root| inside(&resolved, root));
        if !reached {
            continue;
        }
        // The path was already inside the root as it was written: the
        // resolution did not take it there.
        let already = spellings.iter().any(|root| inside(&written_parent, root));
        if already {
            continue;
        }
        return Err(format!(
            "file: refusing to remove {written}: it reaches {resolved}, inside the system root {root}"
        ));
    }
    Ok(())
}

/// The root the path names under another spelling, if any: the device
/// and the inode of the path itself, read without following the final
/// symlink. A path that is not there, or a root that is not either,
/// simply does not match.
fn root_by_identity(path: &Path, roots: &[&str]) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    roots.iter().find_map(|root| {
        let root_meta = std::fs::symlink_metadata(root).ok()?;
        if meta.dev() == root_meta.dev() && meta.ino() == root_meta.ino() {
            return Some((*root).to_string());
        }
        // The system's own layout names a root through a symlink
        // (`/bin` -> `/usr/bin`, `/etc` -> `/private/etc`): the spelling
        // the root is written under is a link, and the candidate names
        // the directory it resolves to. The candidate itself is still
        // read without following a final symlink, so taking a link keeps
        // taking the link.
        let canonical = std::fs::canonicalize(root).ok()?;
        let resolved = std::fs::metadata(&canonical).ok()?;
        (meta.dev() == resolved.dev() && meta.ino() == resolved.ino()).then(|| (*root).to_string())
    })
}

/// Whether a path is a root or lies inside it.
fn inside(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

/// The path in its normal form: the separators are single and the `.`
/// components are gone, so a path that names a root in another spelling
/// (`//etc`) is the root it names.
fn written_path(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => parts.push(String::new()),
            Component::CurDir | Component::Prefix(_) => {}
            Component::ParentDir => parts.push("..".to_string()),
            Component::Normal(name) => parts.push(name.to_string_lossy().to_string()),
        }
    }
    if parts.is_empty() {
        return ".".to_string();
    }
    if parts[0].is_empty() {
        parts.remove(0);
        return format!("/{}", parts.join("/"));
    }
    parts.join("/")
}

/// Whether the path crosses a symlink below its first component.
fn crosses_symlink(path: &Path) -> bool {
    let mut current = PathBuf::new();
    let mut first = true;
    for component in path.components() {
        match component {
            Component::RootDir => current.push("/"),
            Component::CurDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                current.push("..");
                first = false;
            }
            Component::Normal(name) => {
                current.push(name);
                if !first && is_symlink(&current) {
                    return true;
                }
                first = false;
            }
        }
    }
    false
}

/// The names the system itself aliases at the top level: the links of
/// the system's own layout (`/var` -> `/private/var`, `/bin` ->
/// `/usr/bin`). A link of any other name is not the system's own.
const SYSTEM_ALIASES: [&str; 11] = [
    "bin", "sbin", "lib", "lib32", "lib64", "libx32", "tmp", "var", "etc", "home", "usr",
];

/// Whether a name is the system's own alias for a directory of the same
/// name: the link is one of the system's own names, and it points at a
/// directory whose own name is the name of the link.
pub(super) fn is_alias(dir: &impl std::os::fd::AsFd, name: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let text = name.to_string_lossy().to_string();
    if !SYSTEM_ALIASES.contains(&text.as_str()) {
        return false;
    }
    let Ok(target) = rustix::fs::readlinkat(dir, name, Vec::new()) else {
        return false;
    };
    let target = Path::new(std::ffi::OsStr::from_bytes(target.as_bytes()));
    match target.file_name() {
        Some(last) => last == name,
        None => false,
    }
}

/// Whether an entry of a tree is a mount point: it is on another device,
/// so the tree below it belongs to somewhere else.
pub(super) fn is_mount_point(parent: u64, entry: u64) -> bool {
    parent != entry
}

/// The device a stat reports: the platforms type it differently, and
/// the identity of a file holds it as a number.
pub(super) fn device(meta: &rustix::fs::Stat) -> u64 {
    #[cfg(target_os = "macos")]
    let device = u64::from(meta.st_dev.unsigned_abs());
    #[cfg(not(target_os = "macos"))]
    let device = meta.st_dev;
    device
}

/// Whether a path is a symlink.
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// The error a removal of a nonempty directory gives: the message the run
/// itself produces, so check mode and the run agree.
pub(super) fn directory_not_empty() -> String {
    std::io::Error::from_raw_os_error(rustix::io::Errno::NOTEMPTY.raw_os_error()).to_string()
}
