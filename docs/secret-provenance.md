# Secret provenance and execution output

Secret lookup results carry sensitivity through expression evaluation, variables,
facts, loop items, task inputs and registered results. A task receiving a
sensitive input or running under a sensitive condition suppresses its result
payload and disables agent result caching. Other task diagnostics remain visible.
A `noLog` task also suppresses its payload and protects evaluation errors; facts
and registered results produced by that task remain sensitive when reused.

The controller deliberately preserves execution status: task presence, success
or failure, changed status, recap counts and timing can reveal information about
secret-dependent conditions, retries and loop cardinality. These channels are
declassified so operators can observe execution. This protection does not claim
strict noninterference. Avoid using secrets as control inputs when that status
information must remain private.

The local regression in `tests/acceptance/pr66/secret_provenance.py` uses only
synthetic values and a local agent. It checks transformed lookup values, facts,
registered command output, `noLog`, sensitive conditions and evaluation errors
in stdout, the JSONL event log and local agent state. It does not prove that all
possible providers or external programs avoid disclosing secret-derived data
through their own side effects.

Within a run, sensitive Copy, Template and File operations mark their destination
paths for the selected host. Subsequent agent requests mentioning a marked path
or an ancestor or descendant are sensitive. After a path is marked, operations
with unknown read sets, including Shell and Command, are also sensitive. A
sensitive operation whose effects are not known precisely makes subsequent
agent results on that host sensitive for the rest of the run. Requests to the same
host are serialized around this tracking. Controller-only diagnostics remain
available when their inputs are public.

Path tracking compares the path strings supplied by tasks. It cannot discover
symlink, hard-link, mount, relative-path or host-alias equivalence, nor does it
persist provenance across runs. External programs can communicate through
resources on another host or expose data through side effects outside the event
stream. Treat such flows as sensitive explicitly; the tracker does not provide
filesystem-wide information-flow security.
