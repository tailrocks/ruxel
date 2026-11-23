//! The value surface: a value the operating system cannot carry is
//! refused before the file is written and before the live value is
//! touched, so the refusal names the value instead of failing at the
//! spawn of `sysctl -w`.

use serde_json::json;

#[test]
fn a_value_with_a_nul_is_refused_and_the_file_is_untouched() -> std::io::Result<()> {
    // The file lives in a directory of its own: the preflight's own
    // temporary names are pid-scoped, so two tasks that share a
    // directory would clobber each other's probe and rehearsal.
    let dir = std::env::temp_dir().join(format!("ruxel-sysctl-nul-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("x.conf");
    std::fs::write(&path, "\nnet.test=1\n")?;
    let ctx = super::super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    let error = super::super::run(
        &json!({"name":"net.test","value":"a\u{0}b","sysctl_file":path,"reload":false}),
        &ctx,
        &super::super::SysctlManaged::default(),
    )
    .expect_err("a value with a NUL must be refused");
    assert!(
        error.contains("NUL"),
        "the refusal must name the NUL: {error}"
    );
    assert_eq!(std::fs::read_to_string(&path)?, "\nnet.test=1\n");
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
