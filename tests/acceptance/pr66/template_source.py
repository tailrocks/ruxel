#!/usr/bin/env python3
"""Exercise the migrated native Pkl template source with local temp files."""
import argparse
import hashlib
import json
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ASSET = ROOT / 'testdata/workloads/pr66/assets/template.pkl'
ASSET_SHA256 = 'e848f3f641a2b66a80d18c9af88a4906d313545d206b43f1d1d3a9eeb5ee1eab'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    asset = ASSET.read_bytes()
    require(hashlib.sha256(asset).hexdigest() == ASSET_SHA256, 'template asset changed: review and update pin')
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-template-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        fixture = base / 'testdata/workloads/pr66'
        (fixture / 'assets').mkdir(parents=True)
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        source = fixture / 'assets/template.pkl'
        source.write_bytes(asset)
        dest = payload / 'rendered'
        module = fixture / 'workload.pkl'
        pkl = '''import "../../../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "native template"; targets { "all" }; vars {
  ["fixture_lines"] = new Ruxel.ListExpr { items { new Ruxel.Literal { value = "alpha" }; new Ruxel.Literal { value = "beta" } } }
  ["fixture_optional"] = new Ruxel.Literal { value = "" }
  ["fixture_enabled"] = new Ruxel.Literal { value = ENABLED }
 }; tasks { new { name = "template"; operation = new Ruxel.Template { src = SOURCE; dest = DEST; mode = "0640" } } } } }
} }
'''
        def write_module(enabled):
            module.write_text(pkl.replace('ENABLED', 'true' if enabled else 'false')
                              .replace('SOURCE', json.dumps(str(source))).replace('DEST', json.dumps(str(dest))))
        digest = hashlib.sha256(b'synthetic').hexdigest()
        prefix = f'alpha\nbeta\njoined=alpha,beta\nhash={digest}\noptional=fallback\nencoded=synthetic%20value\n'
        def expected(enabled=True, changed=False):
            return prefix + (('conditional=changed\n' if changed else 'conditional=enabled\n') if enabled else '')
        def run(mode):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=20)
            require(result.returncode == 0, f'{mode}: exit {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            tasks = [e['result'] for e in events if e.get('task') == 'template' and 'result' in e]
            recaps = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(tasks) == len(recaps) == 1 and recaps[0]['failed'] == 0, f'{mode}: invalid result')
            return tasks[0]
        def snapshot():
            return (dest.read_bytes(), stat.S_IMODE(dest.stat().st_mode)) if dest.exists() else None
        def predict():
            before = snapshot()
            for mode in ('check', 'diff'):
                event = run(mode)
                require(event.get('changed') is True and snapshot() == before, f'{mode}: prediction mutated or missed drift')
                if mode == 'diff':
                    require(bool(event.get('diff')), 'diff omitted template difference')
        write_module(True)
        fresh = run('apply')
        require(fresh.get('changed') is True and dest.read_text() == expected() and stat.S_IMODE(dest.stat().st_mode) == 0o640,
                'fresh template render incorrect')
        stable = run('apply')
        require(stable.get('changed') is False and dest.read_text() == expected(), 'template did not converge')
        dest.write_text('drift')
        predict()
        repaired = run('apply')
        require(repaired.get('changed') is True and dest.read_text() == expected(), 'template drift not repaired')
        original = source.read_text()
        require(original.count('conditional=enabled') == 1, 'unexpected source shape')
        source.write_text(original.replace('conditional=enabled', 'conditional=changed'))
        predict()
        changed = run('apply')
        require(changed.get('changed') is True and dest.read_text() == expected(changed=True), 'source change ignored')
        write_module(False)
        predict()
        scoped = run('apply')
        require(scoped.get('changed') is True and dest.read_text() == expected(enabled=False), 'scope change ignored')
    print('PASS native template source: render, convergence, drift/source/scope changes, check/diff')


if __name__ == '__main__':
    main()
