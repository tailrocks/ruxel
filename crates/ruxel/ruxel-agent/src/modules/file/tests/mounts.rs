use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-mounts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn mount(root: &str, point: &str, identity: (u64, u64)) -> super::super::mounts::Mount {
    super::super::mounts::Mount {
        root: PathBuf::from(root),
        point: PathBuf::from(point),
        identity,
    }
}

fn etc_identity() -> (u64, u64) {
    let meta = std::fs::metadata("/etc").unwrap();
    (meta.dev(), meta.ino())
}

#[test]
fn a_descendant_reached_through_a_bound_root_is_refused() {
    // The protected tree is bind-mounted under another name, and the
    // target is more than one level below its root: the directory that
    // holds the target is the root's own child, and the path says
    // nothing about where the removal lands.
    let mounts = [mount("/etc", "/scratch/config", etc_identity())];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(
            Path::new("/scratch/config/ssh/sshd_config"),
            &roots,
            &mounts,
        ),
        Some("/etc".to_string())
    );
}

#[test]
fn a_path_inside_the_root_it_names_is_not_a_crossing() {
    // The mount is at the root's own name: the path was inside the
    // root as it was written, and the mount did not take it there.
    let mounts = [mount("/etc", "/etc", etc_identity())];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(Path::new("/etc/ssh/sshd_config"), &roots, &mounts,),
        None
    );
}

#[test]
fn a_descendant_reached_through_a_bound_filesystem_root_is_refused() {
    // The filesystem root is bind-mounted under another name, and the
    // target lies outside the other listed roots: the removal lands in
    // the tree of the root, and the path says nothing about it.
    let meta = std::fs::metadata("/").unwrap();
    let mounts = [mount("/", "/scratch/root", (meta.dev(), meta.ino()))];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(Path::new("/scratch/root/opt/data"), &roots, &mounts,),
        Some("/".to_string())
    );
}

#[test]
fn the_visible_mount_is_the_one_stacked_on_top() {
    // Two trees are mounted at one point: the later record is the tree
    // the removal would enter, so the hidden tree must not excuse it.
    let mounts = [
        mount("/srv/benign", "/scratch/config", (7, 11)),
        mount("/etc", "/scratch/config", etc_identity()),
    ];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(
            Path::new("/scratch/config/ssh/sshd_config"),
            &roots,
            &mounts,
        ),
        Some("/etc".to_string())
    );
}

#[test]
fn a_mount_hidden_below_another_is_not_the_visible_tree() {
    // The bind of the root is stacked below a benign mount: the visible
    // tree carries no root, and the removal is not refused for the tree
    // it would enter.
    let mounts = [
        mount("/etc", "/scratch/config", etc_identity()),
        mount("/srv/benign", "/scratch/config", (7, 11)),
    ];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(
            Path::new("/scratch/config/ssh/sshd_config"),
            &roots,
            &mounts,
        ),
        None
    );
}

#[test]
fn a_mount_of_another_filesystem_is_not_a_crossing() {
    // The mount holds a tree of a filesystem of its own: the path
    // lands there, and not in the root the name resembles.
    let mounts = [mount("/", "/scratch", (u64::MAX, 1))];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(Path::new("/scratch/etc/passwd"), &roots, &mounts),
        None
    );
}

#[test]
fn the_path_of_an_entry_keeps_the_bytes_of_its_name() {
    // A name is not necessarily text: the path of the entry keeps the
    // bytes of the name, or the mount it names is missed and the tree
    // below it is taken away.
    let name = std::ffi::CString::new(vec![b'm', b'o', b'u', b'n', b't', 0xff]).unwrap();
    let path = super::super::mounts::child_path(Path::new("/scratch"), &name);
    assert_eq!(path.as_os_str().as_bytes(), b"/scratch/mount\xff");
}

#[cfg(target_os = "linux")]
#[test]
fn the_preflight_finds_a_mount_point_whose_name_is_not_text() {
    use std::os::unix::ffi::OsStringExt;
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-raw-mount-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // The mount point holds a name that is not text: the path of the
    // entry keeps the bytes of the name, or the mount is missed and the
    // tree below it is taken away.
    let name = std::ffi::OsString::from_vec(vec![b'm', b'o', b'u', b'n', b't', 0xff]);
    let point = base.join(&name);
    std::fs::create_dir(&point).unwrap();
    // The mount point is the path the kernel lists, with the identity
    // of the directory it holds.
    let meta = std::fs::metadata(&point).unwrap();
    let mounts = vec![super::super::mounts::Mount {
        root: point.clone(),
        point,
        identity: (meta.dev(), meta.ino()),
    }];
    let dir = OwnedFd::from(std::fs::File::open(&base).unwrap());
    let error = super::super::mounts::preflight(&dir, &base, &mounts).unwrap_err();
    assert!(error.contains("mount point"), "{error}");
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn the_preflight_finds_a_mount_point_reached_through_an_alias() {
    // The mount point is named elsewhere: the tree is reached through an
    // alias (`tool` for `/usr/bin/tool`), so the spelling of the path
    // misses it and only the identity the entry carries tells.
    let base = scratch("alias-mount");
    let tool = base.join("tool");
    std::fs::create_dir(&tool).unwrap();
    let meta = std::fs::metadata(&tool).unwrap();
    let mounts = vec![mount("/usr/bin", "/usr/bin/tool", (meta.dev(), meta.ino()))];
    let dir = OwnedFd::from(std::fs::File::open(&base).unwrap());
    let error = super::super::mounts::preflight(&dir, &base, &mounts).unwrap_err();
    assert!(error.contains("mount point"), "{error}");
    // The tree is untouched.
    assert!(tool.exists());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_listing_that_cannot_be_read_is_not_an_empty_listing() {
    // A host whose mount table cannot be read has no mounts the
    // removal may trust: the listing is a refusal, and the removal
    // that reads it refuses.
    let error =
        super::super::mounts::mounts_from(Err("the mount table cannot be read".to_string()))
            .expect_err("the listing must fail");
    assert!(error.contains("mount points of the host"), "{error}");
}

#[test]
fn an_entry_that_carries_the_identity_of_a_mount_point_is_a_mount_point() {
    // A bind mount keeps the identity of the tree it is mounted from:
    // the device alone does not tell it apart.
    assert!(super::super::mounts::is_mount_identity(&[(7, 11)], (7, 11)));
    assert!(!super::super::mounts::is_mount_identity(
        &[(7, 11)],
        (7, 12)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn the_mount_points_of_the_host_carry_the_identity_of_their_directories() {
    let identities = super::super::mounts::mount_identities().unwrap();
    assert!(
        !identities.is_empty(),
        "the host holds the mount points it lists"
    );
    for mount in super::super::mounts::mounts().unwrap() {
        let meta = std::fs::metadata(&mount.point).unwrap();
        assert!(identities.contains(&(meta.dev(), meta.ino())), "{mount:?}");
    }
}

#[test]
fn a_bind_of_a_filesystem_relative_root_is_still_the_root() {
    // `/` comes from the `/@` subvolume: the bind of `/` reports the
    // filesystem-relative root `/@`, and no namespace root starts with
    // it. The path still lands in the tree of `/etc`, through the
    // spelling the filesystem mount itself gives it.
    let meta = std::fs::metadata("/").unwrap();
    let identity = (meta.dev(), meta.ino());
    let mounts = [
        mount("/@", "/", identity),
        mount("/@", "/scratch/root", identity),
    ];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(
            Path::new("/scratch/root/etc/ssh/sshd_config"),
            &roots,
            &mounts,
        ),
        Some("/etc".to_string())
    );
}

#[test]
fn a_filesystem_relative_root_outside_every_root_is_not_a_crossing() {
    // The bind of `/` reports `/@`, but the path lands in a tree of its
    // own: no spelling of it is inside a root.
    let meta = std::fs::metadata("/").unwrap();
    let identity = (meta.dev(), meta.ino());
    let mounts = [
        mount("/@", "/", identity),
        mount("/@", "/scratch/root", identity),
    ];
    let roots = super::super::floor::SYSTEM_ROOTS;
    assert_eq!(
        super::super::mounts::bound_root_in(Path::new("/scratch/root/srv/data"), &roots, &mounts,),
        None
    );
}

#[test]
fn the_preflight_walks_a_plain_tree_without_a_table() {
    // The platform writes no mount table to read: the walk still runs
    // over the whole tree and finds no mount point in ordinary entries.
    let base = scratch("no-table");
    std::fs::create_dir(base.join("child")).unwrap();
    std::fs::write(base.join("child/tool"), b"content").unwrap();
    std::os::unix::fs::symlink("child/tool", base.join("link")).unwrap();
    let dir = OwnedFd::from(std::fs::File::open(&base).unwrap());
    super::super::mounts::preflight(&dir, &base, &[]).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
}
