#!/usr/bin/env python3
"""Exercise tag selection and failing notified handlers on temporary files."""
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-handler-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        module = base / 'fixtures/handlers.pkl'
        source = '''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "handlers"; targets { "all" }; tasks {
  new { name = "first"; tags { "config" }; notify { "failed handler" }; operation = new Ruxel.Copy { dest = FIRST; content = "one" } }
  new { name = "second"; tags { "config" }; notify { "failed handler" }; operation = new Ruxel.Copy { dest = SECOND; content = "two" } }
  new { name = "other"; tags { "other" }; operation = new Ruxel.Command { argv = new Listing { "/usr/bin/touch"; OTHER } } }
  new { name = "always"; tags { "always" }; operation = new Ruxel.Debug { msg = "always ran" } }
 }; handlers { new { name = "failed handler"; operation = new Ruxel.Fail { msg = "expected handler failure" } } } } }
} }
'''
        for key, name in [('FIRST', 'first'), ('SECOND', 'second'), ('OTHER', 'other')]:
            source = source.replace(key, json.dumps(str(payload / name)))
        module.write_text(source)
        def snapshot():
            return {p.name: p.read_bytes() for p in payload.iterdir() if p.is_file()}
        def run(mode, tags, code):
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state'), '--tags', tags], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=20)
            require(result.returncode == code, f'{mode} {tags}: exit {result.returncode}, expected {code}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recap = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(recap) == 1, f'{mode}: missing recap')
            return events, recap[0]
        def verify(events, recap, failed):
            handlers = [e['result'] for e in events if e.get('task') == 'failed handler' and 'result' in e]
            if failed:
                require(len(handlers) == 1 and handlers[0].get('failed') is True and recap['failed'] == 1,
                        'handler was not deduplicated or its failure was not reported')
            else:
                require(not handlers and recap['failed'] == 0, 'unexpected handler execution or failure')
            always = [e['result'] for e in events if e.get('task') == 'always' and 'result' in e]
            require(len(always) == 1 and always[0].get('msg') == 'always ran', 'always-tagged task missing')
            require(not (payload / 'other').exists(), 'excluded tagged task executed')
        fresh, recap = run('apply', 'config', 8)
        verify(fresh, recap, True)
        require(snapshot() == {'first': b'one', 'second': b'two'}, 'fresh file state incorrect')
        stable, recap = run('apply', 'config', 0)
        verify(stable, recap, False)
        require(recap['changed'] == 0, 'converged rerun changed')
        (payload / 'first').write_text('drift')
        (payload / 'second').write_text('drift')
        for mode in ('check', 'diff'):
            before = snapshot()
            events, recap = run(mode, 'config', 8)
            verify(events, recap, True)
            require(snapshot() == before, f'{mode} modified files')
        repaired, recap = run('apply', 'config', 8)
        verify(repaired, recap, True)
        require(snapshot() == {'first': b'one', 'second': b'two'}, 'drift repair incorrect')
        events, recap = run('apply', 'other', 0)
        require((payload / 'other').is_file() and recap['failed'] == 0, 'other tag failed')
        require(not any(e.get('task') in ('first', 'second', 'failed handler') for e in events),
                'excluded config or handler ran')
        require(any(e.get('task') == 'always' and e.get('result', {}).get('msg') == 'always ran' for e in events),
                'always tag missing from other selection')
        before = snapshot()
        invalid = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(agent),
                                  '--state-dir', str(base / 'state'), '--hosts', 'missing*'],
                                 stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20)
        require(invalid.returncode != 0 and not invalid.stdout.strip() and snapshot() == before,
                'unmatched host limit executed tasks or altered fixture files')
    print('PASS handler failure, deduplicated notifications, tag selection and check/diff non-mutation')


if __name__ == '__main__':
    main()
