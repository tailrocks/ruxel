# Project goal

Build a Rust controller and agent that configure hosts from typed Pkl, with
executable planning, checks, diffs, and apply operations. Preserve the supported
operational capabilities through the migration and demonstrate them with
synthetic, reproducible tests. Kubernetes bootstrap and broader host automation
remain development goals, not a statement of completed verification.

Configuration and runtime behavior should be explicit, validated, and observable.
Secret references must not require plaintext credentials in configuration or
logs. Tests use synthetic identities, data, and disposable targets. Real host
identities and secrets do not belong in this public repository.

Only contact targets explicitly authorized by the operator and admitted by the
applicable guard policy. Check and diff connect to targets. See
[operator setup](OPERATOR-SETUP.md) and the repository's agent instructions.
