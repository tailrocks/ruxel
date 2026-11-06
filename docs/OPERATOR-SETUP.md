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

## XFS growth prototype prerequisites

The descriptor-based XFS growth prototype requires Linux x86_64 or aarch64
and Python 3.9 or newer with its standard `fcntl` module. Provision Python
from the target distribution's official signed repository before requesting
filesystem or logical-volume growth. The helper checks the pinned mount's
geometry before logical-volume mutation; a missing interpreter or unsupported
ABI or ioctl stops preflight.

The operator must hold exclusive control of resizing and keep the filesystem
unfrozen for the entire operation. The agent waits for and reaps the helper.
Killing the agent or losing its connection does not establish that a kernel
operation has stopped; establish process quiescence before another mutation.
The prototype remains gated on isolated mounted-XFS acceptance. Its mocked
geometry and allocation-group tests do not establish mounted-filesystem safety.

## Filesystem identity limits during migration

Attribute changes use held descriptors. On macOS, a target must be readable,
or a directory must be searchable, to acquire a suitable descriptor. An
unprivileged agent cannot repair an owned mode-`000` target through a pathname
fallback: it returns an error before changing that target's attributes.
Recursive operations also require access to enumerate directories. These
remaining platform limitations keep full migration acceptance open.
