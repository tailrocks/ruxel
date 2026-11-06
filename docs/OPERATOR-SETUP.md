# Isolated fixture setup

Remote tests require explicit operator authorization and disposable targets.
Use an isolated Hetzner Cloud project dedicated to Ruxel fixtures, with a
project-scoped token and no unrelated resources. Store credentials outside the
repository, use the provider's supported authentication mechanism, and keep
tokens and values out of command histories and logs. Do not use an existing
production project as a fixture project.

Before a remote run, verify the selected targets and applicable guard policy,
provision host keys through the operator's approved process, and record the
fixture identity and cleanup plan. Checks and diffs contact hosts. Reap
fixture resources when the run ends and check for leftovers. A project-scoped
cloud token does not itself authorize contacting any other host.

Local synthetic acceptance is available through `mise run acceptance-local`;
it runs real local agent processes and must be used only with disposable local
workloads. Synthetic secret mode does not require a live secret account.
