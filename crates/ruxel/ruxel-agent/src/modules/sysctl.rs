use super::{
    ExecContext, bool_param, params_object, reject_newlines, reject_nuls, str_param, write_atomic,
};
use serde_json::{Value, json};

mod boot;

pub(crate) use boot::boot_fingerprint;

/// One sysctl entry the preflight treats as managed: the persistence
/// file, the key, and the declared value. A `Plain` value is the
/// workflow's own in the clear — this run's plan declarations and the
/// retained entries of recorded tasks. A `Sealed` value is the HMAC
/// commitment of a declaration-only record: a `no_log` task's value
/// must never be stored or logged, so the ledger keeps only the digest
/// and the preflight verifies it by sealing the candidate bytes with
/// the same key. A sealed entry without a seal key fails closed: an
/// unverifiable declaration authorizes nothing.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ManagedSysctl {
    pub file: String,
    pub name: String,
    pub value: ManagedValue,
}

/// The declared value of a managed entry: in the clear, or sealed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ManagedValue {
    Plain(String),
    Sealed(String),
}

impl ManagedSysctl {
    pub(crate) fn plain(file: String, name: String, value: String) -> Self {
        ManagedSysctl {
            file,
            name,
            value: ManagedValue::Plain(value),
        }
    }
}

/// The managed set one preflight consults, with the key that verifies
/// sealed entries: the ledger's host key, `None` where the ledger has
/// none or the entries travel without one. The entries are the plan's
/// own declarations first with the ledger's retained ones behind; the
/// plan's own are also listed separately, because only they authorize a
/// differing later assignment (see `boot_override`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SysctlManaged {
    pub entries: Vec<ManagedSysctl>,
    pub plan: Vec<ManagedSysctl>,
    pub seal: Option<[u8; 32]>,
}

impl SysctlManaged {
    /// A seal-less managed set for tests: plain entries only, so the
    /// byte comparison decides without a ledger key. Every entry counts
    /// as the plan's own, the way a unit preflight models one.
    #[cfg(test)]
    pub(crate) fn testing(entries: Vec<ManagedSysctl>) -> Self {
        SysctlManaged {
            plan: entries.clone(),
            entries,
            seal: None,
        }
    }
}

/// The identity and value commitment of a declaration-only record: the
/// HMAC-SHA256 of the file, the canonical key and the trimmed declared
/// value under the ledger's host key, with a domain the result-cache
/// key derivation never uses. The digest doubles as the record's
/// storage key — stable across applies, since the inputs are — and as
/// the value's seal: the value itself is never stored or logged, and
/// only the candidate bytes that seal to the same digest verify
/// (`None` where the seal cannot initialize, which fails closed
/// downstream). The trimmed value is what the boot surface compares: the file rewrite
/// and the boot reads trim the syntax blanks, so the seal covers the
/// trimmed bytes on both sides.
pub(crate) fn declaration_digest(
    seal: &[u8; 32],
    file: &str,
    name: &str,
    value: &[u8],
) -> Option<String> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(seal).ok()?;
    mac.update(b"ruxel-sysctl-decl-v1");
    mac.update(&[0u8]);
    mac.update(file.as_bytes());
    mac.update(&[0u8]);
    mac.update(sysctl_key(name).as_bytes());
    mac.update(&[0u8]);
    mac.update(value);
    Some(crate::ledger::hex(&mac.finalize().into_bytes()))
}

pub(super) fn run(
    params: &Value,
    ctx: &ExecContext,
    managed: &SysctlManaged,
) -> Result<Value, String> {
    let obj = params_object(params)?;
    let name = str_param(obj, "name").ok_or("sysctl: name required")?;
    if name.is_empty()
        || name.starts_with('-')
        || name
            .split('/')
            .any(|component| component == "." || component == "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:-".contains(&b))
    {
        return Err("sysctl: invalid name".into());
    }
    let value = match obj.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => if *b { "1" } else { "0" }.to_string(),
        other => return Err(format!("sysctl: invalid value {other:?}")),
    };
    reject_newlines("sysctl", &[("name", name), ("value", &value)])?;
    // A NUL cannot be in a process argument: the live value reaches the
    // kernel through `sysctl -w name=value`, and the spawn of that
    // command would fail with an opaque error instead of naming the
    // value at fault. The schema's `SysctlValue` refuses such a value
    // too, but the runtime states the rule itself, so a value that
    // reaches the module by any other way is refused here.
    reject_nuls("sysctl", &[("value", &value)])?;
    let state = str_param(obj, "state").unwrap_or("present");
    if state != "present" {
        return Err(format!(
            "sysctl: state {state:?} outside the closed surface"
        ));
    }
    let sysctl_set = bool_param(obj, "sysctl_set", false);
    let reload = bool_param(obj, "reload", true);
    let file = str_param(obj, "sysctl_file").unwrap_or(ruxel_proto::constants::DEFAULT_SYSCTL_FILE);
    // The managed set is plan-first: the session's own declarations
    // lead, and the ledger's retained entries cover the (file, key)
    // pairs the plan doesn't mention — the earlier stages of a second
    // apply replayed with fresh agent state. A dropped task's record
    // authorizes its shadow for exactly one run: the plan's end
    // expires the records it never encountered.
    preflight_shadow(file, name, managed)?;
    preflight_override(file, name, &value, managed)?;

    let mut changed = false;

    // The file is read and rewritten as bytes: an unrelated byte that
    // is not text belongs to the host's content and must survive the
    // publish verbatim — procps parses the file as bytes and applies
    // the assignment in a file that carries one, so a text-only read
    // would refuse a host file the underlying utility accepts.
    let current = match std::fs::read(file) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("sysctl: cannot read {file}: {error}")),
    };

    // The rewrite is computed first and the publishability probe runs
    // only where the file needs replacement: the probe is a write of its
    // own, and an idempotent no-op — the file already holds the entry, or
    // a drifted live value is corrected with the persistent state already
    // right — must not fail on a target that is never published to. Where
    // replacement IS required the target is still proved publishable
    // before the live value is touched (see `preflight_persistence`).
    let (content, file_changed) = rewrite_file(&current, name, &value);
    changed |= file_changed;
    if file_changed {
        // The rehearsal stages the exact rewritten bytes: a quota
        // that fits the old content but not the new one must refuse
        // before `sysctl -w` changes the live value with no boot
        // entry behind it.
        preflight_persistence(file, &content, !ctx.check_mode)?;
    }

    // The live value is reconciled before the file is published: a key the
    // kernel rejects must not leave a new invalid assignment behind in
    // the persistence file.
    if sysctl_set {
        let live = read_sysctl(name)?;
        if !live_matches(name, &live, &value) {
            changed = true;
            if !ctx.check_mode {
                let st = std::process::Command::new("sysctl")
                    .arg("-w")
                    .arg(format!("{name}={value}"))
                    .output()
                    .map_err(|e| format!("sysctl -w: {e}"))?;
                if !st.status.success() {
                    return Err(format!(
                        "sysctl -w {name}: {}",
                        String::from_utf8_lossy(&st.stderr).trim()
                    ));
                }
            }
        }
    }

    if file_changed && !ctx.check_mode {
        write_atomic(std::path::Path::new(file), &content)?;
    }

    if changed && reload && !ctx.check_mode {
        let st = std::process::Command::new("sysctl")
            .arg("-p")
            .arg(file)
            .output()
            .map_err(|e| format!("sysctl -p: {e}"))?;
        if !st.status.success() {
            return Err(format!(
                "sysctl -p {file}: {}",
                String::from_utf8_lossy(&st.stderr).trim()
            ));
        }
    }
    Ok(json!({"changed": changed, "failed": false}))
}

/// The persistence target can be published before the live value is
/// touched: `sysctl -w` succeeds first, and a publish failure after it
/// would leave the kernel changed with no persistent entry behind it.
/// A missing directory or a non-file at the path is refused up front;
/// where the apply may run, the rehearsal stages the exact rewritten
/// bytes under scratch names in the same directory — a quota that fits
/// the old content but not the new one refuses, and the rename of one
/// scratch name over the other proves the directory admits the
/// replacement mechanics — before the live value is touched. The live
/// path itself is never renamed: the states that belong to its own
/// inode — an immutable or append-only file, a file mounted over its
/// path — are refused by read-only probes (see `refuse_unreplaceable`),
/// so no interruption and no refusal strands rewritten bytes at the
/// live path, and there is no restoration to fail. The probes read the
/// live state; a lock set after them still refuses at publish, but only
/// after `sysctl -w`. The rehearsal runs only where the file needs
/// replacement: an idempotent no-op must not fail on a target that is
/// never published to.
pub(super) fn preflight_persistence(file: &str, content: &[u8], probe: bool) -> Result<(), String> {
    let path = std::path::Path::new(file);
    let live = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Some(meta),
        Ok(_) => {
            return Err(format!(
                "sysctl: cannot persist {file}: the path is not a regular file"
            ));
        }
        Err(_) => None,
    };
    let dir = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| format!("sysctl: cannot persist {file}: its directory does not exist"))?;
    if probe {
        // The probe carries its scratch name and is removed whatever the
        // rehearsal reports: a refusal partway through must not strand it
        // beside the live file.
        let scratch = dir.join(unique_probe_name());
        let outcome = stage_probe(file, &scratch, path, dir, live.as_ref(), content);
        let _ = std::fs::remove_file(&scratch);
        outcome?;
    }
    Ok(())
}

/// The probe's scratch name: unique per attempt — the process id, a
/// nanosecond stamp and a per-process sequence — so a probe a kill
/// strands never collides with a later attempt, even after the pid is
/// reused and every file-changing task in the directory would fail with
/// `AlreadyExists`. The open stays exclusive and owner-only, the handle
/// stays retained, and the caller still removes the name whatever the
/// rehearsal reports.
fn unique_probe_name() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        ".ruxel-sysctl-preflight-{}-{nanos}-{sequence}",
        std::process::id()
    )
}

/// The absent target's rehearsal from the probe that already carries
/// the publish's bytes: the publish stages one inode and renames it
/// into place, so the rehearsal links the staged bytes under the
/// rehearsal's first name instead of writing a second full copy — a
/// quota that fits one copy but not two fails the old rehearsal where
/// the publish succeeds — then replays the exchange the publish
/// performs over scratch names.
fn rehearse_absent(
    file: &str,
    scratch: &std::path::Path,
    staged: &std::fs::File,
    first: &std::path::Path,
    second: &std::path::Path,
) -> Result<(), String> {
    link_payload(file, scratch, first)?;
    staged
        .sync_all()
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    rehearse_exchange(file, first, second)
}

/// The publishability probe under its scratch name: a single retained
/// handle, opened exclusively with no mode but the owner's, never
/// reopened by name. Where the target is absent the handle itself
/// carries the exact bytes the publish writes — a quota that fits an
/// empty file but not the new content refuses here, before `sysctl -w`
/// — and both branches rehearse the staging primitives the publish
/// performs over scratch names, so a mount that allows file creation
/// but refuses subdirectories or hard links refuses here too. The
/// caller removes the scratch name whatever this reports, so no
/// refusal strands it. A refusal names the persistence file and the
/// operating system's error, never the staged bytes.
fn stage_probe(
    file: &str,
    scratch: &std::path::Path,
    path: &std::path::Path,
    dir: &std::path::Path,
    live: Option<&std::fs::Metadata>,
    content: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    // The handle stays open across the whole rehearsal: its scratch
    // name is reserved, so no swap of the path between the open and
    // the write can redirect the staged bytes.
    let mut staged = open_probe(file, scratch)?;
    if let Some(live) = live {
        // The rehearsal's own names retire the link-aside pair:
        // no step below renames the live file, so no step can
        // strand the rewritten bytes at the live path.
        let first = dir.join(format!(".ruxel-sysctl-rehearse-a-{}", std::process::id()));
        let second = dir.join(format!(".ruxel-sysctl-rehearse-b-{}", std::process::id()));
        // A scratch name of a dead process reuses no live pid,
        // so taking the names collides with no live rehearsal.
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        refuse_unreplaceable(path, file, dir)?;
        let outcome = rehearse_replacement(file, live, &first, &second, content);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        outcome
    } else {
        // The target is absent: there is no live file to rehearse
        // against, so the handle carries the exact bytes the
        // publish will write — a quota that fits an empty file but
        // not the new content refuses here, before `sysctl -w` —
        // and the staging primitives the publish performs are
        // rehearsed over scratch names the same way the live branch
        // rehearses them, before `sysctl -w` changes the live value
        // with no persistent entry behind it.
        staged
            .write_all(content)
            .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
        let first = dir.join(format!(".ruxel-sysctl-rehearse-a-{}", std::process::id()));
        let second = dir.join(format!(".ruxel-sysctl-rehearse-b-{}", std::process::id()));
        // A scratch name of a dead process reuses no live pid,
        // so taking the names collides with no live rehearsal.
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        let outcome = rehearse_absent(file, scratch, &staged, &first, &second);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        outcome?;
        Ok(())
    }
}

/// The probe's own handle: one exclusive open carrying no mode but the
/// owner's — the staged bytes are never world-readable on their way to
/// the publish — and the publish never reopens the name, so the handle
/// the rehearsal holds is the file the bytes reach.
fn open_probe(file: &str, scratch: &std::path::Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(scratch)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))
}

/// Refuses a live file the atomic publish cannot replace, without
/// touching the file: a file mounted over its path — a read-only bind
/// mount — lives on another device than its parent directory, and the
/// publish's rename over it fails the way any replacement does. The
/// comparison is `u64` against `u64` on every platform, with no
/// narrowing cast.
fn refuse_unreplaceable(
    path: &std::path::Path,
    file: &str,
    dir: &std::path::Path,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let live =
        std::fs::metadata(path).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    let parent =
        std::fs::metadata(dir).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    if live.dev() != parent.dev() {
        return Err(format!(
            "sysctl: cannot persist {file}: the file is mounted over its path and cannot be replaced"
        ));
    }
    // A file bind-mounted over its own path shares its device with
    // the parent, so the device comparison accepts a target the
    // publish cannot replace: the scratch rename succeeds inside the
    // parent directory while the atomic rename over the mountpoint
    // fails. The mountpoint itself decides, read from the kernel's
    // own table where it exists — elsewhere the device comparison
    // stands alone.
    if is_mountpoint(path).map_err(|error| format!("sysctl: cannot persist {file}: {error}"))? {
        return Err(format!(
            "sysctl: cannot persist {file}: the file is mounted over its path and cannot be replaced"
        ));
    }
    refuse_locked(path, file)
}

/// Whether `path` is itself a mount point, decided by the kernel's
/// mount table: a bind mount over the persistence path fails the
/// publish's atomic rename whatever device it reports. The table's
/// mount-point field carries octal escapes for spaces and other
/// special bytes, which are decoded before the comparison; the path
/// is matched raw and canonicalized, since the table spells the
/// kernel's resolved view. Where the table is absent — every
/// non-Linux host — nothing is a mount point and the device comparison
/// stands alone; where the table is present but unreadable the probe
/// refuses instead of clearing a mount it cannot see.
fn is_mountpoint(path: &std::path::Path) -> Result<bool, String> {
    is_mountpoint_in(std::fs::read_to_string("/proc/self/mountinfo"), path)
}

/// Whether the mount table lists `path` as a mount point: the reader
/// behind [`is_mountpoint`], split out so the unreadable table is
/// provable without a mount of its own. A missing table reads as no
/// mount point — a non-Linux host has no table, and the device
/// comparison stands alone there — while a table the probe cannot
/// read refuses: answering false would clear a bind-mounted file the
/// publish cannot replace.
fn is_mountpoint_in(
    table: Result<String, std::io::Error>,
    path: &std::path::Path,
) -> Result<bool, String> {
    let table = match table {
        Ok(table) => table,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "cannot read the mount table /proc/self/mountinfo: {error}"
            ));
        }
    };
    Ok(mountpoint_listed(
        &table,
        &[
            path.to_path_buf(),
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        ],
    ))
}

/// Whether the mount table lists any of `paths` as a mount point:
/// the pure comparison behind [`is_mountpoint`], split out so the
/// table parsing is provable without a mount of its own.
fn mountpoint_listed(table: &str, paths: &[std::path::PathBuf]) -> bool {
    table.lines().any(|line| {
        let Some(pre) = line.split(" - ").next() else {
            return false;
        };
        let mut fields = pre.split_whitespace();
        let mountpoint = match fields.nth(4) {
            Some(field) => unescape_mountpoint(field),
            None => return false,
        };
        paths
            .iter()
            .any(|path| path.as_os_str().as_encoded_bytes() == mountpoint.as_slice())
    })
}

/// The mount table's octal escapes decoded: a space in a mount point
/// spells `\040`, and the other special bytes spell their own.
fn unescape_mountpoint(field: &str) -> Vec<u8> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
        {
            let code =
                (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(code);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Refuses a file the operating system locks against replacement: the
/// immutable or append-only inode flags, read without modifying
/// anything. A filesystem without flag support carries no such flag,
/// so an unsupported ioctl is not a refusal — the scratch rehearsal
/// still proves the replacement mechanics.
#[cfg(target_os = "linux")]
fn refuse_locked(path: &std::path::Path, file: &str) -> Result<(), String> {
    let live =
        std::fs::File::open(path).map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    let Ok(flags) = rustix::fs::ioctl_getflags(&live) else {
        return Ok(());
    };
    if flags.intersects(rustix::fs::IFlags::APPEND | rustix::fs::IFlags::IMMUTABLE) {
        return Err(format!(
            "sysctl: cannot persist {file}: the file is locked against replacement"
        ));
    }
    Ok(())
}

/// The same lock on the other platforms: the BSD file flags
/// (`<sys/stat.h>`), read through `statat` without modifying anything.
/// The constants are the platform's own `u32` values, compared as
/// `u32` against `u32`, with no narrowing cast.
#[cfg(not(target_os = "linux"))]
fn refuse_locked(path: &std::path::Path, file: &str) -> Result<(), String> {
    const UF_IMMUTABLE: u32 = 0x0000_0002;
    const UF_APPEND: u32 = 0x0000_0004;
    let live = rustix::fs::statat(rustix::fs::CWD, path, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    if live.st_flags & (UF_IMMUTABLE | UF_APPEND) != 0 {
        return Err(format!(
            "sysctl: cannot persist {file}: the file is locked against replacement"
        ));
    }
    Ok(())
}

/// Proves the publish's replacement mechanics under scratch names in
/// the same directory, without ever renaming the live path: the exact
/// rewritten bytes are staged carrying the live identity, linked into
/// a private staging directory, and exchanged with a second scratch
/// name — the `mkdirat`, hard link and exchange rename the atomic
/// publish performs (`atomic/stage.rs`), over scratch names instead of
/// the live file. The directory's own states (permissions, read-only
/// mounts, quotas) refuse here the way they would refuse the publish —
/// and a filesystem that permits file creation and rename but no
/// subdirectory creation or hard links refuses here too, before
/// `sysctl -w` changes the live value with no persistent entry behind
/// it; the live inode's own states are refused by the read-only
/// probes, not exercised. A refusal leaves the live file exactly where
/// it was: the path still holds it, so there is no restoration to
/// fail — and an interruption strands at most scratch names, never
/// rewritten bytes at the live path.
fn rehearse_replacement(
    file: &str,
    live: &std::fs::Metadata,
    first: &std::path::Path,
    second: &std::path::Path,
    content: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let mut stage = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(first)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    // The staged copy carries the live identity: the mode and the owner
    // the publish preserves. The owner is set before the bytes are
    // written and the mode after, the way the atomic publish orders
    // them.
    rustix::fs::fchown(
        &stage,
        Some(rustix::fs::Uid::from_raw(live.uid())),
        Some(rustix::fs::Gid::from_raw(live.gid())),
    )
    .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    stage
        .write_all(content)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    stage
        .sync_all()
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    stage
        .set_permissions(std::fs::Permissions::from_mode(live.mode() & 0o7777))
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    rehearse_exchange(file, first, second)
}

/// The publication primitives the atomic publish performs, over scratch
/// names: the staged bytes are linked into a private staging directory
/// and exchanged with a second scratch name that exists — the
/// `mkdirat`, hard link and exchange rename the publish performs
/// (`atomic/stage.rs`), over scratch names instead of the live file.
/// Both rehearsal branches share it: the live branch stages the exact
/// rewritten bytes carrying the live identity, the missing-target
/// branch stages the exact bytes the publish will write with none — and
/// a directory that refuses any of the three primitives refuses here,
/// before `sysctl -w` changes the live value with no persistent entry
/// behind it. The scratch names are predictable per pid, so the opens
/// are exclusive and follow no link: a planted symlink at a name
/// refuses instead of handing the rehearsal's truncate to whatever it
/// names.
fn rehearse_exchange(
    file: &str,
    first: &std::path::Path,
    second: &std::path::Path,
) -> Result<(), String> {
    let stage = std::path::PathBuf::from(format!("{}.stage", first.display()));
    // A staging directory of a dead rehearsal reuses no live pid, so
    // taking the name collides with no live rehearsal — and a file
    // squatting on it refuses the `mkdirat` the publish needs.
    let _ = std::fs::remove_dir(&stage);
    create_stage_dir(file, &stage)?;
    let payload = stage.join("payload");
    let outcome = (|| {
        link_payload(file, first, &payload)?;
        let vacant = rustix::fs::open(
            second,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
        drop(vacant);
        exchange_names(file, &payload, second)
    })();
    // The staging directory goes with the rehearsal whatever it
    // reports: the payload is a scratch name, and the caller retires
    // the other two.
    let _ = std::fs::remove_file(&payload);
    let _ = std::fs::remove_dir(&stage);
    outcome
}

/// The private staging directory of the rehearsal: the `mkdirat` the
/// atomic publish performs, under the rehearsal's own scratch name.
fn create_stage_dir(file: &str, stage: &std::path::Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(stage.as_os_str().as_bytes())
        .map_err(|_| format!("sysctl: cannot persist {file}: the staging name is not usable"))?;
    rustix::fs::mkdirat(
        rustix::fs::CWD,
        &name,
        rustix::fs::Mode::from_raw_mode(0o700),
    )
    .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    Ok(())
}

/// The staged bytes linked into the staging directory: the hard link
/// the atomic publish performs before exchanging the payload into
/// place.
fn link_payload(
    file: &str,
    first: &std::path::Path,
    payload: &std::path::Path,
) -> Result<(), String> {
    std::fs::hard_link(first, payload)
        .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
    Ok(())
}

/// The exchange the publish performs, over scratch names: both names
/// exist, so the rename swaps them instead of replacing — a plain
/// rename would prove nothing about the exchange the publish needs,
/// which fails where the filesystem does not support it.
/// Whether the exchange's failure is one the atomic publish recovers
/// from: the publish falls back to removing the destination and
/// renaming the payload into its place where the filesystem does not
/// support the exchange — `EINVAL`, `ENOSYS` or `ENOTSUP` — so the
/// rehearsal must prove that same pair instead of refusing a write the
/// publish performs. Every other failure refuses the publish too.
fn exchange_failed_unsupported(error: rustix::io::Errno) -> bool {
    matches!(
        error,
        rustix::io::Errno::INVAL | rustix::io::Errno::NOSYS | rustix::io::Errno::NOTSUP
    )
}

fn exchange_names(
    file: &str,
    payload: &std::path::Path,
    second: &std::path::Path,
) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let payload_name = std::ffi::CString::new(payload.as_os_str().as_bytes())
        .map_err(|_| format!("sysctl: cannot persist {file}: the staging name is not usable"))?;
    let second_name = std::ffi::CString::new(second.as_os_str().as_bytes())
        .map_err(|_| format!("sysctl: cannot persist {file}: the staging name is not usable"))?;
    match rustix::fs::renameat_with(
        rustix::fs::CWD,
        &payload_name,
        rustix::fs::CWD,
        &second_name,
        rustix::fs::RenameFlags::EXCHANGE,
    ) {
        Ok(()) => Ok(()),
        Err(error) if exchange_failed_unsupported(error) => {
            // The publish's own fallback over scratch names: the
            // destination goes away and the payload takes its place.
            std::fs::remove_file(second)
                .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
            std::fs::rename(payload, second)
                .map_err(|e| format!("sysctl: cannot persist {file}: {e}"))?;
            Ok(())
        }
        Err(error) => Err(format!("sysctl: cannot persist {file}: {error}")),
    }
}

/// A shadowed name is refused whatever the file holds: sysctl(8) applies
/// one file name once, from the highest-precedence directory, so the boot
/// value comes from the shadow and the managed entry is ignored — the
/// parameter reverts after reboot even when this apply writes nothing.
/// The exception is a shadow the workflow itself declares: a deliberate
/// transition of the key to the higher-precedence file — the same name in
/// `/etc/sysctl.d` after `/usr/lib/sysctl.d` — leaves the shadow's entry a
/// declared managed value, and the earlier stage's task must be able to
/// run again.
fn preflight_shadow(file: &str, name: &str, managed: &SysctlManaged) -> Result<(), String> {
    preflight_shadow_in(file, name, managed, &boot::DIRS)
}

/// The shadow refusal over `dirs`, split out so the alias exemption
/// is provable without the system directories: a symlink alias of the
/// managed file passes, a declared shadow passes, and anything else
/// that holds the name refuses.
fn preflight_shadow_in(
    file: &str,
    name: &str,
    managed: &SysctlManaged,
    dirs: &[(&str, u8)],
) -> Result<(), String> {
    if let Some(shadow) = boot::shadowing_in(file, dirs) {
        if declared_shadow(&shadow, name, managed) {
            return Ok(());
        }
        return Err(format!(
            "sysctl: cannot persist {file}: {shadow} shadows it at boot; sysctl(8) applies one file name once, from the highest-precedence directory, so the entry would revert after reboot"
        ));
    }
    Ok(())
}

/// Whether the shadowing file carries the managed key as a declared
/// managed value of its own: the workflow's own later-stage file, not
/// an unmanaged shadow. The value need not be this task's — the later
/// stage's declared value is a transition the boot order settles — but
/// the file must declare the key, or the shadow's assignment of it is
/// an unmanaged boot value again.
fn declared_shadow(shadow: &str, name: &str, managed: &SysctlManaged) -> bool {
    managed
        .entries
        .iter()
        .any(|entry| entry.file == shadow && sysctl_key(&entry.name) == sysctl_key(name))
}

/// A persistence entry overridden later at boot is refused whatever the
/// file holds: the system files load together by their base names and
/// `/etc/sysctl.conf` after all of them, so a later-loading file that
/// assigns the managed key hands the boot value away from the managed
/// entry — the parameter reverts after reboot even when this apply
/// writes nothing, just like a same-name shadow. Keys compare
/// canonicalized (the slash-first and dotted spellings are one entry),
/// and an assignment to the same value hands boot the managed value —
/// it is not an override. A file ruxel manages holds a declared managed
/// value instead: the later stage of a workflow that sets the same
/// parameter on purpose is a declared transition, not an unmanaged
/// override, and refusing it would make the workflow non-idempotent.
fn preflight_override(
    file: &str,
    name: &str,
    value: &str,
    managed: &SysctlManaged,
) -> Result<(), String> {
    let host = boot::host_files()?;
    match boot::boot_override(file, name, value, &host, managed) {
        Some((later, other)) => Err(format!(
            "sysctl: cannot persist {file}: {later} assigns {name}={other} and loads after it at boot, so reboot restores {other} instead of the managed value"
        )),
        None => Ok(()),
    }
}

/// The host's boot files for the cache gate: the same surface the
/// override preflight reads, so the cache weighs what the run would.
pub(crate) fn sysctl_host_files() -> Result<Vec<(std::path::PathBuf, Vec<u8>)>, String> {
    boot::host_files()
}

/// Whether the cached sysctl result still stands under the current
/// managed set: the override preflight replayed over the injected
/// surface. The fingerprint pins the surface's bytes, not who manages
/// them — a dropped task leaves its file behind with every probe still
/// matching — so the cache must recheck the declarations: where the
/// record's entry is overridden by a file nothing manages anymore,
/// the task must run (and refuse) instead of reporting a convergence
/// the next reboot reverts.
pub(crate) fn sysctl_cache_covered(
    file: &str,
    name: &str,
    value: &str,
    host: &[(std::path::PathBuf, Vec<u8>)],
    managed: &SysctlManaged,
) -> bool {
    boot::boot_override(file, name, value, host, managed).is_none()
}

/// The managed entry's rewrite of the file: the one entry line is
/// reconciled and every other byte of the host's content survives
/// verbatim. The lines are matched as text (the entry itself is
/// ASCII) but carried as raw bytes, so a line the rewrite does not
/// touch — including one that is not valid text — is published back
/// exactly as it was read.
fn rewrite_file(current: &[u8], name: &str, value: &str) -> (Vec<u8>, bool) {
    let mut found = false;
    let mut changed = false;
    let mut out_lines: Vec<Vec<u8>> = Vec::new();
    // The lines are the `\n`-separated segments of the content; the
    // empty segment after a trailing newline is not a line of its own
    // and is dropped, the way `str::lines` ends the file.
    let mut lines: Vec<&[u8]> = current.split(|byte| *byte == b'\n').collect();
    if !current.is_empty() && lines.last() == Some(&&[][..]) {
        lines.pop();
    }
    for raw in lines {
        // The bytes carry procps syntax whitespace at their edges
        // only — no Unicode trim follows, since procps preserves
        // non-ASCII whitespace as value content — and the lossy text
        // matches the key alone (the entry itself is ASCII) while the
        // value compares as raw bytes: lossy text collapses distinct
        // values (`\xff` and U+FFFD both spell `�`) that boot
        // restores as different bytes.
        let body = trim(raw.strip_suffix(b"\r").unwrap_or(raw));
        let text = String::from_utf8_lossy(body);
        // Entry keys compare canonicalized: the slash-first and dotted
        // spellings of one parameter (`net/ipv4/ip_forward`,
        // `net.ipv4.ip_forward`) address one `/proc/sys` entry, so the
        // alias line is the same entry — matching raw strings would
        // update only one spelling and let boot process the other. A
        // leading '-' is procps's ignore-failure marker: it is not part
        // of the key, and the marked line assigns the entry all the
        // same.
        let is_entry = !text.starts_with('#')
            && text
                .split('=')
                .next()
                .is_some_and(|k| entry_key(k) == sysctl_key(name));
        if is_entry {
            found = true;
            // The entry's delimiter is the FIRST equals sign: a value may
            // carry equals signs of its own (a core_pattern command's
            // --option=value), which a split at every one would truncate.
            let equals = body.iter().position(|byte| *byte == b'=');
            let existing = equals.map_or(&[][..], |at| trim(&body[at + 1..]));
            if existing == trim(value.as_bytes()) {
                out_lines.push(raw.to_vec());
            } else {
                changed = true;
                // The marker belongs to the line: the host's "ignore
                // the error" intent survives the rewrite.
                out_lines.push(format!("{}{name}={value}", marker(&text)).into_bytes());
            }
        } else {
            out_lines.push(raw.to_vec());
        }
    }
    if !found {
        changed = true;
        out_lines.push(format!("{name}={value}").into_bytes());
    }
    if changed {
        let mut content: Vec<u8> = Vec::new();
        for (index, line) in out_lines.iter().enumerate() {
            if index > 0 {
                content.push(b'\n');
            }
            content.extend_from_slice(line);
        }
        content.push(b'\n');
        (content, true)
    } else {
        (current.to_vec(), false)
    }
}

/// Whether the byte is one of procps's syntax blanks: space, tab,
/// vertical tab, form feed — the `isspace` set the `lstrip`/`rstrip`
/// helpers trim in procps-ng `src/sysctl.c`. Newline and carriage
/// return are line endings, never trimmed content (lines arrive split
/// on `\n`, and a trailing `\r` is stripped beside the trim), and
/// every other blank — including every non-ASCII whitespace — is value
/// content the trim keeps: the byte-level non-UTF8 tests pin that
/// preservation.
fn is_syntax_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\x0B' | b'\x0C')
}

/// Whether the character is one of procps's syntax blanks: the same
/// four the byte trim covers, for the text-side trims.
fn is_syntax_ws_char(blank: char) -> bool {
    matches!(blank, ' ' | '\t' | '\u{B}' | '\u{C}')
}

/// The syntax blanks trimmed off both ends of a byte line.
fn trim(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| !is_syntax_ws(*byte))
        .unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|byte| !is_syntax_ws(*byte))
        .map_or(start, |at| at + 1);
    &line[start..end]
}

/// A value with the syntax-level whitespace trimmed off both ends:
/// procps strips the blanks around a value (`isspace`: space, tab,
/// vertical tab, form feed), and the blanks inside it are part of the
/// value itself — `kernel.hostname` holds "abc  def" as it is written,
/// so a value that differs from it in its blanks is another value.
pub(crate) fn trimmed_value(value: &str) -> &str {
    value.trim_matches(is_syntax_ws_char)
}

/// The `/proc/sys` line ending off a live read: the kernel ends the
/// value with `\n` (a `\r` travels beside it on some payloads), and
/// that ending is the read's framing — not syntax whitespace the file
/// trim would cover and not content the compare would weigh — so the
/// live path strips it before any other trim. Only the live path: the
/// file-side trims keep their four blanks exactly.
fn strip_live_ending(live: &str) -> &str {
    live.trim_matches(|ending: char| ending == '\n' || ending == '\r')
}

/// Whether the live kernel value already holds the requested one.
/// The `/proc/sys` read ends with a line ending, which is stripped on
/// the live side before the syntax-blank trim — a value the workflow
/// wrote reads back equal, so the second apply is a no-op — and
/// procps strips the syntax blanks around a value before applying it,
/// so both sides compare trimmed — a vertically padded value is one
/// kernel value with its bare spelling; internal separators compare
/// token-wise for known numeric vector values only —
/// `/proc/sys` separates numeric lists with tabs where workflows
/// write spaces — and exactly otherwise, so `kernel.core_pattern`
/// keeps every blank of "12  34" instead of drifting silently.
fn live_matches(name: &str, live: &str, value: &str) -> bool {
    let live = trimmed_value(strip_live_ending(live));
    let value = trimmed_value(value);
    if live == value {
        return true;
    }
    if !is_numeric_vector(name) {
        return false;
    }
    let live_tokens: Vec<&str> = live.split_ascii_whitespace().collect();
    if live_tokens.len() < 2 {
        return false;
    }
    live_tokens == value.split_ascii_whitespace().collect::<Vec<&str>>()
        && live_tokens
            .iter()
            .all(|token| token.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Whether the parameter is a known numeric vector: `/proc/sys`
/// separates its values with tabs where workflows write spaces, so
/// the live comparison folds the separators token-wise. The set is the
/// kernel's numeric-list sysctls (`ip_local_port_range`,
/// `ping_group_range`, the `tcp_rmem`/`tcp_wmem`/`tcp_mem` triples and
/// `udp_mem`): every other parameter — including string values that
/// merely look numeric — compares exactly, since whitespace is content
/// there.
fn is_numeric_vector(name: &str) -> bool {
    matches!(
        sysctl_key(name).as_str(),
        "net/ipv4/ip_local_port_range"
            | "net/ipv4/ping_group_range"
            | "net/ipv4/tcp_mem"
            | "net/ipv4/tcp_rmem"
            | "net/ipv4/tcp_wmem"
            | "net/ipv4/udp_mem"
    )
}

/// The ignore-failure marker of an assignment line: procps reads a
/// leading '-' as "ignore the error", and the assignment still happens.
fn marker(line: &str) -> &'static str {
    if line.trim_start_matches(is_syntax_ws_char).starts_with('-') {
        "-"
    } else {
        ""
    }
}

fn read_sysctl(name: &str) -> Result<String, String> {
    read_live(&proc_sys_path(name))
}

/// The live read behind `read_sysctl`, split out so the mapping is
/// provable without `/proc/sys`: the read's line ending is stripped
/// before the syntax-blank trim, so what the second apply reads back
/// compares equal to the value the first apply wrote and no
/// `sysctl -w` runs again.
fn read_live(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map(|s| trimmed_value(strip_live_ending(&s)).to_string())
        .map_err(|e| format!("read {path}: {e}"))
}

/// The `/proc/sys` path of a kernel parameter. procps follows the
/// FIRST separator (`slashdot` in procps-ng `src/sysctl.c`): a name
/// whose first separator is `.` is dotted notation with every `.` a
/// separator and every `/` a literal dot of its component, so the two
/// swap — `net.ipv4.conf.enp3s0/200.forwarding` addresses
/// `net/ipv4/conf/enp3s0.200/forwarding` — while a name whose first
/// separator is `/` is slash-first notation and stays as written, every
/// `/` a separator and every `.` a literal of its component.
pub(crate) fn proc_sys_path(name: &str) -> String {
    format!("/proc/sys/{}", sysctl_key(name))
}

/// One parameter's identity: the `/proc/sys` path it addresses. The
/// separators canonicalize the way procps reads them — the first
/// separator decides (`slashdot` in procps-ng `src/sysctl.c` swaps
/// every separator of a dot-first name and leaves a slash-first name
/// alone) — so a dot-first mixed spelling
/// (`net.ipv4.conf.enp3s0/200.forwarding`) compares equal to its slash
/// form (`net/ipv4/conf/enp3s0.200/forwarding`), while a slash-first
/// name with a literal dot (`enp3s0.200`) keeps its identity apart from
/// the dotted spelling of another path.
pub(crate) fn sysctl_key(name: &str) -> String {
    let dot = name.find('.');
    let slash = name.find('/');
    // procps looks at the first separator it meets: a dot-first name
    // swaps every separator, a slash-first (or separator-free) one is
    // already the path. Any `/` alone never meant slash-first — a
    // dot-first mixed spelling holds one too.
    let dot_first = match (dot, slash) {
        (Some(first_dot), Some(first_slash)) => first_dot < first_slash,
        (Some(_), None) => true,
        _ => false,
    };
    if dot_first {
        name.chars()
            .map(|cell| {
                if cell == '.' {
                    '/'
                } else if cell == '/' {
                    '.'
                } else {
                    cell
                }
            })
            .collect()
    } else {
        name.to_string()
    }
}

/// The canonical key of an assignment in a file: the `/proc/sys` path
/// it addresses. A leading '-' is procps's ignore-failure marker, not
/// part of the key — the marked line assigns the entry all the same,
/// so the marker is stripped before the key is compared. The trims
/// cover the syntax blanks only, and the key arrives already trimmed:
/// non-ASCII whitespace is content procps preserves, not syntax.
pub(crate) fn entry_key(text: &str) -> String {
    let text = text.trim_matches(is_syntax_ws_char);
    sysctl_key(
        text.strip_prefix('-')
            .unwrap_or(text)
            .trim_matches(is_syntax_ws_char),
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod probe_handle_tests {
    //! The probe's single retained handle: opened once, exclusively,
    //! with no mode but the owner's — never reopened by name — and
    //! removed whatever the rehearsal reports, with no staged bytes in
    //! the refusal.

    use std::os::unix::fs::PermissionsExt;

    /// A scratch directory of its own: the rehearsal must leave nothing
    /// behind in it, so the directory itself is the assertion surface.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ruxel-sysctl-probe-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).expect("operation succeeds");
        dir
    }

    /// Makes `path` append-only and whether this host could: the owner
    /// sets the user append-only flag itself, so no privilege is needed
    /// where the platform offers one.
    fn lock_append(path: &std::path::Path) -> bool {
        let (program, argument) = if cfg!(target_os = "linux") {
            ("chattr", "+a")
        } else {
            ("chflags", "uappend")
        };
        std::process::Command::new(program)
            .arg(argument)
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Puts the file back into the state `lock_append` took it out of.
    fn unlock_append(path: &std::path::Path) {
        let (program, argument) = if cfg!(target_os = "linux") {
            ("chattr", "-a")
        } else {
            ("chflags", "nouappend")
        };
        let _ = std::process::Command::new(program)
            .arg(argument)
            .arg(path)
            .status();
    }

    #[test]
    fn the_probe_opens_owner_only_and_reserves_its_name() {
        // One exclusive open with no mode but the owner's: the staged
        // bytes are never world-readable on their way to the publish.
        // The retained handle reserves the name — no second open, and
        // no reopen by the stager, can take it.
        let dir = scratch("mode");
        let probe = dir.join("probe");
        let handle = super::open_probe("net.test", &probe).expect("the probe must open");
        let mode = std::fs::metadata(&probe)
            .expect("operation succeeds")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the probe must open owner-only: {mode:o}");
        assert!(
            super::open_probe("net.test", &probe).is_err(),
            "the retained handle reserves the name"
        );
        drop(handle);
        std::fs::remove_file(&probe).expect("operation succeeds");
        std::fs::remove_dir(&dir).expect("operation succeeds");
    }

    #[test]
    fn a_locked_files_refusal_leaves_no_probe_and_no_secret_in_the_error() {
        // The append-only flag refuses the rehearsal at the read-only
        // lock probe. The refusal travels the early-return path — the
        // one that stranded the dropped handle's probe — so the scratch
        // name must be gone, the live file untouched, and the staged
        // bytes nowhere in the error.
        let dir = scratch("locked");
        let path = dir.join("x.conf");
        std::fs::write(&path, "net.test=2\n").expect("operation succeeds");
        if !lock_append(&path) {
            unlock_append(&path);
            std::fs::remove_dir_all(&dir).expect("operation succeeds");
            return;
        }
        let text = path.to_str().expect("path is text").to_owned();
        let content = b"net.test=1\n# probe-sekret-9f31\n".to_vec();
        let error = super::preflight_persistence(&text, &content, true)
            .expect_err("a locked file must refuse");
        unlock_append(&path);
        assert!(error.contains("cannot persist"), "{error}");
        assert!(
            !error.contains("sekret"),
            "the refusal must not carry the staged bytes: {error}"
        );
        assert!(
            !dir.join(format!(".ruxel-sysctl-preflight-{}", std::process::id()))
                .exists(),
            "a refused rehearsal must leave no probe behind"
        );
        assert_eq!(
            std::fs::read(&path).expect("operation succeeds"),
            b"net.test=2\n",
            "a refused rehearsal must leave the live bytes alone"
        );
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }

    #[test]
    fn the_rehearsal_refuses_a_planted_symlink_instead_of_following_it() {
        // Thread 286: the rehearsal's scratch names are predictable
        // per pid, so a local user can recreate one as a symlink after
        // the unlink — a following write would truncate the link's
        // target before the rename. The rehearsal must open its names
        // exclusively without following links, refusing the plant and
        // leaving the target alone.
        let dir = scratch("plant");
        let live_path = dir.join("x.conf");
        std::fs::write(&live_path, "net.test=2\n").expect("operation succeeds");
        let live = std::fs::metadata(&live_path).expect("operation succeeds");
        let victim = dir.join("victim");
        std::fs::write(&victim, "precious\n").expect("operation succeeds");
        let first = dir.join("first");
        let second = dir.join("second");
        std::os::unix::fs::symlink(&victim, &second).expect("operation succeeds");
        let error =
            super::rehearse_replacement("net.test", &live, &first, &second, b"net.test=1\n")
                .expect_err("a planted symlink must refuse, not redirect the rehearsal");
        assert!(error.contains("cannot persist"), "{error}");
        assert_eq!(
            std::fs::read(&victim).expect("operation succeeds"),
            b"precious\n",
            "the rehearsal must not write through the planted link"
        );
        // The caller retires the rehearsal's names whatever it reports,
        // so no refusal strands them; the live file keeps its bytes.
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        assert_eq!(
            std::fs::read(&live_path).expect("operation succeeds"),
            b"net.test=2\n",
            "a refused rehearsal must leave the live bytes alone"
        );
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }

    #[test]
    fn a_same_device_bind_mount_is_its_own_mountpoint() {
        // Thread 300: a file bind-mounted over its own path shares
        // its device with the parent, so the device-id comparison
        // accepts a target the publish cannot replace — the scratch
        // rename succeeds inside the parent while the atomic rename
        // over the mountpoint fails. The mountpoint itself decides,
        // read from the kernel's table.
        let table = "30 1 8:1 / / rw,relatime - ext4 /dev/sda1 rw\n\
             31 30 8:1 /etc/sysctl.d/x.conf /etc/sysctl.d/x.conf rw,bind - ext4 /dev/sda1 rw\n\
             32 30 8:1 /srv/with\\040space /srv/with\\040space rw,bind - ext4 /dev/sda1 rw\n";
        assert!(
            super::mountpoint_listed(table, &[std::path::PathBuf::from("/etc/sysctl.d/x.conf")]),
            "a bind-mounted file is its own mountpoint"
        );
        assert!(
            super::mountpoint_listed(table, &[std::path::PathBuf::from("/srv/with space")]),
            "octal escapes decode before the comparison"
        );
        assert!(
            !super::mountpoint_listed(table, &[std::path::PathBuf::from("/etc/sysctl.d/y.conf")]),
            "an unmounted file is no mountpoint"
        );
        assert!(
            super::mountpoint_listed(table, &[std::path::PathBuf::from("/")]),
            "the root itself is a mountpoint"
        );
        assert!(
            !super::mountpoint_listed(table, &[std::path::PathBuf::from("/etc")]),
            "a parent of mountpoints is no mountpoint itself"
        );
    }

    #[test]
    fn an_ordinary_file_is_no_mountpoint() {
        // A scratch file is never itself mounted over: the live
        // check answers false without a mount of its own.
        let dir = scratch("mountpoint");
        let path = dir.join("x.conf");
        std::fs::write(&path, "net.test=1\n").expect("operation succeeds");
        assert!(
            !super::is_mountpoint(&path).expect("a readable table decides"),
            "an ordinary file is no mountpoint"
        );
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }

    #[test]
    fn an_unreadable_mount_table_refuses_instead_of_clearing_the_file() {
        // Threads 286/300: a mount table the probe cannot read hides
        // the bind mount the probe exists to refuse — the device
        // comparison alone would clear a file the publish cannot
        // replace. The probe refuses instead. A missing table is no
        // refusal: a non-Linux host has no table, and the device
        // comparison stands alone there.
        let path = std::path::PathBuf::from("/etc/sysctl.d/x.conf");
        let error = super::is_mountpoint_in(
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "denied",
            )),
            &path,
        )
        .expect_err("an unreadable mount table must refuse");
        assert!(
            error.contains("mount table"),
            "the refusal must name the table: {error}"
        );
        assert!(
            !super::is_mountpoint_in(
                Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
                &path
            )
            .expect("a missing table reads as empty"),
            "a missing table is no mountpoint"
        );
        let table = "30 1 8:1 / / rw,relatime - ext4 /dev/sda1 rw\n\
             31 30 8:1 /etc/sysctl.d/x.conf /etc/sysctl.d/x.conf rw,bind - ext4 /dev/sda1 rw\n";
        assert!(
            super::is_mountpoint_in(Ok(table.to_string()), &path)
                .expect("a readable table decides"),
            "a listed file is its own mountpoint"
        );
    }

    #[test]
    fn the_rehearsal_refuses_a_symlink_at_the_staging_name() {
        // Thread 312: `first` is the private staging directory the
        // rehearsal creates with `mkdirat` — the publish's staging
        // primitive. A symlink planted at that name must refuse before
        // any byte is written: the retired create+write opened through
        // the link and staged the payload at the link's target before
        // renaming, while the publish it rehearses never follows it.
        let dir = scratch("stage-link");
        let ghost = dir.join("ghost");
        let first = dir.join("first");
        std::os::unix::fs::symlink(&ghost, &first).expect("operation succeeds");
        let live_path = dir.join("x.conf");
        std::fs::write(&live_path, "net.test=2\n").expect("operation succeeds");
        let live = std::fs::metadata(&live_path).expect("operation succeeds");
        let second = dir.join("second");
        let error =
            super::rehearse_replacement("net.test", &live, &first, &second, b"net.test=1\n")
                .expect_err("a symlink at the staging name must refuse, not stage through it");
        assert!(error.contains("cannot persist"), "{error}");
        assert!(
            !ghost.exists(),
            "the rehearsal must not write through the planted link"
        );
        assert_eq!(
            std::fs::read(&live_path).expect("operation succeeds"),
            b"net.test=2\n",
            "a refused rehearsal must leave the live bytes alone"
        );
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }

    #[test]
    fn the_absent_target_stages_through_the_handle_and_leaves_nothing() {
        // The target is absent, so the retained handle carries the exact
        // bytes the publish would write; the rehearsal still publishes
        // nothing and leaves no probe behind.
        let dir = scratch("missing");
        let path = dir.join("x.conf");
        let text = path.to_str().expect("path is text").to_owned();
        super::preflight_persistence(&text, b"net.test=1\n", true)
            .expect("a writable directory must stage the new file's bytes");
        assert!(
            !path.exists(),
            "the rehearsal must not publish the missing target"
        );
        assert!(
            std::fs::read_dir(&dir)
                .expect("operation succeeds")
                .next()
                .is_none(),
            "the rehearsal must leave no probe behind"
        );
        std::fs::remove_dir_all(&dir).expect("operation succeeds");
    }
}

#[cfg(test)]
mod stale_plan_tests {
    //! The dropped task's retained record must not authorize the plan's
    //! own shadow or override: both preflights consult the current
    //! plan's declarations, never the whole ledger's scan. The first
    //! plan manages the key in one file and the second plan drops that
    //! task for the same key in another file; the leftover assignment
    //! is unmanaged under the plan's own set and both preflights refuse.

    use super::boot;

    const KEY: &str = "net.ipv4.ip_forward";
    const ETC: &str = "/etc/sysctl.d/50-ruxel.conf";
    const USR: &str = "/usr/lib/sysctl.d/50-ruxel.conf";
    const OLD: &str = "/etc/sysctl.d/99-old.conf";
    const NEW: &str = "/etc/sysctl.d/10-new.conf";

    /// The whole-ledger scan after the workflow drops the `/etc` tasks:
    /// the retained records still declare the dropped files, with the
    /// values the previous apply wrote there.
    fn stale_managed() -> super::SysctlManaged {
        super::SysctlManaged {
            entries: vec![
                super::ManagedSysctl::plain(ETC.to_string(), KEY.to_string(), "1".to_string()),
                super::ManagedSysctl::plain(OLD.to_string(), KEY.to_string(), "0".to_string()),
                super::ManagedSysctl::plain(USR.to_string(), KEY.to_string(), "1".to_string()),
            ],
            // Retained records alone: the plan that produced this set
            // declared none of them anymore.
            plan: vec![],
            seal: None,
        }
    }

    /// The second plan's own declarations: the dropped tasks are gone,
    /// only the entries the plan still delivers authorize a shadow.
    fn plan_managed() -> super::SysctlManaged {
        super::SysctlManaged {
            entries: vec![super::ManagedSysctl::plain(
                USR.to_string(),
                KEY.to_string(),
                "1".to_string(),
            )],
            plan: vec![super::ManagedSysctl::plain(
                USR.to_string(),
                KEY.to_string(),
                "1".to_string(),
            )],
            seal: None,
        }
    }

    #[test]
    fn a_dropped_tasks_shadow_is_refused_under_the_plans_own_set() {
        // The same basename in `/etc` shadows the `/usr/lib` target at
        // boot. The stale set authorizes it — the retained record
        // declares the shadowing file — while the plan's own set leaves
        // the leftover unmanaged and the shadow refuses.
        assert!(
            super::declared_shadow(ETC, KEY, &stale_managed()),
            "the stale set authorizes the shadow: the hole this pins"
        );
        assert!(
            !super::declared_shadow(ETC, KEY, &plan_managed()),
            "the plan dropped the /etc task, so its shadow refuses"
        );
    }

    #[test]
    fn a_dropped_tasks_override_is_refused_under_the_plans_own_set() {
        // Thread 332: the leftover assignment in the later-loading file
        // is an unmanaged override wherever the current plan no longer
        // declares it — a retained record of the dropped task authorizes
        // nothing. The first updated apply refuses instead of passing
        // the stale value as a declared transition and succeeding while
        // reboot restores it.
        let newcomer = super::SysctlManaged {
            entries: vec![super::ManagedSysctl::plain(
                NEW.to_string(),
                KEY.to_string(),
                "1".to_string(),
            )],
            plan: vec![super::ManagedSysctl::plain(
                NEW.to_string(),
                KEY.to_string(),
                "1".to_string(),
            )],
            seal: None,
        };
        let host = vec![(
            std::path::PathBuf::from(OLD),
            b"net.ipv4.ip_forward=0\n".to_vec(),
        )];
        assert_eq!(
            boot::boot_override(NEW, KEY, "1", &host, &stale_managed()),
            Some((OLD.to_string(), "0".to_string())),
            "a retained record of the dropped task authorizes no override waiver"
        );
        assert_eq!(
            boot::boot_override(NEW, KEY, "1", &host, &newcomer),
            Some((OLD.to_string(), "0".to_string())),
            "the leftover assignment is an unmanaged override again"
        );
    }

    #[test]
    fn a_retained_entry_beside_the_plans_own_does_not_waive() {
        // Thread 332, the merged production set: the plan declares the
        // earlier-loading file while the retained record of the dropped
        // task still names the later-loading leftover. Only the plan's
        // own declaration may waive, so the leftover refuses.
        let merged = super::SysctlManaged {
            entries: vec![
                super::ManagedSysctl::plain(NEW.to_string(), KEY.to_string(), "1".to_string()),
                super::ManagedSysctl::plain(OLD.to_string(), KEY.to_string(), "0".to_string()),
            ],
            plan: vec![super::ManagedSysctl::plain(
                NEW.to_string(),
                KEY.to_string(),
                "1".to_string(),
            )],
            seal: None,
        };
        let host = vec![(
            std::path::PathBuf::from(OLD),
            b"net.ipv4.ip_forward=0\n".to_vec(),
        )];
        assert_eq!(
            boot::boot_override(NEW, KEY, "1", &host, &merged),
            Some((OLD.to_string(), "0".to_string())),
            "the dropped task's retained entry waives nothing beside the plan's own"
        );
    }
}
