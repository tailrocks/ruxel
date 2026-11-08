use super::ExecContext;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const RUNUSER: &str = "/usr/sbin/runuser";
const WRAPPER_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
const READY: &[u8] = b"RUXEL_ENV_READY\n";
// read consumes exactly one line, leaving the rest of stdin for the workload.
// The evaluated line is generated solely from validated names and octal bytes.
const BOOTSTRAP: &str = "IFS= read -r RUXEL_ENV_SCRIPT || exit 125; eval \"$RUXEL_ENV_SCRIPT\" || exit 125; printf 'RUXEL_ENV_READY\\n'; exec \"$@\"";

pub(super) struct TaskCommand {
    command: Command,
    environment_script: Option<Vec<u8>>,
}

impl TaskCommand {
    pub(super) fn current_dir(&mut self, path: &str) {
        self.command.current_dir(path);
    }

    pub(super) fn spawn(&mut self) -> Result<Child, std::io::Error> {
        let mut child = self.command.spawn()?;
        if let Some(script) = &self.environment_script {
            let result = (|| {
                let stdin = child.stdin.as_mut().ok_or_else(bootstrap_error)?;
                stdin.write_all(script)?;
                stdin.flush()?;
                // No buffering here: consuming workload stdout would corrupt
                // persistent PostgreSQL sessions. The marker is emitted before
                // exec and cannot contain environment values.
                let mut ready = [0; READY.len()];
                child
                    .stdout
                    .as_mut()
                    .ok_or_else(bootstrap_error)?
                    .read_exact(&mut ready)?;
                if ready != READY {
                    return Err(bootstrap_error());
                }
                Ok(())
            })();
            if result.is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(bootstrap_error());
            }
        }
        Ok(child)
    }

    pub(super) fn output(&mut self) -> Result<Output, std::io::Error> {
        // wait_with_output closes stdin after the bootstrap. Session callers
        // instead keep the returned child's stdin for subsequent queries.
        self.spawn()?.wait_with_output()
    }
}

fn bootstrap_error() -> std::io::Error {
    std::io::Error::other("privilege environment bootstrap failed")
}

pub(super) fn become_command(
    ctx: &ExecContext,
    program: &str,
    args: &[&str],
) -> Result<TaskCommand, String> {
    for (key, value) in &ctx.environment {
        if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
            return Err("invalid task environment variable".into());
        }
    }
    if let Some(user) = &ctx.become_user {
        if user.is_empty()
            || user.starts_with('-')
            || !user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("invalid become user".into());
        }
        let output = lookup_user(user)?;
        let passwd =
            std::str::from_utf8(&output).map_err(|_| "invalid become_user identity record")?;
        let identity =
            user_environment_from(passwd, user).ok_or("invalid become_user identity record")?;
        let canonical_user = passwd
            .split(':')
            .next()
            .ok_or("invalid become_user identity record")?;
        switched_command(
            ctx,
            canonical_user,
            program,
            args,
            Path::new(RUNUSER),
            std::env::vars_os().collect(),
            Some(identity),
        )
    } else {
        let mut command = Command::new(program);
        command.args(args).envs(ctx.environment.iter().cloned());
        Ok(piped_command(command, None))
    }
}

fn lookup_user(user: &str) -> Result<Vec<u8>, String> {
    const LIMIT: usize = 64 * 1024;
    let mut child = Command::new("/usr/bin/getent")
        .env_clear()
        .env("PATH", WRAPPER_PATH)
        .args(["--", "passwd", user])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "failed to look up become_user identity")?;
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or("identity lookup failed")?;
        let flags = rustix::fs::fcntl_getfl(&stdout).map_err(|_| "identity lookup failed")?;
        rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| "identity lookup failed")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        let mut eof = false;
        let mut status = None;
        loop {
            let mut buffer = [0; 4096];
            if !eof {
                match stdout.read(&mut buffer) {
                    Ok(0) => eof = true,
                    Ok(n) if output.len() + n <= LIMIT => output.extend_from_slice(&buffer[..n]),
                    Ok(_) => return Err("identity lookup output too large"),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err("identity lookup failed"),
                }
            }
            if status.is_none() {
                status = child.try_wait().map_err(|_| "identity lookup failed")?;
            }
            if eof && let Some(status) = status {
                return if status.success() {
                    Ok(output)
                } else {
                    Err("identity lookup failed")
                };
            }
            if Instant::now() >= deadline {
                return Err("identity lookup timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result.map_err(str::to_string)
}

fn piped_command(mut command: Command, environment_script: Option<Vec<u8>>) -> TaskCommand {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    TaskCommand {
        command,
        environment_script,
    }
}

pub(super) fn psql_command(ctx: &ExecContext, args: &[&str]) -> Result<TaskCommand, String> {
    if ctx.become_user.is_some() {
        return become_command(ctx, "psql", args);
    }
    for (key, value) in &ctx.environment {
        if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
            return Err("invalid task environment variable".into());
        }
    }
    #[cfg(test)]
    let executable =
        std::env::var_os("RUXEL_TEST_PSQL_PATH").unwrap_or_else(|| "/usr/bin/psql".into());
    #[cfg(not(test))]
    let executable = "/usr/bin/psql";
    let mut command = Command::new(executable);
    command.args(args).envs(ctx.environment.iter().cloned());
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(untrusted_loader_environment) {
            command.env_remove(key);
        }
    }
    for (key, _) in &ctx.environment {
        if untrusted_loader_environment(key) {
            command.env_remove(key);
        }
    }
    command.env("PATH", WRAPPER_PATH);
    Ok(piped_command(command, None))
}

fn untrusted_loader_environment(key: &str) -> bool {
    key.starts_with("LD_")
        || key.starts_with("DYLD_")
        || key.starts_with("PERL5")
        || matches!(
            key,
            "PERLLIB" | "GCONV_PATH" | "LOCPATH" | "GLIBC_TUNABLES" | "BASH_ENV" | "ENV"
        )
}

fn switched_command(
    ctx: &ExecContext,
    user: &str,
    program: &str,
    args: &[&str],
    wrapper: &Path,
    mut environment: BTreeMap<OsString, OsString>,
    identity: Option<(&str, &str)>,
) -> Result<TaskCommand, String> {
    // Only this fixed environment reaches the privileged loader and runuser.
    // Workload environment is sent over stdin after switching identity, so
    // passwords never become command arguments or privileged loader inputs.
    let mut command = Command::new(wrapper);
    command.env_clear().env("PATH", WRAPPER_PATH);
    command.args([
        "-u",
        user,
        "--",
        "/usr/bin/env",
        "-i",
        "/bin/sh",
        "-c",
        BOOTSTRAP,
        "ruxel-env",
    ]);
    if let Some((home, shell)) = identity {
        for (key, value) in [
            ("HOME", home),
            ("USER", user),
            ("LOGNAME", user),
            ("SHELL", shell),
        ] {
            environment.insert(key.into(), value.into());
        }
    }
    for (key, value) in &ctx.environment {
        environment.insert(key.into(), value.into());
    }
    let has_shell_level = environment.contains_key(std::ffi::OsStr::new("SHLVL"));
    if !has_shell_level {
        // Bash creates SHLVL even if unset. This post-drop env invocation
        // removes that shell metadata without putting a value into argv.
        if program.contains('=') {
            return Err("become_user requires SHLVL for assignment-like executable names".into());
        }
        command.args(["/usr/bin/env", "-u", "SHLVL", "--"]);
    }
    command.arg(program).args(args);
    let mut scratch = String::from("RUXEL_ENV_VALUE");
    while environment.contains_key(std::ffi::OsStr::new(&scratch)) {
        scratch.push('_');
    }
    let mut script = String::new();
    for (key, value) in environment {
        let key = key
            .to_str()
            .filter(|key| shell_name(key) && !shell_managed_name(key))
            .ok_or("become_user requires shell-compatible environment variable names")?;
        // Bash decrements SHLVL on exec; the fixed bootstrap compensates for
        // that, while dash does not. Restrict its numeric shell representation.
        if key == "SHLVL"
            && !value.to_str().is_some_and(|value| {
                value
                    .parse::<u16>()
                    .is_ok_and(|level| level < 999 && level.to_string() == value)
            })
        {
            return Err("become_user requires a numeric SHLVL below 999".into());
        }
        // A trailing sentinel preserves trailing newlines through command
        // substitution. Values are bytes, never shell syntax, even for quotes,
        // newlines, command substitutions, or non-UTF-8 inherited values.
        script.push_str(&scratch);
        script.push_str("=$(printf '");
        for byte in value.as_bytes() {
            use std::fmt::Write as _;
            write!(script, "\\{byte:03o}").map_err(|_| "environment encoding failed")?;
        }
        script.push_str("x'); ");
        script.push_str("export ");
        script.push_str(key);
        script.push_str("=\"${");
        script.push_str(&scratch);
        script.push_str("%x}\"");
        script.push_str(" || exit 125; ");
    }
    if has_shell_level {
        script.push_str(
            "if [ -n \"${BASH_VERSION-}\" ]; then SHLVL=$((SHLVL + 1)); export SHLVL; fi; ",
        );
    }
    script.push_str(":\n");
    Ok(piped_command(command, Some(script.into_bytes())))
}

fn shell_managed_name(key: &str) -> bool {
    // These names have shell arithmetic, array, identity, or descriptor side
    // effects. Reject them before launching anything instead of interpreting
    // their values as shell state. SHLVL remains a normal numeric assignment.
    (key.starts_with("BASH_") && key != "BASH_ENV")
        || matches!(
            key,
            "RANDOM"
                | "SRANDOM"
                | "SECONDS"
                | "OPTIND"
                | "LINENO"
                | "HISTCMD"
                | "BASHPID"
                | "UID"
                | "EUID"
                | "PPID"
                | "GROUPS"
                | "DIRSTACK"
                | "FUNCNAME"
                | "PIPESTATUS"
                | "BASHOPTS"
                | "SHELLOPTS"
                | "BASH_XTRACEFD"
                | "BASH_VERSINFO"
                | "BASH_ARGV"
                | "BASH_ARGC"
                | "BASH_LINENO"
                | "BASH_SOURCE"
        )
}

fn shell_name(key: &str) -> bool {
    let mut bytes = key.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) fn user_environment_from<'a>(
    passwd: &'a str,
    _user: &str,
) -> Option<(&'a str, &'a str)> {
    let mut lines = passwd.lines();
    let fields = lines.next()?.split(':').collect::<Vec<_>>();
    if lines.next().is_some()
        || fields.len() != 7
        || fields[0].is_empty()
        || fields[2].parse::<u32>().is_err()
        || fields[3].parse::<u32>().is_err()
        || fields.iter().any(|field| field.contains('\0'))
    {
        return None;
    }
    Some((fields[5], fields[6]))
}

#[cfg(test)]
#[path = "tests/subprocess.rs"]
mod tests;
