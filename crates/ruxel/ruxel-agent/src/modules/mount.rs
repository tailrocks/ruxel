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
    if fstab_changed && !ctx.check_mode {
        write_atomic(std::path::Path::new(FSTAB), content.as_bytes())?;
    }

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
        if !ctx.check_mode {
            std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
            if action == MountAction::Replace {
                let mut command = std::process::Command::new("umount");
                command.arg(path);
                super::run_checked(command)?;
            }
            let mut command = std::process::Command::new("mount");
            if action == MountAction::Remount {
                command.args(["-o", "remount"]);
            }
            command.arg(path);
            super::run_checked(command)?;
        }
    }

    Ok(json!({"changed": changed, "failed": false, "name": path, "src": src, "fstype": fstype}))
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
        let f: Vec<&str> = t.split_whitespace().collect();

        if f.len() >= 2 && unescape_mount(f[1]) == path {
            found = true;
            let matches = f.len() >= 4
                && unescape_mount(f[0]) == src
                && unescape_mount(f[2]) == fstype
                && opts_eq(&unescape_mount(f[3]), opts);
            if !matches {
                *line = want.join("\t");
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
}

fn opts_eq(a: &str, b: &str) -> bool {
    let mut sa: Vec<&str> = a.split(',').collect();
    let mut sb: Vec<&str> = b.split(',').collect();
    sa.sort_unstable();
    sb.sort_unstable();
    sa == sb
}

#[derive(Debug, PartialEq, Eq)]
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
    if actual != src || current.fstype != fstype {
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
            "defaults" => {}
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
            "auto" | "noauto" | "nofail" | "_netdev" | "user" | "nouser" => {}
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
mod tests {
    use serde_json::json;

    #[test]
    fn rejects_newline_in_fstab_fields_before_io() {
        let ctx = super::ExecContext {
            check_mode: true,
            diff_mode: false,
            no_log: false,
            environment: vec![],
            become_user: None,
        };
        for (field, value) in [
            ("path", "/mnt\nbad"),
            ("src", "/dev/x\rbad"),
            ("fstype", "xfs\nbad"),
            ("opts", "defaults\nbad"),
        ] {
            let mut params =
                json!({"path": "/mnt/x", "src": "/dev/x", "fstype": "xfs", "opts": "defaults"});
            params[field] = json!(value);
            assert!(super::run(&params, &ctx).is_err(), "field {field}");
        }
    }

    #[test]
    fn identifies_mount_drift() {
        use super::{MountAction, find_mount, mount_action};
        let mounts = "/dev/a /srv xfs rw,noatime 0 0\n";
        let current = find_mount(mounts, "/srv");
        assert_eq!(
            mount_action(current.as_ref(), "/dev/a", "xfs", "noatime,rw"),
            MountAction::None
        );
        assert_eq!(
            mount_action(current.as_ref(), "/dev/a", "xfs", "ro"),
            MountAction::Remount
        );
        assert_eq!(
            mount_action(current.as_ref(), "/dev/a", "xfs", "defaults"),
            MountAction::None
        );
        for option in ["ro", "nosuid", "nodev", "noexec", "sync"] {
            let mounts = format!("/dev/a /srv xfs rw,{option} 0 0\n");
            assert_eq!(
                mount_action(
                    find_mount(&mounts, "/srv").as_ref(),
                    "/dev/a",
                    "xfs",
                    "defaults"
                ),
                MountAction::Remount
            );
        }
        let restricted = find_mount("/dev/a /srv xfs rw,noexec 0 0\n", "/srv");
        assert_eq!(
            mount_action(restricted.as_ref(), "/dev/a", "xfs", "defaults,noexec"),
            MountAction::None
        );
        assert_eq!(
            mount_action(restricted.as_ref(), "/dev/a", "xfs", "noexec,defaults"),
            MountAction::None
        );
        assert_eq!(
            mount_action(current.as_ref(), "/dev/b", "xfs", "rw"),
            MountAction::Replace
        );
        assert_eq!(
            mount_action(None, "/dev/a", "xfs", "rw"),
            MountAction::Mount
        );
    }

    #[test]
    fn fstab_escapes_fields_and_converges() {
        let (next, changed) =
            super::rewrite_fstab("", "/dev/a b", "/target with space", "ext4", "rw");
        assert!(changed);
        assert_eq!(
            next,
            "/dev/a\\040b\t/target\\040with\\040space\text4\trw\t0\t0\n"
        );
        assert_eq!(
            super::rewrite_fstab(&next, "/dev/a b", "/target with space", "ext4", "rw"),
            (next, false)
        );
        let (next, _) = super::rewrite_fstab("", "/dev/a\\b", "/mnt/a\tb", "ext4", "rw");
        assert!(next.contains("/dev/a\\134b\t/mnt/a\\011b"));
        assert_eq!(
            super::rewrite_fstab(&next, "/dev/a\\b", "/mnt/a\tb", "ext4", "rw"),
            (next, false)
        );
    }

    #[test]
    fn normalizes_options_and_rewrites_matching_mountpoint() {
        assert!(super::opts_eq("rw,noatime", "noatime,rw"));
        let current = "# keep\n/dev/old /srv ext4 defaults 0 0\n";
        let (next, changed) = super::rewrite_fstab(current, "/dev/new", "/srv", "xfs", "rw");
        assert!(changed);
        assert_eq!(next, "# keep\n/dev/new\t/srv\txfs\trw\t0\t0\n");
        assert_eq!(
            super::rewrite_fstab(&next, "/dev/new", "/srv", "xfs", "rw"),
            (next, false)
        );
    }
}
