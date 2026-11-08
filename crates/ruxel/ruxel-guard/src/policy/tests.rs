use crate::{Policy, deny_hits};

#[test]
fn mapped_deny_network_refuses_both_address_forms() -> Result<(), Box<dyn std::error::Error>> {
    let policy = Policy::new(&["192.0.2.0/24".into()], &["::ffff:192.0.2.0/120".into()])?;
    assert!(policy.decide("192.0.2.7".parse()?).is_err());
    assert!(policy.decide("::ffff:192.0.2.7".parse()?).is_err());
    assert!(!deny_hits(&policy, &["192.0.2.0/24", "::ffff:192.0.2.0/120"]).is_empty());
    Ok(())
}

#[test]
fn mapped_network_literal_intersects_ipv4_deny() -> Result<(), Box<dyn std::error::Error>> {
    let policy = Policy::new(&[], &["192.0.2.7/32".into()])?;
    assert!(!deny_hits(&policy, &["::ffff:192.0.2.0/120"]).is_empty());
    Ok(())
}

#[test]
fn broad_ipv6_deny_covers_mapped_ipv4() -> Result<(), Box<dyn std::error::Error>> {
    let policy = Policy::new(&["192.0.2.0/24".into()], &["::/0".into()])?;
    assert!(policy.decide("192.0.2.7".parse()?).is_err());
    assert!(!deny_hits(&policy, &["192.0.2.7"]).is_empty());
    Ok(())
}
