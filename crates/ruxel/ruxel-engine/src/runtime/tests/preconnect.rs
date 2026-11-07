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
