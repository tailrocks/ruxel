use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    thread,
};
pub const PROXY_ARG: &str = "--internal-acceptance-agent-proxy";
fn varint(bytes: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*offset).context("truncated varint")?;
        *offset += 1;
        ensure!(shift < 64 && (shift != 63 || byte <= 1), "invalid varint");
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    bail!("invalid varint")
}
fn fields(mut bytes: &[u8], mut found: impl FnMut(u64, &[u8]) -> Result<()>) -> Result<()> {
    while !bytes.is_empty() {
        let mut pos = 0;
        let tag = varint(bytes, &mut pos)?;
        let field = tag >> 3;
        let wire = tag & 7;
        let size = match wire {
            0 => {
                let start = pos;
                varint(bytes, &mut pos)?;
                let size = pos - start;
                pos = start;
                size
            }
            1 => 8,
            2 => usize::try_from(varint(bytes, &mut pos)?)?,
            5 => 4,
            _ => bail!("invalid protobuf wire type"),
        };
        let end = pos.checked_add(size).context("field overflow")?;
        let value = bytes.get(pos..end).context("truncated field")?;
        found(field, value)?;
        bytes = &bytes[end..];
    }
    Ok(())
}
pub fn forward() -> Result<()> {
    let agent = std::env::var_os("RUXEL_RUNTIME_PROXY_AGENT").context("missing agent")?;
    let labels = std::env::var_os("RUXEL_RUNTIME_PROXY_LABELS").context("missing labels")?;
    let mut child = Command::new(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().context("missing input")?;
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut input);
    });
    let mut reader = child.stdout.take().context("missing output")?;
    let mut stdout = io::stdout().lock();
    loop {
        let mut header = Vec::new();
        loop {
            let mut byte = [0];
            let n = reader.read(&mut byte)?;
            if n == 0 {
                ensure!(header.is_empty(), "truncated frame");
                let status = child.wait()?;
                ensure!(status.success(), "agent exited {status}");
                return Ok(());
            }
            header.push(byte[0]);
            ensure!(header.len() <= 10, "invalid frame length");
            if byte[0] < 128 {
                break;
            }
        }
        let mut pos = 0;
        let len = usize::try_from(varint(&header, &mut pos)?)?;
        ensure!(len <= 64 * 1024 * 1024, "frame too large");
        let mut body = vec![0; len];
        reader.read_exact(&mut body)?;
        fields(&body, |field, value| {
            if field == 3 {
                fields(value, |field, label| {
                    if field == 2 {
                        let mut file =
                            OpenOptions::new().create(true).append(true).open(&labels)?;
                        writeln!(
                            file,
                            "{}",
                            serde_json::to_string(std::str::from_utf8(label)?)?
                        )?;
                    }
                    Ok(())
                })?;
            }
            Ok(())
        })?;
        stdout.write_all(&header)?;
        stdout.write_all(&body)?;
        stdout.flush()?;
    }
}
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
pub fn wrapper(base: &Path, agent: &Path, labels: &Path) -> Result<std::path::PathBuf> {
    let path = base.join("agent-proxy");
    let exe = std::env::current_exe()?;
    let script = format!(
        "#!/bin/sh\nexport RUXEL_RUNTIME_PROXY_AGENT={}\nexport RUXEL_RUNTIME_PROXY_LABELS={}\nexec {} {}\n",
        quote(agent),
        quote(labels),
        quote(&exe),
        PROXY_ARG
    );
    fs::write(&path, script)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protobuf_fields_handles_varints_and_length_delimited_values() -> Result<()> {
        let mut seen = Vec::new();
        fields(&[8, 150, 1, 18, 3, b'o', b'n', b'e'], |number, data| {
            seen.push((number, data.to_vec()));
            Ok(())
        })?;
        ensure!(
            seen == [(1, vec![150, 1]), (2, b"one".to_vec())],
            "incorrect field parsing"
        );
        ensure!(
            fields(&[18, 4, b'x'], |_, _| Ok(())).is_err(),
            "truncated field accepted"
        );
        Ok(())
    }
}
