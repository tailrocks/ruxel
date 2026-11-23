use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn bounded_connections_run_concurrently_and_keep_result_order() {
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let hosts = ["a", "b", "c"].map(str::to_owned).to_vec();
    let results = bounded(hosts, 2, |host| {
        let barrier = barrier.clone();
        let active = active.clone();
        let peak = peak.clone();
        async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            if host != "c" {
                barrier.wait().await;
            }
            active.fetch_sub(1, Ordering::SeqCst);
            if host == "b" {
                Err("synthetic failure")
            } else {
                Ok(())
            }
        }
    })
    .collect::<Vec<_>>()
    .await;
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(
        results
            .iter()
            .map(|(host, _)| host.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(results[0].1.is_ok());
    assert!(results[1].1.is_err());
    assert!(results[2].1.is_ok());
}

#[tokio::test]
async fn zero_jobs_still_completes_one_at_a_time() {
    let hosts = ["a", "b"].map(str::to_owned).to_vec();
    let results = bounded(hosts, 0, |host| async move { host })
        .collect::<Vec<_>>()
        .await;
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn cancellation_drops_active_connections_and_keeps_gate_until_batch_drops() {
    struct Active(Arc<AtomicUsize>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let active = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let gate = tokio::sync::RwLock::new(());
    let held = gate.write().await;
    let mut pending = Box::pin(bounded(
        ["a", "b", "c"].map(str::to_owned).to_vec(),
        2,
        |_| {
            let active = active.clone();
            let started = started.clone();
            let held = &held;
            async move {
                std::hint::black_box(held);
                active.fetch_add(1, Ordering::SeqCst);
                started.fetch_add(1, Ordering::SeqCst);
                let _active = Active(active);
                std::future::pending::<()>().await;
            }
        },
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), pending.next())
            .await
            .is_err()
    );
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(active.load(Ordering::SeqCst), 2);
    assert!(gate.try_read().is_err());
    drop(pending);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    drop(held);
    assert!(gate.try_write().is_ok());
}
