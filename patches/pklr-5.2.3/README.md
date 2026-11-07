# Required-member evaluation prototype

This patch is an inactive review artifact. Cargo still selects the registry
release. No evaluator, resource allowlist, or dependency configuration changes
are enabled by this directory.

The source is the MIT-licensed `pklr` 5.2.3 crates.io package, already pinned in
`Cargo.lock`. Its package SHA-256 is
`6d94cf92a4e66ac2dae846910ae96622f2f2c8f9d26dda1721f341066387f508`.
The upstream repository is [jdx/pklr](https://github.com/jdx/pklr).
The accompanying LICENSE is copied unchanged from that package.

The patch records absent declared instance members through the evaluator's
existing deferred member-error mechanism. Hidden members fail on access;
visible members fail when rendered. Local members and implicit nullable or
collection defaults retain their existing behavior. Typed defaults retain
deferred errors so an amendment can supply a previously missing member.
Unresolved constructor names are rejected instead of becoming bare objects.

Apply `required-members.patch` with `patch -p1` to an extracted, disposable copy
of the checksum-pinned package. Never patch the Cargo registry cache. The
prototype was tested in an isolated Cargo workspace using a local
`[patch.crates-io]` dependency and the existing schema suite with only its
`evaluate_module` helper changed to `ruxel_config::eval`. The ordinary schema
test helper still uses official Pkl until a reviewed native dependency is
selected.

The isolated native suite passed all 12 groups: all 54 original invalid
expressions, the valid schema fixture, 12 generic valid declarations, eight
generic invalid declarations, all 14 migrated workload evaluations, and the
native template expression. Invalid-expression loops collect every failure
instead of stopping at the first accepted expression. Existing config tests
also passed, including environment-read and network-import refusal.
These are evaluation tests; no workload operation was executed.

The official Pkl renderer omits null properties by default while the native
JSON result includes them. The nullable-default oracle observes `== null`
inside the expression, preserving that known renderer difference rather than
claiming byte-for-byte renderer parity.

The full upstream package is approximately 2.5 MiB and 104 files; `src/` is
approximately 1.6 MiB and 61 files. It has large source files and an embedded
Rust standard-library MIT notice. Vendoring requires independent correctness,
license/provenance and repository-policy review, preserving upstream notices
and testing downstream templates. This prototype does not authorize a policy
exception, vendor the package, pin a fork, or resolve the CI prerequisite.
