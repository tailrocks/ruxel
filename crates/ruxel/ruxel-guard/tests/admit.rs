use ruxel_guard::{Guard, HostSpec, Policy};

#[test]
fn denied_host_is_refused_without_ssh() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("ruxel-guard-integration-{}", std::process::id()));
    let policy = Policy::new(&["192.0.2.0/24".into()], &["198.51.100.0/24".into()])?;
    let guard = Guard::new(policy, dir.clone());
    let spec = HostSpec {
        name: "denied".into(),
        address: "198.51.100.10".parse()?,
        port: 22,
        user: "test".into(),
        identity_file: "/tmp/key".into(),
        known_hosts_file: "/tmp/known".into(),
    };
    let result = guard.admit_all(&[spec], &[]);
    assert!(result.is_err());
    std::fs::remove_dir(dir)?;
    Ok(())
}
