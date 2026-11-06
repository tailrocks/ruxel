#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "replaced by the Ruxel rewrite")
)]

#[cfg(test)]
use super::open_attrs;
use super::{ExecContext, apply_attrs, apply_attrs_fd, params_object, str_param};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, fstat, mkdirat, open, openat, statat};
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
                let (fd, created) = open_directory_path(p, !ctx.check_mode)?;
                changed = created;
                if let Some(fd) = fd {
                    apply_attrs_recursive(&fd, obj, &mut changed, ctx.check_mode)?;
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
    open(
        start,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| e.to_string())
}

fn open_directory_path(path: &Path, create: bool) -> Result<(Option<OwnedFd>, bool), String> {
    let mut current = path_start(path)?;
    let mut created = false;
    for part in path.components() {
        let name = match part {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            _ => return Err("file: parent path components are not supported".into()),
        };
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        current = match openat(&current, name, flags, Mode::empty()) {
            Ok(fd) => fd,
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

fn apply_attrs_recursive(
    fd: impl AsFd,
    obj: &serde_json::Map<String, Value>,
    changed: &mut bool,
    check_mode: bool,
) -> Result<(), String> {
    apply_attrs_fd(&fd, obj, changed, check_mode)?;
    let stat = fstat(&fd).map_err(|e| e.to_string())?;
    if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
        let dir = Dir::read_from(&fd).map_err(|e| e.to_string())?;
        for entry in dir {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            let entry_stat =
                statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| e.to_string())?;
            if !matches!(
                FileType::from_raw_mode(entry_stat.st_mode),
                FileType::Directory | FileType::RegularFile
            ) {
                continue;
            }
            let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
            let child = match openat(&fd, name, flags, Mode::empty()) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::LOOP) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let opened = fstat(&child).map_err(|e| e.to_string())?;
            if !matches!(
                FileType::from_raw_mode(opened.st_mode),
                FileType::Directory | FileType::RegularFile
            ) {
                return Err("file: entry changed type during traversal".into());
            }
            apply_attrs_recursive(&child, obj, changed, check_mode)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn context() -> ExecContext {
        ExecContext {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        }
    }

    #[test]
    fn default_file_state_applies_attrs_and_rejects_missing() {
        let dir = std::env::temp_dir().join(format!("ruxel-file-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("existing");
        std::fs::write(&path, b"content").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let result = run(&json!({"path": path, "mode": "0600"}), &context()).unwrap();
        assert_eq!(result["changed"], true);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let missing = dir.join("missing");
        assert!(
            run(&json!({"path": missing}), &context())
                .unwrap_err()
                .contains("does not exist")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn recursive_attributes_detect_and_repair_nested_drift() {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ruxel-file-recursive-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        let file = dir.join("nested/file");
        std::fs::write(&file, b"content").unwrap();
        let params = json!({"path": dir, "state": "directory", "mode": "0700", "recurse": true});
        assert_eq!(run(&params, &context()).unwrap()["changed"], true);
        assert_eq!(run(&params, &context()).unwrap()["changed"], false);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut check = context();
        check.check_mode = true;
        assert_eq!(run(&params, &check).unwrap()["changed"], true);
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(run(&params, &context()).unwrap()["changed"], true);
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(run(&params, &context()).unwrap()["changed"], false);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn recursive_attributes_do_not_follow_symlinks() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ruxel-file-symlink-{}", std::process::id()));
        let root = base.join("root");
        let external = base.join("external");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        let target = external.join("file");
        std::fs::write(&target, b"safe").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&external, root.join("dirlink")).unwrap();
        std::os::unix::fs::symlink(&target, root.join("filelink")).unwrap();
        let params = json!({"path": root, "state": "directory", "mode": "0755", "recurse": true});
        run(&params, &context()).unwrap();
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&external).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(run(&json!({"path": root.join("dirlink"), "state":"directory", "mode":"0755", "recurse":true}), &context()).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn opened_directory_stays_anchored_when_path_is_swapped() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ruxel-file-swap-{}", std::process::id()));
        let root = base.join("root");
        let external = base.join("external");
        let moved = base.join("moved");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        let victim = external.join("victim");
        std::fs::write(&victim, b"safe").unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
        let fd = open_attrs(&root).unwrap();
        std::fs::rename(&root, &moved).unwrap();
        std::os::unix::fs::symlink(&external, &root).unwrap();
        let params = json!({"mode": "0755"});
        let mut changed = false;
        apply_attrs_recursive(&fd, params.as_object().unwrap(), &mut changed, false).unwrap();
        assert_eq!(
            std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(open_attrs(&root).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
