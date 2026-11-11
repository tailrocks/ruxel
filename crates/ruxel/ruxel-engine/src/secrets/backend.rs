use super::Backend;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug)]
pub struct ProcessBackend {
    fnox: PathBuf,
    op: PathBuf,
}

impl Default for ProcessBackend {
    fn default() -> Self {
        Self::with_programs("fnox", "op")
    }
}

impl ProcessBackend {
    #[must_use]
    pub fn with_programs(fnox: impl Into<PathBuf>, op: impl Into<PathBuf>) -> Self {
        Self {
            fnox: fnox.into(),
            op: op.into(),
        }
    }
}

fn process_output(mut command: Command) -> Result<String, String> {
    let result = command
        .output()
        .map_err(|_| "process could not start".to_owned())?;
    if !result.status.success() {
        return Err("process failed".into());
    }
    // The CLIs behind these lookups append exactly one record terminator:
    // printenv under fnox, and `op read`. A value that legitimately ends in
    // a newline — PEM keys do — must keep it, so at most one trailing `\n`
    // goes, plus the `\r` only when it is part of that terminator.
    let mut stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    if stdout.ends_with('\n') {
        stdout.pop();
        if stdout.ends_with('\r') {
            stdout.pop();
        }
    }
    Ok(stdout)
}

impl Backend for ProcessBackend {
    fn fnox(&self, key: &str) -> Result<String, String> {
        if key.is_empty()
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || key.as_bytes()[0].is_ascii_digit()
        {
            return Err("invalid environment key".into());
        }
        let mut cmd = Command::new(&self.fnox);
        cmd.args([
            "exec",
            "--no-redact",
            "--only",
            key,
            "--",
            "sh",
            "-c",
            "printenv -- \"$1\"",
            "sh",
            key,
        ]);
        process_output(cmd)
    }
    fn onepassword_fields(
        &self,
        vault: &str,
        item: &str,
        section: Option<&str>,
        field: &str,
    ) -> Result<serde_json::Value, String> {
        let selector =
            section.map_or_else(|| field.to_owned(), |section| format!("{section}.{field}"));
        let mut cmd = Command::new(&self.op);
        cmd.args(["item", "get", "--vault"])
            .arg(vault)
            .args(["--fields"])
            .arg(selector)
            .args(["--format", "json", "--"])
            .arg(item);
        match serde_json::from_str(&process_output(cmd)?) {
            Ok(value @ serde_json::Value::Array(_)) => Ok(value),
            // A unique match may come back as a single field object.
            Ok(value @ serde_json::Value::Object(_)) => Ok(serde_json::json!([value])),
            _ => Err("invalid item response".into()),
        }
    }
    fn onepassword_reference(&self, reference: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.op);
        cmd.args(["read", "--", reference]);
        process_output(cmd)
    }
    fn command(&self, argv: &[String]) -> Result<String, String> {
        let (program, arguments) = argv.split_first().ok_or("empty command")?;
        let mut cmd = Command::new(program);
        cmd.args(arguments);
        process_output(cmd)
    }
}
