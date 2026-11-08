use super::{ExecContext, params_object, reject_newlines, str_param, write_atomic};
use serde_json::{Value, json};

const FSTAB: &str = "/etc/fstab";

pub(super) fn run(params: &Value, ctx: &ExecContext) -> Result<Value, String> {
    let obj = params_object(params)?;
    let path = str_param(obj, "path").ok_or("mount: path required")?;
    let src = str_param(obj, "src").ok_or("mount: src required")?;
    let fstype = str_param(obj, "fstype").ok_or("mount: fstype required")?;
    let opts = str_param(obj, "opts").unwrap_or("defaults");
    let state = str_param(obj, "state").unwrap_or("mounted");
    reject_newlines(
        "mount",
        &[
            ("src", src),
            ("path", path),
            ("fstype", fstype),
            ("opts", opts),
            ("dump", "0"),
            ("pass", "0"),
        ],
    )?;
    if state != "mounted" {
        return Err(format!("mount: state {state:?} outside the closed surface"));
    }

    let mut changed = false;
    let current = match std::fs::read_to_string(FSTAB) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("mount: cannot read {FSTAB}: {error}")),
    };
    let (content, fstab_changed) = rewrite_fstab(&current, src, path, fstype, opts);
    changed |= fstab_changed;
    let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| e.to_string())?;
    let current_mount = find_mount(&mounts, path);
    let action = if current_mount.is_some() {
        let desired_source = resolve_source(src)?;
        mount_action(current_mount.as_ref(), &desired_source, fstype, opts)
    } else {
        MountAction::Mount
    };
    if action != MountAction::None {
        changed = true;
    }
    if matches!(action, MountAction::Replace | MountAction::Remount) {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")
            .map_err(|e| format!("mount: cannot inspect existing mount: {e}"))?;
        ensure_mount_root(path, &mountinfo)?;
        if action == MountAction::Replace {
            ensure_replaceable(
                current_mount
                    .as_ref()
                    .ok_or("mount: missing existing mount")?,
            )?;
        }
    }
    if !ctx.check_mode {
        let desired = Mounted {
            source: src.to_string(),
            fstype: fstype.to_string(),
            options: opts.to_string(),
        };
        if action == MountAction::Mount {
            std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
        }
        apply_mount(
            path,
            current_mount.as_ref(),
            &desired,
            action,
            &mut run_mount_command,
            &mut || {
                if fstab_changed {
                    write_atomic(std::path::Path::new(FSTAB), content.as_bytes())?;
                }
                Ok(())
            },
        )?;
    }

    Ok(json!({"changed": changed, "failed": false, "name": path, "src": src, "fstype": fstype}))
}

fn ensure_replaceable(old: &Mounted) -> Result<(), String> {
    if !matches!(old.fstype.as_str(), "ext4" | "xfs") || !old.source.starts_with("/dev/") {
        return Err("mount: cannot safely restore this existing mount type or source".into());
    }
    Ok(())
}

fn ensure_mount_root(path: &str, mountinfo: &str) -> Result<(), String> {
    let root = mountinfo
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.len() >= 6 && unescape_mount(fields[4]) == path)
                .then(|| unescape_mount(fields[3]))
        })
        .next_back()
        .ok_or("mount: existing mount missing from mountinfo")?;
    if root != "/" {
        return Err("mount: cannot safely modify a subtree mount".into());
    }
    Ok(())
}

fn apply_mount(
    path: &str,
    old: Option<&Mounted>,
    desired: &Mounted,
    action: MountAction,
    command: &mut impl FnMut(&str, &[&str]) -> Result<(), String>,
    persist: &mut impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    match action {
        MountAction::None => persist(),
        MountAction::Replace => replace_mount(
            path,
            old.ok_or("mount: missing existing mount")?,
            desired,
            command,
            persist,
        ),
        MountAction::Mount => {
            mount_explicit(path, desired, command)?;
            if let Err(error) = persist() {
                return match command("umount", &["--", path]) {
                    Ok(()) => Err(format!(
                        "mount: fstab update failed; new mount removed: {error}"
                    )),
                    Err(rollback) => Err(format!(
                        "mount: fstab update failed: {error}; new mount removal failed: {rollback}"
                    )),
                };
            }
            Ok(())
        }
        MountAction::Remount => {
            let old = old.ok_or("mount: missing existing mount")?;
            if let Err(error) =
                remount(path, old, &desired.options, command).and_then(|()| persist())
            {
                return match remount(path, old, &old.options, command) {
                    Ok(()) => Err(format!(
                        "mount: remount or fstab update failed; previous options restored: {error}"
                    )),
                    Err(restore) => Err(format!(
                        "mount: remount or fstab update failed: {error}; previous options restoration failed: {restore}"
                    )),
                };
            }
            Ok(())
        }
    }
}

fn remount(
    path: &str,
    old: &Mounted,
    options: &str,
    command: &mut impl FnMut(&str, &[&str]) -> Result<(), String>,
) -> Result<(), String> {
    let options = format!("remount,{options}");
    command(
        "mount",
        &["-t", &old.fstype, "-o", &options, "--", &old.source, path],
    )
}

fn run_mount_command(program: &str, args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    super::run_checked(command)?;
    Ok(())
}

fn mount_explicit(
    path: &str,
    mount: &Mounted,
    command: &mut impl FnMut(&str, &[&str]) -> Result<(), String>,
) -> Result<(), String> {
    command(
        "mount",
        &[
            "-t",
            &mount.fstype,
            "-o",
            &mount.options,
            "--",
            &mount.source,
            path,
        ],
    )
}

fn replace_mount(
    path: &str,
    old: &Mounted,
    desired: &Mounted,
    command: &mut impl FnMut(&str, &[&str]) -> Result<(), String>,
    persist: &mut impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    command("umount", &["--", path])?;
    if let Err(error) = mount_explicit(path, desired, command) {
        return match mount_explicit(path, old, command) {
            Ok(()) => Err(format!(
                "mount: replacement failed; previous mount restored: {error}"
            )),
            Err(restore) => Err(format!(
                "mount: replacement failed: {error}; previous mount restoration failed: {restore}"
            )),
        };
    }
    if let Err(error) = persist() {
        let restored =
            command("umount", &["--", path]).and_then(|()| mount_explicit(path, old, command));
        return match restored {
            Ok(()) => Err(format!(
                "mount: fstab update failed; previous mount restored: {error}"
            )),
            Err(restore) => Err(format!(
                "mount: fstab update failed: {error}; previous mount restoration failed: {restore}"
            )),
        };
    }
    Ok(())
}

fn rewrite_fstab(current: &str, src: &str, path: &str, fstype: &str, opts: &str) -> (String, bool) {
    let want = [src, path, fstype, opts, "0", "0"].map(escape_fstab);
    let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
    let mut found = false;
    let mut changed = false;
    for line in &mut lines {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let comment = line.char_indices().find_map(|(i, c)| {
            (c == '#' && i > 0 && line[..i].ends_with(char::is_whitespace)).then_some(i)
        });
        let data = comment.map_or(line.as_str(), |i| &line[..i]);
        let f: Vec<&str> = data.split_whitespace().collect();

        if f.len() >= 2 && unescape_mount(f[1]) == path {
            found = true;
            let matches = f.len() == 6
                && unescape_mount(f[0]) == src
                && unescape_mount(f[2]) == fstype
                && unescape_mount(f[3]) == opts
                && f[4] == "0"
                && f[5] == "0";
            if !matches {
                let suffix = comment.map(|i| {
                    let start = line[..i].trim_end().len();
                    line[start..].to_string()
                });
                *line = want.join("\t");
                if let Some(suffix) = suffix {
                    line.push_str(&suffix);
                }
                changed = true;
            }
            break;
        }
    }
    if !found {
        lines.push(want.join("\t"));
        changed = true;
    }
    if changed {
        let mut content = lines.join("\n");
        content.push('\n');
        (content, true)
    } else {
        (current.to_string(), false)
    }
}

fn escape_fstab(value: &str) -> String {
    value
        .replace('\\', "\\134")
        .replace(' ', "\\040")
        .replace('\t', "\\011")
        .replace('\n', "\\012")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MountAction {
    None,
    Mount,
    Remount,
    Replace,
}

struct Mounted {
    source: String,
    fstype: String,
    options: String,
}

fn unescape_mount(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn find_mount(mounts: &str, path: &str) -> Option<Mounted> {
    mounts
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.len() >= 4 && unescape_mount(fields[1]) == path).then(|| Mounted {
                source: unescape_mount(fields[0]),
                fstype: fields[2].into(),
                options: fields[3].into(),
            })
        })
        .next_back()
}

fn resolve_source(src: &str) -> Result<String, String> {
    let source = if let Some((kind, value)) = src.split_once('=') {
        if !matches!(kind, "UUID" | "LABEL" | "PARTUUID" | "PARTLABEL") {
            return Ok(src.into());
        }
        let mut command = std::process::Command::new("blkid");
        command.args(["-t", &format!("{kind}={value}"), "-o", "device"]);
        let out = super::run_checked(command)?;
        let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if result.is_empty() || result.lines().count() != 1 {
            return Err(format!("mount: cannot uniquely resolve {src}"));
        }
        result
    } else {
        src.to_string()
    };
    Ok(std::fs::canonicalize(&source).map_or(source, |p| p.to_string_lossy().into_owned()))
}

fn mount_action(current: Option<&Mounted>, src: &str, fstype: &str, opts: &str) -> MountAction {
    let Some(current) = current else {
        return MountAction::Mount;
    };
    let actual = std::fs::canonicalize(&current.source).map_or_else(
        |_| current.source.clone(),
        |p| p.to_string_lossy().into_owned(),
    );
    if actual != src || (fstype != "auto" && current.fstype != fstype) {
        return MountAction::Replace;
    }
    let actual_opts: Vec<_> = current.options.split(',').collect();
    let mut flags: [Option<bool>; 5] = if opts.split(',').any(|option| option == "defaults") {
        [Some(true); 5]
    } else {
        [None; 5]
    };
    let mut other_drift = false;
    for option in opts.split(',') {
        match option {
            "rw" => flags[0] = Some(true),
            "ro" => flags[0] = Some(false),
            "suid" => flags[1] = Some(true),
            "nosuid" => flags[1] = Some(false),
            "dev" => flags[2] = Some(true),
            "nodev" => flags[2] = Some(false),
            "exec" => flags[3] = Some(true),
            "noexec" => flags[3] = Some(false),
            "async" => flags[4] = Some(true),
            "sync" => flags[4] = Some(false),
            "defaults" | "auto" | "noauto" | "nofail" | "_netdev" | "user" | "nouser" => {}
            _ => other_drift |= !actual_opts.contains(&option),
        }
    }
    let negatives = ["ro", "nosuid", "nodev", "noexec", "sync"];
    let flag_drift = flags.iter().zip(negatives).any(|(requested, negative)| {
        requested.is_some_and(|positive| positive == actual_opts.contains(&negative))
    });
    if other_drift || flag_drift {
        MountAction::Remount
    } else {
        MountAction::None
    }
}

#[cfg(test)]
#[path = "mount/tests/core.rs"]
mod tests_core;

#[cfg(test)]
#[path = "mount/tests/arguments.rs"]
mod tests_arguments;

#[cfg(test)]
#[path = "mount/tests/replacement.rs"]
mod tests_replacement;

#[cfg(test)]
#[path = "mount/tests/guards.rs"]
mod tests_guards;
