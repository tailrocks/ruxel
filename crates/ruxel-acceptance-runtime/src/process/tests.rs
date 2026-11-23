use super::*;

#[path = "tests/cleanup.rs"]
mod cleanup;
#[path = "tests/descendants.rs"]
mod descendants;
#[path = "tests/timeout.rs"]
mod timeout;

#[test]
fn sentinel_entry() {
    if std::env::var_os("RUXEL_TEST_GROUP_SENTINEL").is_some() {
        sentinel();
    }
}
