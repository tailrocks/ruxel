# Fake community.general (oracle parity captures only)

Provides exactly one plugin: a `onepassword` lookup that returns the same
deterministic dry-secret fakes as ruxel's `DrySecrets` resolver. Selected
via `ANSIBLE_COLLECTIONS_PATH` by `tools/oracle/render_parity.py`; never
installed, never talks to 1Password.
