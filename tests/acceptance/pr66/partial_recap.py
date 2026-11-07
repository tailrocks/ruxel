#!/usr/bin/env python3
"""Verify partial host recaps and noLog redaction across an execution error."""
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SENTINEL = 'synthetic-sensitive-missing-reference-835792'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-partial-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        for private in (False, True):
            payload = base / ('payload-private' if private else 'payload-ordinary')
            payload.mkdir()
            module = base / 'fixtures/partial.pkl'
            module.write_text('''import "../schema/Ruxel.pkl"
    output { value = new Ruxel.Config {
     inventory { hosts { ["fixture-a"] { address = "127.0.0.1" }; ["fixture-b"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture-a"; "fixture-b" } } } }
     plays {
      new { name = "first"; targets { "all" }; tasks {
       new { name = "write"; operation = new Ruxel.Copy { dest = new Ruxel.Call { `function` = "concat"; args { new Ruxel.Literal { value = PREFIX }; new Ruxel.Ref { path { "inventoryHost" } } } }; content = "synthetic" } }
       new { name = "private failure"; NOLOG
        `when` = new Ruxel.Call { `function` = "eq"; args { new Ruxel.Ref { path { "inventoryHost" } }; new Ruxel.Literal { value = "fixture-a" } } }
        operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "vars"; SECRET } } }
       }
      } }
      new { name = "second"; targets { "all" }; tasks { new { name = "sibling continues"; operation = new Ruxel.Debug { msg = "continued" } } } }
     }
    } }
    '''.replace('PREFIX', json.dumps(str(payload) + '/')).replace('SECRET', json.dumps(SENTINEL if private else 'ordinary_missing')).replace('NOLOG', 'noLog = true' if private else 'noLog = false'))
            log = base / ('private-events.jsonl' if private else 'ordinary-events.jsonl')
            result = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / ('private-state' if private else 'ordinary-state')), '--jobs', '1', '--log-file', str(log)],
                                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=25)
            require(log.is_file() and log.stat().st_size > 0, 'missing private event log')
            logged = log.read_text()
            require(SENTINEL not in result.stdout + result.stderr + logged, 'noLog failure leaked synthetic value')
            require(result.returncode == 8, f'expected failed exit 8, got {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            for name in ('fixture-a', 'fixture-b'):
                require((payload / name).read_text() == 'synthetic', f'{name}: initial copy did not succeed')
            writes = [e for e in events if e.get('task') == 'write' and 'result' in e]
            require(len(writes) == 2 and {e['host'] for e in writes} == {'fixture-a', 'fixture-b'}
                    and all(e['result'].get('changed') is True for e in writes), 'missing successful initial writes')
            recaps = [e for e in events if 'recap' in e]
            require(len(recaps) == 2 and {e['host'] for e in recaps} == {'fixture-a', 'fixture-b'}, 'missing host recaps')
            bad = next(e['recap'] for e in recaps if e['host'] == 'fixture-a')
            good = next(e['recap'] for e in recaps if e['host'] == 'fixture-b')
            require(bad['ok'] >= 1 and bad['changed'] >= 1 and bad['failed'] >= 1, 'partial failed-host recap lost progress')
            require(good['failed'] == 0 and good['changed'] >= 1, 'healthy-host recap incorrect')
            continued = [e for e in events if e.get('task') == 'sibling continues' and 'result' in e]
            require(len(continued) == 1 and continued[0]['host'] == 'fixture-b'
                    and continued[0]['result'].get('msg') == 'continued', 'healthy sibling did not continue')
    print('PASS partial recaps: progress retained, sibling continues, noLog error redacted')


if __name__ == '__main__':
    main()
