# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `SecretText`, the secret type: `module.fnox(key)`, `module.fnoxGenerated(key, format)`, `module.onePassword(vault, item, field, options)` and `module.onePasswordReference(reference)` name a value in the controller's stores, and `SecretText.join(separator, other)` composes the kubeadm token pair. A secret may appear only in `File.content` and `Command`/`Script` `environment` values: every other text field is `Static`, which rejects the reserved secret-reference prefix, so a secret cannot hide in a path, name or script. A file whose content holds a secret must be `0600` or narrower (no permission for group or other, no execute for the owner — `0500` is refused like `0644`; a three-digit mode spells the same file mode as its four-digit form, so `600` is `0600`), and every task that carries a secret lowers with the engine's no-log flag. A validated file without secrets keeps its `.<name>.ruxel-candidate` between runs, so a second apply reports no change; a secret-bearing file validates on a temporary candidate that a block task's `always` removes, success or failure, so no resolved plaintext stays on the host; the staging and removal tasks report no change of their own. A file replacement is published atomically with its requested mode and its requested owner and group resolved to ids before any write: an unresolvable owner or group fails with the destination byte- and mode-identical, and a secret replacement is never published — not even momentarily — under the destination's previous owner or permissions. Every store argument is checked in the function body as well as the signature (pklr skips parameter constraints): a `fnoxGenerated` format outside `hex16`/`hex32`/`base64-32`/`alnum32` is refused, a `join` separator may carry a newline but no secret or run-time value, NUL, or `\u{1f}`/`\u{1e}` (the reference encoding reserves them; a literal `\u{1f}` is escaped as `\u{1e}` so decoding is never ambiguous), and a 1Password selector may carry no secret or run-time prefix, NUL, or reserved control character; a 1Password field or section may contain no `,`, `=` or `.` — `op item get --fields` reads `,` and `=` as its own selector syntax and `.` as the section separator, so a legal label like `API,key` or `API.Password` would fetch a different selector than the author named (vault and item labels are separate arguments and carry no such rule); the engine refuses the same characters at resolution. Lowering re-validates everything it decodes from a `SecretText` reference with the same sentences, so a forged `parts` encoding cannot smuggle an illegal key, format, `op://` reference, label or separator past the decode; malformed encodings are refused instead of reaching file content as literal text. Placeholder segmentation is code-point correct, so non-BMP text around a secret or run-time value is preserved; the secret placeholder payload spans lines, as a `join` separator may carry a newline, is tempered against its own `__` terminator, and never ends on another placeholder's opening, so one placeholder can never swallow a later one and an unterminated placeholder is refused as a malformed secret reference instead of lowering with the later value left behind as literal content. The 1Password fetch is field- and section-scoped (`op item get --fields <section>.<field>`), always names vault, item and field (no defaults), keeps sections in separate cache entries, and fails on a label several sections carry, naming the sections. Reference and address forms are validated at evaluation: an `op://` reference with a trailing `/` or a `?` query is refused (`op://<vault>/<item>/<field>`, or with a section `op://<vault>/<item>/<section>/<field>`, and nothing else), and an IPv4 host address with a trailing dot is refused, like one with an empty octet. Under pklr, workflow files reach the four functions through an import of `modulepath:/ruxel/Core.pkl` (`core.fnox(…)`): calling them through the workflow's own module chain from inside a step mis-lowers the configuration (duplicate plays), the same pre-existing defect that mis-lowers `module.currentHost` in file content.
- A typed workflow language: workflows amend `modulepath:/ruxel/Workflow.pkl` and list stages of steps (`File`, `Packages`, `Service`, `Command`, `Capture`, …); recipes extend `modulepath:/ruxel/Core.pkl` and import the layer modules they use (`Linux`, `Systemd`, `Debian` under the same prefix). `modulepath:/ruxel/internal/Lowering.pkl` lowers them to engine input and rejects broken references with plain messages. Admission refuses a configuration built by a copy of `Workflow.pkl` or `Lowering.pkl` (`Config.languageFingerprint`), as it refuses copies of the engine schema.
- The binary serves its compiled-in schema as layer modules under `modulepath:/ruxel/` (`Core`, `Linux`, `Systemd`, `Debian`, `Workflow`, `internal/Lowering`, `engine/Engine`); there is no single `Ruxel.pkl` module; `plan`, `check`, `diff` and `apply` refuse configuration evaluated against any other copy of the schema.
- `--module-path DIR[:DIR...]` on `config dump`, `plan`, `check`, `diff` and `apply` serves shared Pkl libraries, like `pkl eval --module-path`.
- `ruxel schema export DIR` writes the served schema for editors and the `pkl` CLI.
- `--root-dir DIR` on `config dump`, `plan`, `check`, `diff` and `apply` widens the directory a configuration may read, like `pkl eval --root-dir`.
- Strict guard and SSH configuration types and retained evaluated input for future admission.
- Embed the canonical Pkl schema for configuration tooling.
- Engine secret wrappers, value redaction, and secret-aware ledger keys.
- JSON, YAML-compatible JSON, and TOML rendering for resolved configuration values.
- Typed Pkl configuration, evaluation via pklr, and `ruxel config dump`.
- Controller commands for plan, check, diff, and apply, with synthetic local acceptance coverage in progress.
- alint repository policy (tailrocks rust-strict-v1) in `mise run check` and CI.
- `hk.pkl` git hooks (mise, gitleaks, cargo fmt, check, clippy, deny, rumdl).
- Initial research implementation and closed-workload CLI/agent surface.

### Changed

- The engine schema moves to `modulepath:/ruxel/engine/Engine.pkl` (module `Engine`); the workflow language is now the layer split under `modulepath:/ruxel/` (`Core` for generic blocks, `Linux`, `Systemd`, `Debian` for the layers; recipes extend `Core`). Configuration that imports the engine schema must change its import. The schema fingerprint covers every served module, and `ruxel schema export` writes them all.
- Evaluation allows an expression depth of 256 instead of pklr's default of 32.
- Pkl imports and `file:` resources, in configurations and templates, are confined to the root directory (default: the root module's directory) and the module path directories; symlinks inside them, network imports, and module and resource globs (`import*`, `read*`) are refused.
- Command and shell tasks that run invalidate the agent's package and unit snapshots, so a later apt or systemd task, or its ledger entry, never trusts state read before the command.
- Verify complete SSH argument vectors for guard admission modes.
- Downloads use trusted system clients with an isolated environment that preserves proxy settings.
- Package refresh, upgrade, autoremove and service-manager reload tasks bypass the ledger cache.
- Apt tasks run requested autoremove after distribution upgrades in apply and check mode.
- Agents require one successful handshake before executing batches and reject duplicate handshakes.
- Agents finish the current iteration and flush the ledger on SIGTERM or SIGINT during a batch.
- Sensitive-resource tracking treats parent traversal and relative remote paths conservatively.
- Block check mode reaches nested tasks, rescue/always tasks and controller pauses; child overrides take precedence.
- Workspace lints from the operator's repository standard, clippy configuration.
- Crates moved to `crates/<group>/<package>` (repository policy).
- Dependencies updated to their newest versions.
- Toolchain: Rust 1.99.1, operator mise layout with mbx as the cargo wrapper, lockfile v4; uv removed.
- CI regenerated with velnor-actions 0.1.75; tool versions now come from `mise.toml`.

### Removed

- Copies of `schema/Ruxel.pkl` beside fixtures, and the acceptance tools' `--schema` options.
- Duplicate legacy loop-result reason field; empty loops expose `skip_reason`.
- YAML and INI configuration input and MiniJinja authoring.
- Ansible parity tooling, Ansible-era docs, and agent modules no schema reaches; recoverable from tag `ansible-parity-final`.
- `justfile`; `mise run check` replaces `just check`. The Python Ansible-parity suites no longer run by default.
