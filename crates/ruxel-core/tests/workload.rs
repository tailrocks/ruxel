//! The M1 fidelity gate against the real workload: every playbook in the
//! ChainArgos ansible-configs checkout must parse into the closed-surface
//! model. Runs only when RUXEL_WORKLOAD_DIR points at that checkout (local
//! dev); CI wiring comes with the spec-drift watch (docs/PLAN.md).

use ruxel_core::playbook;

#[test]
fn entire_workload_parses() {
    let Ok(dir) = std::env::var("RUXEL_WORKLOAD_DIR") else {
        eprintln!("RUXEL_WORKLOAD_DIR not set — skipping workload parse gate");
        return;
    };
    let mut parsed = 0;
    let mut failures = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("workload dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "no .yml files in {dir}");

    for path in entries {
        let content = std::fs::read_to_string(&path).expect("playbook readable");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        match playbook::parse(&name, &content) {
            Ok(pb) => {
                parsed += 1;
                let tasks: usize = pb
                    .plays
                    .iter()
                    .map(|p| p.pre_tasks.len() + p.tasks.len() + p.handlers.len())
                    .sum();
                eprintln!(
                    "OK   {name}: {} play(s), {tasks} top-level task(s)",
                    pb.plays.len()
                );
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} playbooks failed to parse:\n{}",
        failures.len(),
        failures.len() + parsed,
        failures.join("\n")
    );
}
