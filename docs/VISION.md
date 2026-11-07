# Vision

Ruxel aims to make host automation understandable and repeatable through typed
Pkl configuration, a Rust controller, and a small host agent. Operators should
be able to inspect intended changes, execute them with useful progress and
results, and rerun safely after a system converges.

The project is pre-1.0. Functionality is delivered with synthetic regression
coverage and evaluated on disposable infrastructure before any authorized
real-host use. The schema is an interface, not evidence that every operation
or platform has passed end-to-end verification. See [GOAL.md](GOAL.md) for scope
and [ARCHITECTURE.md](ARCHITECTURE.md) for the current components.
