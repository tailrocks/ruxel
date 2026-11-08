use crate::host_fixture::Guest;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

fn account(env: &Guest) -> Result<Option<Vec<String>>> {
    let passwd = env.text(&["/bin/cat", "/etc/passwd"])?;
    let matches: Vec<_> = passwd
        .lines()
        .filter(|line| line.split(':').next() == Some("ruxelbecome"))
        .collect();
    ensure!(matches.len() <= 1, "duplicate fixture account");
    Ok(matches
        .first()
        .map(|line| line.split(':').map(str::to_owned).collect()))
}
fn verify_account(env: &Guest, shell: &str) -> Result<()> {
    let fields = account(env)?.context("fixture account absent")?;
    ensure!(
        fields.len() == 7
            && fields[2].parse::<u32>()? >= 1000
            && fields[5] == "/tmp/work/payload/become-home"
            && fields[6] == shell,
        "fixture account mismatch"
    );
    Ok(())
}
fn drift(env: &Guest) -> Result<()> {
    verify_account(env, "/bin/sh")?;
    env.docker.exec(
        &["/usr/sbin/usermod", "--shell", "/bin/false", "ruxelbecome"],
        0,
    )?;
    verify_account(env, "/bin/false")
}
fn verify_result(events: &[Value], home: &str) -> Result<()> {
    let events: Vec<_> = events
        .iter()
        .filter(|v| v["host"] == "fixture" && v["task"] == "Capture synthetic become environment")
        .collect();
    ensure!(
        events.len() == 1
            && events[0]["result"]["stdout_lines"]
                == json!([home, "ruxelbecome", "ruxelbecome", "/bin/sh", "synthetic"])
            && events[0]["result"]["failed"] == false,
        "become environment result mismatch"
    );
    Ok(())
}
fn verify_preview(events: &[Value], home: &str) -> Result<()> {
    ensure!(
        events.len() == 4
            && events[0]["host"] == "fixture"
            && events[0]["task"] == "Create synthetic become account"
            && events[0]["result"]["changed"] == false
            && events[0]["result"]["failed"] == false,
        "become preview account result mismatch"
    );
    verify_result(events, home)?;
    ensure!(
        events[1]["result"]["skipped"] != true,
        "become preview shell skipped"
    );
    ensure!(
        events[2]["host"] == "fixture"
            && events[2]["task"] == "Verify explicit become environment"
            && events[2]["result"]["failed"] == false,
        "become preview assertion mismatch"
    );
    ensure!(
        events[3]["host"] == "fixture"
            && events[3]["recap"]["failed"] == 0
            && events[3]["recap"]["skipped"] == 0,
        "become preview recap mismatch"
    );
    Ok(())
}
fn verify_drift_preview(events: &[Value], home: &str) -> Result<()> {
    ensure!(
        events.len() == 4
            && events[0]["result"]["changed"] == true
            && events[0]["result"]["failed"] == false,
        "drift preview account result mismatch"
    );
    ensure!(
        events[1]["host"] == "fixture"
            && events[1]["task"] == "Capture synthetic become environment"
            && events[1]["result"]["stdout_lines"]
                == json!([
                    home,
                    "ruxelbecome",
                    "ruxelbecome",
                    "/bin/false",
                    "synthetic"
                ])
            && events[1]["result"]["failed"] == false,
        "drift preview environment mismatch"
    );
    ensure!(
        events[2]["task"] == "Verify explicit become environment"
            && events[2]["result"]["failed"] == true
            && events[2]["result"]["msg"] == "Assertion failed"
            && events[3]["recap"]["failed"] == 1,
        "drift preview failure mismatch"
    );
    Ok(())
}
fn verify_drift_limitation(env: &Guest, home: &str) -> Result<()> {
    drift(env)?;
    for mode in ["check", "diff"] {
        let before = env.snapshot()?;
        let events = env.run(mode, 8)?;
        verify_drift_preview(&events, home)?;
        ensure!(env.snapshot()? == before, "drift preview mutated state");
        verify_account(env, "/bin/false")?;
        println!("LIMITATION become-environment drifted {mode}: assertion fails, state unchanged");
    }
    Ok(())
}
pub(super) fn execute(env: &Guest) -> Result<()> {
    ensure!(account(env)?.is_none(), "fixture account already exists");
    let home = "/tmp/work/payload/become-home";
    for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
        if phase == "drift_repair" {
            drift(env)?;
        }
        let preview = matches!(phase, "check" | "diff");
        let before = env.snapshot()?;
        let events = env.run(if preview { phase } else { "apply" }, 0)?;
        if preview {
            verify_preview(&events, home)?;
            ensure!(env.snapshot()? == before, "become preview mutated state");
            verify_account(env, "/bin/sh")?;
        } else {
            verify_account(env, "/bin/sh")?;
            ensure!(
                env.docker.exec(&["/usr/bin/test", "-d", home], 0).is_ok(),
                "fixture home missing"
            );
            verify_result(&events, home)?;
        }
        if phase == "converged" {
            ensure!(
                events
                    .iter()
                    .filter_map(|e| e.get("result"))
                    .all(|r| r["changed"] == false),
                "converged become task changed"
            );
        }
        println!("PASS become-environment {phase}");
    }
    verify_drift_limitation(env, home)
}
#[cfg(test)]
mod tests {
    use super::verify_preview;
    use serde_json::json;
    #[test]
    fn preview_requires_correct_environment_and_successful_assertion() -> anyhow::Result<()> {
        let mut events = vec![
            json!({"host":"fixture","task":"Create synthetic become account","result":{"changed":false,"failed":false}}),
            json!({"host":"fixture","task":"Capture synthetic become environment","result":{"failed":false,"stdout_lines":["/tmp/home","ruxelbecome","ruxelbecome","/bin/sh","synthetic"]}}),
            json!({"host":"fixture","task":"Verify explicit become environment","result":{"failed":false}}),
            json!({"host":"fixture","recap":{"failed":0,"skipped":0}}),
        ];
        verify_preview(&events, "/tmp/home")?;
        events[1]["result"]["skipped"] = json!(true);
        assert!(verify_preview(&events, "/tmp/home").is_err());
        Ok(())
    }
}
