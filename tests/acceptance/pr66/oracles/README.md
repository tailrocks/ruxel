# Controller delegation check and diff oracle

`controller-delegation-check-diff.jsonl` is copied byte for byte from
`tools/oracle/captures/check-controller-delegation.jsonl` at Ruxel commit
`99fb2d17de593f0dbdeac10320061b9cbc9e281a`. Its SHA-256 is
`afc95e99e87c2ff9c41d3282166877e450cb5901c78e7823931d43db32beabe7`.

At that commit, `tools/fixtures/fresh-parity-gate.sh` ran Ruxel with
`--check --diff` and captured the Ansible run with both capture flags set.
`tools/oracle/parity/controller-delegation.json` records the same hash as its
`check_diff` capture. The two recorded tasks show the delegated command skipped
and the following assertion failed. This is evidence for the combined
check-and-diff behavior; it does not establish a separate historical diff-only
run.
