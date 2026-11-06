#!/usr/bin/env python3
"""Exercise variable/environment layering and successful handlers locally."""
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXPECTED = 'G|H|P|B|T|task|fact'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-scope-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        target = payload / 'scopes'
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        module = base / 'fixtures/scope.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
function lit(v: Ruxel.Scalar): Ruxel.Literal = new { value = v }
function ref(n: String): Ruxel.Ref = new { path { "vars"; n } }
output { value = new Ruxel.Config {
 inventory {
  hosts { ["fixture"] { address = "127.0.0.1"; vars { ["host_only"] = lit("H"); ["shared"] = lit("host") } } }
  groups { ["group"] { hosts { "fixture" }; vars { ["group_only"] = lit("G"); ["shared"] = lit("group") } } }
 }
 plays { new { name = "scopes"; targets { "group" }; vars { ["play_only"] = lit("P"); ["shared"] = lit("play") }
  environment { ["RUXEL_SCOPE_A"] = lit("play-a"); ["RUXEL_SCOPE_SHARED"] = lit("play-shared") }
  preTasks { new { name = "prefact"; operation = new Ruxel.SetFact { values { ["fact"] = lit("fact") } } } }
  tasks { new { name = "block"; vars { ["block_only"] = lit("B"); ["shared"] = lit("block") }
   environment { ["RUXEL_SCOPE_B"] = lit("block-b"); ["RUXEL_SCOPE_SHARED"] = lit("block-shared") }
   block {
    new { name = "environment"; checkMode = false; changedWhen = lit(false)
     environment { ["RUXEL_SCOPE_C"] = lit("task-c"); ["RUXEL_SCOPE_SHARED"] = lit("task-shared") }
     operation = new Ruxel.Command { argv = new Listing { "/bin/sh"; "-c"; ENV_COMMAND } }
    }
    new { name = "scoped copy"; vars { ["task_only"] = lit("T"); ["shared"] = lit("task") }; notify { "handler" }
     operation = new Ruxel.Copy { dest = TARGET; content = new Ruxel.Call { `function` = "concat"; args { ref("group_only"); lit("|"); ref("host_only"); lit("|"); ref("play_only"); lit("|"); ref("block_only"); lit("|"); ref("task_only"); lit("|"); ref("shared"); lit("|"); ref("fact") } } }
    }
   }
  } }
  handlers { new { name = "handler"; operation = new Ruxel.Debug { msg = "handled" } } }
 } }
} }
'''.replace('TARGET', json.dumps(str(target))).replace('ENV_COMMAND', json.dumps("printf '%s|%s|%s|%s' \"$RUXEL_SCOPE_A\" \"$RUXEL_SCOPE_B\" \"$RUXEL_SCOPE_C\" \"$RUXEL_SCOPE_SHARED\"")))
        def run(mode):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=20)
            require(result.returncode == 0, f'{mode}: exit {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recap = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(recap) == 1 and recap[0]['failed'] == 0, f'{mode}: failed recap')
            for name in ('prefact', 'environment', 'scoped copy'):
                require(sum(e.get('task') == name for e in events) == 1, f'{mode}: missing task {name}')
            env = next(e['result'] for e in events if e.get('task') == 'environment')
            require(env.get('stdout') == 'play-a|block-b|task-c|task-shared' and not env.get('failed'),
                    f'{mode}: incorrect inherited environment: {env}')
            return events, recap[0]
        def handler(events, expected):
            indices = [i for i, event in enumerate(events) if event.get('task') == 'handler']
            require(len(indices) == int(expected), 'handler notification count incorrect')
            if expected:
                result = events[indices[0]]['result']
                require(result.get('msg') == 'handled' and not result.get('failed') and not result.get('skipped'), 'handler failed')
                require(indices[0] > next(i for i, event in enumerate(events) if event.get('task') == 'scoped copy'), 'handler ran before task')
        events, recap = run('apply')
        require(target.read_text() == EXPECTED and recap['changed'] > 0, 'fresh scoped result incorrect')
        handler(events, True)
        events, recap = run('apply')
        require(target.read_text() == EXPECTED and recap['changed'] == 0, 'converged scoped result incorrect')
        handler(events, False)
        target.write_text('drift')
        def snapshot():
            return {p.name: (p.stat().st_mode, p.read_bytes()) for p in payload.iterdir() if p.is_file()}
        for mode in ('check', 'diff'):
            before = snapshot()
            events, recap = run(mode)
            require(snapshot() == before and recap['changed'] > 0, f'{mode}: prediction or non-mutation failed')
            handler(events, True)
        events, recap = run('apply')
        require(target.read_text() == EXPECTED and recap['changed'] > 0, 'drift repair incorrect')
        handler(events, True)
    print('PASS scoped variables/environment and successful handlers across apply/check/diff')


if __name__ == '__main__':
    main()
