use super::*;
use ruxel_proto::v1;
use serde_json::json;

fn request(module: &str, params: &Value) -> Result<Request> {
    Ok(Request {
        host: "fixture".into(),
        content_fingerprint: None,
        task: v1::RenderedTask {
            module: module.into(),
            iterations: vec![v1::Iteration {
                params_json: serde_json::to_vec(params)?,
                ..Default::default()
            }],
            ..Default::default()
        },
    })
}

#[test]
fn tracks_sensitive_paths_and_unknown_side_effects_per_destination() -> Result<()> {
    let mut resources = Resources::default();
    assert!(resources.observe(
        &request("copy", &json!({"dest":"/tmp/fixture-private"}))?,
        true
    )?);
    assert!(!resources.observe(
        &request("slurp", &json!({"src":"/tmp/fixture-public"}))?,
        false
    )?);
    assert!(resources.observe(
        &request("slurp", &json!({"src":"/tmp/fixture-private"}))?,
        false
    )?);
    assert!(resources.observe(
        &request("shell", &json!({"cmd":"cat /tmp/fixture-private"}))?,
        false
    )?);
    assert!(resources.observe(&request("shell", &json!({"cmd":"synthetic"}))?, true)?);
    assert!(resources.observe(&request("slurp", &json!({"src":"/tmp/any"}))?, false)?);
    let mut other_destination = Resources::default();
    assert!(!other_destination.observe(&request("slurp", &json!({"src":"/tmp/any"}))?, false)?);
    Ok(())
}

#[test]
fn ambiguous_private_destinations_taint_later_reads() -> Result<()> {
    for destination in [
        "/tmp/a/../fixture-private",
        "fixture-private",
        "../fixture-private",
    ] {
        let mut resources = Resources::default();
        assert!(resources.observe(&request("copy", &json!({"dest":destination}))?, true)?);
        assert!(resources.has_sensitive_effects());
        for source in ["/tmp/fixture-private", "/other/alias"] {
            assert!(resources.observe(&request("slurp", &json!({"src":source}))?, false)?);
        }
    }
    Ok(())
}

#[test]
fn ambiguous_reads_cannot_escape_known_private_paths() -> Result<()> {
    let mut resources = Resources::default();
    assert!(resources.observe(
        &request("copy", &json!({"dest":"/tmp/fixture-private"}))?,
        true
    )?);
    for source in [
        "/tmp/a/../fixture-private",
        "fixture-private",
        "../fixture-private",
        "/tmp/./fixture-private",
    ] {
        assert!(resources.observe(&request("slurp", &json!({"src":source}))?, false)?);
    }
    Ok(())
}
