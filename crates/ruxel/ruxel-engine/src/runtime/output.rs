use anyhow::{Context, Result};
use serde_json::Value;
use std::{fs::File, io::Write, path::Path, sync::Mutex};

/// Presentation of redacted execution events.
#[derive(Debug, Default, Clone, Copy)]
pub enum OutputFormat {
    #[default]
    Json,
    Human,
}

pub(super) struct Output {
    format: OutputFormat,
    log: Mutex<Option<File>>,
}
impl Output {
    pub(super) fn new(format: OutputFormat, path: Option<&Path>) -> Result<Self> {
        let log = path
            .map(|path| {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                options.open(path).context("create execution event log")
            })
            .transpose()?;
        Ok(Self {
            format,
            log: Mutex::new(log),
        })
    }
    pub(super) fn event(&self, value: &Value) -> Result<()> {
        // Serialize stdout and the corresponding log entry together across host workers.
        let mut log = self
            .log
            .lock()
            .map_err(|_| anyhow::anyhow!("event output lock poisoned"))?;
        let mut stdout = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => writeln!(stdout, "{value}"),
            OutputFormat::Human => {
                let host = value
                    .get("host")
                    .and_then(Value::as_str)
                    .unwrap_or("controller");
                if let Some(recap) = value.get("recap") {
                    writeln!(stdout, "{host}: {recap}")
                } else {
                    let event = value.get("event").and_then(Value::as_str).unwrap_or("task");
                    writeln!(stdout, "{host}: {event} {value}")
                }
            }
        }
        .context("write execution event")?;
        if let Some(log) = log.as_mut() {
            writeln!(log, "{value}").context("write execution event log")?;
            log.flush().context("flush execution event log")?;
        }
        Ok(())
    }
}
