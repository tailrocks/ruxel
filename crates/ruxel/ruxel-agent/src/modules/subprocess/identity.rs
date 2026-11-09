use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) fn helper(user: &str) -> i32 {
    let Ok(Some(account)) = nix::unistd::User::from_name(user) else {
        return 1;
    };
    let (Some(home), Some(shell)) = (account.dir.to_str(), account.shell.to_str()) else {
        return 1;
    };
    if account.name != user
        || [user, home, shell].iter().any(|field| {
            field
                .bytes()
                .any(|byte| matches!(byte, b':' | b'\n' | b'\r' | 0))
        })
    {
        return 1;
    }
    let record = format!(
        "{}:x:{}:{}::{}:{}\n",
        user, account.uid, account.gid, home, shell
    );
    i32::from(std::io::stdout().write_all(record.as_bytes()).is_err())
}

#[expect(clippy::let_underscore_must_use, reason = "best-effort child cleanup")]
pub(crate) fn lookup_user(user: &str) -> Result<Vec<u8>, String> {
    const LIMIT: usize = 64 * 1024;
    #[cfg(target_os = "linux")]
    let executable = std::path::PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let executable =
        std::env::current_exe().map_err(|_| "failed to look up become_user identity")?;
    let mut child = Command::new(executable)
        .env_clear()
        .args(["--ruxel-nss-helper", user])
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

pub(crate) fn user_environment_from<'a>(passwd: &'a str, user: &str) -> Option<(&'a str, &'a str)> {
    let mut lines = passwd.lines();
    let fields = lines.next()?.split(':').collect::<Vec<_>>();
    if lines.next().is_some()
        || fields.len() != 7
        || fields[0].is_empty()
        || fields[0] != user
        || fields[2].parse::<u32>().is_err()
        || fields[3].parse::<u32>().is_err()
        || fields.iter().any(|field| field.contains('\0'))
    {
        return None;
    }
    Some((fields[5], fields[6]))
}
