use super::*;

#[test]
fn formats_only_the_secret_name() {
    let secret = Secret::new("K8S_TOKEN", "sensitive-value".to_owned());
    assert_eq!(format!("{secret:?}"), "<secret K8S_TOKEN>");
    assert_eq!(format!("{secret}"), "<secret K8S_TOKEN>");
    assert!(!format!("{secret:?}").contains("sensitive-value"));
}
