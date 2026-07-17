//! The repository-owned fixture project is the executable compatibility
//! corpus. Real workload files are offline extraction input only.

use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver};

#[test]
fn every_synthetic_fixture_parses_and_compiles() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixture-project");
    let mut playbooks: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yml"))
        .collect();
    playbooks.sort();
    assert!(!playbooks.is_empty());

    let engine = Engine::new(std::sync::Arc::new(MemoizedResolver::new(DrySecrets)));
    for path in playbooks {
        let name = path.file_name().unwrap().to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();
        let playbook = ruxel_core::playbook::parse(&name, &source)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        ruxel_core::compiler::compile(&playbook, &engine)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}
