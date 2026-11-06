#!/usr/bin/env python3
"""Exercise safe local controller flow with the real CLI and agent."""
import argparse
import json
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SENTINEL = 'synthetic-flow-private-759241'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-flow-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        marker = base / 'retry-marker'
        quoted = shlex.quote(str(marker))
        retry = f'test -f {quoted} && exit 0; touch {quoted}; exit 1'
        source = '''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "flow"; targets { "all" }; tasks {
  new { name = "set fact"; operation = new Ruxel.SetFact { values { ["answer"] = new Ruxel.Literal { value = "local-fact" } } } }
  new { name = "read fact"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "vars"; "answer" } } } }
  new { name = "skip condition"; `when` = new Ruxel.Literal { value = false }; operation = new Ruxel.Fail { msg = "should not execute" } }
  new { name = "loop"; `loop` = new Ruxel.ListExpr { items { new Ruxel.Literal { value = "one" }; new Ruxel.Literal { value = "two" }; new Ruxel.Literal { value = "three" } } }; operation = new Ruxel.Command { argv = new Ruxel.ListExpr { items { new Ruxel.Literal { value = "/usr/bin/printf" }; new Ruxel.Ref { path { "item" } } } } } }
  new { name = "environment"; environment { ["RUXEL_FIXTURE_VALUE"] = new Ruxel.Literal { value = "injected-value" } }; operation = new Ruxel.Command { argv = new Listing { "/usr/bin/printenv"; "RUXEL_FIXTURE_VALUE" } }; changedWhen = new Ruxel.Literal { value = false } }
  new { name = "overrides"; operation = new Ruxel.Command { argv = new Listing { "/usr/bin/false" } }; changedWhen = new Ruxel.Literal { value = false }; failedWhen = new Ruxel.Literal { value = false } }
  new { name = "ignore"; ignoreErrors = true; operation = new Ruxel.Fail { msg = "expected ignored" } }
  new { name = "block"; block { new { name = "fail inside"; operation = new Ruxel.Fail { msg = "expected rescue" } } }; rescue { new { name = "rescue"; operation = new Ruxel.Debug { msg = "rescued" } } }; always { new { name = "always"; operation = new Ruxel.Debug { msg = "always ran" } } } }
  new { name = "retry"; checkMode = false; operation = new Ruxel.Shell { cmd = RETRY }; register = "retry_result"; retries = 3; delaySeconds = 1; until = new Ruxel.Call { `function` = "eq"; args { new Ruxel.Ref { path { "results"; "retry_result"; "rc" } }; new Ruxel.Literal { value = 0 } } }; changedWhen = new Ruxel.Literal { value = false } }
  new { name = "private"; noLog = true; operation = new Ruxel.Command { argv = new Listing { "/usr/bin/printf"; SECRET } } }
 } } }
} }
'''.replace('RETRY', json.dumps(retry)).replace('SECRET', json.dumps(SENTINEL))
        module = base / 'fixtures/flow.pkl'
        module.write_text(source)
        def run(mode):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=20)
            require(SENTINEL not in result.stdout + result.stderr, f'{mode}: private value leaked in output')
            require(result.returncode == 0, f'{mode} exited {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recaps = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(recaps) == 1 and recaps[0]['failed'] == 0, f'{mode}: failed recap')
            return events, recaps[0]
        def task(events, name):
            matches = [e['result'] for e in events if e.get('task') == name and 'result' in e]
            require(len(matches) == 1, f'{name}: missing/duplicate result')
            return matches[0]
        for mode in ('apply', 'apply', 'check', 'diff'):
            before = marker.read_bytes() if marker.exists() else None
            events, recap = run(mode)
            require(task(events, 'read fact').get('msg') == 'local-fact', f'{mode}: fact not propagated')
            require(task(events, 'skip condition').get('skipped') is True, f'{mode}: condition did not skip')
            require(task(events, 'rescue').get('msg') == 'rescued', f'{mode}: rescue did not run')
            require(task(events, 'always').get('msg') == 'always ran', f'{mode}: always did not run')
            require(recap['rescued'] >= 1 and recap['ignored'] >= 1, f'{mode}: rescue/ignore not reported')
            require(task(events, 'retry').get('rc') == 0 and marker.is_file(), f'{mode}: retry failed')
            require(task(events, 'private').get('censored') is True, f'{mode}: noLog result not censored')
            if mode in ('check', 'diff'):
                require(marker.read_bytes() == before, f'{mode}: fixture marker changed')
                results = task(events, 'loop').get('results', [])
                require(len(results) == 3 and all(item.get('skipped') is True for item in results),
                        f'{mode}: loop commands were not skipped')
            else:
                env = task(events, 'environment')
                require(env.get('stdout') == 'injected-value' and env.get('changed') is False,
                        f'{mode}: environment or changed override failed: {env}')
                overridden = task(events, 'overrides')
                require(overridden.get('rc') == 1 and overridden.get('failed') is False
                        and overridden.get('changed') is False, f'{mode}: result overrides failed')
                loop = task(events, 'loop')
                results = loop.get('results', [])
                require([item.get('stdout') for item in results] == ['one', 'two', 'three'], f'{mode}: loop results incorrect')
        for path in (base / 'state').rglob('*'):
            if path.is_file():
                require(SENTINEL.encode() not in path.read_bytes(), 'private value leaked in agent state')
    print('PASS local flow: facts, condition, loop, environment, overrides, rescue, retry, noLog, check/diff')


if __name__ == '__main__':
    main()
