You are the orchestrator for https://github.com/tailrocks/ruxel. The priority order below is a dependency order, not a license to work on one thing at a time. Wall-clock time is parallel. A unit waits only for the files it will change. Everything else runs now.

Repo: tailrocks/ruxel. One git worktree per unit. Never commit on the main checkout. Commit as Alexey Zhokhov <alexey@zhokhov.com> with `git commit -s` (one Signed-off-by, added by Git). Push and merge finished work. The Ansible playbooks in ChainArgos/infrastructure are the specification. Do not edit them.

The orchestrator session is `muse-spark-1.3` at effort `medium`. It dispatches, checks diffs, and merges. It does not implement.

## Models and effort

Every dispatch names a model and an effort. Never omit effort. `muse-ember-1.0` and `muse-wonder-1.0` default to `max`, and `muse-spark-1.3` defaults to `high`. An omitted effort bills the expensive default.

Almost all units are Ember or Spark. Wonder is two design passes. Wonder Full is one last-resort call per phase, and only after Wonder has failed that same decision twice.

| Work | Model | Effort |
| --- | --- | --- |
| Board, PR lists, commit-identity check, deletion of an already-decided branch | `muse-ember-1.0` | `minimal` |
| Branch diff triage (value or debris), log reading, gate pass/fail, CI poll | `muse-ember-1.0` | `low` |
| Renovate rebase, lockfile stack, predicted replay, mechanical conflict replay | `muse-ember-1.0` | `low` |
| Extract a hunk after the triage decision, apply a specified patch | `muse-ember-1.0` | `medium` |
| #120 rebase and generated-workflow update, when the migration diff already exists | `muse-ember-1.0` | `medium` |
| Review of a mechanical or generated diff (lockfile, rebase, workflow regen) | `muse-ember-1.0` | `medium` |
| Upstream lookup of one flag or command (LVM, XFS, apt, PostgreSQL, kubeadm) | `muse-ember-1.0` | `low` |
| Implement a step whose signature and behavior are already written | `muse-spark-1.3` | `medium` |
| #100 rebase, catalogue and engine implementation, workflow authoring | `muse-spark-1.3` | `medium` |
| Correctness review of catalogue or engine code | `muse-spark-1.3` | `medium` |
| Security review of a track diff (secrets, SSH, disk format, privilege) | `muse-spark-1.3` | `high` |
| Phase 3 catalogue shape, once | `muse-wonder-1.0` | `medium` |
| Phase 4 Kubernetes shape, once | `muse-wonder-1.0` | `medium` |
| Same decision after Spark `high` has failed it twice | `muse-wonder-1.0` | `medium` |
| Same decision after that Wonder pass has also failed twice | `muse-wonder-full-1.0` | `medium` |

Escalation is one step, and only for the unit that failed. The others keep running.

1. Same model and effort, once, with the failure evidence.
2. Ember `low` or `medium` becomes Spark `medium`.
3. Spark `medium` becomes Spark `high`.
4. Spark `high` twice becomes Wonder `medium`.
5. Wonder `medium` twice becomes one Wonder Full `medium` call for that decision. Then stop and ask.

Do not use `xhigh` or `max`. Do not send a whole file or a whole branch diff to Ember when a path and a hunk range will do: an Ember request over 200K input tokens is billed entirely at the long-context rate.

## Parallel by default

You orchestrate. From the first minute, fill every free slot with a subagent at the model in the table. An idle slot while any unit in this prompt is not done is a failure.

Launch the first wave in one turn, before any of them finish:

- Ember `medium`: land the mechanical half of PR #120 (`ci/velnor-gen`).
- Ember `medium`: review that generated workflow diff as soon as it exists.
- Ember `low`, one agent per remaining Renovate PR: rebase onto the #120 head and leave it merge-ready. Do not wait for #120 to merge. Rebase onto the `ci/velnor-gen` tip, then replay onto main the moment #120 lands.
- Spark `medium`: rebase #100 `feat/catalogue-cat07` onto the #120 head, keep the feature, drop conflict junk.
- Ember `low`, several agents: split the branch list into chunks and judge every branch.
- Wonder `medium`, one agent: the phase 3 catalogue shape. This is the only Wonder call in the wave.
- Wonder `medium`, one agent: the phase 4 Kubernetes shape. This is the other Wonder call. It does not wait for phase 3 code.

Rules:

- One writer owns one worktree, one branch, and one file set. Two writers never share a file, branch, or worktree.
- Read-only agents have no cap other than the number of units. Writers are capped by non-overlapping file sets, not by caution.
- The only serial step in the whole job is the final merge of two changes that touch the same file. Rebase, conflict resolution, local gates, review, push, and merge of disjoint changes all run at the same time.
- Speculative rebase: while change A is in CI, change B is already rebased onto A's head in another worktree and its CI is already running. When A merges, B replays. If main moved exactly as predicted, that replay is empty and B merges immediately. The replay agents are Ember `low`.
- Do not babysit one PR, one branch, or one CI check. Ember `minimal` polls them together. The moment one goes green and its files are free, merge it and give that slot the next unit.
- Subagents do not spawn subagents. You are the only dispatcher.
- Pipeline each unit: Ember `low` locates, Wonder `medium` designs only when the table says so, Spark `medium` implements, Ember `low` runs the repo gate, the table's reviewer reads the diff. Pass reviewers the diff, not the implementer's summary. Check the reviewer's claims against the diff before you merge.
- Keep a live board: unit, model, effort, owner, worktree, branch, files owned, status (running / blocked on which files / ready to merge / done). A blocked unit names the file it is waiting on. It does not name a phase.

## Phase 1 — merge the open pull requests in parallel

Process every open pull request at https://github.com/tailrocks/ruxel/pulls. Land them. #120 is the only PR whose merge precedes the others, because the others rebase onto it. Its wait is not a wait for the rest of the phase.

1. PR #120, branch `ci/velnor-gen`: `chore(ci): migrate from velnor-actions to velnor-gen`. This replaces the deleted velnor-actions 0.1.80 engine with velnor-gen (tailrocks/velnor-new): `.velnor/rust-ci.toml` and the generated workflows, old `.velnor/config.toml` and the old workflows removed with no shims. Ember `medium` rebases it and updates the generated workflows. Ember `medium` reviews that diff. Spark `medium` touches it only if the generated CI is wrong in a way Ember cannot repair. Merge it as soon as it is green. Do not hold it for any other PR.

2. At the same time, one agent per remaining PR, each in its own worktree:
   - Spark `medium` on #100 `feat/catalogue-cat07`. It is a real feature (absent forms, including removing a firewall rule) and it conflicts. Rebase onto #120, keep the feature, drop conflict junk. Ember `medium` reviews the conflict resolution. Spark `medium` reviews the feature diff.
   - Ember `low` on each Renovate PR: #104, #114, #116, and #105 through #122. These are value when they pin a current tool or crate and the gate is green. If two bumps are the same crate, keep one and close the other with that reason.

3. Partition those PR branches by the files they touch. Ember `low` does the partition.
   - Disjoint groups merge together, in one shot, as soon as each is green on the post-#120 main. Do not form a queue of one.
   - The group that shares `Cargo.lock`, `Cargo.toml`, or `mise.lock` is a speculative stack of Ember `low` agents, not a serial babysit. Order toolchain and lockfile maintenance before single-crate bumps. Agent N rebases bump N onto bump N-1's head while bump N-1's CI is still running. Merge the stack from the bottom as each predicted rebase comes back empty. A bump that fails the gate for a real incompatibility is closed with the reason and the name of the successor that replaces it. A mechanical failure is fixed in that bump's worktree by Ember `medium` without stopping the others.

A PR is done when it is merged to main, or closed with a specific reason and any valuable commit preserved on main. Do not leave them open. Merging one PR does not pause the agents still rebasing the others.

## Phase 2 — judge every leftover branch in parallel

This starts in the first wave, while #120's CI runs. Do not wait for phase 1 to finish before reading branches.

Inventory every branch at https://github.com/tailrocks/ruxel/branches except `main` and branches whose PR is already being merged. As of this writing that leftover set is about ninety `backup/local-2026-11-17/<sha>` branches plus `refactor/flat-crates`. Fetch the live list. Do not trust a local clone.

Ember `low` agents split the list. Each diffs its chunk against current main and against the #120 head, and returns a decision per branch from the diff, not from the name:

- Land the whole branch when the whole diff adds value: a feature, an extension of the catalogue or the engine, an improvement, a bug fix, or any other change that makes ruxel more capable or more correct.
- Extract onto a clean branch when only part of the diff adds value.
- Drop the rest. Progress logs, session notes, debug transcripts, temporary fixtures, commented-out experiments, and drive-by reformatting have no value. Extract the valuable hunks first.

Ember `low` then groups the extractions and whole-branch lands by the files they touch. Disjoint groups get writers at the same time. A pure extraction or a mechanical replay is Ember `medium`. A landing that still needs a real code fix is Spark `medium`. A group that touches a file an open PR also touches waits only for that file, and the wait is spent preparing the extraction in a worktree rebased onto that PR's head.

Before you delete a remote branch, the valuable part is already on main, and the dropped part is one line: what it was, and why it has no value. Do not delete a branch whose unique commits you have not read. Deletions of already-decided branches are Ember `minimal` and run in parallel.

`refactor/flat-crates` gets the same test, in its own worktree, during this wave. Ember `low` reads the diff. Spark `medium` lands it only if the crate move is still true on the new main and the tests prove it. Otherwise Ember `medium` takes the useful idea and closes the branch.

## Phase 3 — the three Ansible playbooks, built in parallel tracks

This is the product. The Wonder `medium` design starts in wave 1 and is the only design pass. Spark `medium` writers start the moment their files are free of phase 1 and phase 2 changes. They do not wait for the last backup branch to be deleted. A backup branch that only deletes logs does not block catalogue code.

Ruxel must become a pure alternative for setting up a new empty server with these three playbooks, and only these behaviors:

- https://github.com/ChainArgos/infrastructure/blob/main/ansible-configs/install-base.yml
- https://github.com/ChainArgos/infrastructure/blob/main/ansible-configs/init-postgresql-nova-drives.yml
- https://github.com/ChainArgos/infrastructure/blob/main/ansible-configs/setup-postgresql-nova.yml

"Pure alternative" means an operator writes workflows in the catalogue language the binary serves (`modulepath:/ruxel/`, workflows amending `Workflow.pkl`), runs `plan`, `check`, `diff`, and `apply` against a new host, and gets the same end state a second time with no changes. Hand-written `Engine.pkl` that bypasses the catalogue is not the alternative. A shell script that hides the PostgreSQL and disk work is not the alternative.

The Wonder `medium` design re-reads the playbooks and current main. The gaps known on `0190069` are the starting list. Verify each one. Close it in the catalogue, with an agent implementation and a test, or record why the playbook behavior is intentionally narrower. That design names the signature of every new step. Writers do not invent a second shape.

After that design, these Spark `medium` writers run at once, each on files the design assigned and no others:

- Catalogue types and lowering for the new steps.
- Base host: apt, git, files, terminfo, installers (`install-base.yml`).
- LVM and XFS (`init-postgresql-nova-drives.yml`).
- Mount and sysctl.
- PostgreSQL objects and secrets.
- The three workflows, drafted against the agreed signatures while the steps are still being implemented, and rebased when the signatures land.
- Ember `low` prepares the disposable-host proof (hosts, images, guard policy, script) while the code is in review, and runs it the hour the code is on main.

Base host (`install-base.yml`), already close, still required. Spark `medium`:

- Timezone UTC, the apt package set, Nushell and mise repositories and packages, holla at `latest` with its signed-by key, root shell zsh, batcat symlink, Git LFS, Oh My Zsh, zsh-autosuggestions, Starship, the zshrc, and the mise toolchains (GraalVM, Rust, cargo-binstall, rust-script, just, shellfirm, tirith, bottom, zellij).
- xterm-ghostty terminfo is built on the controller and compiled on the host. Catalogue captures run on a target. Add a real controller-side read, or an equivalent step, so the host still ends with that terminfo entry.
- Workflow git today is HTTPS plus a full commit SHA. This play tracks `zsh-autosuggestions` at `master`. Implement the tracking form the design chose.
- Starship is installed from a downloaded script on stdin. Model it so a second apply does not reinstall.
- `meta: flush_handlers` does not exist, and handlers run only at the end of a play. The holla apt refresh must be an ordinary step ordered before the install.

Disks (`init-postgresql-nova-drives.yml`), missing from the catalogue. Spark `medium`, following the Wonder design:

- Add catalogue steps for a volume group, a logical volume of `+100%FREE`, and an XFS filesystem, plus the packages those steps need (`lvm2`, `xfsprogs`). The three by-id disks are inputs, not hardcoded serials.
- A first apply formats an empty device. A second apply must converge. Today a second apply with `resizefs: true` demands the filesystem already be mounted and then hits the XFS grow prototype, which `docs/OPERATOR-SETUP.md` says is not mounted-filesystem acceptance. `xfs_growfs` is refused because it reopens a mutable mount path. Replace that with a grow that is safe on this play's order: format in the disk play, mount later at `/var/lib/postgresql`, and a re-run of the disk play must succeed whether or not the volume is mounted yet. Prove it on a disposable Linux host with real block devices, not a mock. Until that run is green, this play is not done.
- `vgcreate` is unforced and does not wipe signatures. Match the playbook: fail closed on a disk that already has a signature. Say so in the step.

PostgreSQL host (`setup-postgresql-nova.yml`), missing from the catalogue. Spark `medium`, following the Wonder design:

- Mount the XFS LV at `/var/lib/postgresql` by UUID, with the playbook's options (`logbufs=8,noatime,inode64,allocsize=16m,logbsize=256k`). The UUID is learned from the host and used in the same run.
- Sysctl is in the catalogue as of CAT.06. Express `vm.swappiness`, `vm.overcommit_memory`, `vm.nr_hugepages`, and `vm.overcommit_ratio` in the files the play uses. Also swap off, the fstab edit, pam_limits, and the postgres memlock and nofile limits.
- PGDG repository using the host's Debian codename (`facts.distribution_release` already exists), then PostgreSQL 18 and the dev packages.
- Catalogue steps for roles, databases, schemas, and privileges. The engine modules already speak this SQL (`SUPERUSER`, port 40000, `CONNECT`, `USAGE`, `SELECT`, `ALL_IN_SCHEMA`, default privileges on tables, SCRAM). Lift that into workflow steps. Do not leave operators on raw engine config.
- 1Password secrets must be able to be a role password and the body of `pg_hba.conf` at mode `0640`, and an SSH public key at `0644`. Today a workflow secret may only sit in `File.content` or a command environment, and secret file content must be `0600` or narrower, so this play is inexpressible. Fix the type so a secret can be a password and so `pg_hba.conf` can contain secret usernames at `0640`, without letting a secret leak into logs. Private key stays `0600`. The secret-type and privilege choice is part of the one Wonder `medium` design. Spark implements it. Spark `high` reviews that diff.
- `pg_hba.conf` is Jinja in Ansible. Ruxel templates are Pkl. Write the file in the catalogue so the 1Password usernames are filled in and a second apply is a no-change.
- SSH clone of `git@github.com:ChainArgos/java-monorepo.git` at branch `main` with host-key acceptance, after the key is installed. Workflow git must be able to do that, or you add the step the design named. Pinning a SHA does not match this play.
- Build and install `pg_parquet` 0.5.1 with `cargo-pgrx` 0.16.0 only when `/usr/lib/postgresql/18/lib/pg_parquet.so` is absent. Second apply does not rebuild.
- systemd drop-in `LimitNOFILE=1048576` for `postgresql@.service`, daemon-reload before the service is used, `postgresql@18-main` enabled and started, config and `pg_hba.conf` owned by postgres. Reload PostgreSQL when those files change. Never restart it automatically. Report settings that still have `pending_restart`.
- MultiXact monitor script, service, and timer, enabled.
- Databases, owners, and grants exactly as the playbook: chainargos SUPERUSER, redshift, wallet-screening, the four chain roles and their primary plus verify databases, looker grants on redshift, kestra and its two databases. Login port 40000.

Agent privilege: the agent runs as the SSH user and has no sudo, and `become_user` works only for command, shell, and PostgreSQL modules. These plays are `become: true` throughout. An empty server must be configurable by SSH as root, or you add a real privilege step for apt, LVM, files, mounts, and systemd and test it. The Wonder `medium` design picks one. Spark `medium` makes the three workflows use it and documents the choice in the operator setup.

Spark `high` reviews secrets, the SSH clone, disk format, and privilege escalation on each track's diff as it appears, in parallel with the writers. Ember `medium` reviews the other diffs.

Done for phase 3 means all three:

- Three workflows in the repo that are the playbooks, with secrets referenced and not inlined.
- `check` then `apply` then `apply` on a disposable Linux host: the second apply reports no changes, PostgreSQL accepts a login on port 40000, and the data LV is mounted at `/var/lib/postgresql`.
- A re-run of the disk workflow alone, after the mount exists, also reports no changes and does not hit the unaccepted XFS prototype.
- The repo gate is green on that main. A schema class with no host run does not count.

Those three proof runs are independent once the code is on main. Ember `low` starts them together. A failed proof goes to the Spark `medium` writer that owns those files, then up the escalation ladder. It does not get a Wonder Full call unless that ladder says so.

## Phase 4 — Kubernetes, designed while phase 3 is written

The Wonder `medium` design starts in wave 1. That is the second and last planned Wonder call. Spark `medium` implementation starts the hour the phase 3 proofs are green, and not before. Do not implement Kubernetes while the disk and PostgreSQL tracks are still moving. Do use the wait: the design, the signatures, and an Ember `low` disposable-host plan are finished before the proofs return, so coding starts immediately when they pass.

Out of the box means a new empty server, already through the base workflow, can become a Kubernetes cluster without an operator assembling kubeadm by hand. Today `tests/workflow/recipes/kubeadm-join.pkl` only writes a join config and reads a CA hash from a control plane that is already up. That is not a cluster.

Build on that recipe. Add the missing catalogue: container runtime, kubeadm, kubelet, kubectl, cluster init for the first control plane, join for more control planes and for workers, and the network addon the design chooses. Split those into disjoint Spark `medium` writers the same way as phase 3. Ember `low` looks up the pinned kubeadm flags. One workflow brings up a single control plane from a new host. A second workflow joins a node. A second apply of either converges. Ember `low` proves both on disposable hosts, in parallel. Spark `medium` reviews the diffs. Spark `high` reviews only the join credentials and the network addon. Defer multi-cluster products, GitOps, and anything the join recipe does not need for that proof.

## Report

Ember `minimal` keeps one running board, updated whenever a unit changes state: model, effort, what is running in parallel right now, what merged, what was deleted and why, and the file each blocked unit is waiting on. At the end, list merged PRs, branch decisions, the three workflow paths, the host-proof result, and the Kubernetes proof. Claim a phase done only when its proof has been run.
