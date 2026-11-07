use super::redacted;
#[test]
fn rejects_missing_duplicate_or_uncensored_events() {
    let pass = "{\"task\":\"private\",\"result\":{\"censored\":true}}\n";
    let visible = "{\"task\":\"private\",\"result\":{\"censored\":false}}\n";
    assert!(redacted(pass, pass, &["private"]).is_ok());
    assert!(redacted(pass, visible, &["private"]).is_err());
    assert!(redacted(pass, "", &["private"]).is_err());
    assert!(redacted(&format!("{pass}{pass}"), pass, &["private"]).is_err());
}
