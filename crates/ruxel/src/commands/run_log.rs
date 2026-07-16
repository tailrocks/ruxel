use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

const KEEP_RUNS: usize = 50;

pub struct RunLog {
    writer: Option<BufWriter<std::fs::File>>,
}

impl RunLog {
    pub fn open(run_id: &str) -> Self {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| PathBuf::from(".local/state"));
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        match Self::open_in(&root, run_id, timestamp) {
            Ok(log) => log,
            Err(error) => {
                eprintln!("warning: run log disabled: {error}");
                Self { writer: None }
            }
        }
    }

    fn open_in(root: &Path, run_id: &str, timestamp: u64) -> io::Result<Self> {
        let runs = root.join("ruxel/runs");
        std::fs::create_dir_all(&runs)?;
        set_mode(&runs, 0o700)?;
        prune(&runs, KEEP_RUNS.saturating_sub(1))?;
        let safe_run_id: String = run_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        let path = runs.join(format!("{timestamp}-{safe_run_id}.jsonl"));
        let file = open_private(&path)?;
        Ok(Self {
            writer: Some(BufWriter::new(file)),
        })
    }

    pub fn record_recap(&mut self, host: &str, recap: ruxel_cli::scheduler::Recap) {
        let _ = writeln!(
            self,
            "{}",
            serde_json::json!({
                "event": "recap", "host": host,
                "ok": recap.ok, "changed": recap.changed, "failed": recap.failed,
                "skipped": recap.skipped, "rescued": recap.rescued, "ignored": recap.ignored,
            })
        );
    }

    pub fn record_unreachable(&mut self, host: &str, error: &str) {
        let _ = writeln!(self, "{}", unreachable_record(host, error));
    }
}

pub(crate) fn unreachable_record(host: &str, error: &str) -> serde_json::Value {
    serde_json::json!({
        "event": "unreachable", "host": host, "msg": error,
        "changed": false, "unreachable": true,
    })
}

impl Write for RunLog {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if let Some(writer) = &mut self.writer
            && let Err(error) = writer.write_all(buffer)
        {
            eprintln!("warning: run log write failed: {error}");
            self.writer = None;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(writer) = &mut self.writer
            && let Err(error) = writer.flush()
        {
            eprintln!("warning: run log flush failed: {error}");
            self.writer = None;
        }
        Ok(())
    }
}

impl Drop for RunLog {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn prune(directory: &Path, keep: usize) -> io::Result<()> {
    let mut logs: Vec<_> = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    logs.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split_once('-'))
            .and_then(|(timestamp, _)| timestamp.parse::<u64>().ok())
            .unwrap_or(0)
    });
    let remove = logs.len().saturating_sub(keep);
    for path in logs.into_iter().take(remove) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn open_private(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::create(path)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_jsonl_and_prunes_to_limit() {
        let root = std::env::temp_dir().join(format!("ruxel-run-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for timestamp in 0..55 {
            let mut log = RunLog::open_in(&root, "synthetic", timestamp).unwrap();
            writeln!(log, "{{\"event\":\"task\"}}").unwrap();
        }
        let runs = root.join("ruxel/runs");
        let files: Vec<_> = std::fs::read_dir(&runs).unwrap().collect();
        assert_eq!(files.len(), KEEP_RUNS);
        assert!(!runs.join("0-synthetic.jsonl").exists());
        assert!(runs.join("5-synthetic.jsonl").exists());
        assert!(
            std::fs::read_to_string(runs.join("54-synthetic.jsonl"))
                .unwrap()
                .contains("\"event\":\"task\"")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disabled_writer_is_non_fatal() {
        let mut log = RunLog { writer: None };
        assert_eq!(log.write(b"ignored").unwrap(), 7);
    }

    #[test]
    fn unreachable_record_has_ansible_observable_shape() {
        assert_eq!(
            unreachable_record("fixture", "refused"),
            serde_json::json!({
                "event": "unreachable", "host": "fixture", "msg": "refused",
                "changed": false, "unreachable": true,
            })
        );
    }
}
