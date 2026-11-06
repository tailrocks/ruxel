#!/usr/bin/env python3
"""Execute supported PR66 expectation cases; fail if any selected case is unsupported.

Only hash-pinned, locally reviewed unprivileged fixtures may execute here.
A passed case is evidence for its asserted expectations, not the entire matrix.
"""
import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[3]
WORKLOADS = ROOT / 'testdata/workloads/pr66'
SCHEMA = ROOT / 'schema/Ruxel.pkl'
ORACLE = ROOT / 'tests/acceptance/pr66/oracles/controller-delegation-check-diff.jsonl'
ORACLE_SHA256 = 'afc95e99e87c2ff9c41d3282166877e450cb5901c78e7823931d43db32beabe7'
SCENARIOS = {
    'become-environment', 'check-semantics', 'control-flow', 'controller-delegation',
    'files-content', 'multihost', 'performance-snapshots', 'postgresql-ownership',
    'restart-actions', 'shape-variants', 'storage-ext4', 'storage-two-tier',
    'storage-xfs', 'system-surface',
}
PINS = {
    'check-semantics': '1e6b201dc0db963ef6b94be616341c99a063a6cc06c40a4cf469167efc476a94',
    'multihost': '05ec8dd45d82391528d6cdc3e504e2ffe471c1a291484eb8d1971f108c50ed8a',
    'controller-delegation': '34a0ac522dd9d9ff34b18f2fe6564c9d9626735fd2df7a0a88e6ecbd56520b2b',
}


class Incomplete(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def path_in(root, value):
    prefix = '__RUN_ROOT__/'
    require(isinstance(value, str) and value.startswith(prefix), 'path must be under __RUN_ROOT__')
    relative = PurePosixPath(value[len(prefix):])
    require(relative.parts and all(p not in ('.', '..') for p in relative.parts), 'invalid fixture path')
    target = root.joinpath(*relative.parts)
    require(target.resolve().is_relative_to(root.resolve()), 'fixture path escapes root')
    return target


def snapshot(root):
    state = {}
    for path in root.rglob('*'):
        info = path.lstat()
        key = str(path.relative_to(root))
        if path.is_symlink():
            state[key] = ('link', os.readlink(path))
        elif path.is_dir():
            state[key] = ('directory', stat.S_IMODE(info.st_mode))
        elif path.is_file():
            state[key] = ('file', stat.S_IMODE(info.st_mode), path.read_bytes())
        else:
            raise RuntimeError(f'unsupported payload object: {key}')
    return state


def validate_case(name, case):
    require(name in PINS, f'{name}: no reviewed safe local adapter')
    require(set(case) <= {'fixture', 'observations', 'drift', 'phases', 'pending'},
            f'{name}: unsupported expectation fields')
    pending = case.get('pending', [])
    require(isinstance(pending, list) and (not pending or (name == 'controller-delegation'
            and pending == ['verify frozen diff assertion behavior independently'])),
            f'{name}: unsupported pending assertions')
    require(case.get('fixture') == f'{name}.pkl', f'{name}: fixture mismatch')
    phases = case.get('phases')
    require(isinstance(phases, dict), f'{name}: phases missing')
    required = {'fresh', 'converged', 'check', 'diff'} | ({'drift_repair'} if name == 'check-semantics' else set())
    require(set(phases) == required, f'{name}: unsupported or missing phases')
    for phase, assertions in phases.items():
        if name == 'controller-delegation':
            keys = {'succeeds'} if phase in ('fresh', 'converged') else {'state_unchanged'}
            if phase == 'check':
                keys.add('succeeds')
            if phase == 'diff' and isinstance(assertions, dict) and 'succeeds' in assertions:
                keys.add('succeeds')
        elif phase in ('check', 'diff'):
            keys = {'state_unchanged'}
        elif name == 'check-semantics':
            keys = {'observations_match', 'changed'}
        else:
            keys = {'succeeds_on_each_host'}
        require(isinstance(assertions, dict) and set(assertions) == keys,
                f'{name}: unsupported or incomplete assertions in {phase}')
    if name == 'controller-delegation':
        require(phases['fresh']['succeeds'] is True and phases['converged']['succeeds'] is True
                and phases['check']['succeeds'] is False, 'delegation success expectations differ from baseline')
        require('succeeds' not in phases['diff'] or phases['diff']['succeeds'] is False,
                'delegation diff expectation differs from frozen check+diff baseline')
    observations = case.get('observations')
    require(isinstance(observations, list) and observations, f'{name}: no observations')
    expected_kind = {'check-semantics': 'file', 'multihost': 'task_result_each_host',
                     'controller-delegation': 'task_result'}[name]
    require(all(isinstance(item, dict) and item.get('kind') == expected_kind for item in observations),
            f'{name}: unexpected observation type')
    for item in observations:
        require(isinstance(item, dict), 'invalid observation')
        if item.get('kind') == 'file':
            require(set(item) == {'kind', 'path', 'content'} and isinstance(item['content'], str), 'unsupported file observation')
        elif item.get('kind') == 'task_result':
            require(set(item) == {'kind', 'host', 'task', 'field', 'equals'}, 'unsupported task observation')
        elif item.get('kind') == 'task_result_each_host':
            require(set(item) == {'kind', 'hosts', 'task', 'field', 'equals'} and item['hosts'], 'unsupported host observation')
        else:
            raise RuntimeError(f'{name}: unsupported observation {item.get("kind")}')
    for item in case.get('drift', []):
        require(isinstance(item, dict) and set(item) == {'kind', 'path', 'content'}
                and item['kind'] == 'write_file' and isinstance(item['content'], str), 'unsupported drift')
    require(bool(case.get('drift')) == (name == 'check-semantics'), f'{name}: unexpected drift contract')


def execute(name, case, cli, agent):
    validate_case(name, case)
    if name == 'controller-delegation':
        # Frozen 99fb2d17 fresh-parity-gate.sh captured Ansible with both
        # --check and --diff; parity/controller-delegation.json records this hash.
        oracle = ORACLE.read_bytes()
        require(hashlib.sha256(oracle).hexdigest() == ORACLE_SHA256, 'frozen check+diff oracle hash mismatch')
        rows = [json.loads(line) for line in oracle.splitlines()]
        require([(row.get('task_name'), row.get('status')) for row in rows] == [
            ('Read synthetic value on controller', 'skipped'),
            ('Assert delegated result shape', 'failed'),
        ], 'unexpected frozen check+diff oracle events')
    data = (WORKLOADS / case['fixture']).read_bytes()
    require(hashlib.sha256(data).hexdigest() == PINS[name], f'{name}: fixture changed; review before updating pin')
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-expect-') as temp:
        base = Path(temp)
        fixture_dir = base / 'testdata/workloads/pr66'
        fixture_dir.mkdir(parents=True)
        (base / 'schema').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(SCHEMA, base / 'schema/Ruxel.pkl')
        module = fixture_dir / case['fixture']
        module.write_text(data.decode().replace('__RUN_ROOT__', str(payload)))
        def run(mode, expected=True):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=30)
            allowed = (0,) if expected is True else (8,) if expected is False else (0, 8)
            require(result.returncode in allowed, f'{name} {mode}: exit {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recaps = [e for e in events if 'recap' in e]
            require(recaps and all(e['recap'].get('failed') == (0 if result.returncode == 0 else 1) for e in recaps),
                    f'{name} {mode}: unexpected failed/missing recaps')
            if name == 'controller-delegation':
                require(len(recaps) == 1 and recaps[0].get('host') == 'fixture', f'{mode}: invalid delegation recap')
            if name == 'multihost':
                expected_hosts = {host for item in case['observations'] for host in item['hosts']}
                require(len(recaps) == len(expected_hosts) and {e['host'] for e in recaps} == expected_hosts,
                        f'{name} {mode}: missing or duplicate host recaps')
            return events, recaps
        def observe(events):
            for item in case['observations']:
                if item['kind'] == 'file':
                    target = path_in(payload, item['path'])
                    require(target.is_file() and not target.is_symlink() and target.read_text() == item['content'],
                            f'{name}: file observation failed')
                elif item['kind'] == 'task_result':
                    results = [e for e in events if e.get('host') == item['host'] and e.get('task') == item['task'] and 'result' in e]
                    require(len(results) == 1 and results[0]['result'].get(item['field']) == item['equals'],
                            f'{name}: task result mismatch')
                else:
                    hosts = item['hosts']
                    require(isinstance(hosts, list) and len(hosts) == len(set(hosts)), 'invalid expected host list')
                    results = [e for e in events if e.get('task') == item['task'] and 'result' in e]
                    require(len(results) == len(hosts) and {e['host'] for e in results} == set(hosts),
                            f'{name}: missing/duplicate host task results')
                    require(all(e['result'].get(item['field']) == item['equals'] for e in results),
                            f'{name}: host result mismatch')
        def check_phase(phase, events, recaps, before=None):
            expected = case['phases'][phase]
            if 'observations_match' in expected:
                require(expected['observations_match'] is True, f'{phase}: unsupported negative observation')
                observe(events)
            if 'changed' in expected:
                require(isinstance(expected['changed'], bool), f'{phase}: invalid changed expectation')
                require(any(e['recap'].get('changed', 0) > 0 for e in recaps) == expected['changed'], f'{name} {phase}: changed mismatch')
            if 'state_unchanged' in expected:
                require(expected['state_unchanged'] is True and before is not None and snapshot(payload) == before,
                        f'{name} {phase}: payload changed')
            if 'succeeds' in expected and expected['succeeds'] is True:
                observe(events)
            if 'succeeds_on_each_host' in expected:
                expected_hosts = {host for item in case['observations'] if item['kind'] == 'task_result_each_host' for host in item['hosts']}
                require(len(recaps) == expected['succeeds_on_each_host'] == len(expected_hosts)
                        and {e['host'] for e in recaps} == expected_hosts, f'{name} {phase}: host recap mismatch')
                observe(events)
        for phase in ('fresh', 'converged'):
            expected_success = case['phases'][phase].get('succeeds', True)
            require(isinstance(expected_success, bool), f'{phase}: invalid succeeds expectation')
            events, recaps = run('apply', expected_success)
            check_phase(phase, events, recaps)
        for item in case.get('drift', []):
            target = path_in(payload, item['path'])
            require(target.is_file() and not target.is_symlink(), 'unsafe or missing drift target')
            target.write_text(item['content'])
        for phase in ('check', 'diff'):
            before = snapshot(payload)
            expected_success = case['phases'][phase].get('succeeds', None if name == 'controller-delegation' else True)
            require(expected_success is None or isinstance(expected_success, bool), f'{phase}: invalid succeeds expectation')
            events, recaps = run(phase, expected_success)
            check_phase(phase, events, recaps, before)
            if name == 'check-semantics':
                handlers = [e['result'] for e in events if e.get('task') == 'Record predicted handler' and 'result' in e]
                require(len(handlers) == 1 and handlers[0].get('msg') == 'predicted handler ran'
                        and not handlers[0].get('failed') and not handlers[0].get('skipped'), f'{phase}: predicted handler missing')
                require(any(e['recap'].get('changed', 0) > 0 for e in recaps), f'{phase}: no change predicted')
                if phase == 'diff':
                    require(any(e.get('result', {}).get('diff') for e in events), 'diff missing')
            elif name == 'controller-delegation':
                tasks = [e['result'] for e in events if e.get('task') == case['observations'][0]['task'] and 'result' in e]
                require(len(tasks) == 1 and tasks[0].get('skipped') is True and tasks[0].get('changed') is False
                        and recaps[0]['recap'].get('changed') == 0, f'{phase}: skipped command or changed count incorrect')
                if recaps[0]['recap']['failed']:
                    assertions = [e['result'] for e in events if e.get('task') == 'Assert delegated result shape' and 'result' in e]
                    require(len(assertions) == 1 and assertions[0].get('failed') is True, f'{phase}: expected failed assertion missing')
            else:
                expected_hosts = set(case['observations'][0]['hosts'])
                task_events = [e for e in events if e.get('task') == case['observations'][0]['task'] and 'result' in e]
                require(len(task_events) == len(expected_hosts) and {e['host'] for e in task_events} == expected_hosts
                        and all(e['result'].get('skipped') for e in task_events),
                        f'{phase}: missing, duplicate or unskipped host commands')
        if 'drift_repair' in case['phases']:
            events, recaps = run('apply')
            check_phase('drift_repair', events, recaps)
        if case.get('pending') or (name == 'controller-delegation' and 'succeeds' not in case['phases']['diff']):
            raise Incomplete(f'{name}: declared pending or unspecified diff outcome remains unverified')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    parser.add_argument('--scenario', action='append', choices=sorted(SCENARIOS))
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    manifest = json.loads((WORKLOADS / 'expectations.json').read_text())
    require(set(manifest) == {'version', 'scenarios'} and manifest.get('version') == 1
            and set(manifest.get('scenarios', {})) == SCENARIOS,
            'expectations manifest version or scenario inventory mismatch')
    selected = args.scenario or sorted(SCENARIOS)
    failures = []
    for name in selected:
        try:
            execute(name, manifest['scenarios'][name], cli, agent)
            print(f'PASS {name}: declared expectations verified')
        except Incomplete as error:
            failures.append(name)
            print(f'INCOMPLETE {name}: {error}')
        except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.TimeoutExpired) as error:
            failures.append(name)
            print(f'FAIL {name}: {error}')
    print(f'{len(selected) - len(failures)}/{len(selected)} selected scenarios passed; unsupported scenarios fail closed')
    return bool(failures)


if __name__ == '__main__':
    raise SystemExit(main())
