#!/usr/bin/env python3
"""Run the check-semantics workload using the real local controller and agent.

This deliberately accepts only the reviewed, file-only fixture: local transport
is not a sandbox for package managers, storage, firewall, or system services.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / 'testdata/workloads/pr66/check-semantics.pkl'
SCHEMA = ROOT / 'schema/Ruxel.pkl'
FIXTURE_SHA256 = '1e6b201dc0db963ef6b94be616341c99a063a6cc06c40a4cf469167efc476a94'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(cli, agent, module, state, mode):
    command = [str(cli), mode, str(module), '--local', '--agent', str(agent), '--state-dir', str(state)]
    result = subprocess.run(command, text=True, capture_output=True, timeout=30, check=False)
    require(result.returncode == 0, f'{mode} exited {result.returncode}: {result.stderr}\n{result.stdout}')
    try:
        events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise RuntimeError(f'{mode} did not emit JSONL: {error}') from error
    recaps = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
    require(len(recaps) == 1 and recaps[0]['failed'] == 0, f'{mode}: missing or failed host recap')
    return events, recaps[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    fixture = FIXTURE.read_bytes()
    require(hashlib.sha256(fixture).hexdigest() == FIXTURE_SHA256,
            'fixture changed: review it for safe local execution before updating its hash')
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-') as temp:
        base = Path(temp)
        # Keep the original relative schema import valid in the isolated tree.
        fixture_dir = base / 'testdata/workloads/pr66'
        fixture_dir.mkdir(parents=True)
        (base / 'schema').mkdir()
        shutil.copyfile(SCHEMA, base / 'schema/Ruxel.pkl')
        payload = base / 'payload'
        payload.mkdir()
        rendered = fixture.decode().replace('__RUN_ROOT__', str(payload))
        module = fixture_dir / FIXTURE.name
        module.write_text(rendered)
        state = base / 'state'
        target = payload / 'check-notify'
        cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
        _, plan = run(cli, agent, module, state, 'plan')
        require(plan['changed'] == 0 and not target.exists(), 'plan mutated or reported a change')
        for mode in ('check', 'diff'):
            events, recap = run(cli, agent, module, state, mode)
            require(recap['changed'] > 0 and not target.exists(), f'{mode} failed to predict without writing')
            handlers = [e.get('result', {}) for e in events if e.get('task') == 'Record predicted handler']
            require(len(handlers) == 1 and handlers[0].get('msg') == 'predicted handler ran'
                    and not handlers[0].get('failed') and not handlers[0].get('skipped'),
                    f'{mode} did not successfully run the notified handler')
            if mode == 'diff':
                require(any(e.get('result', {}).get('diff') for e in events), 'diff omitted file difference')
        _, fresh = run(cli, agent, module, state, 'apply')
        require(fresh['changed'] > 0 and target.read_text() == 'synthetic', 'fresh apply failed')
        _, converged = run(cli, agent, module, state, 'apply')
        require(converged['changed'] == 0 and target.read_text() == 'synthetic', 'rerun was not converged')
        target.write_text('drift')
        for mode in ('check', 'diff'):
            _, recap = run(cli, agent, module, state, mode)
            require(recap['changed'] > 0 and target.read_text() == 'drift', f'{mode} changed drift')
        _, repaired = run(cli, agent, module, state, 'apply')
        require(repaired['changed'] > 0 and target.read_text() == 'synthetic', 'drift was not repaired')
    print('PASS check-semantics: plan, check, diff, fresh, convergence, drift repair')


if __name__ == '__main__':
    main()
