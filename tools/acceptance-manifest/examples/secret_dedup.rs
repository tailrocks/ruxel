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

const ALPHA: &str = "RUXEL_SYNTHETIC_ALPHA_824";
const BETA: &str = "RUXEL_SYNTHETIC_BETA_627";
const WRONG: &str = "RUXEL_SYNTHETIC_WRONG_913";

/// Check and diff resolve every lookup synthetically, so the same task must
/// succeed there whatever the stores hold: the assertion compares each value
/// with itself. Apply reads the real store, so the same shape compares the
/// values with the fixture's.
fn config(compare_with_fixture: bool) -> String {
    let alpha = if compare_with_fixture {
        format!("literal(\"{ALPHA}\")")
    } else {
        "ref(new {\"vars\"; \"alpha\"})".to_owned()
    };
    let beta = if compare_with_fixture {
        format!("literal(\"{BETA}\")")
    } else {
        "ref(new {\"vars\"; \"beta\"})".to_owned()
    };
    format!(
        r#"
import "modulepath:/ruxel/engine/Engine.pkl" as Ruxel
function literal(v: Ruxel.Scalar): Ruxel.Literal = new {{ value = v }}
function ref(p: Listing<String>): Ruxel.Ref = new {{ path = p }}
function call(n: String, a: Listing<Ruxel.Expr>): Ruxel.Call = new {{ `function` = n; args = a }}
output {{ value = new Ruxel.Config {{
  inventory {{ hosts {{ ["fixture"] {{ address = "127.0.0.1" }} }}; groups {{ ["all"] {{ hosts {{ "fixture" }} }} }} }}
  plays {{ new {{
    name = "Same item lookup"
    targets {{ "all" }}
    vars {{
      ["alpha"] = new Ruxel.Lookup {{ provider = "op"; vault = "fixture-vault"; item = "fixture-item"; field = "alpha" }}
      ["beta"] = new Ruxel.Lookup {{ provider = "op"; vault = "fixture-vault"; item = "fixture-item"; field = "beta" }}
    }}
    tasks {{ new {{
      name = "Verify both item fields"
      noLog = true
      operation = new Ruxel.Assert {{ that = call("and", new Listing {{
        call("eq", new Listing {{ ref(new Listing {{ "vars"; "alpha" }}); {alpha} }})
        call("eq", new Listing {{ ref(new Listing {{ "vars"; "beta" }}); {beta} }})
      }}) }}
    }} }}
  }} }}
}} }}
"#
    )
}

fn prepare(dir: &Path, compare_with_fixture: bool) -> Result<()> {
    fs::write(dir.join("fixture.pkl"), config(compare_with_fixture))?;
    let op = dir.join("op");
    fs::write(
        &op,
        r##"#!/bin/sh
[ "$#" -eq 10 ] && [ "$1" = item ] && [ "$2" = get ] && [ "$3" = --vault ] && [ "$4" = fixture-vault ] && [ "$5" = --fields ] && [ "$7" = --format ] && [ "$8" = json ] && [ "$9" = -- ] && [ "${10}" = fixture-item ] || exit 7
printf 'call\n' >> "${0%/*}/calls"
case "$6" in
alpha) printf '%s' '[{"id":"alpha","value":"RUXEL_SYNTHETIC_ALPHA_824"}]';;
beta)
    if [ -f "${0%/*}/wrong" ]; then
        printf '%s' '[{"id":"beta","value":"RUXEL_SYNTHETIC_WRONG_913"}]'
    else
        printf '%s' '[{"id":"beta","value":"RUXEL_SYNTHETIC_BETA_627"}]'
    fi;;
*) exit 8;;
esac
"##,
    )?;
    fs::set_permissions(op, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn phase(root: &Path, cli: &Path, agent: &Path, mode: &str, success: bool) -> Result<()> {
    let calls = root.join("calls");
    if calls.exists() {
        fs::remove_file(&calls)?;
    }
    let mut command = Command::new(cli);
    command
        .arg(mode)
        .arg(root.join("fixture.pkl"))
        .args(["--local", "--agent"])
        .arg(agent)
        .arg("--state-dir")
        .arg(root.join(format!("state-{mode}")))
        .arg("--no-cache")
        .env_clear()
        .env("PATH", root)
        .env("HOME", root)
        .current_dir(root)
        .stdin(Stdio::null());
    let (status, stdout, stderr) = process::run(&mut command, Duration::from_secs(30))?;
    ensure!(
        status.success() == success,
        "{mode}: unexpected controller result: {}",
        String::from_utf8_lossy(&stderr)
    );
    for secret in [ALPHA, BETA, WRONG] {
        ensure!(
            !stdout.windows(secret.len()).any(|w| w == secret.as_bytes())
                && !stderr.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{mode}: synthetic secret leaked"
        );
    }
    let events: Vec<Value> = std::str::from_utf8(&stdout)?
        .lines()
        .filter(|l| !l.is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let task: Vec<_> = events
        .iter()
        .filter(|e| e["task"] == "Verify both item fields")
        .collect();
    ensure!(
        task.len() == 1 && task[0]["result"]["censored"] == true,
        "{mode}: missing censored task result"
    );
    ensure!(
        events
            .iter()
            .any(|e| e["recap"]["failed"] == u64::from(!success)),
        "{mode}: unexpected recap"
    );
    let calls_made = fs::read_to_string(&calls).map(|calls| calls.lines().count());
    if mode == "apply" {
        // Apply is the only mode that reads the store, and the fetch is
        // field-scoped: one request per named field, never the whole item.
        ensure!(
            calls_made.context("fake op was not invoked")? == 2,
            "apply: expected one field-scoped request per named field"
        );
    } else {
        // Check and diff are synthetic: no store is contacted.
        ensure!(
            calls_made.is_err(),
            "{mode}: the store was contacted in a synthetic mode"
        );
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
    // Check and diff run synthetic: the fixture compares values with
    // themselves, and the fake op must never be called.
    prepare(temp.path(), false)?;
    for mode in ["check", "diff"] {
        phase(temp.path(), &cli, &agent, mode, true)?;
    }
    // Apply reads the real store through the same field-scoped fetch.
    prepare(temp.path(), true)?;
    phase(temp.path(), &cli, &agent, "apply", true)?;
    fs::write(temp.path().join("wrong"), b"")?;
    phase(temp.path(), &cli, &agent, "apply", false)?;
    println!(
        "check and diff: synthetic, no store contact; apply: one field-scoped request per named field, values redacted"
    );
    Ok(())
}
