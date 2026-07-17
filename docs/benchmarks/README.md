# Verified synthetic benchmarks

These are current acceptance measurements for the closed workload. Every case
uses provider-verified disposable twins or locally labeled disposable
containers, three alternating repetitions per executor, exact binary/source
hashes, sanitized raw logs, result/diff/state correctness gates, and
resource-reaping proof.

| Case | Ruxel median | Ansible median | Speedup |
|---|---:|---:|---:|
| fresh | 1.856 s | 21.238 s | 11.44x |
| converged | 1.190 s | 19.822 s | 16.66x |
| one-task-drift | 1.130 s | 19.869 s | 17.58x |
| check-diff | 1.157 s | 19.729 s | 17.06x |
| secret | 1.811 s | 18.817 s | 10.39x |
| storage | 1.451 s | 21.787 s | 15.01x |
| PostgreSQL | 1.224 s | 38.713 s | 31.62x |
| simulated RTT | 1.766 s | 29.920 s | 16.94x |
| six-host | 1.968 s | 4.378 s | 2.22x |
| 65-task/52-lookup scale | 1.785 s | 64.912 s | 36.36x |

The scale case passes its explicit Ruxel median target of less than 5 seconds.
These are synthetic compatibility measurements, not production-workload runs.

Raw evidence and manifests live in [`results/`](results/). Verify integrity,
statistics, source binding, correctness flags, safety, and completeness with:

```sh
python3 tools/benchmarks/verify.py docs/benchmarks/results
```

Historical Markdown reports in this directory predate the acceptance matrix
and are retained only as design history; this page and `results/` are
authoritative.
