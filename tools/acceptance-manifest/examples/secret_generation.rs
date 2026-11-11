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
/// value from a file — `--from-file`, the byte-preserving store of the
/// pinned CLI — and counts its calls per key; the stdin form is modelled
/// too, and it trims the way the pinned CLI trims. A missing key exits 1
/// with `not found`. The store's own question about a key — `fnox env
/// --json --describe`, the unfiltered query — lists every key of the
/// active profile with its `env` and `as_file` flags, and `--keys` names
/// the keys to describe; this fixture's keys hold plain values, never
/// file-backed ones. No real secret value is read.
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
    # The CLI prints the value with one record terminator; the backend
    # removes that terminator, never one the value carries.
    printf '\n'
    exit 0
  fi
  echo "secret $key not found" >&2
  exit 1
fi
if [ "$1" = "set" ]; then
  key="$2"
  if [ "$#" -eq 4 ] && [ "$3" = "--from-file" ]; then
    # `--from-file` is the byte-preserving store of the pinned CLI: the
    # store keeps the file's exact bytes, terminator and all.
    printf '1\n' >> "$counts/$key"
    mkdir -p "$store"
    cat "$4" > "$store/$key"
    exit 0
  fi
  [ "$#" -eq 2 ] || { echo "set takes the key and --from-file <file>" >&2; exit 9; }
  # The stdin store of the pinned CLI trims: '  padded value  ' is kept
  # as 'padded value', and the trailing newline goes with the trim, so a
  # value that must round-trip byte for byte never travels this path.
  printf '1\n' >> "$counts/$key"
  value=$(cat)
  value=$(printf '%s' "$value" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
  mkdir -p "$store"
  printf '%s' "$value" > "$store/$key"
  exit 0
fi
if [ "$1" = "env" ]; then
  # The store's own question about a key, the way the CLI answers it:
  # the unfiltered query lists every key of the active profile with its
  # `env` and `as_file` flags, and `--keys` names the keys to describe.
  [ "$2" = "--json" ] || { echo "env takes --json" >&2; exit 7; }
  shift 2
  keys=''
  while [ "$#" -gt 0 ]; do
    case "$1" in
    --describe) shift ;;
    --keys) keys="$2"; shift 2 ;;
    *) echo "env takes --describe and --keys" >&2; exit 7 ;;
    esac
  done
  if [ -n "$keys" ]; then
    # `--keys` names only the keys of the profile: an unknown key is
    # refused with the CLI's own invalid_keys error document.
    if [ -f "$store/$keys" ]; then
      printf '{"schema":1,"keys":[{"key":"%s","kind":"secret","env":true,"as_file":false}]}' "$keys"
      exit 0
    fi
    printf '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: unknown keys: %s","unknown":["%s"]}}' "$keys" "$keys"
    exit 1
  fi
  # The unfiltered query lists every key of the active profile with its
  # `env` and `as_file` flags: the store's own answer to whether a key is
  # a file-backed secret. This fixture's keys hold plain values.
  printf '{"schema":1,"keys":['
  separator=''
  if [ -d "$store" ]; then
    for file in "$store"/*; do
      [ -f "$file" ] || continue
      printf '%s{"key":"%s","kind":"secret","env":true,"as_file":false}' "$separator" "${file##*/}"
      separator=','
    done
  fi
  printf ']}'
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
                !output
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
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
