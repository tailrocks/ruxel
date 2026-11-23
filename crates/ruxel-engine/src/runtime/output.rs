use anyhow::{Context, Result};
use serde_json::Value;
use std::{fs::File, io::Write, path::Path, sync::Mutex};

pub(super) fn sensitive_event(value: &Value) -> Value {
    let mut output = serde_json::Map::new();
    if value.get("host").is_some() {
        output.insert("host".into(), Value::String("[censored]".into()));
    }
    if let Some(task) = value.get("task") {
        output.insert(
            "task".into(),
            if task.is_object() {
                // A task object is a plan projection the controller built,
                // and the known-after-apply marker it carries is the same
                // controller-built metadata the plan's own conservative
                // projection keeps: an apply-only step still says it happens
                // at apply, and nothing else of the task's fields.
                let mut censored = serde_json::Map::new();
                censored.insert("censored".into(), Value::Bool(true));
                if let Some(true) = task.get("knownAfterApply").and_then(Value::as_bool) {
                    censored.insert("knownAfterApply".into(), Value::Bool(true));
                }
                Value::Object(censored)
            } else {
                Value::String("[censored]".into())
            },
        );
    }
    if let Some(phase) = value.get("phase").and_then(Value::as_str)
        && ["preTask", "task", "handler"].contains(&phase)
    {
        output.insert("phase".into(), Value::String(phase.into()));
    }
    if let Some(planned) = value.get("planned").and_then(Value::as_bool) {
        output.insert("planned".into(), Value::Bool(planned));
    }
    if let Some(result) = value.get("result") {
        let mut censored = serde_json::Map::new();
        censored.insert("censored".into(), Value::Bool(true));
        for key in ["changed", "failed", "skipped"] {
            if let Some(value) = result.get(key).and_then(Value::as_bool) {
                censored.insert(key.into(), Value::Bool(value));
            }
        }
        // The known-after-apply marker and the keyed fingerprint are
        // controller-built and carry no value, so a sensitive run keeps
        // them: an apply-only step still reports known-after-apply, and a
        // secret-bearing file still shows its keyed fingerprint. The
        // result is untrusted input, so the fingerprint passes only in
        // the controller's own shape — eight lowercase hex — and the diff
        // only as the controller's own sentence over that same
        // fingerprint: shape alone proves nothing about who built a
        // value, and eight hex characters hold four bytes of anything, so
        // a sentence that stands alone, or names a fingerprint the
        // controller did not set beside it, is censored.
        if let Some(true) = result.get("knownAfterApply").and_then(Value::as_bool) {
            censored.insert("knownAfterApply".into(), Value::Bool(true));
        }
        if let Some(fingerprint) = result
            .get("fingerprint")
            .and_then(Value::as_str)
            .filter(|fingerprint| crate::secrets::is_fingerprint(fingerprint))
        {
            censored.insert("fingerprint".into(), Value::String(fingerprint.to_owned()));
            if let Some(diff) = result
                .get("diff")
                .and_then(Value::as_str)
                .filter(|diff| crate::secrets::is_sentence_over(fingerprint, diff))
            {
                censored.insert("diff".into(), Value::String(diff.to_owned()));
            }
        }
        output.insert("result".into(), Value::Object(censored));
    }
    if value.get("pause").is_some() {
        output.insert("pause".into(), Value::String("[censored]".into()));
    }
    Value::Object(output)
}

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
