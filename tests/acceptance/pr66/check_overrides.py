#!/usr/bin/env python3
"""Check explicit task checkMode overrides through real local agents."""
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-check-override-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        for mode in ('check', 'diff'):
            payload = base / mode
            payload.mkdir()
            tasks = []
            for name, option in [('default', ''), ('forced_check', 'checkMode = true;'),
                                 ('forced_apply', 'checkMode = false;')]:
                argv = '; '.join(map(json.dumps, ['/usr/bin/touch', str(payload / name)]))
                tasks.append(f'new {{ name = {json.dumps(name)}; {option} operation = new Ruxel.Command {{ argv = new Listing {{ {argv} }} }} }}')
            module = base / 'fixtures' / f'{mode}.pkl'
            module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "override"; targets { "all" }; tasks { TASKS } } }
} }
'''.replace('TASKS', '\n'.join(tasks)))
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / f'{mode}-state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=15)
            require(result.returncode == 0, f'{mode} exited {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            results = {e['task']: e['result'] for e in events if e.get('task') in ('default', 'forced_check', 'forced_apply') and 'result' in e}
            require(set(results) == {'default', 'forced_check', 'forced_apply'}, f'{mode}: missing task results')
            for name in ('default', 'forced_check'):
                require(results[name].get('skipped') is True and not (payload / name).exists(),
                        f'{mode}: {name} did not skip')
            require(not results['forced_apply'].get('skipped') and results['forced_apply'].get('rc') == 0
                    and (payload / 'forced_apply').is_file(), f'{mode}: explicit false did not execute')
    print('PASS check/diff task overrides: default and true skip; explicit false executes')


if __name__ == '__main__':
    main()
