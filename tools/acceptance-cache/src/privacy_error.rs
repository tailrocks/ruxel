use crate::{
    common::{Env, events, no_leak, qpath, quote},
    privacy::{FAKE, output},
};
use anyhow::{Result, ensure};
use std::fs;

pub fn run(env: &Env, encoded: &str, twice: &str) -> Result<()> {
    let source = env.path("template-error-source.pkl");
    let dest = env.path("template-error-output");
    let prefix = "output { value = new Dynamic { kind = \"ref\"; path = new Listing { \"";
    let suffix = "\" } } }";
    let module = env.write(
        "template-error-workload.pkl",
        include_str!("../fixtures/template_module.pkl"),
        &[
            ("SOURCE", qpath(&source)?),
            ("DEST", qpath(&dest)?),
            ("PREFIX", quote(prefix)?),
            ("SUFFIX", quote(suffix)?),
            ("FAKE_VALUE", quote(FAKE)?),
        ],
    )?;
    let log_path = env.path("template-error.jsonl");
    let result = output(env, &module, "template-error-state", &log_path)?;
    let values = events(&result.stdout)?;
    let errors: Vec<_> = values
        .iter()
        .filter(|v| v["event"] == "host_failed")
        .collect();
    let recaps: Vec<_> = values
        .iter()
        .filter(|v| v["host"] == "[censored]")
        .filter_map(|v| v.get("recap"))
        .collect();
    ensure!(
        result.status == 8
            && source.is_file()
            && errors.len() == 1
            && errors[0]["host"] == "[censored]"
            && errors[0]["failed"] == true
            && errors[0]["msg"] == "sensitive execution failed"
            && recaps.len() == 1
            && recaps[0]["failed"] == 1,
        "sensitive template failure not sanitized"
    );
    let log = fs::read_to_string(log_path)?;
    let logs = events(&log)?;
    let logged: Vec<_> = logs
        .iter()
        .filter(|v| v["event"] == "host_failed")
        .collect();
    ensure!(logged == errors, "sensitive failure missing from log");
    for text in [&result.stdout, &result.stderr, &log] {
        no_leak(text, &[FAKE, encoded, twice])?;
    }
    for (fixture, marker) in [
        ("failing", "synthetic-private-path-marker"),
        ("handler", "synthetic-handler-path-marker"),
    ] {
        let source = match fixture {
            "failing" => include_str!("../fixtures/failing.pkl"),
            _ => include_str!("../fixtures/handler.pkl"),
        };
        let module = env.write(&format!("{fixture}.pkl"), source, &[])?;
        let result = env.run(&module, &format!("{fixture}-state"), &[])?;
        ensure!(result.status != 0, "private failure unexpectedly succeeded");
        no_leak(&result.stdout, &[marker])?;
        no_leak(&result.stderr, &[marker])?;
    }
    Ok(())
}
