use super::ExecContext;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

const RUNUSER: &str = "/usr/sbin/runuser";
/// Host programs that resolve the switched account. They are dynamically
/// linked, so they use the host's NSS stack (files, SSSD, LDAP), which the
/// static agent cannot load.
const HOST_IDENTITY: IdentityLookup<'static> = IdentityLookup {
    timeout: "/usr/bin/timeout",
    getent: "/usr/bin/getent",
    id: "/usr/bin/id",
};
/// Seconds the in-context NSS lookup may take before the task fails.
const IDENTITY_TIMEOUT_SECS: u8 = 10;
/// Bytes of `getent` output (record and status) the bootstrap accepts.
const IDENTITY_RECORD_LIMIT: usize = 4096;
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
        switched_command(
            ctx,
            user,
            program,
            args,
            Path::new(RUNUSER),
            std::env::vars_os().collect(),
            Some(&HOST_IDENTITY),
        )
    } else {
        let mut command = Command::new(program);
        command.args(args).envs(ctx.environment.iter().cloned());
        Ok(piped_command(command, None))
    }
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

/// Programs run inside the switched context to read the account's identity.
pub(super) struct IdentityLookup<'a> {
    pub timeout: &'a str,
    pub getent: &'a str,
    pub id: &'a str,
}

/// Become users are account names. All-digit names are refused: util-linux
/// `runuser` (`xgetuserpw`) and `getent passwd` both treat a numeric key as a
/// UID, so the name could select a different account.
pub(super) fn valid_become_user(user: &str) -> bool {
    !user.is_empty()
        && !user.starts_with('-')
        && !user.bytes().all(|b| b.is_ascii_digit())
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Shell code, run as the switched user, that resolves the account by name
/// through the host NSS stack within bounded time and size, proves it is the account `runuser` switched to
/// (its UID equals `id -u`), and exports its HOME, SHELL, USER and LOGNAME.
fn identity_script(user: &str, lookup: &IdentityLookup<'_>) -> Result<String, String> {
    let quoted = |value: &str| (!value.contains(['\'', '\0'])).then(|| format!("'{value}'"));
    let timeout = quoted(lookup.timeout).ok_or("invalid identity lookup program")?;
    let getent = quoted(lookup.getent).ok_or("invalid identity lookup program")?;
    let id = quoted(lookup.id).ok_or("invalid identity lookup program")?;
    // The record and getent's status are read through head, so an oversized
    // NSS answer is never buffered whole; reaching the limit fails closed.
    let limit = IDENTITY_RECORD_LIMIT + 1;
    Ok(format!(
        "RUXEL_ID_NL=$(printf '\\nx'); RUXEL_ID_NL=${{RUXEL_ID_NL%x}}; \
         RUXEL_ID_OUT=$({{ {timeout} {IDENTITY_TIMEOUT_SECS} {getent} passwd -- '{user}'; \
         printf '\\n%s' \"$?\"; }} | /usr/bin/head -c {limit}) || exit 125; \
         [ ${{#RUXEL_ID_OUT}} -lt {limit} ] || exit 125; \
         [ \"${{RUXEL_ID_OUT##*\"$RUXEL_ID_NL\"}}\" = 0 ] || exit 125; \
         RUXEL_ID_RECORD=${{RUXEL_ID_OUT%\"$RUXEL_ID_NL\"*}}; \
         RUXEL_ID_RECORD=${{RUXEL_ID_RECORD%\"$RUXEL_ID_NL\"}}; \
         RUXEL_ID_UID=$({id} -u) || exit 125; \
         case $RUXEL_ID_RECORD in *\"$RUXEL_ID_NL\"*) exit 125;; esac; \
         RUXEL_ID_NAME=${{RUXEL_ID_RECORD%%:*}}; RUXEL_ID_REST=${{RUXEL_ID_RECORD#*:}}; \
         RUXEL_ID_REST=${{RUXEL_ID_REST#*:}}; \
         [ \"${{RUXEL_ID_REST%%:*}}\" = \"$RUXEL_ID_UID\" ] || exit 125; \
         RUXEL_ID_REST=${{RUXEL_ID_REST#*:}}; RUXEL_ID_REST=${{RUXEL_ID_REST#*:}}; \
         RUXEL_ID_REST=${{RUXEL_ID_REST#*:}}; \
         case $RUXEL_ID_REST in *:*:*) exit 125;; *:*) ;; *) exit 125;; esac; \
         HOME=${{RUXEL_ID_REST%%:*}}; SHELL=${{RUXEL_ID_REST#*:}}; \
         [ -n \"$SHELL\" ] || SHELL=/bin/sh; \
         [ -n \"$RUXEL_ID_NAME\" ] && [ -n \"$HOME\" ] || exit 125; \
         USER=$RUXEL_ID_NAME; LOGNAME=$RUXEL_ID_NAME; \
         export HOME SHELL USER LOGNAME || exit 125; "
    ))
}

fn switched_command(
    ctx: &ExecContext,
    user: &str,
    program: &str,
    args: &[&str],
    wrapper: &Path,
    mut environment: BTreeMap<OsString, OsString>,
    identity: Option<&IdentityLookup<'_>>,
) -> Result<TaskCommand, String> {
    if !valid_become_user(user) {
        return Err("invalid become user".into());
    }
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
    let mut script = String::new();
    if let Some(lookup) = identity {
        // The switched account, not the agent, supplies these four values;
        // explicit task environment entries still override them below.
        for key in ["HOME", "USER", "LOGNAME", "SHELL"] {
            environment.remove(std::ffi::OsStr::new(key));
        }
        script.push_str(&identity_script(user, lookup)?);
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

#[cfg(test)]
#[path = "tests/subprocess.rs"]
mod tests;
