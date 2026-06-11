//! The M1 fidelity gate against the real workload: every playbook in the
//! ChainArgos ansible-configs checkout must parse into the closed-surface
//! model and compile to a plan. Runs only when RUXEL_WORKLOAD_DIR points
//! at that checkout (local dev); CI wiring comes with the spec-drift watch
//! (docs/PLAN.md).

use ruxel_core::playbook;

#[test]
fn entire_workload_compiles_to_plans() {
    use ruxel_core::compiler::{self, PlanBody, PlanTask, Readiness};
    use ruxel_core::engine::{DrySecrets, Engine, MemoizedResolver};

    let Ok(dir) = std::env::var("RUXEL_WORKLOAD_DIR") else {
        eprintln!("RUXEL_WORKLOAD_DIR not set — skipping workload compile gate");
        return;
    };
    let engine = Engine::new(std::sync::Arc::new(MemoizedResolver::new(DrySecrets)));
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("workload dir readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml"))
        .collect();
    entries.sort();

    fn count(tasks: &[PlanTask], stats: &mut (usize, usize)) {
        for t in tasks {
            match &t.body {
                PlanBody::Module { readiness, .. } => match readiness {
                    Readiness::Static { .. } => stats.0 += 1,
                    Readiness::Deferred { .. } => stats.1 += 1,
                },
                PlanBody::Block {
                    block,
                    rescue,
                    always,
                } => {
                    count(block, stats);
                    count(rescue, stats);
                    count(always, stats);
                }
            }
        }
    }

    let mut compiled = 0;
    let mut totals = (0usize, 0usize);
    for path in &entries {
        let content = std::fs::read_to_string(path).expect("playbook readable");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let pb = playbook::parse(&name, &content).expect("parses");
        let plan = compiler::compile(&pb, &engine)
            .unwrap_or_else(|e| panic!("{name}: compile failed: {e}"));
        let mut stats = (0usize, 0usize);
        for play in &plan.plays {
            count(&play.pre_tasks, &mut stats);
            count(&play.tasks, &mut stats);
            count(&play.handlers, &mut stats);
        }
        eprintln!("OK   {name}: {} static, {} deferred", stats.0, stats.1);
        totals.0 += stats.0;
        totals.1 += stats.1;
        compiled += 1;
    }
    assert_eq!(compiled, 16, "the workload is 16 playbooks");
    assert!(
        totals.0 > totals.1,
        "most of the workload renders statically"
    );
    assert!(totals.1 > 0, "the register chains must defer");
    eprintln!(
        "compile gate: 16/16 playbooks → plans ({} static / {} deferred tasks)",
        totals.0, totals.1
    );
}

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
