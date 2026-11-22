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
