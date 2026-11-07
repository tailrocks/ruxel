use crate::protocol;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn locked(path: &Path) -> Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    f.lock()?;
    ensure!(
        f.metadata()?.len() < 1024 * 1024,
        "fixture journal too large"
    );
    Ok(f)
}
pub fn rows(file: &mut File) -> Result<Vec<Value>> {
    file.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    text.lines()
        .map(|s| serde_json::from_str(s).map_err(Into::into))
        .collect()
}
fn append(file: &mut File, value: &Value) -> Result<()> {
    writeln!(file, "{value}")?;
    file.flush()?;
    Ok(())
}
fn launch(path: &Path) -> Result<usize> {
    let mut f = locked(path)?;
    let rows = rows(&mut f)?;
    let state = std::env::var("RUXEL_STATE_DIR")?;
    let mut known = BTreeMap::new();
    for row in rows {
        known.insert(
            row[1].as_str().context("invalid state")?.to_owned(),
            row[0].as_u64().context("invalid launch index")? as usize,
        );
    }
    let index = known.get(&state).copied().unwrap_or(known.len());
    append(&mut f, &json!([index, state]))?;
    Ok(index)
}
fn event(path: &Path, phase: &str) -> Result<usize> {
    let mut f = locked(path)?;
    let rows = rows(&mut f)?;
    let starts = rows.iter().filter(|v| v[0] == "start").count();
    append(&mut f, &json!([phase, std::process::id(), rows.len()]))?;
    Ok(starts)
}
fn instrument(path: &Path) -> Result<()> {
    event(path, "start")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut f = locked(path)?;
        let count = rows(&mut f)?.iter().filter(|r| r[0] == "start").count();
        drop(f);
        if count >= 2 || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(100));
    event(path, "end")?;
    Ok(())
}
pub fn run(args: Vec<OsString>) -> Result<()> {
    let mode = args
        .first()
        .and_then(|v| v.to_str())
        .context("missing fixture mode")?;
    if mode == "instrument" {
        ensure!(args.len() == 2, "instrument arguments");
        return instrument(Path::new(&args[1]));
    }
    ensure!(args.len() == 4, "wrapper arguments");
    let agent = Path::new(&args[1]);
    let log = Path::new(&args[2]);
    let failed = args[3]
        .to_str()
        .context("invalid index")?
        .parse::<usize>()?;
    if mode == "shutdown" {
        let _ = Command::new(agent).status()?;
        let mut f = locked(log)?;
        append(&mut f, &json!(std::env::var("RUXEL_STATE_DIR")?))?;
        std::process::exit(23);
    }
    ensure!(
        matches!(mode, "handshake" | "post-handshake"),
        "unknown fixture mode"
    );
    let index = launch(log)?;
    if index != failed {
        return Err(Command::new(agent).exec().into());
    }
    if mode == "post-handshake" {
        protocol::hello_then_kill(agent)?;
    }
    std::process::exit(23)
}
