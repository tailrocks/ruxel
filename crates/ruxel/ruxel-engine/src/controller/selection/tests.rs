use super::*;
use serde_json::json;

#[test]
fn group_and_glob_filters_resolve_to_concrete_targets() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"node-a":{},"node-b":{},"db":{}},"groups":{"web":{"hosts":["node-a","node-b"]}}},"plays":[{"targets":["all"]}]});
    let options = Options {
        hosts: vec!["web".into()],
        ..Options::default()
    };
    assert_eq!(selected_hosts(&config, &options)?, ["node-a", "node-b"]);
    let options = Options {
        hosts: vec!["node-?".into()],
        ..Options::default()
    };
    assert_eq!(selected_hosts(&config, &options)?, ["node-a", "node-b"]);
    let options = Options {
        hosts: vec!["missing*".into()],
        ..Options::default()
    };
    assert!(selected_hosts(&config, &options).is_err());
    Ok(())
}

#[test]
fn wildcard_play_target_and_disjoint_filter_are_checked() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"node-a":{},"db":{}},"groups":{}},"plays":[{"targets":["node-*"]}]});
    assert_eq!(selected_hosts(&config, &Options::default())?, ["node-a"]);
    let options = Options {
        hosts: vec!["db".into()],
        ..Options::default()
    };
    assert!(selected_hosts(&config, &options)?.is_empty());
    Ok(())
}
