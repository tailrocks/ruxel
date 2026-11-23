//! The plan's static sysctl declarations: the frame must carry the
//! full unconditional declaration list before anything executes, so
//! an earlier task's preflight weighs the later stages' files on the
//! first run — with no ledger history and no executed task yet.

use super::*;
use serde_json::json;

fn decl(file: &str, name: &str, value: &str) -> v1::SysctlDeclaration {
    v1::SysctlDeclaration {
        file: file.into(),
        name: name.into(),
        value: value.into(),
    }
}

fn play(tasks: &Value) -> Value {
    json!({"targets": ["h"], "tasks": tasks.clone()})
}

#[test]
fn only_unconditional_literal_sysctl_tasks_declare_up_front() {
    // The scan is a sound under-approximation: a gated, looped,
    // delegated, nested or templated task stays out and keeps today's
    // behavior, while an unconditional literal task is guaranteed to
    // declare exactly its entry.
    let tasks = json!([
        {"name": "plain", "operation": {"kind": "sysctl", "name": "net.test", "value": "1", "sysctl_file": "/etc/sysctl.d/10-a.conf"}},
        {"name": "coerced", "operation": {"kind": "sysctl", "name": "net.test", "value": 2, "sysctl_file": "/etc/sysctl.d/10-b.conf"}},
        {"name": "default-file", "operation": {"kind": "sysctl", "name": "net.test", "value": true}},
        {"name": "gated", "operation": {"kind": "sysctl", "name": "net.test", "value": "9", "sysctl_file": "/etc/sysctl.d/50-g.conf"}, "when": {"kind": "literal", "value": true}},
        {"name": "looped", "operation": {"kind": "sysctl", "name": "net.test", "value": "9", "sysctl_file": "/etc/sysctl.d/50-l.conf"}, "loop": {"kind": "literal", "value": [1]}},
        {"name": "delegated", "operation": {"kind": "sysctl", "name": "net.test", "value": "9", "sysctl_file": "/etc/sysctl.d/50-d.conf"}, "delegateTo": "other"},
        {"name": "nested", "block": [{"name": "child", "operation": {"kind": "sysctl", "name": "net.test", "value": "9", "sysctl_file": "/etc/sysctl.d/50-n.conf"}}]},
        {"name": "templated-name", "operation": {"kind": "sysctl", "name": {"kind": "literal", "value": "net.test"}, "value": "9", "sysctl_file": "/etc/sysctl.d/50-t.conf"}},
        {"name": "templated-value", "operation": {"kind": "sysctl", "name": "net.test", "value": {"kind": "ref", "path": ["vars", "v"]}, "sysctl_file": "/etc/sysctl.d/50-v.conf"}},
        {"name": "placeholder-file", "operation": {"kind": "sysctl", "name": "net.test", "value": "9", "sysctl_file": "/etc/sysctl.d/__ruxel_rt_8d41.x__.conf"}},
        {"name": "other-kind", "operation": {"kind": "command", "argv": ["true"]}}
    ]);
    assert_eq!(
        super::super::static_sysctl::static_sysctl_declarations(&play(&tasks), &[]),
        vec![
            decl("/etc/sysctl.d/10-a.conf", "net.test", "1"),
            decl("/etc/sysctl.d/10-b.conf", "net.test", "2"),
            decl(ruxel_proto::constants::DEFAULT_SYSCTL_FILE, "net.test", "1"),
        ],
        "only unconditional literal sysctl tasks declare up front"
    );
}

#[test]
fn tag_selection_gates_the_static_set() {
    // With a tag filter, only a selected task is guaranteed to run, so
    // only it declares up front. Without one, every unconditional task
    // runs and declares.
    let tasks = json!([
        {"name": "tagged", "operation": {"kind": "sysctl", "name": "net.test", "value": "1", "sysctl_file": "/etc/sysctl.d/10-a.conf"}, "tags": ["x"]},
        {"name": "untagged", "operation": {"kind": "sysctl", "name": "net.test", "value": "2", "sysctl_file": "/etc/sysctl.d/10-b.conf"}}
    ]);
    assert_eq!(
        super::super::static_sysctl::static_sysctl_declarations(&play(&tasks), &["x".into()]),
        vec![decl("/etc/sysctl.d/10-a.conf", "net.test", "1")],
        "the filter keeps only the selected task's declaration"
    );
    assert_eq!(
        super::super::static_sysctl::static_sysctl_declarations(&play(&tasks), &[]),
        vec![
            decl("/etc/sysctl.d/10-a.conf", "net.test", "1"),
            decl("/etc/sysctl.d/10-b.conf", "net.test", "2"),
        ],
        "without a filter every unconditional task declares"
    );
}

#[test]
fn forced_check_tasks_declare_nothing_up_front() {
    // Thread 343: a task with `checkMode: true` never writes its
    // persistence file (`effective_check_mode`), so its declaration
    // must not authorize earlier preflights — a pre-existing
    // later-loading file matching the declaration would read as
    // managed and suppress the override error, letting the apply
    // succeed while reboot restores the overriding value. An explicit
    // `checkMode: false` still executes, so it still declares.
    let tasks = json!([
        {"name": "forced", "checkMode": true, "operation": {"kind": "sysctl", "name": "net.test", "value": "1", "sysctl_file": "/etc/sysctl.d/10-forced.conf"}},
        {"name": "override-off", "checkMode": false, "operation": {"kind": "sysctl", "name": "net.test", "value": "2", "sysctl_file": "/etc/sysctl.d/10-override.conf"}},
        {"name": "plain", "operation": {"kind": "sysctl", "name": "net.test", "value": "3", "sysctl_file": "/etc/sysctl.d/10-plain.conf"}}
    ]);
    assert_eq!(
        super::super::static_sysctl::static_sysctl_declarations(&play(&tasks), &[]),
        vec![
            decl("/etc/sysctl.d/10-override.conf", "net.test", "2"),
            decl("/etc/sysctl.d/10-plain.conf", "net.test", "3"),
        ],
        "a forced-check task declares nothing up front"
    );
}

#[tokio::test]
async fn every_frame_carries_later_plays_declarations() -> Result<()> {
    // Thread 325: the declaration lives in a later play (a later
    // workflow stage), not a later task of the same play. The first
    // play's frames already carry it, so the agent's earlier preflight
    // weighs it before the later stage executes — on a first run with
    // no ledger history, where nothing declares it yet.
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[
        {"targets":["h"],"tasks":[
            {"name":"earlier","operation":{"kind":"command","argv":["true"]}}
        ]},
        {"targets":["h"],"tasks":[
            {"name":"later","operation":{"kind":"sysctl","name":"net.test","value":"2","sysctl_file":"/etc/sysctl.d/99-b.conf"}}
        ]}
    ]});
    let mut backend = Fake::default();
    run(
        &config,
        &std::collections::BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut std::collections::BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 2);
    assert_eq!(
        backend.calls[0].plan_sysctl,
        vec![decl("/etc/sysctl.d/99-b.conf", "net.test", "2")],
        "the first play's frame carries the later play's declaration"
    );
    assert_eq!(
        backend.calls[1].plan_sysctl,
        vec![decl("/etc/sysctl.d/99-b.conf", "net.test", "2")],
        "the declaring play's own frame carries it too"
    );
    Ok(())
}

#[tokio::test]
async fn delegated_requests_carry_the_destinations_declarations() -> Result<()> {
    // Thread 327: the delegated request runs on the destination, so it
    // carries the destination's static set — recomputed from the full
    // configuration — not the source host's. A source-only declaration
    // must never authorize the destination's preflight.
    let config = json!({"inventory":{"hosts":{"h1":{},"h2":{}},"groups":{}},"plays":[
        {"targets":["h1"],"tasks":[
            {"name":"source-only","operation":{"kind":"sysctl","name":"net.test","value":"1","sysctl_file":"/etc/sysctl.d/10-a.conf"}},
            {"name":"delegated","operation":{"kind":"command","argv":["true"]},"delegateTo":"h2","delegateAllow":["h2"]}
        ]},
        {"targets":["h2"],"tasks":[
            {"name":"destination-only","operation":{"kind":"sysctl","name":"net.test","value":"2","sysctl_file":"/etc/sysctl.d/10-b.conf"}}
        ]}
    ]});
    let mut backend = Fake::default();
    run(
        &config,
        &std::collections::BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut std::collections::BTreeMap::new(),
    )
    .await?;
    let delegated = backend
        .calls
        .iter()
        .find(|call| call.host == "h2" && call.task.name == "delegated")
        .expect("the delegated task reaches h2");
    assert_eq!(
        delegated.plan_sysctl,
        vec![decl("/etc/sysctl.d/10-b.conf", "net.test", "2")],
        "the delegated request carries h2's declarations, not h1's source-only set"
    );
    Ok(())
}

#[tokio::test]
async fn split_play_runs_carry_the_full_configs_declarations() -> Result<()> {
    // Thread 325, the scheduler's shape: the controller run covers one
    // play split out of the full configuration, and its frames still
    // carry the full configuration's host-applicable set.
    let full = json!({"inventory":{"hosts":{"h":{}}},"plays":[
        {"targets":["h"],"tasks":[
            {"name":"earlier","operation":{"kind":"command","argv":["true"]}}
        ]},
        {"targets":["h"],"tasks":[
            {"name":"later","operation":{"kind":"sysctl","name":"net.test","value":"2","sysctl_file":"/etc/sysctl.d/99-b.conf"}}
        ]}
    ]});
    let mut play_config = full.clone();
    play_config["plays"] =
        serde_json::Value::Array(vec![full["plays"].as_array().expect("plays")[0].clone()]);
    let mut backend = Fake::default();
    run_with_full_config(
        &play_config,
        &full,
        &std::collections::BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut std::collections::BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.calls.len(), 1);
    assert_eq!(
        backend.calls[0].plan_sysctl,
        vec![decl("/etc/sysctl.d/99-b.conf", "net.test", "2")],
        "the split play's frame carries the later play's declaration"
    );
    Ok(())
}

#[test]
fn declarations_for_host_cover_only_the_plays_that_run_there() {
    // Thread 325, per-host filtering: a declaration from a play that
    // never runs on the host must not authorize that host's preflight
    // — the machines share nothing, so the entry would be unmanaged
    // there.
    let config = json!({"inventory":{"hosts":{"h1":{},"h2":{}},"groups":{}},"plays":[
        {"targets":["h1"],"tasks":[
            {"name":"first","operation":{"kind":"sysctl","name":"net.test","value":"1","sysctl_file":"/etc/sysctl.d/10-a.conf"}}
        ]},
        {"targets":["h2"],"tasks":[
            {"name":"second","operation":{"kind":"sysctl","name":"net.test","value":"2","sysctl_file":"/etc/sysctl.d/99-b.conf"}}
        ]}
    ]});
    assert_eq!(
        super::super::static_sysctl::static_sysctl_declarations_for_host(&config, "h1", &[]),
        vec![decl("/etc/sysctl.d/10-a.conf", "net.test", "1")],
        "h1 carries only its own play's declaration"
    );
    assert_eq!(
        super::super::static_sysctl::static_sysctl_declarations_for_host(&config, "h2", &[]),
        vec![decl("/etc/sysctl.d/99-b.conf", "net.test", "2")],
        "h2 carries only its own play's declaration"
    );
    assert!(
        super::super::static_sysctl::static_sysctl_declarations_for_host(&config, "elsewhere", &[])
            .is_empty(),
        "a host no play runs on declares nothing up front"
    );
}

#[tokio::test]
async fn every_frame_carries_the_full_static_set() -> Result<()> {
    // Thread 314 end-to-end at the controller seam: the first request
    // already carries the later stage's declaration, so the agent's
    // earlier preflight weighs it before the later task executes.
    // The gated and expression-named tasks stay out of the set.
    let config = json!({"inventory":{"hosts":{"h":{}}},"plays":[{"targets":["h"],"tasks":[
        {"name":"earlier","operation":{"kind":"sysctl","name":"net.test","value":"1","sysctl_file":"/etc/sysctl.d/10-a.conf"}},
        {"name":"later","operation":{"kind":"sysctl","name":"net.test","value":"2","sysctl_file":"/etc/sysctl.d/99-b.conf"}},
        {"name":"gated","operation":{"kind":"sysctl","name":"net.test","value":"3","sysctl_file":"/etc/sysctl.d/50-g.conf"},"when":{"kind":"literal","value":false}},
        {"name":"named-by-expression","operation":{"kind":"sysctl","name":{"kind":"literal","value":"net.test"},"value":"4","sysctl_file":"/etc/sysctl.d/60-e.conf"}}
    ]}]});
    let mut backend = Fake::default();
    let options = Options::default();
    let recaps = run(
        &config,
        &std::collections::BTreeMap::new(),
        &options,
        &mut backend,
        &mut std::collections::BTreeMap::new(),
    )
    .await?;
    assert_eq!(recaps["h"].failed, 0);
    assert!(recaps["h"].filtered, "the gated task filters the run");
    let full = vec![
        decl("/etc/sysctl.d/10-a.conf", "net.test", "1"),
        decl("/etc/sysctl.d/99-b.conf", "net.test", "2"),
    ];
    assert_eq!(backend.calls.len(), 3, "the gated task never executes");
    for call in &backend.calls {
        assert_eq!(
            call.plan_sysctl, full,
            "{} carries the full static set",
            call.task.name
        );
    }
    Ok(())
}
