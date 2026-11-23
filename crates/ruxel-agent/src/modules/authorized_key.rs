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
    // The key type is the first field, or the second when the first is
    // the options field: sshd rejects a line whose leading field is
    // neither, so a key-looking pair further along the line is not an
    // active key.
    let key_at = if fields.first().is_some_and(|field| is_key_type_bytes(field)) {
        0
    } else if fields.len() > 1 && is_options_bytes(first_field_span(text)) {
        1
    } else {
        return None;
    };
    let material = fields.get(key_at + 1)?;
    Some((fields[key_at].clone(), material.clone()))
}

/// The first field of the line, with its quotes intact: whitespace
/// inside quotes does not end it. The options check reads this span,
/// because a quoted comma is part of a value, not a separator.
fn first_field_span(line: &[u8]) -> &[u8] {
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in line.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            byte if byte.is_ascii_whitespace() && !quoted => return &line[..index],
            _ => {}
        }
    }
    line
}

/// Whether the field is a valid `authorized_keys` options field: a
/// comma-separated list of the options sshd accepts, where a comma
/// inside quotes stays in the value. A leading field that is neither a
/// key type nor valid options makes sshd reject the line, so the line
/// carries no key.
fn is_options_bytes(field: &[u8]) -> bool {
    let mut any = false;
    // The options sshd reads once: a second `command`, `principals` or
    // `from` is a line sshd rejects, not a line that carries a key.
    let mut singletons: Vec<&[u8]> = Vec::new();
    for option in split_options_bytes(field) {
        if !is_option_bytes(option) {
            return false;
        }
        if let Some(name) = singleton_name(option) {
            if singletons
                .iter()
                .any(|seen| seen.eq_ignore_ascii_case(name))
            {
                return false;
            }
            singletons.push(name);
        }
        any = true;
    }
    any
}

/// The singleton name of an option, when the option names one: `command`,
/// `principals` and `from` hold once, and a second one rejects the line.
fn singleton_name(option: &[u8]) -> Option<&[u8]> {
    let (name, _) = match option.iter().position(|byte| *byte == b'=') {
        Some(at) => (&option[..at], Some(&option[at + 1..])),
        None => (option, None),
    };
    [
        b"command".as_slice(),
        b"principals".as_slice(),
        b"from".as_slice(),
    ]
    .into_iter()
    .find(|known| name.eq_ignore_ascii_case(known))
}

/// The options of the field, split on the commas that are not quoted.
fn split_options_bytes(field: &[u8]) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in field.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                parts.push(&field[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&field[start..]);
    parts
}

/// Whether the option names one sshd accepts, with the value the name
/// requires: a flag stands alone, and a valued option holds a quoted
/// value of the grammar the name requires. Keywords match the way
/// sshd matches them, without regard to case.
fn is_option_bytes(option: &[u8]) -> bool {
    let (name, value) = match option.iter().position(|byte| *byte == b'=') {
        Some(at) => (&option[..at], Some(&option[at + 1..])),
        None => (option, None),
    };
    if is_flag(name) {
        return value.is_none();
    }
    let Some(inner) = value.and_then(dequoted) else {
        return false;
    };
    // An empty value reaches the grammar of its own option: `command=""`
    // is the empty command sshd reads, while an empty `expiry-time` or
    // `environment` is a time or an assignment no grammar accepts. A
    // blanket rejection here would read `command=""` as a line sshd
    // rejects, and the key as one that is not there.
    valid_valued_option(name, &inner)
}

/// Whether the name is a flag sshd accepts: `restrict` and
/// `cert-authority` stand alone, and every other flag holds either way,
/// with or without `no-`.
fn is_flag(name: &[u8]) -> bool {
    const PLAIN: [&[u8]; 2] = [b"restrict", b"cert-authority"];
    const NEGATABLE: [&[u8]; 7] = [
        b"port-forwarding",
        b"agent-forwarding",
        b"x11-forwarding",
        b"touch-required",
        b"verify-required",
        b"pty",
        b"user-rc",
    ];
    if PLAIN.iter().any(|flag| name.eq_ignore_ascii_case(flag)) {
        return true;
    }
    if NEGATABLE.iter().any(|flag| name.eq_ignore_ascii_case(flag)) {
        return true;
    }
    let unnegated = (name.len() > 3 && name[..3].eq_ignore_ascii_case(b"no-")).then(|| &name[3..]);
    unnegated.is_some_and(|name| NEGATABLE.iter().any(|flag| name.eq_ignore_ascii_case(flag)))
}

/// The quoted value with its quotes read the way sshd reads them: the
/// value must open and close with a quote, `\"` holds a quote, and
/// nothing follows the closing quote. A value sshd would stop reading
/// at is not the value written here.
fn dequoted(value: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = value.iter();
    if bytes.next() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut rest = bytes.as_slice();
    loop {
        let (first, tail) = rest.split_first()?;
        rest = tail;
        if *first == b'"' {
            return rest.is_empty().then_some(out);
        }
        if *first == b'\\' && rest.first() == Some(&b'"') {
            out.push(b'"');
            rest = &rest[1..];
        } else {
            out.push(*first);
        }
    }
}

/// Whether the quoted value holds the grammar the valued option
/// requires: sshd reads `command`, `from` and `principals` as far as
/// the quotes reach, and checks the grammar of every other value.
fn valid_valued_option(name: &[u8], value: &[u8]) -> bool {
    if [
        b"command".as_slice(),
        b"from".as_slice(),
        b"principals".as_slice(),
    ]
    .iter()
    .any(|known| name.eq_ignore_ascii_case(known))
    {
        return true;
    }
    if name.eq_ignore_ascii_case(b"expiry-time") {
        return valid_expiry_time(value);
    }
    if name.eq_ignore_ascii_case(b"environment") {
        return valid_environment(value);
    }
    if name.eq_ignore_ascii_case(b"permitopen") {
        return valid_permit(value, false);
    }
    if name.eq_ignore_ascii_case(b"permitlisten") {
        return valid_permit(value, true);
    }
    if name.eq_ignore_ascii_case(b"tunnel") {
        return valid_tunnel(value);
    }
    false
}

/// Whether the value is a time sshd accepts: eight, twelve or fourteen
/// digits with an optional `Z` or `UTC` suffix, naming a calendar date
/// and, where given, a clock time inside the day.
fn valid_expiry_time(value: &[u8]) -> bool {
    let digits = if value.len() > 3 && value[value.len() - 3..].eq_ignore_ascii_case(b"utc") {
        &value[..value.len() - 3]
    } else if value.len() > 1
        && value
            .last()
            .is_some_and(|last| last.eq_ignore_ascii_case(&b'z'))
    {
        &value[..value.len() - 1]
    } else {
        value
    };
    if !matches!(digits.len(), 8 | 12 | 14) || !digits.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let number = |from: usize, to: usize| {
        digits[from..to]
            .iter()
            .fold(0u32, |acc, byte| acc * 10 + u32::from(*byte - b'0'))
    };
    let month = number(4, 6);
    let day = number(6, 8);
    if !(1..=12).contains(&month) {
        return false;
    }
    let leap = number(0, 4) % 4 == 0 && (number(0, 4) % 100 != 0 || number(0, 4) % 400 == 0);
    let last = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=last).contains(&day) {
        return false;
    }
    if digits.len() >= 12 && (number(8, 10) > 23 || number(10, 12) > 59) {
        return false;
    }
    if digits.len() == 14 && number(12, 14) > 59 {
        return false;
    }
    true
}

/// Whether the value names an environment assignment sshd accepts: a
/// nonempty name of letters, digits and underscores, then `=`.
fn valid_environment(value: &[u8]) -> bool {
    let Some(at) = value.iter().position(|byte| *byte == b'=') else {
        return false;
    };
    let name = &value[..at];
    !name.is_empty()
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// Whether the value permits what sshd would let through: a host and a
/// port, where the port is `*`, a number, or a service name the
/// services db holds, and a bare port for `permitlisten`, which listens
/// anywhere. The host holds fewer than the 1025 bytes the lookup needs.
fn valid_permit(value: &[u8], allow_bare_port: bool) -> bool {
    const HOST_LIMIT: usize = 1025;
    let (host, port) = if let Some(pair) = split_permit(value) {
        pair
    } else if allow_bare_port {
        (b"*".as_slice(), value)
    } else {
        return false;
    };
    if host.len() >= HOST_LIMIT {
        return false;
    }
    port == b"*" || valid_port(port)
}

/// The host and the port of a permit value: the brackets hold an IPv6
/// address, and otherwise the first `:` or `/` ends the host, the way
/// sshd reads them.
fn split_permit(value: &[u8]) -> Option<(&[u8], &[u8])> {
    if let Some(rest) = value.strip_prefix(b"[") {
        let end = rest.iter().position(|byte| *byte == b']')?;
        let port = rest.get(end + 1..)?.strip_prefix(b":")?;
        return Some((&value[..end + 2], port));
    }
    let at = value
        .iter()
        .position(|byte| *byte == b':' || *byte == b'/')?;
    Some((&value[..at], &value[at + 1..]))
}

/// Whether the port is one sshd dials or listens on: `*` is anywhere,
/// a number is a port only inside the sixteen bits, and a name is a
/// port where the services db names it — the way sshd's own lookup
/// reads it.
fn valid_port(port: &[u8]) -> bool {
    if !port.is_empty() && port.iter().all(u8::is_ascii_digit) {
        return port.len() <= 5
            && matches!(
                port.iter()
                    .fold(0u32, |acc, byte| acc * 10 + u32::from(*byte - b'0')),
                1..=65535
            );
    }
    service_port_in(port, &system_services())
}

/// The services db of the host, as sshd reads it: empty when the table
/// cannot be read, and then only numbers are ports.
fn system_services() -> String {
    std::fs::read_to_string("/etc/services").unwrap_or_default()
}

/// Whether the services text names the port: the name or one of its
/// aliases, on a `tcp` or `udp` line. A comment names nothing, and a
/// service name holds no whitespace, so the first two fields are the
/// whole entry.
fn service_port_in(port: &[u8], services: &str) -> bool {
    if port.is_empty()
        || !port.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_' || *byte == b'.'
        })
    {
        return false;
    }
    services.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        let (Some(service), Some(portproto)) = (fields.next(), fields.next()) else {
            return false;
        };
        let Some((_, proto)) = portproto.split_once('/') else {
            return false;
        };
        if proto != "tcp" && proto != "udp" {
            return false;
        }
        service.as_bytes() == port || fields.any(|alias| alias.as_bytes() == port)
    })
}

/// Whether the value names a tunnel device sshd takes: `any`, or the
/// number of the device.
fn valid_tunnel(value: &[u8]) -> bool {
    const TUNID_MAX: u32 = 0x7fff_ffff - 2;
    value.eq_ignore_ascii_case(b"any")
        || (!value.is_empty()
            && value.iter().all(u8::is_ascii_digit)
            && value.len() <= 10
            && value
                .iter()
                .fold(0u64, |acc, byte| acc * 10 + u64::from(*byte - b'0'))
                <= u64::from(TUNID_MAX))
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
    // The key type is the first field, or the second when the first is
    // the options field: sshd rejects a line whose leading field is
    // neither, so a key-looking pair further along the line is not an
    // active key.
    let key_at = if fields.first().is_some_and(|field| is_key_type(field)) {
        0
    } else if fields.len() > 1 && is_options_bytes(first_field_span(text.as_bytes())) {
        1
    } else {
        return None;
    };
    let material = fields.get(key_at + 1)?;
    Some((fields[key_at].clone(), material.clone()))
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
