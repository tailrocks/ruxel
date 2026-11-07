# Secret provenance and execution output

Secret lookup results carry sensitivity through expression evaluation, variables,
facts, loop items, task inputs and registered results. A task receiving a
sensitive input or running under a sensitive condition suppresses its result
payload and disables agent result caching. A `noLog` task also suppresses its
payload and protects evaluation errors; facts and registered results produced
by that task remain sensitive when reused.

The controller deliberately preserves execution status: task presence, success
or failure, changed status, recap counts and timing can reveal information about
secret-dependent conditions, retries and loop cardinality. These channels are
declassified so operators can observe execution. This protection does not claim
strict noninterference. Avoid using secrets as control inputs when that status
information must remain private.

## Durable agent state

Before executing a sensitive external request, including a read or a check-mode
request, the agent creates and syncs an empty, private `sensitive-effects`
marker in its state directory. It contains no values or paths and is not cleared
automatically. A failure to read or persist the marker stops the affected agent
operation. Protocol version 3 requires agents to report their marker state at
handshake; missing state or an incompatible version fails the connection.

A later run that connects to a marked state directory treats the run
conservatively. A failed initial connection or a possible delegation to an
unchecked destination also makes the run conservative: the controller scans
delegation syntax without evaluating skipped expressions or connecting to extra
hosts. The controller synchronizes sensitive requests with concurrent public
requests, and synchronizes source-backed template preparation with agent
operations. Public requests can overlap while no sensitivity is known.

Once the controller treats a run conservatively, subsequent task results and
metadata, including host names, task names and pause prompts, are censored in
events and logs. Agent requests bypass the result cache. This can hide unrelated
public results and host identities, including on later runs sharing the marker;
internal values remain available for task conditions and retries. An agent that
has loaded a marker also treats subsequent requests as sensitive.

Plan does not connect to agents, so their state is unknown. Its task, handler
and host projections are censored; phase and recap counts remain available.
Configuration evaluation and validation errors for plan, apply, check and diff
are generic because evaluated Pkl errors can include file contents without
provenance. `config dump` intentionally prints the evaluated configuration and
must be handled as sensitive.

## Limits

The marker belongs to one agent state directory. It cannot establish provenance
across separately reset or deleted state directories, other controllers or
machines, or another agent with a different state directory that can access the
same resources. External programs can expose data through their own side effects
outside the event stream. Use explicit sensitivity for such flows; the marker
is not filesystem-wide information-flow tracking or a substitute for access
controls.

The local Rust acceptance cases use synthetic values and agents to exercise
marker persistence, conservative output and plan redaction. These tests do not
prove every provider, shared-resource topology, or external program is free of
disclosure. Treat generated output and state as sensitive and verify the
workflow in use.
