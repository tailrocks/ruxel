use super::*;

#[tokio::test]
async fn evaluated_url_and_timezone_are_validated_before_dispatch() -> Result<()> {
    for (kind, field, value, accepted) in [
        ("get_url", "url", "file:///tmp/x", false),
        ("get_url", "url", "https://example.invalid/x", true),
        ("timezone", "name", "", false),
        ("timezone", "name", "--help", false),
        ("timezone", "name", "UTC", true),
    ] {
        let mut operation = json!({"kind":kind});
        operation[field] = json!({"kind":"ref", "path":["vars", "value"]});
        if kind == "get_url" {
            operation["dest"] = json!("/tmp/x");
        }
        let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"vars":{"value":{"kind":"literal","value":value}},"tasks":[{"name":"test","operation":operation}]}]});
        let mut backend = Fake::default();
        let result = run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(result.is_ok(), accepted, "{kind}: {value:?}");
        assert_eq!(
            backend.calls.len(),
            usize::from(accepted),
            "{kind}: {value:?}"
        );
        if accepted {
            let params: Value =
                serde_json::from_slice(&backend.calls[0].task.iterations[0].params_json)?;
            assert_eq!(params[field], value);
        }
    }
    Ok(())
}
