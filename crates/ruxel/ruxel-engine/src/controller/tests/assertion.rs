use super::fact_control::Precise;
use super::*;

#[tokio::test]
async fn assertions_skip_unneeded_conditions_and_failure_messages() -> Result<()> {
    for list_expr in [false, true] {
        let missing = json!({"kind":"ref","path":["vars","missing"]});
        let entries = json!([false, missing]);
        let that = if list_expr {
            json!({"kind":"list","items":[{"kind":"literal","value":false},missing]})
        } else {
            entries
        };
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
            {"name":"success","operation":{"kind":"assert","that":true,"fail_msg":missing}},
            {"name":"false first","ignoreErrors":true,"operation":{"kind":"assert","that":that}},
            {"name":"false before invalid","ignoreErrors":true,"operation":{"kind":"assert","that":[false,{"kind":"literal","value":"wrong"}]}},
            {"name":"after","operation":{"kind":"debug","msg":"continued"}}
        ]}]});
        let mut backend = Fake::default();
        let recap = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await?;
        assert_eq!(recap["h"].failed, 0);
        assert_eq!(recap["h"].ignored, 2);
        assert_eq!(backend.events[1]["result"]["failed"], true);
        assert_eq!(backend.events[2]["result"]["failed"], true);
        assert_eq!(backend.events[3]["result"]["msg"], "continued");
    }
    Ok(())
}

#[tokio::test]
async fn assertions_still_evaluate_needed_conditions_and_failure_messages() {
    for operation in [
        json!({"kind":"assert","that":[true,{"kind":"ref","path":["vars","missing"]}]}),
        json!({"kind":"assert","that":false,"fail_msg":{"kind":"ref","path":["vars","missing"]}}),
    ] {
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"operation":operation}]}]});
        assert!(
            run(
                &config,
                &BTreeMap::new(),
                &Options::default(),
                &mut Fake::default(),
                &mut BTreeMap::new()
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn assertion_result_inherits_accessed_sensitive_condition() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"private condition","noLog":true,"operation":{"kind":"set_fact","values":{"gate":false}}},
        {"name":"assert","ignoreErrors":true,"operation":{"kind":"assert","that":[{"kind":"ref","path":["vars","gate"]}, {"kind":"ref","path":["vars","missing"]}]}}
    ]}]});
    let mut backend = Precise::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.0.events[1]["result"]["censored"], true);
    Ok(())
}

#[tokio::test]
async fn computed_assertion_list_validates_every_materialized_value() -> Result<()> {
    for (values, expected_error) in [
        (json!([false, "wrong"]), true),
        (json!([false, true]), false),
    ] {
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"computed":{"kind":"literal","value":values}},"tasks":[
            {"name":"computed","ignoreErrors":true,"operation":{"kind":"assert","that":{"kind":"ref","path":["vars","computed"]}}}
        ]}]});
        let mut backend = Fake::default();
        let result = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(result.is_err(), expected_error);
        if expected_error {
            assert!(
                result
                    .expect_err("invalid list")
                    .to_string()
                    .contains("assertion must be boolean")
            );
        } else {
            assert_eq!(result?["h"].ignored, 1);
        }
    }
    Ok(())
}
