#!/usr/bin/env python3
"""Exercise two reviewed, non-destructive scenarios through the real local agent.

Local transport is not a sandbox. Fixture hashes fail closed before execution;
review any changed fixture before changing its pin. These tests do not prove
scheduler concurrency or cover the privileged storage/system scenarios.
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORKLOADS = ROOT / 'testdata/workloads/pr66'
SCHEMA = ROOT / 'schema/Ruxel.pkl'
PINS = {
    'controller-delegation': '34a0ac522dd9d9ff34b18f2fe6564c9d9626735fd2df7a0a88e6ecbd56520b2b',
    'multihost': '05ec8dd45d82391528d6cdc3e504e2ffe471c1a291484eb8d1971f108c50ed8a',
}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(cli, agent, module, state, mode, *options, expected_exit=0):
    result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                             '--state-dir', str(state), *options], capture_output=True, text=True, timeout=30)
    require(result.returncode == expected_exit, f'{mode} exited {result.returncode}, expected {expected_exit}: {result.stderr}\n{result.stdout}')
    return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]


def check_events(events, hosts, task, mode, value, expected_assert_failure=False):
    recaps = [e for e in events if 'recap' in e]
    require(len(recaps) == len(hosts) and {e['host'] for e in recaps} == hosts,
            f'{mode}: missing, duplicate, or unexpected host recaps')
    expected_failed = 1 if expected_assert_failure else 0
    require(all(e['recap']['failed'] == expected_failed for e in recaps), f'{mode}: unexpected host failure count')
    if expected_assert_failure:
        assertions = [e['result'] for e in events if e.get('task') == 'Assert delegated result shape' and 'result' in e]
        require(len(assertions) == 1 and assertions[0].get('failed') is True,
                f'{mode}: expected downstream assertion failure')
        require(all(e['recap']['changed'] == 0 for e in recaps), f'{mode}: reported changes')
    if mode in ('check', 'diff'):
        results = [e for e in events if e.get('task') == task and 'result' in e]
        require(len(results) == len(hosts) and {e['host'] for e in results} == hosts,
                f'{mode}: missing or duplicate command results')
        require(all(e['result'].get('skipped') and not e['result'].get('changed') for e in results),
                f'{mode}: command was not skipped')
    if mode == 'apply':
        results = [e for e in events if e.get('task') == task and 'result' in e]
        require(len(results) == len(hosts) and {e['host'] for e in results} == hosts,
                f'{mode}: missing, duplicate, or unexpected task results')
        for event in results:
            require(event['result'].get(value[0]) == value[1], f'{mode}: unexpected task result: {event}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scenario', choices=sorted(PINS))
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    fixture = WORKLOADS / f'{args.scenario}.pkl'
    data = fixture.read_bytes()
    require(hashlib.sha256(data).hexdigest() == PINS[args.scenario],
            'fixture changed: review safe local execution before updating its hash')
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    if args.scenario == 'multihost':
        hosts = {f'fixture-{i}' for i in range(1, 7)}
        task, value = 'Run one-second independent host work', ('rc', 0)
    else:
        hosts = {'fixture'}
        task, value = 'Read synthetic value on controller', ('stdout', 'ruxel-fixture-controller')
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-') as temp:
        base = Path(temp)
        fixture_dir = base / 'testdata/workloads/pr66'
        fixture_dir.mkdir(parents=True)
        (base / 'schema').mkdir()
        shutil.copyfile(SCHEMA, base / 'schema/Ruxel.pkl')
        module = fixture_dir / fixture.name
        module.write_bytes(data)
        for mode in ('plan', 'apply', 'apply', 'check', 'diff'):
            # Agent ledger is execution metadata; snapshot the workload tree and
            # schema, which are the only fixture files this reviewed scenario uses.
            snapshot = {p.relative_to(base): p.read_bytes() for p in base.rglob('*')
                        if p.is_file() and not p.is_relative_to(base / 'state')}
            expected_failure = args.scenario == 'controller-delegation' and mode in ('check', 'diff')
            events = run(cli, agent, module, base / 'state', mode,
                         expected_exit=8 if expected_failure else 0)
            check_events(events, hosts, task, mode, value, expected_failure)
            if mode in ('check', 'diff'):
                after = {p.relative_to(base): p.read_bytes() for p in base.rglob('*')
                         if p.is_file() and not p.is_relative_to(base / 'state')}
                require(after == snapshot, f'{mode}: fixture files changed')
        if args.scenario == 'multihost':
            for selector, expected in [('multihost', hosts), ('fixture-*', hosts),
                                       ('fixture-1,fixture-3', {'fixture-1', 'fixture-3'})]:
                events = run(cli, agent, module, base / 'state', 'plan', '--hosts', selector)
                check_events(events, expected, task, 'plan', value)
            invalid = subprocess.run([str(cli), 'plan', str(module), '--local', '--agent', str(agent),
                                      '--state-dir', str(base / 'state'), '--hosts', 'missing*'],
                                     capture_output=True, text=True, timeout=30)
            require(invalid.returncode != 0 and not invalid.stdout.strip(),
                    'unmatched selector was accepted or emitted host events')
    print(f'PASS {args.scenario}: plan, two applies, check, diff; see limitations in module docstring')


if __name__ == '__main__':
    main()
