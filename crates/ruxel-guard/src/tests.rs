#![allow(
    clippy::unwrap_used,
    clippy::assert_is_empty,
    reason = "test fixtures and assertions fail the test on invalid data"
)]

use crate::{
    HostSpec, Policy, cidr_file,
    resolve::{self, ArgsMode},
    scan::deny_hits,
};
use std::{net::IpAddr, path::PathBuf};

fn policy() -> Policy {
    Policy::new(
        &["192.0.2.0/24".into(), "2001:db8::/32".into()],
        &["192.0.2.7/32".into(), "2001:db8:dead::/64".into()],
    )
    .unwrap()
}
fn spec() -> HostSpec {
    HostSpec {
        name: "test".into(),
        address: "192.0.2.10".parse().unwrap(),
        port: 22,
        user: "test".into(),
        identity_file: PathBuf::from("/tmp/key"),
        known_hosts_file: PathBuf::from("/tmp/known"),
    }
}
fn config() -> &'static str {
    "hostname 192.0.2.10\nuser test\nport 22\nforwardagent no\nstricthostkeychecking true\nuserknownhostsfile /tmp/known\n"
}
#[test]
fn deny_beats_allow() {
    assert!(policy().decide("192.0.2.7".parse().unwrap()).is_err());
}
#[test]
fn allow_works() {
    assert!(policy().decide("192.0.2.10".parse().unwrap()).is_ok());
}
#[test]
fn ipv6_denied() {
    assert!(
        policy()
            .decide("2001:db8:dead::1".parse().unwrap())
            .is_err()
    );
}
#[test]
fn mapped_denied() {
    assert!(
        policy()
            .decide("::ffff:192.0.2.7".parse().unwrap())
            .is_err()
    );
}
#[test]
fn empty_allow() {
    assert!(
        Policy::new(&[], &[])
            .unwrap()
            .decide("192.0.2.10".parse().unwrap())
            .is_err()
    );
}
#[test]
fn parse_file() {
    assert_eq!(
        cidr_file::parse(include_str!("../testdata/deny-test.cidr"))
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn bad_cidr() {
    assert!(cidr_file::parse("bad").is_err());
}
#[test]
fn hostname_name_rejected() {
    assert!(
        resolve::check_ssh_g(&config().replace("192.0.2.10", "example.invalid"), &spec()).is_err()
    );
}
#[test]
fn proxy_rejected() {
    assert!(resolve::check_ssh_g(&format!("{}proxyjump jump\n", config()), &spec()).is_err());
}
#[test]
fn ask_rejected() {
    assert!(
        resolve::check_ssh_g(&config().replace("checking true", "checking ask"), &spec()).is_err()
    );
}
#[test]
fn valid_output() {
    assert!(resolve::check_ssh_g(config(), &spec()).is_ok());
}
#[test]
fn scan_host_port() {
    assert!(!deny_hits(&policy(), &["192.0.2.7:6443"]).is_empty());
}
#[test]
fn scan_ipv6_url() {
    assert!(!deny_hits(&policy(), &["https://[2001:db8:dead::1]:6443/healthz"]).is_empty());
}
#[test]
fn scan_mapped() {
    assert!(!deny_hits(&policy(), &["::ffff:192.0.2.7"]).is_empty());
}
#[test]
fn scan_catchalls() {
    assert_eq!(
        deny_hits(&policy(), &["0.0.0.0/0", "::/0"]),
        vec![
            ("0.0.0.0/0".into(), "deny 192.0.2.7/32".into()),
            ("::/0".into(), "deny 192.0.2.7/32".into()),
            ("::/0".into(), "deny 2001:db8:dead::/64".into()),
        ]
    );
    assert!(deny_hits(&policy(), &["kubectl get node x", "1.2.3"]).is_empty());
}
#[test]
fn ip_type() {
    assert!("192.0.2.10".parse::<IpAddr>().is_ok());
}

mod mapped;
mod quoted_paths;

mod scan_extra;

mod args;
