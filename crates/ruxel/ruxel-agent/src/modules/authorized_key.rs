#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

use super::{ExecContext, params_object, str_param};
use publish::{publish_checked, read_authorized_keys_with_identity};

use rustix::fs::{
    AtFlags, FlockOperation, Mode, OFlags, fchmod, fchown, flock, fstat, mkdirat, open, openat,
    statat, unlinkat,
};
use serde_json::{Value, json};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

mod publish;
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
    let want_material = (want_material.0.into_bytes(), want_material.1.into_bytes());
    let home = File::from(open(home, dir_flags(), Mode::empty()).map_err(|e| e.to_string())?);
    // Lock before inspecting .ssh so concurrent first-time creation is serialized too.
    flock(&home, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
    let ssh = open_ssh(&home, uid, gid, !check_mode)?;
    if let Some(dir) = &ssh {
        flock(dir, FlockOperation::LockExclusive).map_err(|e| e.to_string())?;
        ensure_attached(&home, dir)?;
    }
    let (current, identity) = match &ssh {
        Some(dir) => read_authorized_keys_with_identity(dir)?,
        None => (Vec::new(), None),
    };
    let present = split_lines(&current)
        .into_iter()
        .filter_map(key_material_bytes)
        .any(|material| material == want_material);
    let changed = !present;
    if changed && !check_mode {
        let mut next = current;
        if !next.is_empty() && !next.ends_with(b"\n") {
            next.push(b'\n');
        }
        next.extend_from_slice(key.trim_end().as_bytes());
        next.push(b'\n');
        ensure_attached(&home, ssh.as_ref().ok_or("authorized_key: missing .ssh")?)?;
        publish_checked(
            ssh.as_ref().ok_or("authorized_key: missing .ssh")?,
            &next,
            uid,
            gid,
            identity,
        )?;
    }

    if let Some(dir) = &ssh {
        ensure_attached(&home, dir)?;
    }
    Ok(changed)
}

#[cfg(test)]
fn publish(dir: &File, content: &[u8], uid: u32, gid: u32) -> Result<(), String> {
    publish::publish(dir, content, uid, gid)
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

#[cfg(test)]
fn read_authorized_keys(dir: &File) -> Result<Vec<u8>, String> {
    read_authorized_keys_with_identity(dir).map(|(content, _)| content)
}

/// The lines of the file, each with the terminator it had: a retained
/// line is written back byte for byte, and a file without a final newline
/// stays without one.
fn split_lines(content: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in content.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&content[start..=index]);
            start = index + 1;
        }
    }
    if start < content.len() {
        lines.push(&content[start..]);
    }
    lines
}

/// The key type and the material of a line, read as bytes: a comment may
/// hold bytes that are not text, and sshd ignores the comment anyway.
fn key_material_bytes(line: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let text = match line.iter().position(|byte| !byte.is_ascii_whitespace()) {
        Some(start) => &line[start..],
        None => &[],
    };
    if text.is_empty() || text.starts_with(b"#") {
        // sshd ignores a blank line and a comment: neither carries a key.
        return None;
    }
    let fields = key_fields_bytes(text);
    for (index, field) in fields.iter().enumerate() {
        if !is_key_type_bytes(field) {
            continue;
        }
        let Some(material) = fields.get(index + 1) else {
            continue;
        };
        return Some((field.clone(), material.clone()));
    }
    None
}

/// The fields of a line of `authorized_keys`, as bytes: a quoted value
/// stays in one field, and a value that is not text does not end the line.
fn key_fields_bytes(line: &[u8]) -> Vec<Vec<u8>> {
    let mut fields = Vec::new();
    let mut field = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut started = false;
    for byte in line {
        if escaped {
            field.push(*byte);
            escaped = false;
            started = true;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => {
                quoted = !quoted;
                started = true;
            }
            byte if byte.is_ascii_whitespace() && !quoted => {
                if started {
                    fields.push(std::mem::take(&mut field));
                    started = false;
                }
            }
            byte => {
                field.push(*byte);
                started = true;
            }
        }
    }
    if started {
        fields.push(field);
    }
    fields
}

/// Whether a field names a key type.
fn is_key_type_bytes(field: &[u8]) -> bool {
    const TYPES: [&[u8]; 4] = [b"ssh-", b"ecdsa-", b"sk-", b"xmss-"];
    TYPES.iter().any(|prefix| field.starts_with(prefix))
}

/// The key type and the material of a line of `authorized_keys`.
///
/// A line may carry options before the key, and an option's value may be
/// quoted and hold spaces (`command="echo hi"`), so the line is split into
/// fields that keep a quoted value whole. The key type is the first field
/// that names one, and the material is the field after it.
fn key_material(line: &str) -> Option<(String, String)> {
    let text = line.trim_start();
    if text.is_empty() || text.starts_with('#') {
        // sshd ignores a blank line and a comment: neither carries a key.
        return None;
    }
    let fields = key_fields(text);
    for (index, field) in fields.iter().enumerate() {
        if !is_key_type(field) {
            continue;
        }
        let Some(material) = fields.get(index + 1) else {
            continue;
        };
        return Some((field.clone(), material.clone()));
    }
    None
}

/// The fields of a line of `authorized_keys`: a quoted value stays in one
/// field and loses its quotes.
fn key_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut started = false;
    for character in line.chars() {
        if escaped {
            field.push(character);
            escaped = false;
            started = true;
            continue;
        }
        match character {
            '\\' if quoted => escaped = true,
            '"' => {
                quoted = !quoted;
                started = true;
            }
            character if character.is_whitespace() && !quoted => {
                if started {
                    fields.push(std::mem::take(&mut field));
                    started = false;
                }
            }
            character => {
                field.push(character);
                started = true;
            }
        }
    }
    if started {
        fields.push(field);
    }
    fields
}

/// Whether a field names a key type.
fn is_key_type(field: &str) -> bool {
    // The names of the key types the format lists. `rsa-sha2-256` and
    // `rsa-sha2-512` are signature algorithms, not key types: a key of
    // that name is not a key at all.
    field.starts_with("ssh-")
        || field.starts_with("ecdsa-")
        || field.starts_with("sk-")
        || field.starts_with("xmss")
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
mod tests;
