# Lazy variable layers: unfinished runtime work

At the starting revision, the controller materializes every group, host and play variable in
`Runner::extend_vars`, and every block variable before running the block. It
also materializes all remaining task variables before operation dispatch. A
variable that is never referenced can therefore fail a host, and a variable
that references a result produced by an earlier task can fail before that task
runs. The frozen scheduler retained raw definitions in its scope and resolved
names on access.

The local-only Pkl fixtures in `testdata/workloads/pr66/lazy-layers` make this
observable. Each should emit `before`, `future`, then `after` with message
`available`, and finish with three successful tasks. At the time of writing,
both previously failed before the first task with a missing `results.future.msg`
reference. The current isolated work-in-progress executes both successfully.
They use a synthetic local inventory entry and do not require a network target
or credentials.

A complete implementation needs to retain layered definitions until an
expression accesses them. It must preserve distinct binding identities across
group, host, play, block and task layers, inherited self-overrides, dependency
cycles, fact precedence, loop item scope, per-value sensitivity, and lazy
conditional branches. Resolution must see results and facts available at the
point of use, without replaying side-effecting lookups unnecessarily. Bare
`vars` references must materialize the visible layer, and shadowed definitions
must not be evaluated merely because a later definition exists.

This cannot be fixed by scanning descendants for references or flattening the
layers: those approaches either evaluate unused branches or lose inherited
self-overrides. Operation parameters, environment, delegation, loop labels,
retry conditions, assertions and template preparation currently consume a
materialized scope, so a layered resolver must be threaded through those paths
and through the default backend contract. Existing sensitive-control,
no-logging, per-item scope, cross-play facts and handler tests remain required.
This branch remains under independent review. The default custom-backend API
retains eager inherited-variable materialization and delegates evaluation to the
backend's existing hooks. Group, host and play bindings materialize once at play
entry, block bindings once after block selection, and task bindings once per
iteration. Unused inherited bindings can still fail before execution; this is
the opaque backend's compatibility behavior, not lazy parity.

Backends opt into demand-driven layers by returning `true` from
`Backend::lazy_variable_layers` and overriding `evaluate_layered_expression`.
Backends using standard expression semantics can delegate that method to
`controller::evaluate_standard_layered` with their own lookup resolver. This is
an explicit opt-in and does not preserve backend-specific expression extensions.
The Rust runtime uses this path. Cross-name references see the newest visible
definition, matching the frozen scheduler; direct inherited self-overrides are
an explicit behavior extension. Retry sensitivity is conservative for dynamic branches whose outcome
cannot be known before dispatch; this avoids placing potentially sensitive agent
results in the cache, but can censor results unnecessarily.

The local Rust runtime acceptance runner passes its six scenarios with the
revised tag check, which requires exactly one skipped event for each excluded
leaf and confirms that the existing payload is unchanged. The play and block
fixtures, focused custom-backend tests, and those acceptance scenarios cover
bounded cases; they do not establish full parity or replace independent review.
