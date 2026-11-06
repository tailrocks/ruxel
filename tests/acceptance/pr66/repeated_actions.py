#!/usr/bin/env python3
"""Verify repeated shell actions run on every apply and skip in check/diff."""
import argparse
import json
import shlex
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-actions-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        log = base / 'actions.log'
        helper = base / 'action'
        helper.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> ' + shlex.quote(str(log)) + '\n')
        helper.chmod(0o700)
        items = [f'{i:02}' for i in range(1, 37)]
        values = '; '.join(f'new Ruxel.Literal {{ value = {json.dumps(i)} }}' for i in items)
        command_prefix = json.dumps(shlex.quote(str(helper)) + ' ')
        module = base / 'fixtures/actions.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "repeated actions"; targets { "all" }; tasks { new {
  name = "action"; `loop` = new Ruxel.ListExpr { items { VALUES } }
  operation = new Ruxel.Shell { cmd = new Ruxel.Call { `function` = "concat"; args { new Ruxel.Literal { value = PREFIX }; new Ruxel.Ref { path { "item" } } } } }
 } } } }
} }
'''.replace('VALUES', values).replace('PREFIX', command_prefix))
        def run(mode):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=30)
            require(result.returncode == 0, f'{mode} failed: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recaps = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(recaps) == 1 and recaps[0]['failed'] == 0, f'{mode}: missing or failed recap')
        run('apply')
        require(log.read_text().splitlines() == items, 'fresh apply did not run 36 ordered actions')
        run('apply')
        require(log.read_text().splitlines() == items * 2, 'rerun did not repeat all 36 actions')
        for mode in ('check', 'diff'):
            before = log.read_bytes()
            run(mode)
            require(log.read_bytes() == before, f'{mode} executed an action')
    print('PASS repeated actions: 36 ordered actions on each apply, check/diff non-mutation')


if __name__ == '__main__':
    main()
