//! `authorized_key` (SEMANTICS §6): exact key present in
//! ~user/.ssh/authorized_keys. Matching is comment-insensitive on the key
//! material (type + base64), Ansible's rule.

use super::{ExecContext, params_object, str_param, write_atomic_with};
use serde_json::{Value, json};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let user = str_param(obj, "user").ok_or("authorized_key: user required")?;
    let key = str_param(obj, "key").ok_or("authorized_key: key required")?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "authorized_key: state {state:?} outside the closed surface"
        ));
    }

    let home = home_of(user)?;
    let ssh_dir = PathBuf::from(&home).join(".ssh");
    let auth_file = ssh_dir.join("authorized_keys");
    let (uid, gid) = ids_of(user)?;
    validate_paths(&ssh_dir, &auth_file, uid)?;
    let current = std::fs::read_to_string(&auth_file).unwrap_or_default();

    let want_material = key_material(key).ok_or("authorized_key: malformed key")?;
    let present = current
        .lines()
        .filter_map(key_material)
        .any(|m| m == want_material);

    let changed = !present;
    if changed && !ctx.check_mode {
        if !ssh_dir.exists() {
            std::fs::create_dir(&ssh_dir).map_err(|e| e.to_string())?;
            std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
            std::os::unix::fs::chown(&ssh_dir, Some(uid), Some(gid)).map_err(|e| e.to_string())?;
        }
        let mut next = current.clone();
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(key.trim_end());
        next.push('\n');
        write_atomic_with(&auth_file, next.as_bytes(), Some(0o600), Some((uid, gid)))?;
    }

    Ok(json!({"changed": changed, "failed": false, "user": user}))
}

fn validate_paths(ssh_dir: &Path, auth_file: &Path, uid: u32) -> Result<(), String> {
    match std::fs::symlink_metadata(ssh_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("authorized_key: refusing symlink .ssh directory".into());
        }
        Ok(meta) if !meta.is_dir() => return Err("authorized_key: .ssh is not a directory".into()),
        Ok(meta) if meta.uid() != uid => {
            return Err("authorized_key: .ssh is not owned by target user".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    }
    match std::fs::symlink_metadata(auth_file) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err("authorized_key: refusing symlink authorized_keys".into())
        }
        Ok(meta) if !meta.is_file() => Err("authorized_key: authorized_keys is not a file".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// (key type, base64 material) — comments and options ignored.
fn key_material(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let (ktype, material) = if first.starts_with("ssh-") || first.starts_with("ecdsa-") {
        (first, parts.next()?)
    } else {
        // options field precedes the type
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
    use super::{key_material, validate_paths};
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    #[test]
    fn material_ignores_comment() {
        let a = key_material("ssh-ed25519 AAAAC3Nza host-a").unwrap();
        let b = key_material("ssh-ed25519 AAAAC3Nza completely-different-comment").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn refuses_authorized_keys_symlink_without_touching_target() {
        let root = scratch("auth-symlink");
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        let uid = std::fs::metadata(&ssh).unwrap().uid();
        let target = root.join("victim");
        std::fs::write(&target, "untouched").unwrap();
        symlink(&target, ssh.join("authorized_keys")).unwrap();
        assert!(validate_paths(&ssh, &ssh.join("authorized_keys"), uid).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_authorized_key_has_private_mode() {
        let root = scratch("auth-atomic");
        let ssh = root.join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        let meta = std::fs::metadata(&ssh).unwrap();
        super::write_atomic_with(
            &ssh.join("authorized_keys"),
            b"key\n",
            Some(0o600),
            Some((meta.uid(), meta.gid())),
        )
        .unwrap();
        let written = std::fs::metadata(ssh.join("authorized_keys")).unwrap();
        assert_eq!(written.permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("ruxel-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        path
    }
}
