use super::*;

#[test]
fn retry_delay_defaults_to_five_and_preserves_explicit_values() {
    assert_eq!(super::super::retry::delay_seconds(&json!({})), 5);
    assert_eq!(
        super::super::retry::delay_seconds(&json!({"delaySeconds":0})),
        0
    );
    assert_eq!(
        super::super::retry::delay_seconds(&json!({"delaySeconds":2})),
        2
    );
}

#[tokio::test]
async fn retry_count_is_extra_attempts_for_remote_and_local_operations() -> Result<()> {
    for kind in ["command", "debug"] {
        for retries in [None, Some(0), Some(1), Some(2)] {
            for done in [false, true] {
                let operation = if kind == "command" {
                    json!({"kind":"command","cmd":"fixture"})
                } else {
                    json!({"kind":"debug","msg":"fixture"})
                };
                let mut task = json!({"name":"retry","ignoreErrors":true,"delaySeconds":0,"until":{"kind":"literal","value":done},"operation":operation});
                if let Some(retries) = retries {
                    task["retries"] = json!(retries);
                }
                let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[task]}]});
                let mut backend = Fake::default();
                let recaps = run(
                    &config,
                    &BTreeMap::new(),
                    &Options::default(),
                    &mut backend,
                    &mut BTreeMap::new(),
                )
                .await?;
                let expected = if done { 1 } else { retries.unwrap_or(0) + 1 };
                assert_eq!(backend.events[0]["result"]["attempts"], expected);
                assert_eq!(
                    backend.events[0]["result"]["failed"]
                        .as_bool()
                        .unwrap_or(false),
                    !done
                );
                assert_eq!(recaps["h"].ignored, u64::from(!done));
                if kind == "command" {
                    assert_eq!(backend.calls.len(), usize::try_from(expected)?);
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn retries_without_until_do_not_repeat_failed_operation() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"failed","retries":2,"operation":{"kind":"command","cmd":"fixture"}}]}]});
    let mut backend = Fake {
        results: [json!({"failed":true})].into(),
        ..Fake::default()
    };
    let recap = run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1);
    assert_eq!(recap["h"].failed, 1);
    assert!(backend.events[0]["result"].get("attempts").is_none());
    Ok(())
}
