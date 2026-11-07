use super::fact_control::Precise;
use super::*;

#[tokio::test]
async fn private_loop_labels_are_redacted_before_agent_dispatch() -> Result<()> {
    for explicit in [false, true] {
        for private in [false, true] {
            let mut task = json!({"name":"loop","loop":{"kind":"ref","path":["vars","items"]},"operation":{"kind":"command","cmd":"synthetic"}});
            if explicit {
                task["loopLabel"] = json!({"kind":"ref","path":["item"]});
            }
            let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[task]}]});
            let mut facts = BTreeMap::from([(
                "h".into(),
                TrackedValue::object(BTreeMap::from([(
                    "items".into(),
                    if private {
                        TrackedValue::secret(json!(["fixture"]))
                    } else {
                        TrackedValue::public(json!(["fixture"]))
                    },
                )])),
            )]);
            let mut backend = Precise::default();
            run(
                &config,
                &BTreeMap::new(),
                &Options::default(),
                &mut backend,
                &mut facts,
            )
            .await?;
            let request = &backend.0.calls[0].task;
            let expected = if private {
                "[censored]"
            } else if explicit {
                "fixture"
            } else {
                "\"fixture\""
            };
            assert_eq!(request.iterations[0].item_label, expected);
            assert_eq!(request.no_log, private);
        }
    }
    Ok(())
}

#[tokio::test]
async fn sensitive_explicit_label_is_redacted_for_public_item() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"loop","loop":{"kind":"ref","path":["vars","items"]},"loopLabel":{"kind":"ref","path":["vars","privateLabel"]},"operation":{"kind":"command","cmd":"synthetic"}}
    ]}]});
    let mut facts = BTreeMap::from([(
        "h".into(),
        TrackedValue::object(BTreeMap::from([
            ("items".into(), TrackedValue::public(json!(["public"]))),
            (
                "privateLabel".into(),
                TrackedValue::secret(json!("synthetic")),
            ),
        ])),
    )]);
    let mut backend = Precise::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut facts,
    )
    .await?;
    let request = &backend.0.calls[0].task;
    assert_eq!(request.iterations[0].item_label, "[censored]");
    assert!(request.no_log);
    Ok(())
}
