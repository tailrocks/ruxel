# Rootless restart-actions groundwork

This separate runner derives an explicitly rootless variant of the pinned
restart-actions fixture. It is not an adapter for baseline root ownership and
is not added to acceptance-manifest SUPPORTED or mandatory acceptance-local.
The original Pkl, expectations, pins and scenario refusal remain unchanged.

It verifies the original fixture SHA-256 and exact restart contract, then removes
only the single `owner = "root"; group = "root";` pair in a temporary copy and
substitutes its temporary payload path. It refuses UID0. Files must belong to
the invoking UID/GID with mode0755 and exact shell bytes. This tests current-user
creation, not root metadata, privilege escalation or a real service restart.

The real CLI/agent run in --local --synthetic-secrets mode with a private state
directory, no stdin and bounded subprocess output/time/process-group cleanup.
Fresh apply and rerun verify36 ordered01..36 shell results, rc0, uncensored
nonfailed aggregate/item results, one successful recap and copy changed true
then false. The restart commands intentionally rerun; whole-play zero-change is
not asserted. Drift changes only the private script bytes. Check/diff must leave
payload and all fixture files outside agent state byte/mode/link-identical;
repair must restore the exact script and report the copy changed. The no-op
restart script touches no system service. No remote/network/credential tool is
requested. Preview snapshots prove non-mutation, not independent command-call
counts for the no-op script.

Run with reviewed binaries built from the exact source checkout:

```sh
cargo run --locked --offline --manifest-path tools/acceptance-restart-rootless/Cargo.toml -- \
  --root /absolute/repo --ruxel /absolute/ruxel --agent /absolute/ruxel-agent
```

Owner verification on macOS at base7ef8a90: clean isolated CLI+agent build used
pinned Rust1.99.1, locked/offline Cargo, CC/CXX=/usr/bin/false, blake3/pure and
RUSTFLAGS='-C linker=/usr/bin/ld -C linker-flavor=ld' under build-slot. Harness
used the same environment and a separate target directory. No C compiler,
Python, root execution, OS service or server/cloud operation was invoked.
Actual results:

```text
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
PASS rootless restart groundwork: fresh/rerun/drift/check/diff/repair and36 ordered results; baseline root ownership unsupported
```

Strict all-target Clippy, fmt and diff checks passed. Synthetic tests cover pin
and ownership-variant integrity, payload quoting, all36 ordered/status results,
missing/duplicate/censored/failed aggregate events, snapshot state exclusion and
bounded child/descendant handling. Actual replay used a newly built CLI/agent;
raw event output is checked in memory and not retained after temporary cleanup.
Commands/results are retained in the task tool transcript. An independently
reviewed exact baseline root-owned fixture lifecycle remains future acceptance;
this rootless run closes no privileged matrix row or G-SLICE/rehearsal gate.

Replay source:7ef8a90f139cb40409fa69d24dea35c68ce2f731. Binary SHA-256:

```text
ruxel       2a7746958c772dd3e759eebe33bde457f2ebc69bf37430634c25dc2fc9a36145
ruxel-agent ee8c6c5b1304ace221e9f46a501fbc87100c4a8f794e057dfbd7e4852e9a7e8c
```

## Reviewed path and file-size follow-up

The review found that the interpolated payload path could contain shell syntax,
and fixture.rs had154 lines. A new negative test reproduced the path rejection
gap without executing any hostile string. Payloads now require an absolute path
containing only ASCII letters/digits and `/_.-`; spaces, shell metacharacters,
quotes, escapes, controls, Unicode and relative paths refuse before substitution
or CLI execution. Fixture tests moved to fixture_tests.rs; fixture.rs is136 lines
and every new source file is below150 lines.

Final owner verification:7 tests passed with0 ignored, strict all-target Clippy,
fmt and diff checks passed, and the ordinary rootless actual CLI/agent lifecycle
passed again using the same fresh7ef8a90 binaries. Hostile paths were only pure
string-validation inputs. Root/full acceptance remains unsupported.
