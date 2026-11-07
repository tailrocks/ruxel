use crate::common::{Env, events, no_leak, qpath, quote, scan, shell_quote, task};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{fs, path::Path};
pub const FAKE: &str = "synthetic-provenance-marker-918347";
const PRIVATE_TASKS: &[&str] = &[
    "lookup",
    "encoded",
    "fact",
    "fact-reference",
    "no-log-fact",
    "no-log-reference",
    "copy-sensitive",
    "read-sensitive",
    "shell-cat",
    "shell-cat-result",
    "readback",
    "command",
    "shell-read",
    "shell-readback",
    "registered",
    "conditional",
    "conditional-child",
];
pub const NOLOG: &str = "synthetic-no-log-marker-721934";

pub fn output(env: &Env, module: &Path, state: &str, log: &Path) -> Result<crate::common::Output> {
    env.run(
        module,
        state,
        &[
            "--synthetic-secrets",
            "--log-file",
            log.to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid log path"))?,
        ],
    )
}
pub fn redacted(stdout: &str, log: &str, names: &[&str]) -> Result<()> {
    for text in [stdout, log] {
        let values = events(text)?;
        let rows: Vec<_> = values
            .iter()
            .filter(|value| {
                value.get("result").is_some()
                    && (value["task"] == "[censored]"
                        || names.iter().any(|name| value["task"] == *name))
            })
            .collect();
        ensure!(
            rows.len() == names.len(),
            "missing or duplicate private task events"
        );
        for (row, name) in rows.iter().zip(names) {
            ensure!(
                row["result"]["censored"] == true
                    && ((row["task"] == *name && row["host"] == "fixture")
                        || (row["task"] == "[censored]" && row["host"] == "[censored]")),
                "private task order, metadata or censorship mismatch"
            );
        }
    }
    Ok(())
}
pub fn execute(cli: &Path, agent: &Path, schema: &Path) -> Result<()> {
    let env = Env::new(cli, agent, schema)?;
    let executed = env.path("command-executed");
    let private = env.path("secret-payload");
    let shell = env.path("shell-payload");
    let public = env.path("public-payload");
    fs::write(&public, b"public file")?;
    let command_suffix = format!(" | base64; touch {}; printf %s ", shell_quote(&executed)?);
    let shell_suffix = format!(" > {}", shell_quote(&shell)?);
    let cat = format!("cat {}", shell_quote(&private)?);
    let module = env.write(
        "test.pkl",
        include_str!("../fixtures/module.pkl"),
        &[
            ("NO_LOG_VALUE", quote(NOLOG)?),
            ("FAKE_VALUE", quote(FAKE)?),
            ("COMMAND_SUFFIX", quote(&command_suffix)?),
            ("RESOURCE_PATH", qpath(&private)?),
            ("SHELL_PATH", qpath(&shell)?),
            ("PUBLIC_PATH", qpath(&public)?),
            ("CAT_COMMAND", quote(&cat)?),
            ("SHELL_SUFFIX", quote(&shell_suffix)?),
        ],
    )?;
    let log_path = env.path("events.jsonl");
    let result = output(&env, &module, "state", &log_path)?;
    ensure!(
        result.status == 0 && executed.is_file(),
        "private command did not run"
    );
    let encoded = STANDARD.encode(FAKE);
    let twice = STANDARD.encode(encoded.as_bytes());
    let no_log = STANDARD.encode(NOLOG);
    ensure!(
        fs::read(&private)? == encoded.as_bytes() && fs::read(&shell)? == FAKE.as_bytes(),
        "private files mismatch"
    );
    let log = fs::read_to_string(log_path)?;
    let secrets = [
        FAKE,
        encoded.as_str(),
        twice.as_str(),
        NOLOG,
        no_log.as_str(),
    ];
    for text in [&result.stdout, &result.stderr, &log] {
        no_leak(text, &secrets)?;
    }
    redacted(&result.stdout, &log, PRIVATE_TASKS)?;
    for text in [&result.stdout, &log] {
        let values = events(text)?;
        ensure!(
            task(&values, "public-read")?["content"] == STANDARD.encode(b"public file"),
            "public read was suppressed"
        );
        ensure!(
            task(&values, "public")?["msg"] == "public diagnostic",
            "public diagnostic was suppressed"
        );
    }
    crate::privacy_secondary::run(&env, &encoded, &twice)?;
    crate::privacy_error::run(&env, &encoded, &twice)?;
    scan(&env.path("state"), &secrets, false)?;
    println!(
        "PASS synthetic secret provenance: transforms, facts, commands, templates, failures and public data"
    );
    Ok(())
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;
