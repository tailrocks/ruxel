use std::path::Path;

#[test]
fn missing_module_fails() {
    assert!(ruxel_config::eval(Path::new("/nonexistent.pkl")).is_err());
}
