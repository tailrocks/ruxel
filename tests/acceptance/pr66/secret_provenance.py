#!/usr/bin/env python3
"""Verify synthetic secret provenance in local stdout and private JSONL events."""
import argparse
import base64
import json
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
FAKE_VALUE = 'synthetic-provenance-marker-918347'
NO_LOG_VALUE = 'synthetic-no-log-marker-721934'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-secret-provenance-') as temp:
        base = Path(temp)
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'Ruxel.pkl')
        module = base / 'test.pkl'
        executed = base / 'command-executed'
        module.write_text('''import "Ruxel.pkl"
function secret(): Ruxel.Lookup = new { provider = "synthetic"; key = FAKE_VALUE }
function ref(a: String, b: String): Ruxel.Ref = new { path { a; b } }
function encode(v: Ruxel.Expr): Ruxel.Call = new { `function` = "base64Encode"; args { v } }
function lit(v: String): Ruxel.Literal = new { value = v }
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "synthetic"; targets { "fixture" }; tasks {
  new { name = "lookup"; operation = new Ruxel.Debug { msg = secret() } }
  new { name = "encoded"; operation = new Ruxel.Debug { msg = encode(secret()) } }
  new { name = "fact"; operation = new Ruxel.SetFact { values { ["private"] = secret() } } }
  new { name = "fact-reference"; operation = new Ruxel.Debug { msg = encode(ref("vars", "private")) } }
  new { name = "no-log-fact"; noLog = true; operation = new Ruxel.SetFact { values { ["private_no_log"] = lit(NO_LOG_VALUE) } } }
  new { name = "no-log-reference"; operation = new Ruxel.Debug { msg = encode(ref("vars", "private_no_log")) } }
  new { name = "command"; register = "cmd"; operation = new Ruxel.Shell { cmd = new Ruxel.Call { `function` = "concat"; args { lit("printf %s "); secret(); lit(COMMAND_SUFFIX) } } } }
  new { name = "registered"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "cmd"; "stdout" } } } }
  new { name = "public"; operation = new Ruxel.Debug { msg = "public diagnostic" } }
 } } }
} }
'''.replace('NO_LOG_VALUE', json.dumps(NO_LOG_VALUE)).replace('FAKE_VALUE', json.dumps(FAKE_VALUE)).replace('COMMAND_SUFFIX', json.dumps(' | base64; touch ' + shlex.quote(str(executed)))))
        event_log = base / 'events.jsonl'
        result = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(agent),
                                 '--state-dir', str(base / 'state'), '--synthetic-secrets',
                                 '--log-file', str(event_log)], stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=30)
        if result.returncode != 0:
            raise RuntimeError(f'run failed: exit {result.returncode}, stderr={result.stderr}')
        if not executed.is_file():
            raise RuntimeError('sensitive command did not execute')
        encoded = base64.b64encode(FAKE_VALUE.encode()).decode()
        encoded_no_log = base64.b64encode(NO_LOG_VALUE.encode()).decode()
        for label, text in [('stdout', result.stdout), ('log', event_log.read_text()), ('stderr', result.stderr)]:
            if FAKE_VALUE in text or encoded in text or NO_LOG_VALUE in text or encoded_no_log in text:
                raise RuntimeError(f'{label} contains a synthetic secret or its encoding')
        for label, text in [('stdout', result.stdout), ('log', event_log.read_text())]:
            events = [json.loads(line) for line in text.splitlines() if line]
            for name in ('lookup', 'encoded', 'fact', 'fact-reference', 'no-log-fact', 'no-log-reference', 'command', 'registered'):
                matches = [event for event in events if event.get('task') == name]
                if len(matches) != 1 or not matches[0]['result'].get('censored'):
                    raise RuntimeError(f'{label}: {name} was not censored')
            public = [event for event in events if event.get('task') == 'public']
            if len(public) != 1 or public[0]['result'].get('msg') != 'public diagnostic':
                raise RuntimeError(f'{label}: public diagnostic was suppressed')
        for path in (base / 'state').rglob('*'):
            if path.is_file():
                data = path.read_bytes()
                if FAKE_VALUE.encode() in data or encoded.encode() in data or NO_LOG_VALUE.encode() in data or encoded_no_log.encode() in data:
                    raise RuntimeError('synthetic secret was persisted in local agent state')
        print('PASS: synthetic lookup, transforms, facts and command results are redacted in stdout and JSONL; public diagnostic remains visible')


if __name__ == '__main__':
    main()
