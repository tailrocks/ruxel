use super::*;

pub fn check_semantics(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let bytes = fs::read(root.join("testdata/workloads/pr66/check-semantics.pkl"))?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        actual == FIXTURE_HASH,
        "fixture changed; review it before running locally"
    );
    let temp = prepare(root)?;
    let fixture_dir = temp.path().join("testdata/workloads/pr66");
    fs::create_dir_all(&fixture_dir)?;
    let payload = temp.path().join("payload");
    fs::create_dir(&payload)?;
    let escaped = pkl_string(path_text(&payload)?)?;
    let rendered =
        std::str::from_utf8(&bytes)?.replace("__RUN_ROOT__", &escaped[1..escaped.len() - 1]);
    let module = fixture_dir.join("check-semantics.pkl");
    fs::write(&module, rendered)?;
    let state = temp.path().join("state");
    let target = payload.join("check-notify");
    let events = run(cli, agent, &module, &state, "plan")?;
    ensure!(
        recap_for_host(&events, "[censored]")?["changed"] == 0 && !target.exists(),
        "plan mutated or reported a change"
    );
    for mode in ["check", "diff"] {
        let events = run(cli, agent, &module, &state, mode)?;
        ensure!(
            recap(&events)?["changed"].as_u64().unwrap_or(0) > 0 && !target.exists(),
            "{mode} failed to predict without writing"
        );
        let handler = result(&events, "Record predicted handler")?;
        ensure!(
            handler["msg"] == "predicted handler ran"
                && handler["failed"] != true
                && handler["skipped"] != true,
            "{mode} handler failed or skipped"
        );
        if mode == "diff" {
            ensure!(
                events.iter().any(|event| event["result"]["diff"]
                    .as_str()
                    .is_some_and(|diff| !diff.is_empty())),
                "diff omitted file difference"
            );
        }
    }
    let events = run(cli, agent, &module, &state, "apply")?;
    ensure!(
        recap(&events)?["changed"].as_u64().unwrap_or(0) > 0
            && fs::read_to_string(&target)? == "synthetic",
        "fresh apply failed"
    );
    let events = run(cli, agent, &module, &state, "apply")?;
    ensure!(
        recap(&events)?["changed"] == 0 && fs::read_to_string(&target)? == "synthetic",
        "rerun did not converge"
    );
    fs::write(&target, "drift")?;
    for mode in ["check", "diff"] {
        let events = run(cli, agent, &module, &state, mode)?;
        ensure!(
            recap(&events)?["changed"].as_u64().unwrap_or(0) > 0
                && fs::read_to_string(&target)? == "drift",
            "{mode} changed drift or failed to predict"
        );
    }
    let events = run(cli, agent, &module, &state, "apply")?;
    ensure!(
        recap(&events)?["changed"].as_u64().unwrap_or(0) > 0
            && fs::read_to_string(&target)? == "synthetic",
        "drift was not repaired"
    );
    println!("PASS check semantics: plan, check, diff, convergence and drift repair");
    Ok(())
}
