use ruxel_guard::{Guard, HostSpec, Policy};

#[test]
fn denied_host_is_refused_without_ssh() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::DirBuilderExt;
    let home = std::fs::canonicalize(std::env::var_os("HOME").ok_or("HOME is required")?)?;
    let mut random = [0; 8];
    getrandom::fill(&mut random)?;
    let root = home.join(format!(".ruxel-admit-{:016x}", u64::from_ne_bytes(random)));
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let dir = root.join("mux");
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
    let Err(refusals) = result else {
        return Err("denied host must be refused".into());
    };
    assert_eq!(refusals.0.len(), 1);
    assert_eq!(refusals.0[0].host, "denied");
    assert!(refusals.0[0].rule.starts_with("deny "));
    std::fs::remove_dir(dir)?;
    std::fs::remove_dir(root)?;
    Ok(())
}
