#!/usr/bin/env python3
"""Exercise the real local agent's file cache, invalidation, bypass and noLog."""
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-cache-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixture').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        source, dest = base / 'source', base / 'destination'
        source.write_text('first-version')
        module = base / 'fixture/workload.pkl'
        template = '''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
  inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
  plays { new { name = "cache"; targets { "all" }; tasks { new {
    name = "copy"; NOLOG operation = new Ruxel.Copy { src = SOURCE; dest = DEST }
  } } } }
} }
'''
        def write_module(no_log=False):
            module.write_text(template.replace('NOLOG', 'noLog = true;' if no_log else '')
                              .replace('SOURCE', json.dumps(str(source))).replace('DEST', json.dumps(str(dest))))
        def run(state, *options):
            result = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(state), *options], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=15)
            require(result.returncode == 0, f'apply failed: {result.stderr}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            tasks = [e['result'] for e in events if e.get('task') == 'copy']
            require(len(tasks) == 1, 'missing copy task result')
            return tasks[0], result.stdout + result.stderr
        state = base / 'state'
        write_module()
        first, _ = run(state)
        require(first.get('changed') is True and dest.read_text() == 'first-version', 'fresh copy failed')
        files = list(state.rglob('ledger.json'))
        require(len(files) == 1, 'missing unique agent ledger')
        ledger = files[0]
        initial = ledger.read_bytes()
        initial_stat = ledger.stat()
        second, _ = run(state)
        require(second.get('changed') is False and dest.read_text() == 'first-version', 'rerun failed')
        require(ledger.read_bytes() == initial and ledger.stat().st_ino == initial_stat.st_ino,
                'rerun did not preserve the cached ledger entry')
        bypass, _ = run(state, '--no-cache')
        require(bypass.get('changed') is False and ledger.read_bytes() != initial,
                '--no-cache did not execute and refresh the entry')
        dest.write_text('drift')
        repaired, _ = run(state)
        require(repaired.get('changed') is True and dest.read_text() == 'first-version', 'drift not repaired')
        source.write_text('second-version')
        changed, _ = run(state)
        require(changed.get('changed') is True and dest.read_text() == 'second-version', 'source change ignored')
        secret = 'synthetic-sensitive-cache-marker-739264'
        source.write_text(secret)
        write_module(no_log=True)
        private_state = base / 'private-state'
        private, output = run(private_state)
        require(private.get('censored') is True and dest.read_text() == secret, 'noLog result or file incorrect')
        require(secret not in output, 'noLog output leaked synthetic value')
        require(not list(private_state.rglob('ledger.json')), 'noLog operation was cached')
        for path in private_state.rglob('*'):
            if path.is_file():
                require(secret.encode() not in path.read_bytes(), 'noLog state leaked synthetic value')
    print('PASS local cache: hit, bypass, drift repair, source change and noLog')


if __name__ == '__main__':
    main()
