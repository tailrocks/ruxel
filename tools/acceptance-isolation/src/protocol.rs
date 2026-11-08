use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
};
fn hello(input: &mut impl Read) -> Result<Vec<u8>> {
    let mut header = Vec::new();
    let mut length = 0usize;
    for shift in (0..=21).step_by(7) {
        let mut byte = [0];
        input.read_exact(&mut byte)?;
        header.push(byte[0]);
        length |= usize::from(byte[0] & 127) << shift;
        if byte[0] < 128 {
            ensure!(
                length > 0 && length <= 4 * 1024 * 1024,
                "invalid HelloAck size"
            );
            let mut body = vec![0; length];
            input.read_exact(&mut body)?;
            ensure!(body[0] == 10, "first frame is not HelloAck");
            header.extend(body);
            return Ok(header);
        }
    }
    anyhow::bail!("oversized frame length")
}
pub fn hello_then_kill(agent: &Path) -> Result<()> {
    let mut child = Command::new(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().context("missing agent input")?;
    thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut input);
    });
    let result: Result<()> = (|| {
        let frame = hello(&mut child.stdout.take().context("missing agent output")?)?;
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&frame)?;
        stdout.flush()?;
        Ok(())
    })();
    let _ = child.kill();
    let waited = child.wait();
    result?;
    waited?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_frame_preserved() {
        assert_eq!(hello(&mut &[3, 10, 7, 8][..]).unwrap(), [3, 10, 7, 8]);
    }
    #[test]
    fn invalid_frames_rejected() {
        for bytes in [
            &[0][..],
            &[2, 10][..],
            &[1, 9][..],
            &[255, 255, 255, 255, 255][..],
        ] {
            assert!(hello(&mut &*bytes).is_err());
        }
    }
}
