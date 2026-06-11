# OPERATOR-SETUP — Three One-Time Actions

What the operator provides before M0, and exactly how. Everything here is
designed so that afterwards the agent works autonomously **with hard
isolation from production**: the fixture token physically cannot reach the
dedicated servers, and the CI secret token physically cannot read real
secrets.

---

## 1. Hetzner Cloud access (fixture VMs)

Why it is safe by construction: the production servers are **Hetzner
Robot dedicated machines** — a completely different API and account scope
from Hetzner Cloud projects. A Cloud API token is scoped to **one project**
and can only see resources inside it. A token for a fresh, empty project
can create/destroy VMs there and nothing else, anywhere.

Steps (≈2 minutes, in https://console.hetzner.cloud):

0. **Account check (the only moment it can be verified):** log in as
   **alexey@chainargos.com** — the ChainArgos Hetzner account (credentials
   in the ChainArgos vault's existing `Hetzner` item), not a personal
   account. The Cloud API has no account-identity endpoint, so a token's
   owning account cannot be verified after creation; this console glance
   is the verification.
1. **New Project** → name it `ruxel-fixtures`. Keep it empty of anything
   else, forever.
2. Open the project → **Security → API tokens → Generate API token** →
   description `ruxel-fixtures-agent`, permissions **Read & Write**.
3. Store the token in 1Password: vault `ChainArgos`, new item named
   **`ruxel Hetzner Cloud`**, field **`token`** (type: password).

That's all. The agent then reads it at runtime — never written to disk,
never committed:

```bash
export HCLOUD_TOKEN="$(op read 'op://ChainArgos/ruxel Hetzner Cloud/token')"
```

`tools/fixtures/` scripts (M0) use `hcloud` CLI with that env var to
create a CX-line x86_64 Debian 12 VM per test session and destroy it
afterwards (cost: cents per session; a forgotten VM is a few €/month and
the scripts list+reap leftovers on every run).

## 2. 1Password test vault + service account (CI secrets path)

Why this exists: the playbooks' secrets all come from
`lookup('community.general.onepassword', …)`. Ruxel's resolver must be
tested against a real `op`, including in CI — but CI must never be able to
read the real `ChainArgos` vault, and your local `op` uses biometric
unlock, which CI cannot do. The fix is 1Password's **service account**:
a token-authenticated identity granted access to exactly one vault.

Steps (≈3 minutes, on https://my.1password.com):

1. **New vault** → name `ruxel-test`. (Leave it empty — the agent will
   populate it with synthetic items that mirror the *shapes* of the real
   ones: an item named like an SSH item with `private key`/`public key`
   fields, a fake PostgreSQL password item, etc. Dummy values only.)
2. **Developer → Service Accounts → New service account** → name
   `ruxel-ci`, grant **read & write access to the `ruxel-test` vault
   only** (write lets the test suite create its own fixtures; it can be
   downgraded to read-only after M1 seeds them).
3. Copy the token (`ops_…`) once, store it in 1Password: vault
   `ChainArgos`, item **`ruxel CI service account`**, field **`token`**.

The agent then installs it as a GitHub Actions secret itself:

```bash
gh secret set OP_SERVICE_ACCOUNT_TOKEN \
  --repo tailrocks/ruxel \
  --body "$(op read 'op://ChainArgos/ruxel CI service account/token')"
```

In CI, `OP_SERVICE_ACCOUNT_TOKEN` in the environment makes `op read`/`op
item get` work non-interactively — against `ruxel-test` only. Locally,
nothing changes: your normal biometric `op` session is used.

## 3. Baseline timings (you run, when convenient)

Purpose: the honest "before" numbers. These are your **normal maintenance
reruns** — the same commands you already run, just with per-task timing
turned on and the output kept. Run each against a server that you believe
is already converged (the painful no-op case), whenever you would touch
that server anyway. **One run per playbook below is enough; no need to do
them all in one sitting, and no need to run the drive-init or restart
playbooks at all.**

```bash
cd ~/Projects/ChainArgos/java-monorepo/ansible-configs
export ANSIBLE_CALLBACKS_ENABLED=ansible.posix.profile_tasks
export ANSIBLE_LOCAL_TEMP=/private/tmp/ansible-local

time ansible-playbook -i hosts.ini --limit sentry           setup-sentry.yml           2>&1 | tee /tmp/baseline-sentry.log
time ansible-playbook -i hosts.ini --limit delorean         setup-delorean.yml         2>&1 | tee /tmp/baseline-delorean.log
time ansible-playbook -i hosts.ini --limit postgresql-nova  setup-postgresql-nova.yml  2>&1 | tee /tmp/baseline-nova.log
time ansible-playbook -i hosts.ini --limit sentry           install-base.yml           2>&1 | tee /tmp/baseline-install-base.log
```

`profile_tasks` (from the `ansible.posix` collection you already install)
prints a sorted per-task timing table at the end; `time` gives the total.
Afterwards just say "baseline logs are in /tmp" — the agent copies them
into `docs/benchmarks/baseline/` and they become the comparison anchor for
every ruxel benchmark.

Priority if you only do one: `setup-sentry.yml` (it is 44% of all reruns).

---

After actions 1–2 the agent operates without you: fixture VMs, CI, oracle
captures, benchmarks. Action 3 is the only thing that inherently needs
your hands, because it touches production — which the agent never does.
