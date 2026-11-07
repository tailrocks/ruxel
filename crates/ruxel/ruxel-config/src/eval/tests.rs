use super::eval;
use std::path::Path;

#[test]
fn minimal_evaluates() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/minimal.pkl"
        )))
        .is_ok()
    );
}

#[test]
fn environment_reads_are_denied() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/env.pkl"
        )))
        .is_err()
    );
}

#[test]
fn network_imports_are_denied() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/network.pkl"
        )))
        .is_err()
    );
}
