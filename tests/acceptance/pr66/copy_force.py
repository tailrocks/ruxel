#!/usr/bin/env python3
"""Exercise Copy force=false content preservation and metadata drift locally."""
import argparse
import json
import shutil
import stat
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-copy-force-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        source = base / 'source'
        source.write_bytes(b'original\x00\xff')
        dest = payload / 'destination'
        module = base / 'fixtures/copy.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "copy force false"; targets { "all" }; tasks {
  new { name = "copy"; operation = new Ruxel.Copy { src = SOURCE; dest = DEST; force = false; mode = "0640" } }
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
        def predict_unchanged(expected_change):
            before = snapshot()
            for mode in ('check', 'diff'):
                result = run(mode)
                require(result.get('changed') is expected_change and snapshot() == before,
                        f'{mode}: incorrect prediction or payload mutation')
        fresh = run('apply')
        require(fresh.get('changed') is True and dest.read_bytes() == b'original\x00\xff'
                and stat.S_IMODE(dest.stat().st_mode) == 0o640, 'fresh creation incorrect')
        require(run('apply').get('changed') is False, 'initial rerun did not converge')
        dest.write_bytes(b'local-content')
        dest.chmod(0o600)
        predict_unchanged(True)
        metadata = run('apply')
        require(metadata.get('changed') is True and dest.read_bytes() == b'local-content'
                and stat.S_IMODE(dest.stat().st_mode) == 0o640, 'metadata repair overwrote content or failed')
        require(run('apply').get('changed') is False, 'metadata rerun did not converge')
        source.write_bytes(b'new-source\x80')
        predict_unchanged(False)
        require(run('apply').get('changed') is False and dest.read_bytes() == b'local-content',
                'force=false overwrote an existing destination')
        dest.unlink()
        predict_unchanged(True)
        recreated = run('apply')
        require(recreated.get('changed') is True and dest.read_bytes() == b'new-source\x80'
                and stat.S_IMODE(dest.stat().st_mode) == 0o640, 'missing destination was not recreated')
    print('PASS copy force=false: create, metadata repair, preserve content, recreate, check/diff')


if __name__ == '__main__':
    main()
