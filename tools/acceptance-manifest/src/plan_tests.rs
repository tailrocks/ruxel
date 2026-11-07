use super::valid;
use std::collections::BTreeSet;
#[test]
fn exact_plan_hosts_and_failures_are_required() {
    let hosts = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
    let good =
        b"{\"host\":\"a\",\"recap\":{\"failed\":0}}\n{\"host\":\"b\",\"recap\":{\"failed\":0}}\n";
    let duplicate =
        b"{\"host\":\"a\",\"recap\":{\"failed\":0}}\n{\"host\":\"a\",\"recap\":{\"failed\":0}}\n";
    let failed =
        b"{\"host\":\"a\",\"recap\":{\"failed\":1}}\n{\"host\":\"b\",\"recap\":{\"failed\":0}}\n";
    assert!(valid(Some(0), good, &hosts).is_ok());
    assert!(valid(Some(0), duplicate, &hosts).is_err());
    assert!(valid(Some(0), failed, &hosts).is_err());
    assert!(valid(Some(1), good, &hosts).is_err());
}
