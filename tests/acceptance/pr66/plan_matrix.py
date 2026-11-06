#!/usr/bin/env python3
"""Plan all required scenarios locally; this is not apply or provider evidence."""
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORKLOADS = ROOT / 'testdata/workloads/pr66'
SCENARIOS = (
    'become-environment', 'check-semantics', 'control-flow', 'controller-delegation',
    'files-content', 'multihost', 'performance-snapshots', 'postgresql-ownership',
    'restart-actions', 'shape-variants', 'storage-ext4', 'storage-two-tier',
    'storage-xfs', 'system-surface',
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    failures = []
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-plans-') as temp:
        base = Path(temp)
        fixtures = base / 'testdata/workloads/pr66'
        fixtures.parent.mkdir(parents=True)
        shutil.copytree(WORKLOADS, fixtures)
        (base / 'schema').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        payload = base / 'payload'
        payload.mkdir()
        for name in SCENARIOS:
            module = fixtures / f'{name}.pkl'
            try:
                source = module.read_text().replace('__RUN_ROOT__', str(payload)).replace('__FIXTURE_ROOT__', str(fixtures))
                module.write_text(source)
                result = subprocess.run([str(cli), 'plan', str(module), '--local', '--agent', str(agent),
                                         '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                        capture_output=True, text=True, timeout=30)
                if result.returncode:
                    raise RuntimeError(f'exit {result.returncode}: {result.stderr.strip()}')
                events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
                if not any(event.get('planned') is True for event in events):
                    raise RuntimeError('no planned task events')
                recaps = [event['recap'] for event in events if 'recap' in event]
                if not recaps or any(recap.get('failed') or recap.get('changed') for recap in recaps):
                    raise RuntimeError('missing or failed/changed recap')
                if any(payload.iterdir()):
                    raise RuntimeError('plan wrote to the fixture payload directory')
                print(f'PASS plan {name}')
            except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
                failures.append(name)
                print(f'FAIL plan {name}: {error}')
    print(f'{len(SCENARIOS) - len(failures)}/{len(SCENARIOS)} plans passed; execution and provider behavior not tested')
    return bool(failures)


if __name__ == '__main__':
    raise SystemExit(main())
