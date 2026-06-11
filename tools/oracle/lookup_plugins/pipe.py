# Fake `pipe` lookup for parity captures: returns the same deterministic
# dry-secret value as ruxel's DrySecrets resolver (crates/ruxel-core/src/
# engine.rs). Never executes anything. Loaded via ANSIBLE_LOOKUP_PLUGINS,
# shadowing the builtin.
from __future__ import annotations

import hashlib

from ansible.plugins.lookup import LookupBase

DOCUMENTATION = """
    name: pipe
    short_description: dry-secrets fake of the builtin pipe lookup
    description: deterministic fake for ruxel render-parity captures.
    options:
      _terms:
        description: command (never executed)
        required: true
"""


def dry_value(key: str) -> str:
    return "dry-secret-" + hashlib.sha256(key.encode()).hexdigest()[:16]


class LookupModule(LookupBase):
    def run(self, terms, variables=None, **kwargs):
        return [dry_value("pipe\x1f" + str(term)) for term in terms]
