# pklr 5.2.3

Upstream: [jdx/pklr](https://github.com/jdx/pklr), MIT license.
Source: the crates.io package already pinned in the original Cargo.lock.
Package SHA-256:
`6d94cf92a4e66ac2dae846910ae96622f2f2c8f9d26dda1721f341066387f508`.

The complete library source is retained, including inline upstream unit tests,
documentation, LICENSE and the Rust Project MIT notice at
src/eval/stdlib/RUST-LICENSE-MIT. The original README and author manifest are
retained; the latter is named UPSTREAM-Cargo.toml.txt to avoid a misleading
merge-backup suffix. UPSTREAM.sha256 records the original package file hashes.

The normalized Cargo.toml exposes only eval-core, native-io and miette-diagnostics,
the feature subset Ruxel already selected with default-features disabled. Unused
HTTP, asynchronous and package-zip integrations and their dependencies are not
part of this internal library distribution. Their source and the original
manifest are retained unchanged for attribution. Inactive feature names remain
known to check-cfg so retaining that source does not produce spurious warnings.
This lets mandatory standalone CI test the actual embedded feature set without
C-backed TLS or unrelated downloads during all-feature metadata discovery.
Ruxel's selected feature set and resource behavior are unchanged. Cargo.lock
independently pins this excluded-workspace package's native test graph.
The native syntax, imports, numbers, strings and rendering integration modules
and their fixtures are retained. Their test entry point selects those five
modules and removes an unused helper. The remaining upstream integration
targets/dependencies require networking features, or contain unsafe environment
mutation, and are outside this native-only distribution. Neither the retained
library nor retained test sources contain unsafe Rust.

One inherited-property test expected an unassigned required String property to
disappear when rendered. It now requires rejection, then verifies successful
amendment with an explicit assignment. Its amended property is declared nullable
so official Pkl can isolate the missing required member rather than reject an
undeclared property first. Official Pkl 0.32.2 independently rejects the missing
case and renders both assigned fields as "assigned" in the filled case. The
integration delta is recorded in ../../patches/pklr-5.2.3/native-tests.patch.

The only library behavior delta is the reviewed patch at
../../patches/pklr-5.2.3/required-members.patch: defer missing required instance
members, permit amendment of incomplete typed defaults, and reject unresolved
constructors. Documentation-only markup corrections are recorded in
../../patches/pklr-5.2.3/rustdoc.patch. All library implementation outside the
required-member patch, including resource handling, is unchanged.
The Ruxel evaluator's resource configuration is unchanged.

The dependency is selected directly by path and exact version in ruxel-config;
there is no Cargo patch table. It is excluded from the first-party workspace.
The repository policy excludes this attributed third-party source from its
first-party layout/size rules so the upstream source remains reviewable. Ruxel's
schema tests and downstream tests cover the local semantics change. The mandatory
standalone CI job runs both retained upstream binaries: 117 library tests and
393 native integration tests. `cargo test --manifest-path vendor/pklr/Cargo.toml
--offline --locked --all-targets --all-features` exercises the same complete set.
This fork does not claim full official Pkl language or renderer equivalence.
