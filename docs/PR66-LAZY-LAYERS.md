# Demand-driven variable layers

The controller retains group, host, play, block and task variable definitions
until an expression accesses them. An unused binding must not fail execution or
perform a lookup. A binding that reads a registered result sees the result
available at its point of use, including the current retry attempt.

`Backend::evaluate_layered_expression` is a required backend method. Its contract
is demand-driven evaluation: preserve layer precedence, inherited self-overrides,
current runtime scope, lazy conditional branches and per-value provenance.
There is no eager fallback or opt-in flag. The controller does not materialize
all inherited variables at play entry, block selection or task dispatch, and it
does not persist resolved selection bindings into later scopes.

Backends using the standard Pkl runtime expression semantics implement this
method by calling `controller::evaluate_standard_layered` with their own
`LookupResolver`. The Rust runtime uses the same layered resolver. Backends with
custom expression semantics must implement the demand-driven contract explicitly;
the controller does not infer their dependencies by scanning expressions or
flattening variable layers.

Each binding has a distinct layer identity. Cross-name references see the newest
visible definition. A direct self-reference in an override can resolve the
inherited binding, while a genuine dependency cycle fails. Resolution preserves
fact precedence, loop item scope and secret provenance. Accessed bindings and
errors are memoized within an evaluation; later evaluations use the current
results, facts and item scope. An explicit reference to the complete `vars` map
requests all visible bindings. The controller must not synthesize such a
reference to prepare an unrelated expression.

Operation parameters, environment, delegation, loop labels, retry conditions,
assertions and template preparation use layered evaluation. Existing
sensitive-control, no-logging, per-item scope, cross-play facts and handler tests
remain required. Retry sensitivity is conservative for dynamic branches whose
outcome cannot be known before dispatch; this avoids placing potentially
sensitive agent results in the cache, but can censor results unnecessarily.

The local-only Pkl fixtures in `testdata/workloads/pr66/lazy-layers` exercise
play and block variables that read a later registered result. Each should emit
`before`, `future`, then `after` with message `available`, and finish with three
successful tasks. They use a synthetic local inventory entry and require no
network target or credentials.

Focused custom-backend tests check that unused parent errors and lookup effects
remain untouched, accessed secrets retain provenance, and unused play and block
bindings stay lazy across skipped tasks, empty loops and a successful child
variable task. Standard-expression and runtime tests cover inherited overrides,
current retry results, short-circuit branches and item scope. These bounded tests
do not replace the migration workload evidence or independent review of the
final integrated head.
