use super::{Fixture, Result};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
struct Request<'a>(&'a [u8]);
impl Request<'_> {
    fn bytes(&mut self, size: usize) -> Result<&[u8]> {
        let (value, remaining) = self.0.split_at_checked(size).ok_or("short SFTP packet")?;
        self.0 = remaining;
        Ok(value)
    }
    fn number(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.bytes(4)?.try_into()?))
    }
    fn string(&mut self) -> Result<Vec<u8>> {
        let size = self.number()? as usize;
        Ok(self.bytes(size)?.to_vec())
    }
    fn text(&mut self) -> Result<String> {
        Ok(String::from_utf8(self.string()?)?)
    }
    fn end(&self) {
        assert!(self.0.is_empty());
    }
}
fn string(value: &[u8]) -> Result<Vec<u8>> {
    let mut data = u32::try_from(value.len())?.to_be_bytes().to_vec();
    data.extend_from_slice(value);
    Ok(data)
}
fn packet(output: &mut impl Write, data: &[u8]) -> Result<()> {
    output.write_all(&u32::try_from(data.len())?.to_be_bytes())?;
    output.write_all(data)?;
    output.flush()?;
    Ok(())
}
fn status(id: &[u8], code: u32, message: &[u8]) -> Result<Vec<u8>> {
    let mut data = vec![101];
    data.extend_from_slice(id);
    data.extend_from_slice(&code.to_be_bytes());
    data.extend(string(message)?);
    data.extend(string(b"")?);
    Ok(data)
}
pub(super) fn serve(fixture: &Fixture) -> Result<()> {
    let mut handles: HashMap<Vec<u8>, File> = HashMap::new();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let mut prefix = [0; 4];
        if input.read(&mut prefix[..1])? == 0 {
            return Ok(());
        }
        input.read_exact(&mut prefix[1..])?;
        let size = u32::from_be_bytes(prefix) as usize;
        assert!(size <= 16 * 1024 * 1024);
        let mut data = vec![0; size];
        input.read_exact(&mut data)?;
        let mut request = Request(&data);
        let kind = request.bytes(1)?[0];
        if kind == 1 {
            assert_eq!(request.number()?, 3);
            request.end();
            let mut version = vec![2, 0, 0, 0, 3];
            version.extend(string(b"posix-rename@openssh.com")?);
            version.extend(string(b"1")?);
            packet(&mut output, &version)?;
            continue;
        }
        let id = request.bytes(4)?.to_vec();
        match kind {
            3 => {
                let path = request.text()?;
                let flags = request.number()?;
                assert_eq!(flags & (0x2 | 0x8 | 0x20), 0x2 | 0x8 | 0x20);
                // OPEN attributes are followed by descriptor FSETSTAT below.
                let handle = handles.len().to_string().into_bytes();
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(fixture.local(&path))?;
                handles.insert(handle.clone(), file);
                fixture.record_json(&format!("[\"open\",{},{flags}]", super::quote(&path)))?;
                let mut reply = vec![102];
                reply.extend(&id);
                reply.extend(string(&handle)?);
                packet(&mut output, &reply)?;
                continue;
            }
            6 => {
                let handle = request.string()?;
                let offset = u64::from_be_bytes(request.bytes(8)?.try_into()?);
                let content = request.string()?;
                request.end();
                if fixture.root.join("sftp-fail").exists() {
                    packet(&mut output, &status(&id, 4, b"synthetic write failure")?)?;
                    continue;
                }
                let file = handles.get_mut(&handle).ok_or("unknown write handle")?;
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(&content)?;
            }
            10 => {
                let handle = request.string()?;
                assert_eq!(request.number()?, 4);
                let mode = request.number()?;
                assert_eq!(mode, 0o600);
                request.end();
                handles
                    .get(&handle)
                    .ok_or("unknown permissions handle")?
                    .set_permissions(fs::Permissions::from_mode(mode))?;
                fixture.record_json(&format!("[\"permissions\",{mode}]"))?;
            }
            4 => {
                let handle = request.string()?;
                request.end();
                handles.remove(&handle).ok_or("unknown close handle")?;
            }
            18 | 200 => {
                if kind == 200 {
                    assert_eq!(request.string()?, b"posix-rename@openssh.com");
                }
                let source = request.text()?;
                let target = request.text()?;
                request.end();
                fs::rename(fixture.local(&source), fixture.local(&target))?;
                fixture.record("rename", &[source, target])?;
            }
            _ => return Err(format!("unsupported synthetic SFTP request: {kind}").into()),
        }
        packet(&mut output, &status(&id, 0, b"")?)?;
    }
}
