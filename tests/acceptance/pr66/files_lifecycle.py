#!/usr/bin/env python3
"""Exercise file providers through the real local agent in a temporary tree."""
import argparse
import base64
import json
import os
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def snapshot(root):
    result = {}
    for path in root.rglob('*'):
        key = str(path.relative_to(root))
        info = path.lstat()
        if path.is_symlink():
            result[key] = ('link', os.readlink(path))
        elif path.is_dir():
            result[key] = ('directory', stat.S_IMODE(info.st_mode))
        elif path.is_file():
            result[key] = ('file', stat.S_IMODE(info.st_mode), path.read_bytes())
        else:
            raise RuntimeError(f'unexpected fixture entry {key}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-files-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        (payload / 'line').write_text('setting=old\n')
        (payload / 'replace').write_text('fruit=apple\n')
        (payload / 'block').write_text('header\n')
        (payload / 'absent').write_text('remove-me\n')
        def quoted(name):
            return json.dumps(str(payload / name))
        source = '''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "files"; targets { "all" }; vars { ["value"] = new Ruxel.Literal { value = "rendered" } }; tasks {
  new { name = "remove"; operation = new Ruxel.File { path = ABSENT; state = "absent" } }
  new { name = "directory"; operation = new Ruxel.File { path = DIR; state = "directory"; mode = "0700" } }
  new { name = "content"; operation = new Ruxel.Copy { dest = CONTENT; content = "synthetic-content\\n"; mode = "0640" } }
  new { name = "template"; operation = new Ruxel.Template { dest = TEMPLATE; content = new Ruxel.Call { `function` = "concat"; args { new Ruxel.Literal { value = "prefix-" }; new Ruxel.Ref { path { "vars"; "value" } } } } } }
  new { name = "link"; operation = new Ruxel.File { path = LINK; src = CONTENT; state = "link" } }
  new { name = "line"; operation = new Ruxel.LineInFile { path = LINE; regexp = "^setting="; line = "setting=new"; state = "present" } }
  new { name = "replace"; operation = new Ruxel.Replace { path = REPLACE; regexp = "apple"; replace = "pear" } }
  new { name = "block"; operation = new Ruxel.BlockInFile { path = BLOCK; block = "alpha\\nbeta" } }
  new { name = "stat"; operation = new Ruxel.Stat { path = CONTENT } }
  new { name = "slurp"; operation = new Ruxel.Slurp { src = CONTENT } }
 } } }
} }
'''
        for name in ('dir', 'content', 'template', 'link', 'line', 'replace', 'block', 'absent'):
            source = source.replace(name.upper(), quoted(name))
        module = base / 'fixtures/files.pkl'
        module.write_text(source)
        def run(mode):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=25)
            require(result.returncode == 0, f'{mode} exited {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recap = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(recap) == 1 and recap[0]['failed'] == 0, f'{mode}: missing or failed recap')
            return events, recap[0]
        def observed(events):
            require(not (payload / 'absent').exists(), 'absent path still exists')
            require((payload / 'dir').is_dir() and stat.S_IMODE((payload / 'dir').stat().st_mode) == 0o700, 'directory incorrect')
            require((payload / 'content').read_bytes() == b'synthetic-content\n', 'copy content incorrect')
            require(stat.S_IMODE((payload / 'content').stat().st_mode) == 0o640, 'copy mode incorrect')
            require((payload / 'template').read_text() == 'prefix-rendered', 'template output incorrect')
            require((payload / 'link').is_symlink() and (payload / 'link').resolve() == (payload / 'content').resolve(), 'link incorrect')
            require((payload / 'line').read_text() == 'setting=new\n', 'line edit incorrect')
            require((payload / 'replace').read_text() == 'fruit=pear\n', 'replace incorrect')
            require((payload / 'block').read_text() == 'header\n# BEGIN ANSIBLE MANAGED BLOCK\nalpha\nbeta\n# END ANSIBLE MANAGED BLOCK\n', 'block content incorrect')
            found = {e['task']: e['result'] for e in events if e.get('task') in ('stat', 'slurp') and 'result' in e}
            require(found.get('stat', {}).get('stat', {}).get('exists') is True, 'stat result incorrect')
            require(base64.b64decode(found.get('slurp', {}).get('content', ''), validate=True) == b'synthetic-content\n', 'slurp result incorrect')
        fresh, recap = run('apply')
        require(recap['changed'] > 0, 'fresh apply reported no change')
        observed(fresh)
        stable, recap = run('apply')
        require(recap['changed'] == 0, 'rerun did not converge')
        observed(stable)
        (payload / 'content').write_text('drift\n')
        (payload / 'template').write_text('drift')
        (payload / 'absent').write_text('remove-again\n')
        (payload / 'line').write_text('setting=old\n')
        (payload / 'replace').write_text('fruit=apple\n')
        (payload / 'block').write_text('header\n')
        for mode in ('check', 'diff'):
            before = snapshot(payload)
            events, recap = run(mode)
            require(recap['changed'] > 0 and snapshot(payload) == before, f'{mode}: prediction or non-mutation failed')
            if mode == 'diff':
                require(any(e.get('result', {}).get('diff') for e in events), 'diff omitted differences')
        repaired, recap = run('apply')
        require(recap['changed'] > 0, 'repair reported no change')
        observed(repaired)
    print('PASS file lifecycle: fresh, convergence, check/diff non-mutation and drift repair')


if __name__ == '__main__':
    main()
