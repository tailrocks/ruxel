#![forbid(unsafe_code)]
mod fake_sftp;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn local(&self, path: &str) -> PathBuf {
        assert!(
            matches!(path, "/" | "/var" | "/var/lib" | "/var/lib/ruxel")
                || path.starts_with("/var/lib/ruxel/")
        );
        assert!(
            !Path::new(path)
                .components()
                .any(|p| p == Component::ParentDir)
        );
        self.root.join("remote").join(path.trim_start_matches('/'))
    }
    fn record(&self, kind: &str, fields: &[String]) -> Result<()> {
        let mut values = vec![quote(kind)];
        values.extend(fields.iter().map(|value| quote(value)));
        self.record_json(&format!("[{}]", values.join(",")))
    }
    fn record_json(&self, value: &str) -> Result<()> {
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("calls.jsonl"))?;
        writeln!(log, "{value}")?;
        Ok(())
    }
    fn stat(&self, path: &str) -> Result<u8> {
        let metadata = match fs::symlink_metadata(self.local(path)) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(1),
            Err(error) => return Err(error.into()),
        };
        let mut owner = u64::from(metadata.uid());
        if let Ok(overrides) = fs::read_to_string(self.root.join("metadata")) {
            for line in overrides.lines() {
                let fields: Vec<_> = line.split('\t').collect();
                assert_eq!(fields.len(), 3);
                if fields[0] == path
                    && (fields[2] == "*" || fields[2].parse::<u64>()? == metadata.ino())
                {
                    owner = fields[1].parse()?;
                }
            }
        }
        println!("{owner}:{:x}:{}", metadata.mode(), metadata.nlink());
        Ok(0)
    }
    fn remote(&self, args: &[String]) -> Result<u8> {
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        match args.as_slice() {
            ["id", "-u"] => println!("{}", fs::metadata(&self.root)?.uid()),
            ["stat", "-c", "%u:%f:%h", "--", path] => return self.stat(path),
            ["test", "-e", path] => return Ok(u8::from(!self.local(path).exists())),
            ["test", "!", "-L", path] => return Ok(u8::from(self.local(path).is_symlink())),
            ["test", "-x", path] => {
                return Ok(u8::from(
                    fs::metadata(self.local(path))?.mode() & 0o111 == 0,
                ));
            }
            ["mkdir", "-m", "700", path] => {
                fs::create_dir(self.local(path))?;
                fs::set_permissions(self.local(path), fs::Permissions::from_mode(0o700))?;
            }
            ["mkdir", "-p", path] => fs::create_dir_all(self.local(path))?,
            ["chmod", mode, path] => fs::set_permissions(
                self.local(path),
                fs::Permissions::from_mode(u32::from_str_radix(mode, 8)?),
            )?,
            ["sha256sum", path] => {
                let matching = !self.root.join("mismatch").exists()
                    && fs::read(self.local(path))? == fs::read(self.root.join("agent"))?;
                let digest = if matching {
                    fs::read_to_string(self.root.join("digest"))?
                } else {
                    "0".repeat(64)
                };
                println!("{digest}  {path}");
            }
            ["rm", "-f", path] => {
                if self.root.join("cleanup-fail").exists() {
                    return Ok(23);
                }
                match fs::remove_file(self.local(path)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            [path] if path.starts_with("/var/lib/ruxel/agent/") => {
                self.local(path);
                self.record("launch", &[(*path).into()])?;
                let mut input = io::stdin().lock();
                fs::write(self.root.join("hello"), frame(&mut input)?)?;
                let mut output = io::stdout().lock();
                output.write_all(&fs::read(self.root.join("ack"))?)?;
                output.flush()?;
                fs::write(self.root.join("done"), frame(&mut input)?)?;
            }
            _ => return Err(format!("unsupported synthetic command: {args:?}").into()),
        }
        Ok(0)
    }
}
fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => result.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}
fn frame(input: &mut impl Read) -> Result<Vec<u8>> {
    let mut size = 0_u32;
    for shift in (0..35).step_by(7) {
        let mut byte = [0];
        input.read_exact(&mut byte)?;
        size |= u32::from(byte[0] & 127)
            .checked_shl(shift)
            .ok_or("frame size overflow")?;
        if byte[0] & 128 == 0 {
            assert!(size <= 16 * 1024 * 1024);
            let mut content = vec![0; size as usize];
            input.read_exact(&mut content)?;
            return Ok(content);
        }
    }
    Err("invalid synthetic frame length".into())
}
fn run() -> Result<u8> {
    let root = std::env::current_exe()?
        .parent()
        .ok_or("no bin directory")?
        .parent()
        .ok_or("no fixture directory")?
        .to_path_buf();
    let fixture = Fixture { root };
    let args: Vec<_> = std::env::args().skip(1).collect();
    let separator = args
        .iter()
        .position(|arg| arg == "--")
        .ok_or("missing separator")?;
    assert_eq!(
        args.get(separator + 1).map(String::as_str),
        Some("fixture.invalid")
    );
    let options = &args[..separator];
    for required in ["StrictHostKeyChecking=yes", "IdentitiesOnly=yes"] {
        assert!(options.iter().any(|arg| arg == required));
    }
    let port = options
        .iter()
        .position(|arg| arg == "-p")
        .ok_or("missing port")?;
    assert_eq!(options.get(port + 1).map(String::as_str), Some("2222"));
    assert!(
        !options
            .iter()
            .any(|arg| ["-F", "-M", "-S", "-O", "-A", "-J"].contains(&arg.as_str()))
    );
    assert!(
        options
            .iter()
            .any(|arg| Path::new(arg) == fixture.root.join("identity key"))
    );
    let pins: Vec<_> = options
        .iter()
        .filter(|arg| arg.starts_with("UserKnownHostsFile="))
        .collect();
    let pin = fixture.root.join("known hosts").display().to_string();
    assert_eq!(
        pins,
        vec![&format!(
            "UserKnownHostsFile=\"{}\"",
            pin.replace('\\', "\\\\").replace('"', "\\\"")
        )]
    );
    fixture.record_json(&format!(
        "[\"ssh\",[{}]]",
        args.iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(",")
    ))?;
    let remote = &args[separator + 2..];
    if options.iter().any(|arg| arg == "-s") {
        assert_eq!(remote, &["sftp"]);
        fake_sftp::serve(&fixture)?;
        Ok(0)
    } else {
        fixture.remote(remote)
    }
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(code) => std::process::ExitCode::from(code),
        Err(error) => {
            eprintln!("native transport fixture: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
