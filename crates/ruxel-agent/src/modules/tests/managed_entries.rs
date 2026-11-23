//! The managed sysctl entries: only the sysctl module consumes them,
//! so only the sysctl module pays for them.

use crate::ledger::Ledger;

#[test]
fn only_the_sysctl_module_computes_the_managed_entries() {
    // The managed entries are the scan of every persisted record: a
    // workflow of ordinary tasks would pay 1+2+…+N inspections for a
    // result none of them reads.
    let dir = std::env::temp_dir().join(format!("ruxel-managed-entries-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(&dir).expect("operation succeeds");
    let file = dir.join("90-ruxel.conf");
    std::fs::write(&file, b"net.ipv4.ip_forward=1\n").expect("operation succeeds");
    let mut ledger = Ledger::load(&dir);
    ledger.record(
        "sysctl-task",
        "sysctl",
        &serde_json::json!({
            "name": "net.ipv4.ip_forward",
            "value": "1",
            "sysctl_file": file.to_string_lossy(),
        }),
        "ok",
        &serde_json::json!({"changed": true}),
    );
    assert!(
        super::managed_sysctl_entries("file", &ledger).is_empty(),
        "a task that reads none of them gets none of them"
    );
    assert_eq!(
        ledger.scans(),
        0,
        "a task that reads none of them scans none of the records"
    );
    assert_eq!(
        super::managed_sysctl_entries("sysctl", &ledger).len(),
        1,
        "the sysctl module reads the entries of the sysctl tasks"
    );
    assert_eq!(ledger.scans(), 1);
    drop(std::fs::remove_dir_all(&dir));
}
