use super::*;

#[tokio::test]
async fn body_and_rescue_controller_errors_still_run_always() -> Result<()> {
    for rescue_error in [false, true] {
        let body = if rescue_error {
            json!({"name":"body","operation":{"kind":"fail","msg":"failed"}})
        } else {
            json!({"name":"body","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","body_missing"]}}})
        };
        let rescue = if rescue_error {
            vec![
                json!({"name":"rescue","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","rescue_missing"]}}}),
            ]
        } else {
            Vec::new()
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
            {"name":"block","block":[body],"rescue":rescue,"always":[{"name":"cleanup","operation":{"kind":"debug","msg":"cleanup"}}]},
            {"name":"after","operation":{"kind":"debug","msg":"not reached"}}
        ]}]});
        let mut backend = Fake::default();
        let error = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await
        .err()
        .context("expected controller failure")?;
        assert!(error.to_string().contains(if rescue_error {
            "rescue_missing"
        } else {
            "body_missing"
        }));
        let names: Vec<_> = backend
            .events
            .iter()
            .filter_map(|event| event["task"].as_str())
            .collect();
        assert!(names.contains(&"cleanup"));
        assert!(!names.contains(&"after"));
    }
    Ok(())
}

#[tokio::test]
async fn original_error_wins_when_cleanup_also_errors() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"block","block":[{"name":"body","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","body_missing"]}}}],"always":[{"name":"cleanup","operation":{"kind":"debug","msg":{"kind":"ref","path":["vars","cleanup_missing"]}}}]}
    ]}]});
    let error = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut Fake::default(),
        &mut BTreeMap::new(),
    )
    .await
    .err()
    .context("expected controller failure")?;
    assert!(error.to_string().contains("body_missing"));
    assert!(!error.to_string().contains("cleanup_missing"));
    Ok(())
}
