#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, params_object, str_param};
use rustix::fs::{
    AtFlags, FlockOperation, Mode, OFlags, fchmod, fchown, flock, fstat, mkdirat, open, openat,
    renameat, statat, unlinkat,
};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

mod remove;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let user = str_param(obj, "user").ok_or("authorized_key: user required")?;
    let key = str_param(obj, "key").ok_or("authorized_key: key required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" && state != "absent" {
        return Err(format!(
            "authorized_key: state {state:?} outside the closed surface"
        ));
    }

    let home = home_of(user)?;
    let (uid, gid) = ids_of(user)?;
    let changed = if state == "absent" {
        remove::remove_key(Path::new(&home), uid, gid, key, ctx.check_mode)?
    } else {
        update(Path::new(&home), uid, gid, key, ctx.check_mode)?
    };
    Ok(json!({"changed": changed, "failed": false, "user": user}))
}

fn update(home: &Path, uid: u32, gid: u32, key: &str, check_mode: bool) -> Result<bool, String> {
    let want_material = key_material(key).ok_or("authorized_key: malformed key")?;
    let home = File::from(open(home, dir_flags(), Mode::empty()).map_err(|e| e.to_string())?);
    // Lock before inspecting .ssh so concurrent first-time creation is serialized too.
    flock(&home, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
    let ssh = open_ssh(&home, uid, gid, !check_mode)?;
    if let Some(dir) = &ssh {
        flock(dir, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
        ensure_attached(&home, dir)?;
    }
    let current = match &ssh {
        Some(dir) => read_authorized_keys(dir)?,
        None => String::new(),
    };
    let present = current
        .lines()
        .filter_map(key_material)
        .any(|m| m == want_material);
    let changed = !present;
    if changed && !check_mode {
        let mut next = current;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(key.trim_end());
        next.push('\n');
        ensure_attached(&home, ssh.as_ref().ok_or("authorized_key: missing .ssh")?)?;
        publish(
            ssh.as_ref().ok_or("authorized_key: missing .ssh")?,
            next.as_bytes(),
            uid,
            gid,
        )?;
    }

    if let Some(dir) = &ssh {
        ensure_attached(&home, dir)?;
    }
    Ok(changed)
}

fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn open_ssh(home: &File, uid: u32, gid: u32, create: bool) -> Result<Option<File>, String> {
    let mut created = false;
    let fd = match openat(home, ".ssh", dir_flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !create => return Ok(None),
        Err(rustix::io::Errno::NOENT) => {
            match mkdirat(home, ".ssh", Mode::from_raw_mode(0o700)) {
                Ok(()) => created = true,
                Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(e.to_string()),
            }
            openat(home, ".ssh", dir_flags(), Mode::empty()).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(format!("authorized_key: cannot open .ssh: {e}")),
    };
    let dir = File::from(fd);
    let meta = dir.metadata().map_err(|e| e.to_string())?;
    if created {
        // A replacement directory must not be chowned. The newly created directory
        // is owned by the caller; verify it using a fresh file's owner.
        let probe = openat(
            &dir,
            ".ruxel-owner-probe",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| e.to_string())?;
        let probe = File::from(probe);
        let owner = probe.metadata().map_err(|e| e.to_string())?.uid();
        unlinkat(&dir, ".ruxel-owner-probe", AtFlags::empty()).map_err(|e| e.to_string())?;
        if meta.uid() != owner {
            return Err("authorized_key: .ssh replaced during creation".into());
        }
        fchown(
            &dir,
            Some(rustix::fs::Uid::from_raw(uid)),
            Some(rustix::fs::Gid::from_raw(gid)),
        )
        .map_err(|e| e.to_string())?;
        fchmod(&dir, Mode::from_raw_mode(0o700)).map_err(|e| e.to_string())?;
    } else if meta.uid() != uid {
        return Err("authorized_key: .ssh is not owned by target user".into());
    }
    Ok(Some(dir))
}

fn ensure_attached(home: &File, dir: &File) -> Result<(), String> {
    let entry = statat(home, ".ssh", AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| format!("authorized_key: .ssh changed: {e}"))?;
    let opened = fstat(dir).map_err(|e| e.to_string())?;
    if entry.st_dev != opened.st_dev || entry.st_ino != opened.st_ino {
        return Err("authorized_key: .ssh changed during operation".into());
    }
    Ok(())
}

fn read_authorized_keys(dir: &File) -> Result<String, String> {
    let fd = match openat(
        dir,
        "authorized_keys",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(String::new()),
        Err(e) => return Err(format!("authorized_key: cannot open authorized_keys: {e}")),
    };
    let mut file = File::from(fd);
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("authorized_key: authorized_keys is not a file".into());
    }
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("authorized_key: cannot read authorized_keys: {e}"))?;
    Ok(content)
}

static NEXT: AtomicU64 = AtomicU64::new(0);

fn publish(dir: &File, content: &[u8], uid: u32, gid: u32) -> Result<(), String> {
    // A private directory protects the staging name from substitution by the
    // owner of .ssh while the descriptor pins the destination directory.
    for _ in 0..128 {
        let name = format!(
            ".ruxel-key-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        match mkdirat(dir, name.as_str(), Mode::from_raw_mode(0o700)) {
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(e.to_string()),
            Ok(()) => {}
        }
        let result = (|| {
            let stage = File::from(
                openat(dir, name.as_str(), dir_flags(), Mode::empty())
                    .map_err(|e| e.to_string())?,
            );
            let result = (|| {
                let mut file = File::from(
                    openat(
                        &stage,
                        "key",
                        OFlags::WRONLY
                            | OFlags::CREATE
                            | OFlags::EXCL
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC,
                        Mode::from_raw_mode(0o600),
                    )
                    .map_err(|e| e.to_string())?,
                );
                let sm = stage.metadata().map_err(|e| e.to_string())?;
                let fm = file.metadata().map_err(|e| e.to_string())?;
                if sm.uid() != fm.uid() || sm.mode() & 0o777 != 0o700 {
                    return Err("authorized_key: unsafe staging directory".into());
                }
                fchown(
                    &file,
                    Some(rustix::fs::Uid::from_raw(uid)),
                    Some(rustix::fs::Gid::from_raw(gid)),
                )
                .map_err(|e| e.to_string())?;
                fchmod(&file, Mode::from_raw_mode(0o600)).map_err(|e| e.to_string())?;
                file.write_all(content).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                renameat(&stage, "key", dir, "authorized_keys").map_err(|e| e.to_string())?;
                Ok(())
            })();
            let _ = unlinkat(&stage, "key", AtFlags::empty());
            result
        })();
        let _ = unlinkat(dir, name.as_str(), AtFlags::REMOVEDIR);
        return result;
    }
    Err("authorized_key: cannot reserve staging directory".into())
}

fn key_material(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let (ktype, material) = if first.starts_with("ssh-") || first.starts_with("ecdsa-") {
        (first, parts.next()?)
    } else {
        let t = parts.next()?;
        (t, parts.next()?)
    };
    Some((ktype.to_string(), material.to_string()))
}

fn home_of(user: &str) -> Result<String, String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() == Some(&user) && f.len() >= 6 {
            return Ok(f[5].to_string());
        }
    }
    Err(format!("authorized_key: user {user:?} not found"))
}

fn ids_of(user: &str) -> Result<(u32, u32), String> {
    let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.first() == Some(&user) && f.len() >= 4 {
            return Ok((
                f[2].parse().map_err(|_| "bad uid")?,
                f[3].parse().map_err(|_| "bad gid")?,
            ));
        }
    }
    Err(format!("user {user:?} not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn material_ignores_comment() {
        assert_eq!(
            key_material("ssh-ed25519 AAAAC3Nza host-a"),
            key_material("ssh-ed25519 AAAAC3Nza other")
        );
        assert_eq!(
            key_material("from=\"10.0.0.1\" ssh-ed25519 AAAAC3Nza comment").unwrap(),
            ("ssh-ed25519".into(), "AAAAC3Nza".into())
        );
    }

    #[test]
    fn refuses_symlinks_and_invalid_content() {
        let root = scratch("refuse");
        let meta = std::fs::metadata(&root).unwrap();
        let home = File::open(&root).unwrap();
        let victim = root.join("victim");
        std::fs::write(&victim, "untouched").unwrap();
        symlink(&victim, root.join(".ssh")).unwrap();
        assert!(open_ssh(&home, meta.uid(), meta.gid(), true).is_err());
        std::fs::remove_file(root.join(".ssh")).unwrap();
        std::fs::create_dir(root.join(".ssh")).unwrap();
        let ssh = open_ssh(&home, meta.uid(), meta.gid(), false)
            .unwrap()
            .unwrap();
        symlink(&victim, root.join(".ssh/authorized_keys")).unwrap();
        assert!(read_authorized_keys(&ssh).is_err());
        std::fs::remove_file(root.join(".ssh/authorized_keys")).unwrap();
        std::fs::write(root.join(".ssh/authorized_keys"), [0xff]).unwrap();
        assert!(read_authorized_keys(&ssh).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_swap_cannot_redirect_write() {
        let root = scratch("swap");
        let meta = std::fs::metadata(&root).unwrap();
        let home = File::open(&root).unwrap();
        let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
            .unwrap()
            .unwrap();
        std::fs::rename(root.join(".ssh"), root.join("original")).unwrap();
        let victim = root.join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("authorized_keys"), "untouched").unwrap();
        symlink(&victim, root.join(".ssh")).unwrap();
        assert!(ensure_attached(&home, &ssh).is_err());
        publish(&ssh, b"new key\n", meta.uid(), meta.gid()).unwrap();
        assert_eq!(
            std::fs::read_to_string(victim.join("authorized_keys")).unwrap(),
            "untouched"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("original/authorized_keys")).unwrap(),
            "new key\n"
        );
        assert_eq!(
            std::fs::metadata(root.join("original/authorized_keys"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_swap_cannot_write_through_symlink() {
        let root = scratch("file-swap");
        let meta = std::fs::metadata(&root).unwrap();
        let home = File::open(&root).unwrap();
        let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
            .unwrap()
            .unwrap();
        assert_eq!(read_authorized_keys(&ssh).unwrap(), "");
        let victim = root.join("victim");
        std::fs::write(&victim, "untouched").unwrap();
        symlink(&victim, root.join(".ssh/authorized_keys")).unwrap();
        publish(&ssh, b"key\n", meta.uid(), meta.gid()).unwrap();
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "untouched");
        assert_eq!(
            std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap(),
            "key\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_updates_preserve_both_keys() {
        let root = scratch("concurrent");

        let meta = std::fs::metadata(&root).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = ["ssh-ed25519 FIRST first", "ssh-ed25519 SECOND second"]
            .into_iter()
            .map(|key| {
                let root = root.clone();
                let barrier = barrier.clone();
                let meta = meta.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    assert!(update(&root, meta.uid(), meta.gid(), key, false).unwrap());
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let keys = std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap();
        assert!(
            keys.contains("ssh-ed25519 FIRST first\n")
                && keys.contains("ssh-ed25519 SECOND second\n")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    // The per-process counter keeps two tests that pick the same label from
    // sharing one directory: `ruxel-file-swap-<pid>` is also a `file` test's
    // scratch base, and a shared path made each test's `remove_dir_all` race
    // the other's writes.
    static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

    fn scratch(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ruxel-{label}-{}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        path
    }
}
