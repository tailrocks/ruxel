use super::{parse_mode, resolve_gid, resolve_uid, str_param};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use serde_json::{Map, Value};
use std::os::unix::ffi::OsStrExt;
use std::{
    os::fd::{AsFd, OwnedFd},
    path::Path,
};

fn anchor_flags() -> OFlags {
    let flags = OFlags::CLOEXEC | OFlags::DIRECTORY;
    #[cfg(target_os = "linux")]
    {
        flags | OFlags::PATH
    }
    #[cfg(target_vendor = "apple")]
    {
        flags | OFlags::from_bits_retain(libc::O_SEARCH.cast_unsigned())
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    {
        flags | OFlags::RDONLY
    }
}

fn parent_and_name(path: &Path) -> Result<(OwnedFd, &Path), String> {
    if path.as_os_str().is_empty() {
        return Err("attribute path is empty".into());
    }
    // Initial parent resolution accepts the author's existing aliases and `..`.
    // All subsequent attribute operations use this held directory descriptor.
    let raw = path.as_os_str().as_bytes();
    let directory_suffix = raw.ends_with(b"/") || raw.ends_with(b"/.");
    let (parent, name) = match path.file_name().filter(|_| !directory_suffix) {
        Some(name) => (
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
            Path::new(name),
        ),
        None => (path, Path::new(".")),
    };
    let fd = rustix::fs::open(parent, anchor_flags(), Mode::empty()).map_err(|e| e.to_string())?;
    Ok((fd, name))
}

pub(super) fn open_attrs(path: &Path) -> Result<OwnedFd, String> {
    let (parent, name) = parent_and_name(path)?;
    open_attrs_at(parent, name)
}

fn open_attrs_at(parent: impl AsFd, name: &Path) -> Result<OwnedFd, String> {
    #[cfg(not(target_os = "linux"))]
    let before = rustix::fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "linux"))]
    regular_or_directory(before.st_mode)?;
    #[cfg(target_os = "linux")]
    let flags = OFlags::PATH;
    #[cfg(target_vendor = "apple")]
    let flags = if FileType::from_raw_mode(before.st_mode) == FileType::Directory {
        OFlags::RDONLY | OFlags::DIRECTORY
    } else {
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY
    };
    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY;
    let opened = rustix::fs::openat(
        &parent,
        name,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    );
    #[cfg(target_vendor = "apple")]
    let opened = match opened {
        Err(rustix::io::Errno::ACCESS)
            if FileType::from_raw_mode(before.st_mode) == FileType::Directory =>
        {
            rustix::fs::openat(
                &parent,
                name,
                anchor_flags() | OFlags::NOFOLLOW,
                Mode::empty(),
            )
        }
        result => result,
    };
    let fd = opened.map_err(|e| e.to_string())?;
    let after = rustix::fs::fstat(&fd).map_err(|e| e.to_string())?;
    regular_or_directory(after.st_mode)?;
    #[cfg(not(target_os = "linux"))]
    if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
        return Err("attribute target changed while acquiring descriptor".into());
    }
    Ok(fd)
}

fn regular_or_directory(mode: rustix::fs::RawMode) -> Result<(), String> {
    if matches!(
        FileType::from_raw_mode(mode),
        FileType::RegularFile | FileType::Directory
    ) {
        Ok(())
    } else {
        Err("attributes require a regular file or directory".into())
    }
}

fn mode(value: &Value) -> Result<Mode, String> {
    let value = parse_mode(value)?;
    #[cfg(target_os = "linux")]
    {
        Ok(Mode::from_raw_mode(value))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Mode::from_raw_mode(
            value
                .try_into()
                .map_err(|_| "file mode exceeds platform range")?,
        ))
    }
}

pub(super) fn apply_attrs(
    path: &Path,
    obj: &Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    apply_attrs_fd(open_attrs(path)?, obj, changed, check_mode)
}

pub(super) fn apply_attrs_fd(
    fd: impl AsFd,
    obj: &Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    let meta = rustix::fs::fstat(&fd).map_err(|e| e.to_string())?;
    regular_or_directory(meta.st_mode)?;
    let uid = str_param(obj, "owner").map(resolve_uid).transpose()?;
    let gid = str_param(obj, "group").map(resolve_gid).transpose()?;
    if uid.is_some_and(|uid| uid != meta.st_uid) || gid.is_some_and(|gid| gid != meta.st_gid) {
        *changed = true;
        if !check_mode {
            #[cfg(target_os = "linux")]
            rustix::fs::chownat(
                &fd,
                "",
                uid.map(rustix::fs::Uid::from_raw),
                gid.map(rustix::fs::Gid::from_raw),
                AtFlags::EMPTY_PATH,
            )
            .map_err(|e| e.to_string())?;
            #[cfg(not(target_os = "linux"))]
            rustix::fs::fchown(
                &fd,
                uid.map(rustix::fs::Uid::from_raw),
                gid.map(rustix::fs::Gid::from_raw),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if let Some(value) = obj.get("mode") {
        let desired = mode(value)?;
        let current = rustix::fs::fstat(&fd).map_err(|e| e.to_string())?;
        if Mode::from_raw_mode(current.st_mode) != desired {
            *changed = true;
            if !check_mode {
                #[cfg(target_os = "linux")]
                rustix::fs::chmodat(
                    rustix_linux_procfs::proc_self_fd().map_err(|e| e.to_string())?,
                    rustix::path::DecInt::from_fd(&fd).as_c_str(),
                    desired,
                    AtFlags::empty(),
                )
                .map_err(|e| e.to_string())?;
                #[cfg(not(target_os = "linux"))]
                rustix::fs::fchmod(&fd, desired).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[cfg(target_vendor = "apple")]
pub(super) fn apply_attrs_at(
    parent: impl AsFd,
    name: &Path,
    obj: &Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    let fd = open_attrs_at(parent, name)
        .map_err(|error| format!("cannot pin attribute target without access: {error}"))?;
    apply_attrs_fd(fd, obj, changed, check_mode)
}

#[cfg(test)]
#[path = "tests/attrs.rs"]
mod tests;
