use crate::{
    common::{Env, no_leak, qpath, quote, shell_quote},
    privacy::{FAKE, output, redacted},
};
use anyhow::{Result, ensure};
use std::fs;

pub fn run(env: &Env, encoded: &str, twice: &str) -> Result<()> {
    let file = env.path("environment-payload");
    let marker = env.path("environment-executed");
    let cmd = format!("cat \"$RUXEL_READ_PATH\"; touch {}", shell_quote(&marker)?);
    let module = env.write(
        "environment.pkl",
        include_str!("../fixtures/environment_module.pkl"),
        &[
            ("TARGET", qpath(&file)?),
            ("FAKE_VALUE", quote(FAKE)?),
            ("ENV_COMMAND", quote(&cmd)?),
        ],
    )?;
    let log_path = env.path("environment.jsonl");
    let result = output(env, &module, "environment-state", &log_path)?;
    ensure!(
        result.status == 0 && fs::read(&file)? == encoded.as_bytes() && marker.is_file(),
        "environment fixture failed"
    );
    let log = fs::read_to_string(log_path)?;
    for text in [&result.stdout, &result.stderr, &log] {
        no_leak(text, &[FAKE, encoded])?;
    }
    redacted(
        &result.stdout,
        &log,
        &["sensitive copy", "environment read", "environment result"],
    )?;
    let source = env.path("generated-template.pkl");
    let dest = env.path("template-output");
    let prefix = "output { value = new Dynamic { kind = \"literal\"; value = \"";
    let suffix = "\" } }";
    let module = env.write(
        "template-workload.pkl",
        include_str!("../fixtures/template_module.pkl"),
        &[
            ("SOURCE", qpath(&source)?),
            ("DEST", qpath(&dest)?),
            ("PREFIX", quote(prefix)?),
            ("SUFFIX", quote(suffix)?),
            ("FAKE_VALUE", quote(FAKE)?),
        ],
    )?;
    let log_path = env.path("template.jsonl");
    let result = output(env, &module, "template-state", &log_path)?;
    ensure!(
        result.status == 0 && fs::read(&dest)? == encoded.as_bytes(),
        "template fixture failed"
    );
    let log = fs::read_to_string(log_path)?;
    for text in [&result.stdout, &result.stderr, &log] {
        no_leak(text, &[FAKE, encoded, twice])?;
    }
    redacted(
        &result.stdout,
        &log,
        &[
            "write template",
            "render template",
            "read template",
            "template result",
        ],
    )?;
    crate::common::scan(&env.path("template-state"), &[FAKE, encoded, twice], false)?;
    Ok(())
}
