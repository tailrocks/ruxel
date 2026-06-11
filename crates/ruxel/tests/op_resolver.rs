//! Verifies the op-backed resolver against the synthetic `ruxel-test`
//! vault (never a real ChainArgos secret). Runs only when a 1Password
//! session is available — set OP_SERVICE_ACCOUNT_TOKEN (the ruxel-ci
//! service account) or have a desktop `op` session:
//!
//!   set -a; source ~/.config/ruxel/op-ci.env; set +a
//!   cargo test -p ruxel-cli --test op_resolver -- --ignored --nocapture

use ruxel_cli::secrets::OpResolver;
use ruxel_core::engine::LookupResolver;

#[test]
#[ignore = "needs a 1Password session (OP_SERVICE_ACCOUNT_TOKEN) + ruxel-test vault"]
fn resolves_synthetic_vault_item() {
    let r = OpResolver;
    let pw = r
        .onepassword(
            "ruxel-test PostgreSQL",
            Some("password"),
            Some("ruxel-test"),
            None,
        )
        .expect("op read password");
    eprintln!("resolved password field: {pw}");
    assert!(!pw.is_empty());
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

    // pipe('op read …') path — the workload's other lookup form.
    let via_pipe = r
        .pipe("op read 'op://ruxel-test/ruxel-test PostgreSQL/username'")
        .expect("pipe op read");
    assert_eq!(via_pipe, "ruxel_test_user");
}
