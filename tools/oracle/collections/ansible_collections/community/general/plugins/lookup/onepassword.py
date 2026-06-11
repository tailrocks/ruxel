# Fake `community.general.onepassword` lookup for parity captures: returns
# the same deterministic dry-secret value as ruxel's DrySecrets resolver
# (crates/ruxel-core/src/engine.rs). Never talks to 1Password.
from __future__ import annotations

import hashlib

from ansible.plugins.lookup import LookupBase

DOCUMENTATION = """
    name: onepassword
    short_description: dry-secrets fake of the onepassword lookup
    description: deterministic fake for ruxel render-parity captures.
    options:
      _terms:
        description: item name
        required: true
      field:
        description: field name
        required: false
      vault:
        description: vault name
        required: false
      section:
        description: section name
        required: false
"""


def dry_value(key: str) -> str:
    return "dry-secret-" + hashlib.sha256(key.encode()).hexdigest()[:16]


class LookupModule(LookupBase):
    def run(self, terms, variables=None, **kwargs):
        field = kwargs.get("field") or ""
        vault = kwargs.get("vault") or ""
        section = kwargs.get("section") or ""
        return [
            dry_value(
                "onepassword\x1f" + str(term) + "\x1f" + str(field) + "\x1f" + str(vault) + "\x1f" + str(section)
            )
            for term in terms
        ]
