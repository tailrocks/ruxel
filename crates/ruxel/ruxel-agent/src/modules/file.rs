#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

#[cfg(test)]
use super::open_attrs;
use super::{ExecContext, apply_attrs, apply_attrs_fd, params_object, str_param};
use rustix::fs::{Dir, FileType, Mode, OFlags, fstat, mkdirat, open, openat};
use serde_json::{Value, json};
use std::os::fd::{AsFd, OwnedFd};
use std::path::Component;
use std::path::Path;

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path")
        .or_else(|| str_param(obj, "dest"))
        .ok_or("file: path required")?;
    let state = str_param(obj, "state").unwrap_or("file");
    let p = Path::new(path);
    let mut changed = false;

    match state {
        "file" => {
            if p.symlink_metadata().is_err() {
                return Err(format!("file: {path} does not exist"));
            }
            apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
            Ok(json!({"path": path, "state": "file", "changed": changed, "failed": false}))
        }
        "directory" => {
            if super::bool_param(obj, "recurse", false) {
                let (fd, created) = open_directory_path(p, !ctx.check_mode, obj, &mut changed)?;
                changed |= created;
                if let Some(fd) = fd {
                    apply_directory(&fd, obj, &mut changed, ctx.check_mode)?;
                }
                return Ok(
                    json!({"path": path, "state": "directory", "changed": changed, "failed": false}),
                );
            }
            if !p.is_dir() {
                if p.exists() {
                    return Err(format!("{path} exists and is not a directory"));
                }
                changed = true;
                if !ctx.check_mode {
                    std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
            }
            if p.exists() || !ctx.check_mode {
                apply_attrs(p, obj, &mut changed, ctx.check_mode)?;
            }
            Ok(json!({"path": path, "state": "directory", "changed": changed, "failed": false}))
        }
        "absent" => {
            let existed = p.symlink_metadata().is_ok();
            if existed {
                changed = true;
                if !ctx.check_mode {
                    if p.is_dir() && !p.is_symlink() {
                        std::fs::remove_dir_all(p).map_err(|e| e.to_string())?;
                    } else {
                        std::fs::remove_file(p).map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok(json!({"path": path, "state": "absent", "changed": changed, "failed": false}))
        }
        "link" => {
            let src = str_param(obj, "src").ok_or("file state=link: src required")?;
            let current = std::fs::read_link(p).ok();
            if current.as_deref() != Some(Path::new(src)) {
                changed = true;
                if !ctx.check_mode {
                    if p.symlink_metadata().is_ok() {
                        std::fs::remove_file(p).map_err(|e| e.to_string())?;
                    }
                    std::os::unix::fs::symlink(src, p).map_err(|e| e.to_string())?;
                }
            }
            Ok(
                json!({"dest": path, "src": src, "state": "link", "changed": changed, "failed": false}),
            )
        }
        other => Err(format!("file: state {other:?} outside the closed surface")),
    }
}

fn path_start(path: &Path) -> Result<OwnedFd, String> {
    let start = if path.is_absolute() { "/" } else { "." };
    open(start, directory_anchor_flags(), Mode::empty()).map_err(|e| e.to_string())
}

fn open_directory_path(
    path: &Path,
    create: bool,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
) -> Result<(Option<OwnedFd>, bool), String> {
    let current = match path_start(path) {
        Ok(fd) => fd,
        Err(error) if !path.is_absolute() => {
            let absolute = std::env::current_dir().map_err(|_| error)?.join(path);
            return open_directory_path(&absolute, create, obj, changed);
        }
        Err(error) => return Err(error),
    };
    open_directory_from(current, path, create, obj, changed, 0)
}

fn open_directory_from(
    mut current: OwnedFd,
    path: &Path,
    create: bool,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    depth: usize,
) -> Result<(Option<OwnedFd>, bool), String> {
    #[cfg(not(target_os = "macos"))]
    let _ = (&obj, &changed, depth);
    if depth > 40 {
        return Err("file: too many root symlink expansions".into());
    }
    let mut created = false;
    let mut parts = path
        .components()
        .filter(|part| !matches!(part, Component::RootDir | Component::CurDir))
        .peekable();
    while let Some(part) = parts.next() {
        let name = match part {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            Component::ParentDir => std::ffi::OsStr::new(".."),
            Component::Prefix(_) => return Err("file: invalid path prefix".into()),
        };
        let flags = directory_anchor_flags();
        current = match openat(&current, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            #[cfg(target_os = "macos")]
            Err(rustix::io::Errno::ACCESS) if create && parts.peek().is_none() => {
                let stat =
                    rustix::fs::statat(&current, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|e| e.to_string())?;
                if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
                    use std::os::unix::ffi::OsStrExt;
                    let target = rustix::fs::readlinkat(&current, name, Vec::new())
                        .map_err(|e| e.to_string())?;
                    let target = Path::new(std::ffi::OsStr::from_bytes(target.to_bytes()));
                    let start = if target.is_absolute() {
                        path_start(target)?
                    } else {
                        current
                    };
                    return open_directory_from(start, target, create, obj, changed, depth + 1);
                }
                super::apply_attrs_at(&current, Path::new(name), obj, changed, false)?;
                openat(&current, name, flags, Mode::empty()).map_err(|e| e.to_string())?
            }
            Err(rustix::io::Errno::NOENT) if !create => return Ok((None, true)),
            Err(rustix::io::Errno::NOENT) => {
                mkdirat(&current, name, Mode::from_raw_mode(0o777)).map_err(|e| e.to_string())?;
                created = true;
                openat(&current, name, flags, Mode::empty()).map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        };
    }
    Ok((Some(current), created))
}

#[cfg(target_os = "linux")]
fn directory_anchor_flags() -> OFlags {
    OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC
}

#[cfg(target_os = "macos")]
fn directory_anchor_flags() -> OFlags {
    // Darwin O_EXEC | O_DIRECTORY is O_SEARCH; rustix doesn't expose it.
    OFlags::from_bits_retain(0x4000_0000) | OFlags::DIRECTORY | OFlags::CLOEXEC
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn directory_anchor_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
}

fn apply_directory(
    fd: impl AsFd,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let readable = match openat(&fd, ".", flags, Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::ACCESS) if !check_mode => {
            #[cfg(target_os = "macos")]
            super::apply_attrs_at(&fd, Path::new("."), obj, changed, false)?;
            #[cfg(not(target_os = "macos"))]
            apply_attrs_fd(&fd, obj, changed, false)?;
            openat(&fd, ".", flags, Mode::empty()).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };
    apply_attrs_recursive(&readable, obj, changed, check_mode)
}

fn apply_attrs_recursive(
    fd: impl AsFd,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    let stat = fstat(&fd).map_err(|e| e.to_string())?;
    if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
        let dir = Dir::read_from(&fd).map_err(|e| e.to_string())?;
        for entry in dir {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            apply_entry(&fd, name, obj, changed, check_mode)?;
        }
    }
    apply_attrs_fd(&fd, obj, changed, check_mode)
}

#[cfg(target_os = "linux")]
fn apply_entry(
    dir: impl AsFd,
    name: &std::ffi::CStr,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    let fd = openat(
        &dir,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| e.to_string())?;
    let stat = fstat(&fd).map_err(|e| e.to_string())?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => apply_attrs_fd(&fd, obj, changed, check_mode),
        FileType::Directory => apply_directory(&fd, obj, changed, check_mode),
        _ => Ok(()),
    }
}

#[cfg(target_os = "macos")]
fn apply_entry(
    dir: impl AsFd,
    name: &std::ffi::CStr,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let stat = rustix::fs::statat(&dir, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| e.to_string())?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => {
            let path = Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()));
            super::apply_attrs_at(&dir, path, obj, changed, check_mode)
        }
        FileType::Directory => {
            let path = Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()));
            let flags = directory_anchor_flags() | OFlags::NOFOLLOW;
            let child = match openat(&dir, name, flags, Mode::empty()) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::ACCESS) if !check_mode => {
                    super::apply_attrs_at(&dir, path, obj, changed, false)?;
                    openat(&dir, name, flags, Mode::empty()).map_err(|e| e.to_string())?
                }
                Err(e) => return Err(e.to_string()),
            };
            apply_directory(&child, obj, changed, check_mode)
        }
        _ => Ok(()),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn apply_entry(
    _dir: impl AsFd,
    _name: &std::ffi::CStr,
    _obj: &serde_json::Map<String, Value>,
    _changed: &mut bool,
    _check_mode: bool,
) -> Result<(), String> {
    Err("file: safe recursive entry traversal is unsupported on this platform".into())
}

#[cfg(test)]
#[path = "file/tests.rs"]
mod tests;
