//! vault (never a real exampleorg secret). Runs only when a 1Password
//! session is available — set `OP_SERVICE_ACCOUNT_TOKEN` (the ruxel-ci
//!   cargo nextest run -p ruxel-cli --test `op_resolver` --run-ignored all --no-capture

use ruxel_cli::secrets::OpResolver;
use ruxel_core::engine::LookupResolver;

#[test]
#[ignore = "needs a 1Password session (OP_SERVICE_ACCOUNT_TOKEN) + ruxel-test vault"]
fn resolves_synthetic_vault_item() {
    let r = OpResolver::default();
    let pw = r
        .onepassword(
            "ruxel-test PostgreSQL",
            Some("password"),
            Some("ruxel-test"),
            None,
        )
        .expect("op read password");
    eprintln!("resolved password field: {pw}");
    assert_ne!(pw, "");
    assert!(!pw.contains('\n'), "value must be newline-trimmed");

    let user = r
        .onepassword(
            "ruxel-test PostgreSQL",
            Some("username"),
            Some("ruxel-test"),
            None,
        )
        .expect("op read username");
    assert_eq!(user, "ruxel_test_user");

    let via_pipe = r
        .pipe("op read 'op://ruxel-test/ruxel-test PostgreSQL/username'")
        .expect("pipe op read");
    assert_eq!(via_pipe, "ruxel_test_user");
}
