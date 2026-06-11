//! `authorized_key` (SEMANTICS §6): exact key present in
//! ~user/.ssh/authorized_keys. Matching is comment-insensitive on the key
//! material (type + base64), Ansible's rule.

use super::{ExecContext, params_object, str_param};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

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
    let current = std::fs::read_to_string(&auth_file).unwrap_or_default();

    let want_material = key_material(key).ok_or("authorized_key: malformed key")?;
    let present = current
        .lines()
        .filter_map(key_material)
        .any(|m| m == want_material);

    let changed = !present;
    if changed && !ctx.check_mode {
        let (uid, gid) = ids_of(user)?;
        if !ssh_dir.exists() {
            std::fs::create_dir_all(&ssh_dir).map_err(|e| e.to_string())?;
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
        std::fs::write(&auth_file, next).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&auth_file, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        std::os::unix::fs::chown(&auth_file, Some(uid), Some(gid)).map_err(|e| e.to_string())?;
    }

    Ok(json!({"changed": changed, "failed": false, "user": user}))
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
    use super::key_material;

    #[test]
    fn material_ignores_comment() {
        let a = key_material("ssh-ed25519 AAAAC3Nza host-a").unwrap();
        let b = key_material("ssh-ed25519 AAAAC3Nza completely-different-comment").unwrap();
        assert_eq!(a, b);
    }
}
