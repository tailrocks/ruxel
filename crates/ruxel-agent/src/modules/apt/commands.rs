use super::{CommandResult, ExecContext};
use serde_json::{Value, json};

/// apt-get and apt-mark share one command: the arguments name the
/// operation, so `showhold`, `hold` and `unhold` run apt-mark and every
/// other argument runs apt-get.
pub(super) fn apt_command(args: &[&str], ctx: &ExecContext) -> CommandResult {
    match args.first() {
        Some(&"showhold" | &"hold" | &"unhold") => apt_mark(args, ctx),
        _ => apt_get(args, ctx),
    }
}

/// `apt-mark` asks no questions and changes nothing but the marks, so it
/// needs neither `-y` nor the dpkg options apt-get carries.
fn apt_mark(args: &[&str], ctx: &ExecContext) -> CommandResult {
    let mut cmd = std::process::Command::new("apt-mark");
    cmd.args(args);
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    force_stable_locale(&mut cmd);
    let out = cmd.output().map_err(|e| format!("exec apt-mark: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn apt_get(args: &[&str], ctx: &ExecContext) -> CommandResult {
    let mut cmd = std::process::Command::new("apt-get");
    // A known output level before the summaries are parsed: a host
    // apt.conf that sets `quiet` suppresses every summary line and a
    // successful changing run would report `changed: false`. `quiet` is
    // a configuration item and `-q=#` is apt-get(8)'s command-line
    // override, so level 0 restores the summaries (and the format-30
    // lines) beside the stable locale `LC_ALL=C` forces below.
    cmd.arg("-y")
        .arg("-q=0")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confdef")
        .arg("-o")
        .arg("Dpkg::Options::=--force-confold");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    for (k, v) in &ctx.environment {
        cmd.env(k, v);
    }
    force_stable_locale(&mut cmd);
    let out = cmd.output().map_err(|e| format!("exec apt-get: {e}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

/// A stable locale for the apt subprocesses: `summary_changed` recognizes
/// only the English summary strings (`upgraded,`, `newly installed`,
/// `not fully installed or removed`), and apt-get prints translated prose
/// under a non-C `LANG` or `LC_MESSAGES`, so a successful localized run
/// would report no change. Forced after the task environment, like the
/// `apt-cache` reader of the package state.
fn force_stable_locale(cmd: &mut std::process::Command) {
    cmd.env("LC_ALL", "C");
}

/// The numbers one apt-get output line names, in the order they appear.
fn numbers_of(line: &str) -> Vec<u64> {
    line.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Whether the apt-get output reports a change. The summary counters —
/// `N upgraded, N newly installed, ...` — name the packages the run
/// moves, and only their first three count: the last one is the packages
/// left not upgraded. A run that merely configures an unpacked package or
/// processes pending triggers moves none of them but still changes dpkg
/// state, and apt-get reports that pending work on its own line —
/// `N not fully installed or removed.` A maintenance step must report
/// that change too, or the watchers of the system-upgrade step never run.
pub(super) fn summary_changed(stdout: &str) -> bool {
    for line in stdout.lines() {
        if line.contains("upgraded,") && line.contains("newly installed") {
            if numbers_of(line).iter().take(3).any(|n| *n > 0) {
                return true;
            }
        } else if line.contains("Upgrading:")
            && line.contains("Installing:")
            && line.contains("Removing:")
        {
            // Apt's format-30 summary names the same three counters
            // (`Upgrading: 1, Installing: 0, Removing: 0, Not Upgrading:
            // 0`) in the order the legacy line lists them: the first three
            // are the packages the run moves, the last one the packages
            // left behind. Both formats are recognized rather than one
            // forced on the host: `LC_ALL=C` stabilizes the language, not
            // the format, and a host apt.conf can select the format for
            // the agent's commands too.
            if numbers_of(line).iter().take(3).any(|n| *n > 0) {
                return true;
            }
        } else if line.contains("not fully installed or removed")
            && numbers_of(line).iter().any(|n| *n > 0)
        {
            return true;
        }
    }
    false
}

pub(super) fn fill_exec_fields(result: &mut Value, stdout: &str, stderr: &str, with_msg: bool) {
    let stdout = stdout.trim_end_matches('\n');
    let stderr = stderr.trim_end_matches('\n');
    let lines = |s: &str| -> Vec<String> {
        if s.is_empty() {
            vec![]
        } else {
            s.lines().map(str::to_string).collect()
        }
    };
    if with_msg {
        result["msg"] = json!(stdout);
    }
    result["stdout"] = json!(stdout);
    result["stderr"] = json!(stderr);
    result["stdout_lines"] = json!(lines(stdout));
    result["stderr_lines"] = json!(lines(stderr));
}
