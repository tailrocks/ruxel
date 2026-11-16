use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

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
fn a_mount_point_that_holds_several_mounts_shows_the_topmost_one() {
    // A benign tree is mounted at the point, and a tree of the root is
    // mounted over it: the tree a walk enters is the visible one, and
    // the record of the hidden mount must not name where the path
    // lands.
    let listing = b"31 20 8:1 /benign /scratch/config rw - ext4 /dev/sda1 rw\n\
32 20 8:1 /etc /scratch/config rw - ext4 /dev/sda1 rw\n";
    let mounts = super::super::mounts::mounts_from(Ok(listing.to_vec())).unwrap();
    assert_eq!(mounts.len(), 1, "{mounts:?}");
    assert_eq!(mounts[0].root, PathBuf::from("/etc"));
    assert_eq!(mounts[0].point, PathBuf::from("/scratch/config"));
}

#[test]
fn the_visible_mount_of_a_stacked_point_names_where_the_path_lands() {
    // Mounts stacked at one point have the same depth: the one mounted
    // over the others is the visible one, and it is the one that names
    // where the path lands. Reasoning from the hidden mount would
    // permit the removal of a file of the root.
    let mounts = [
        mount("/benign", "/scratch/config", etc_identity()),
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
    let mounts = vec![point];
    let dir = std::os::fd::OwnedFd::from(std::fs::File::open(&base).unwrap());
    let error = super::super::mounts::preflight(&dir, &base, &mounts).unwrap_err();
    assert!(error.contains("mount point"), "{error}");
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_listing_that_cannot_be_read_is_not_an_empty_listing() {
    // A host whose mount table cannot be read has no mounts the
    // removal may trust: the listing is a refusal, and the removal
    // that reads it refuses.
    let error =
        super::super::mounts::mounts_from(Err("the mount table cannot be read".to_string()))
            .unwrap_err();
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
    assert!(!identities.is_empty());
    for mount in super::super::mounts::mounts().unwrap() {
        let meta = std::fs::metadata(&mount.point).unwrap();
        assert!(identities.contains(&(meta.dev(), meta.ino())), "{mount:?}");
    }
}
