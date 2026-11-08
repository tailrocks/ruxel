# Development handoff

Ruxel is migrating to Pkl configuration and an executable Rust controller and
agent. The migration is in progress. Consult the active pull request and its
capability matrix for the exact implementation, outstanding reviews, and test
evidence; do not infer complete coverage from this file.

Start with `mise run check` for repository checks and `mise run acceptance-local`
for the synthetic local acceptance suite. The latter executes an agent and can
modify local fixture paths; review its inputs before running it. Cloud and
remote tests require explicit authorization and isolated fixtures. Evidence for
operator-specific environments belongs in the operator's private repository.

Do not commit real host identities, credentials, or private captures here.
Only connect to explicitly authorized targets under the applicable guard policy.
