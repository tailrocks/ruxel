use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

#[path = "../src/process.rs"]
mod process;
#[path = "../src/process_guard.rs"]
mod process_guard;

const GENERATED_KEY: &str = "PROCESSOR_DB_PASSWORD";
const FETCHED_KEY: &str = "K8S_ADMIN_KUBECONFIG";
const KUBECONFIG: &str = "apiVersion: v1\nkind: Config\ncurrent-context: fixture-admin\n";

fn workflow(root: &Path) -> String {
    let root = root.display();
    format!(
        r#"amends "modulepath:/ruxel/Workflow.pkl"
import "modulepath:/ruxel/Core.pkl" as core

local fixture = new core.Host {{ name = "fixture"; address = "127.0.0.1" }}

stages {{ new Stage {{
  title = "Generated and fetched secrets"
  hosts {{ fixture }}
  steps {{
    new core.File {{
      path = "{root}/processor.env"
      mode = "0600"
      content = "PASSWORD=\(core.fnoxGenerated("{GENERATED_KEY}", "hex32"))\n"
    }}
    new core.FetchSecret {{
      title = "Store the cluster admin kubeconfig"
      fromPath = "{root}/admin.conf"
      runOn = fixture
      fnoxKey = "{FETCHED_KEY}"
    }}
  }}
}} }}
"#
    )
}

/// The fake fnox: values live in files beside the script, reads go through
/// `fnox get --if-missing error` (the store's read path), `set` takes the
/// value on stdin (never argv) and counts its calls per key, and a missing
/// key exits 1 with `not found`. No real secret value is read.
fn prepare(root: &Path) -> Result<()> {
    fs::write(root.join("workflow.pkl"), workflow(root))?;
    fs::write(root.join("admin.conf"), KUBECONFIG)?;
    let fnox = root.join("fnox");
    fs::write(
        &fnox,
        r##"#!/bin/sh
store="${0%/*}/store"
counts="${0%/*}/counts"
if [ "$1" = "get" ]; then
  [ "$#" -eq 4 ] && [ "$2" = "--if-missing" ] && [ "$3" = "error" ] || { echo "get takes --if-missing error and the key" >&2; exit 7; }
  key="$4"
  if [ -f "$store/$key" ]; then
    cat "$store/$key"
    exit 0
  fi
  echo "secret $key not found" >&2
  exit 1
fi
if [ "$1" = "set" ]; then
  [ "$#" -eq 2 ] || { echo "set takes only the key argument" >&2; exit 9; }
  key="$2"
  printf '1\n' >> "$counts/$key"
  mkdir -p "$store"
  cat > "$store/$key"
  exit 0
fi
exit 8
"##,
    )?;
    fs::set_permissions(fnox, fs::Permissions::from_mode(0o700))?;
    fs::create_dir_all(root.join("counts"))?;
    Ok(())
}

fn run(
    root: &Path,
    cli: &Path,
    agent: &Path,
    mode: &str,
    expected_status: i32,
) -> Result<Vec<Value>> {
    let mut command = Command::new(cli);
    command
        .arg(mode)
        .arg(root.join("workflow.pkl"))
        .args(["--local", "--agent"])
        .arg(agent)
        .arg("--state-dir")
        .arg(root.join(format!("state-{mode}")))
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
        .env("HOME", root)
        .current_dir(root)
        .stdin(Stdio::null());
    let (status, stdout, stderr) = process::run(&mut command, Duration::from_secs(60))?;
    ensure!(
        status.code() == Some(expected_status),
        "{mode}: exit {}, expected {expected_status}: {} {}",
        status.code().unwrap_or_default(),
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    let events: Vec<Value> = std::str::from_utf8(&stdout)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()
        .context("controller output is not event JSON")?;
    Ok(events)
}

fn count(root: &Path, key: &str) -> usize {
    fs::read_to_string(root.join("counts").join(key))
        .map_or(0, |counts| counts.lines().filter(|l| !l.is_empty()).count())
}

fn stored(root: &Path, key: &str) -> Result<String> {
    fs::read_to_string(root.join("store").join(key)).context("key missing from the fake store")
}

fn assert_no_plaintext(stdout: &[u8], stderr: &[u8], secrets: &[String], mode: &str) -> Result<()> {
    for secret in secrets {
        for (name, output) in [("stdout", stdout), ("stderr", stderr)] {
            ensure!(
                !output.windows(secret.len()).any(|window| window == secret.as_bytes()),
                "{mode}: plaintext reached {name}"
            );
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let cli = fs::canonicalize(args.next().context("expected CLI path")?)?;
    let agent = fs::canonicalize(args.next().context("expected agent path")?)?;
    ensure!(
        args.next().is_none(),
        "expected exactly CLI and agent paths"
    );
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    prepare(root)?;

    // Check contacts no store and generates nothing: the generated key is a
    // selector-derived placeholder, and FetchSecret is known-after-apply.
    let events = run(root, &cli, &agent, "check", 0)?;
    ensure!(
        count(root, GENERATED_KEY) == 0 && count(root, FETCHED_KEY) == 0,
        "check contacted the store"
    );
    // A sensitive run censors task names, so the marker is the result's.
    let fetch: Vec<_> = events
        .iter()
        .filter(|event| event["result"]["knownAfterApply"] == true)
        .collect();
    ensure!(
        fetch.len() == 1,
        "check: FetchSecret is not known-after-apply: {events:?}"
    );
    ensure!(
        fetch[0]["result"]["changed"] == false,
        "check: FetchSecret reported a change"
    );

    // First apply generates once and stores the fetched file once.
    let first = run(root, &cli, &agent, "apply", 0)?;
    ensure!(
        count(root, GENERATED_KEY) == 1,
        "first apply: generated key stored {} times",
        count(root, GENERATED_KEY)
    );
    ensure!(
        count(root, FETCHED_KEY) == 1,
        "first apply: fetched key stored {} times",
        count(root, FETCHED_KEY)
    );
    let generated = stored(root, GENERATED_KEY)?;
    ensure!(
        generated.len() == 32 && generated.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "generated value is not hex32"
    );
    ensure!(
        stored(root, FETCHED_KEY)? == KUBECONFIG,
        "fetched value does not match the file"
    );
    // The run reports the generation.
    ensure!(
        serde_json::to_string(&first)?.contains(&format!("generated secret {GENERATED_KEY}")),
        "first apply: no generated-secret event"
    );

    // The second apply of the same workflow generates exactly once in total:
    // the key is present, so nothing is stored again, and the unchanged file
    // is not fetched into fnox twice.
    let _second = run(root, &cli, &agent, "apply", 0)?;
    ensure!(
        count(root, GENERATED_KEY) == 1,
        "two applies stored the generated key {} times",
        count(root, GENERATED_KEY)
    );
    ensure!(
        count(root, FETCHED_KEY) == 1,
        "two applies stored the fetched key {} times",
        count(root, FETCHED_KEY)
    );

    // No plaintext in captured output: neither the generated value nor the
    // fetched file, whole or line by line.
    let stdout = serde_json::to_vec(&first)?;
    let mut secrets: Vec<String> = vec![generated.clone(), KUBECONFIG.to_owned()];
    secrets.extend(KUBECONFIG.lines().map(str::to_owned));
    assert_no_plaintext(&stdout, &[], &secrets, "apply")?;
    println!(
        "check: synthetic and known-after-apply, no store contact; two applies: generated exactly once, fetched exactly once, no plaintext in captured output"
    );
    Ok(())
}
