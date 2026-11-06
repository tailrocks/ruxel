#!/usr/bin/env python3
"""Exercise binary controller source copies through the real local agent."""
import argparse
import json
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
FIRST = bytes([0, 255, 128, 65, 10, 0, 254, 66])
SECOND = bytes([255, 0, 1, 128, 10, 67, 253])


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-binary-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        source = base / 'source.bin'
        source.write_bytes(FIRST)
        dest = payload / 'copy.bin'
        module = base / 'fixtures/copy.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "binary copy"; targets { "all" }; tasks {
  new { name = "copy"; operation = new Ruxel.Copy { src = SOURCE; dest = DEST; mode = "0640" } }
 } } }
} }
'''.replace('SOURCE', json.dumps(str(source))).replace('DEST', json.dumps(str(dest))))
        def snapshot():
            return {p.name: (stat.S_IMODE(p.stat().st_mode), p.read_bytes()) for p in payload.iterdir() if p.is_file()}
        def run(mode):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=20)
            require(result.returncode == 0, f'{mode}: exit {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            tasks = [e['result'] for e in events if e.get('task') == 'copy' and 'result' in e]
            recaps = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(tasks) == len(recaps) == 1 and recaps[0]['failed'] == 0, f'{mode}: invalid result')
            return tasks[0]
        first = run('apply')
        require(first.get('changed') is True and dest.read_bytes() == FIRST
                and stat.S_IMODE(dest.stat().st_mode) == 0o640, 'fresh binary copy incorrect')
        stable = run('apply')
        require(stable.get('changed') is False and dest.read_bytes() == FIRST, 'binary copy did not converge')
        dest.write_bytes(b'drift')
        before = snapshot()
        for mode in ('check', 'diff'):
            result = run(mode)
            require(result.get('changed') is True and snapshot() == before, f'{mode}: prediction or non-mutation failed')
        repaired = run('apply')
        require(repaired.get('changed') is True and dest.read_bytes() == FIRST, 'binary drift not repaired')
        source.write_bytes(SECOND)
        before = snapshot()
        for mode in ('check', 'diff'):
            result = run(mode)
            require(result.get('changed') is True and snapshot() == before, f'{mode}: source change prediction failed')
        updated = run('apply')
        require(updated.get('changed') is True and dest.read_bytes() == SECOND, 'binary source update ignored')
        source.write_bytes(b'')
        before = snapshot()
        for mode in ('check', 'diff'):
            result = run(mode)
            require(result.get('changed') is True and snapshot() == before, f'{mode}: empty source prediction failed')
        emptied = run('apply')
        require(emptied.get('changed') is True and dest.read_bytes() == b'', 'empty source ignored')
        source.write_bytes(FIRST)
        module.write_text(module.read_text().replace('name = "copy";', 'name = "copy"; noLog = true;'))
        private = run('diff')
        require(private.get('censored') is True and not private.get('diff') and dest.read_bytes() == b'',
                'noLog binary prediction leaked or mutated destination')
        private = run('apply')
        require(private.get('censored') is True and dest.read_bytes() == FIRST, 'noLog binary copy failed')
    print('PASS binary copy: fresh, convergence, drift/source repair, check/diff non-mutation')


if __name__ == '__main__':
    main()
