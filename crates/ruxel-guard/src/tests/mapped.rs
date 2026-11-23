use super::*;
#[test]
fn mapped_deny_network_blocks_ipv4_and_mapped_hosts() {
    let policy = Policy::new(&["192.0.2.0/24".into()], &["::ffff:192.0.2.0/120".into()]).unwrap();
    assert!(policy.decide("192.0.2.7".parse().unwrap()).is_err());
    assert!(policy.decide("::ffff:192.0.2.7".parse().unwrap()).is_err());
    assert!(!deny_hits(&policy, &["192.0.2.0/24"]).is_empty());
}
#[test]
fn mapped_literal_network_intersects_ipv4_deny() {
    assert!(!deny_hits(&policy(), &["::ffff:192.0.2.0/120"]).is_empty());
}
#[test]
fn ipv6_allow_does_not_admit_ipv4() {
    let policy = Policy::new(&["::/0".into()], &[]).unwrap();
    assert!(policy.decide("192.0.2.7".parse().unwrap()).is_err());
    assert!(policy.decide("::ffff:192.0.2.7".parse().unwrap()).is_err());
}
