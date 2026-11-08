use crate::common::{Env, events, no_leak, qpath, scan, task};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
const SECRET: &str = "synthetic-sensitive-cache-marker-739264";
fn ledger(root: &Path, results: &mut Vec<PathBuf>) -> Result<()> {
    for item in fs::read_dir(root)? {
        let path = item?.path();
        let meta = fs::symlink_metadata(&path)?;
        ensure!(!meta.file_type().is_symlink(), "unexpected state symlink");
        if meta.is_dir() {
            ledger(&path, results)?;
        } else if meta.is_file() && path.file_name().is_some_and(|n| n == "ledger.json") {
            results.push(path);
        }
    }
    Ok(())
}
fn module(env: &Env, source: &Path, dest: &Path, private: bool) -> Result<PathBuf> {
    env.write(
        "cache.pkl",
        include_str!("../fixtures/cache.pkl"),
        &[
            ("NOLOG", if private { "noLog = true;" } else { "" }.into()),
            ("SOURCE", qpath(Path::new(source.file_name().context("fixture source has no filename")?))?),
            ("DEST", qpath(dest)?),
        ],
    )
}
fn run(
    env: &Env,
    module: &Path,
    state: &str,
    options: &[&str],
    private: bool,
) -> Result<(serde_json::Value, String)> {
    let output = env.run(module, state, options)?;
    ensure!(output.status == 0, "cache apply failed: {}", output.stderr);
    let rows = events(&output.stdout)?;
    if private {
        ensure!(
            rows.iter().all(|row| row["host"] == "[censored]"),
            "private cache metadata was visible"
        );
    }
    let event = task(&rows, if private { "[censored]" } else { "copy" })?.clone();
    Ok((event, format!("{}{}", output.stdout, output.stderr)))
}
pub fn execute(cli: &Path, agent: &Path, schema: &Path) -> Result<()> {
    let env = Env::new(cli, agent, schema)?;
    fs::create_dir(env.path("fixture"))?;
    fs::create_dir(env.path("schema"))?;
    fs::copy(schema, env.path("schema/Ruxel.pkl"))?;

    let source = env.path("fixture/source");
    let dest = env.path("destination");
    fs::write(&source, "first-version")?;
    let mut workload = module(&env, &source, &dest, false)?;
    let placed = env.path("fixture/workload.pkl");
    fs::rename(&workload, &placed)?;
    workload = placed;
    let (first, _) = run(&env, &workload, "state", &[], false)?;
    ensure!(
        first["changed"] == true && fs::read(&dest)? == b"first-version",
        "fresh copy failed"
    );
    let mut files = Vec::new();
    ledger(&env.path("state"), &mut files)?;
    ensure!(files.len() == 1, "missing unique ledger");
    let initial = fs::read(&files[0])?;
    let inode = fs::metadata(&files[0])?.ino();
    let (second, _) = run(&env, &workload, "state", &[], false)?;
    ensure!(
        second["changed"] == false
            && fs::read(&dest)? == b"first-version"
            && fs::read(&files[0])? == initial
            && fs::metadata(&files[0])?.ino() == inode,
        "cache hit failed"
    );
    let (bypass, _) = run(&env, &workload, "state", &["--no-cache"], false)?;
    ensure!(
        bypass["changed"] == false && fs::read(&files[0])? != initial,
        "cache bypass failed"
    );
    fs::write(&dest, "drift")?;
    let (repair, _) = run(&env, &workload, "state", &[], false)?;
    ensure!(
        repair["changed"] == true && fs::read(&dest)? == b"first-version",
        "drift not repaired"
    );
    fs::write(&source, "second-version")?;
    let (changed, _) = run(&env, &workload, "state", &[], false)?;
    ensure!(
        changed["changed"] == true && fs::read(&dest)? == b"second-version",
        "source change ignored"
    );
    fs::write(&source, SECRET)?;
    let private = module(&env, &source, &dest, true)?;
    fs::rename(private, &workload)?;
    let (event, output) = run(&env, &workload, "private-state", &[], true)?;
    ensure!(
        event["censored"] == true && fs::read(&dest)? == SECRET.as_bytes(),
        "private copy failed"
    );
    no_leak(&output, &[SECRET])?;
    scan(&env.path("private-state"), &[SECRET], true)?;
    println!("PASS local cache: hit, bypass, drift repair, source change and noLog");
    Ok(())
}
