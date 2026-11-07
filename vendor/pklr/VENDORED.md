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

The normalized Cargo.toml retains the published library features and dependencies.
External test targets and dev-dependencies are removed because this distribution
contains the library, not the upstream integration suite. The published library
source has no unsafe Rust. Some excluded upstream integration tests mutate
process environment through unsafe code, or enable HTTP/package dependencies;
they are not part of Ruxel's dependency.

The only library-source delta is the reviewed patch at
../../patches/pklr-5.2.3/required-members.patch: defer missing required instance
members, permit amendment of incomplete typed defaults, and reject unresolved
constructors. All other library files, including resource handling, are unchanged.
The Ruxel evaluator's resource configuration is unchanged.

The dependency is selected directly by path and exact version in ruxel-config;
there is no Cargo patch table. It is excluded from the first-party workspace.
The repository policy excludes this attributed third-party source from its
first-party layout/size rules so the upstream source remains reviewable. Ruxel's
schema tests and downstream tests cover the local semantics change; the retained
upstream inline suite runs with `cargo test -p pklr --lib --offline --locked`.
This fork does not claim full official Pkl language or renderer equivalence.
