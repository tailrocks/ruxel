use super::WRAPPER_PATH;
use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn lookup_user(user: &str) -> Result<Vec<u8>, String> {
    const LIMIT: usize = 64 * 1024;
    if user.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("numeric become_user names cannot be resolved safely".into());
    }
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

pub(in crate::modules) fn user_environment_from<'a>(
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
