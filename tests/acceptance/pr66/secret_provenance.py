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
        secret_file = base / 'secret-payload'
        shell_file = base / 'shell-payload'
        public_file = base / 'public-payload'
        public_file.write_text('public file')
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
  new { name = "copy-sensitive"; operation = new Ruxel.Copy { dest = RESOURCE_PATH; content = encode(secret()) } }
  new { name = "read-sensitive"; register = "readback"; operation = new Ruxel.Slurp { src = RESOURCE_PATH } }
  new { name = "public-read"; operation = new Ruxel.Slurp { src = PUBLIC_PATH } }
  new { name = "shell-cat"; register = "cat_result"; operation = new Ruxel.Shell { cmd = CAT_COMMAND } }
  new { name = "shell-cat-result"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "cat_result"; "stdout" } } } }
  new { name = "readback"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "readback"; "content" } } } }
  new { name = "command"; register = "cmd"; operation = new Ruxel.Shell { cmd = new Ruxel.Call { `function` = "concat"; args { lit("printf %s "); secret(); lit(COMMAND_SUFFIX); secret(); lit(SHELL_SUFFIX) } } } }
  new { name = "shell-read"; register = "shell_readback"; operation = new Ruxel.Slurp { src = SHELL_PATH } }
  new { name = "shell-readback"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "shell_readback"; "content" } } } }
  new { name = "registered"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "cmd"; "stdout" } } } }
  new { name = "conditional"; `when` = new Ruxel.Call { `function` = "eq"; args { secret(); secret() } }; operation = new Ruxel.Debug { msg = "controlled diagnostic" } }
  new { name = "conditional-block"; `when` = new Ruxel.Call { `function` = "eq"; args { secret(); secret() } }; block { new { name = "conditional-child"; operation = new Ruxel.Debug { msg = "controlled child" } } } }
  new { name = "public"; operation = new Ruxel.Debug { msg = "public diagnostic" } }
 } } }
} }
'''.replace('NO_LOG_VALUE', json.dumps(NO_LOG_VALUE)).replace('FAKE_VALUE', json.dumps(FAKE_VALUE)).replace('COMMAND_SUFFIX', json.dumps(' | base64; touch ' + shlex.quote(str(executed)) + '; printf %s ')).replace('RESOURCE_PATH', json.dumps(str(secret_file))).replace('SHELL_PATH', json.dumps(str(shell_file))).replace('PUBLIC_PATH', json.dumps(str(public_file))).replace('CAT_COMMAND', json.dumps('cat ' + shlex.quote(str(secret_file)))).replace('SHELL_SUFFIX', json.dumps(' > ' + shlex.quote(str(shell_file)))))
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
        encoded_twice = base64.b64encode(encoded.encode()).decode()
        encoded_no_log = base64.b64encode(NO_LOG_VALUE.encode()).decode()
        if secret_file.read_bytes() != encoded.encode() or shell_file.read_bytes() != FAKE_VALUE.encode():
            raise RuntimeError('sensitive files did not contain the expected synthetic bytes')
        for label, text in [('stdout', result.stdout), ('log', event_log.read_text()), ('stderr', result.stderr)]:
            if FAKE_VALUE in text or encoded in text or encoded_twice in text or NO_LOG_VALUE in text or encoded_no_log in text:
                raise RuntimeError(f'{label} contains a synthetic secret or its encoding')
        for label, text in [('stdout', result.stdout), ('log', event_log.read_text())]:
            events = [json.loads(line) for line in text.splitlines() if line]
            for name in ('lookup', 'encoded', 'fact', 'fact-reference', 'no-log-fact', 'no-log-reference', 'copy-sensitive', 'read-sensitive', 'readback', 'shell-cat', 'shell-cat-result', 'command', 'shell-read', 'shell-readback', 'registered', 'conditional', 'conditional-child'):
                matches = [event for event in events if event.get('task') == name]
                if len(matches) != 1 or not matches[0]['result'].get('censored'):
                    raise RuntimeError(f'{label}: {name} was not censored')
            public_read = [event for event in events if event.get('task') == 'public-read']
            if len(public_read) != 1 or public_read[0]['result'].get('content') != base64.b64encode(b'public file').decode():
                raise RuntimeError(f'{label}: unrelated public file result was suppressed')
            public = [event for event in events if event.get('task') == 'public']
            if len(public) != 1 or public[0]['result'].get('msg') != 'public diagnostic':
                raise RuntimeError(f'{label}: public diagnostic was suppressed')
        environment_module = base / 'environment.pkl'
        environment_file = base / 'environment-payload'
        environment_marker = base / 'environment-executed'
        environment_module.write_text('''import "Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "environment"; targets { "fixture" }; tasks {
  new { name = "sensitive copy"; operation = new Ruxel.Copy { dest = TARGET; content = new Ruxel.Call { `function` = "base64Encode"; args { new Ruxel.Lookup { provider = "synthetic"; key = FAKE_VALUE } } } } }
  new { name = "environment read"; register = "envread"; environment { ["RUXEL_READ_PATH"] = new Ruxel.Literal { value = TARGET } }; operation = new Ruxel.Command { argv = new Listing { "/bin/sh"; "-c"; ENV_COMMAND } } }
  new { name = "environment result"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "envread"; "stdout" } } } }
 } } }
} }
'''.replace('TARGET', json.dumps(str(environment_file))).replace('FAKE_VALUE', json.dumps(FAKE_VALUE)).replace('ENV_COMMAND', json.dumps('cat \"$RUXEL_READ_PATH\"; touch ' + shlex.quote(str(environment_marker)))))
        environment_log = base / 'environment.jsonl'
        environment_run = subprocess.run([str(cli), 'apply', str(environment_module), '--local', '--agent', str(agent),
                                          '--state-dir', str(base / 'environment-state'), '--synthetic-secrets',
                                          '--log-file', str(environment_log)], stdin=subprocess.DEVNULL,
                                         capture_output=True, text=True, timeout=30)
        if environment_run.returncode != 0 or environment_file.read_bytes() != encoded.encode() or not environment_marker.is_file():
            raise RuntimeError(f'environment read fixture failed: {environment_run.stderr}')
        for label, text in [('stdout', environment_run.stdout), ('stderr', environment_run.stderr), ('log', environment_log.read_text())]:
            if encoded in text or FAKE_VALUE in text:
                raise RuntimeError(f'{label}: environment-mediated read exposed the synthetic value')
        for text in (environment_run.stdout, environment_log.read_text()):
            events = [json.loads(line) for line in text.splitlines() if line]
            for name in ('environment read', 'environment result'):
                matches = [event for event in events if event.get('task') == name]
                if len(matches) != 1 or not matches[0]['result'].get('censored'):
                    raise RuntimeError(f'{name} was not censored')
        template_module = base / 'template-workload.pkl'
        template_source = base / 'generated-template.pkl'
        template_dest = base / 'template-output'
        template_module.write_text('''import "Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "template"; targets { "fixture" }; tasks {
  new { name = "write template"; operation = new Ruxel.Copy { dest = SOURCE; content = new Ruxel.Call { `function` = "concat"; args {
   new Ruxel.Literal { value = PREFIX }; new Ruxel.Call { `function` = "base64Encode"; args { new Ruxel.Lookup { provider = "synthetic"; key = FAKE_VALUE } } }; new Ruxel.Literal { value = SUFFIX }
  } } } }
  new { name = "render template"; operation = new Ruxel.Template { src = SOURCE; dest = DEST } }
  new { name = "read template"; register = "rendered"; operation = new Ruxel.Slurp { src = DEST } }
  new { name = "template result"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "results"; "rendered"; "content" } } } }
 } } }
} }
'''.replace('SOURCE', json.dumps(str(template_source))).replace('DEST', json.dumps(str(template_dest)))
            .replace('PREFIX', json.dumps('output { value = new Dynamic { kind = "literal"; value = "'))
            .replace('SUFFIX', json.dumps('" } }')).replace('FAKE_VALUE', json.dumps(FAKE_VALUE)))
        template_log = base / 'template.jsonl'
        template_run = subprocess.run([str(cli), 'apply', str(template_module), '--local', '--agent', str(agent),
                                       '--state-dir', str(base / 'template-state'), '--synthetic-secrets', '--log-file', str(template_log)],
                                      stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30)
        if template_run.returncode != 0 or template_dest.read_bytes() != encoded.encode():
            raise RuntimeError(f'template readback fixture failed: {template_run.stderr}')
        for label, text in [('stdout', template_run.stdout), ('stderr', template_run.stderr), ('log', template_log.read_text())]:
            if FAKE_VALUE in text or encoded in text or encoded_twice in text:
                raise RuntimeError(f'{label}: template readback exposed synthetic data')
        for text in (template_run.stdout, template_log.read_text()):
            events = [json.loads(line) for line in text.splitlines() if line]
            for name in ('render template', 'read template', 'template result'):
                matches = [event for event in events if event.get('task') == name]
                if len(matches) != 1 or not matches[0]['result'].get('censored'):
                    raise RuntimeError(f'{name} was not censored')
        template_error = base / 'template-error-workload.pkl'
        template_error_source = base / 'template-error-source.pkl'
        template_error_dest = base / 'template-error-output'
        template_error.write_text(template_module.read_text()
            .replace(json.dumps(str(template_source)), json.dumps(str(template_error_source)))
            .replace(json.dumps(str(template_dest)), json.dumps(str(template_error_dest)))
            .replace(json.dumps('output { value = new Dynamic { kind = "literal"; value = "'),
                     json.dumps('output { value = new Dynamic { kind = "ref"; path = new Listing { "'))
            .replace(json.dumps('" } }'), json.dumps('" } } }')))
        template_error_log = base / 'template-error.jsonl'
        failed_template = subprocess.run([str(cli), 'apply', str(template_error), '--local', '--agent', str(agent),
                                          '--state-dir', str(base / 'template-error-state'), '--synthetic-secrets',
                                          '--log-file', str(template_error_log)], stdin=subprocess.DEVNULL,
                                         capture_output=True, text=True, timeout=30)
        error_events = [json.loads(line) for line in failed_template.stdout.splitlines() if line]
        errors = [event for event in error_events if event.get('event') == 'host_failed']
        recaps = [event['recap'] for event in error_events if event.get('host') == 'fixture' and 'recap' in event]
        if (failed_template.returncode != 8 or not template_error_source.exists()
                or len(errors) != 1 or errors[0].get('host') != 'fixture'
                or errors[0].get('failed') is not True
                or errors[0].get('msg') != 'sensitive operation evaluation failed'
                or len(recaps) != 1 or recaps[0].get('failed') != 1):
            raise RuntimeError('sensitive template error did not produce the sanitized host failure and recap')
        logged_errors = [json.loads(line) for line in template_error_log.read_text().splitlines() if line]
        if [event for event in logged_errors if event.get('event') == 'host_failed'] != errors:
            raise RuntimeError('sensitive template failure event missing from private JSONL log')
        for text in (failed_template.stdout, failed_template.stderr, template_error_log.read_text()):
            if FAKE_VALUE in text or encoded in text or encoded_twice in text:
                raise RuntimeError('sensitive template expression failure exposed synthetic data')
        for path in (base / 'template-state').rglob('*'):
            if path.is_file():
                data = path.read_bytes()
                if any(value.encode() in data for value in (FAKE_VALUE, encoded, encoded_twice)):
                    raise RuntimeError('synthetic template data was persisted in local agent state')
        failing = base / 'failure.pkl'
        failing.write_text('''import "Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "failure"; targets { "fixture" }; tasks {
  new { name = "private failure"; noLog = true; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "synthetic-private-path-marker" } } } }
 } } }
} }
''')
        failure = subprocess.run([str(cli), 'apply', str(failing), '--local', '--agent', str(agent),
                                  '--state-dir', str(base / 'failure-state')], stdin=subprocess.DEVNULL,
                                 capture_output=True, text=True, timeout=30)
        if failure.returncode == 0 or 'synthetic-private-path-marker' in (failure.stdout + failure.stderr):
            raise RuntimeError('noLog evaluation failure was not sanitized')
        handler = base / 'handler.pkl'
        handler.write_text('''import "Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "fixture.invalid" } } }
 plays { new { name = "handler"; targets { "fixture" }; tasks {
  new { name = "trigger"; changedWhen = new Ruxel.Literal { value = true }; notify { "private handler" }; operation = new Ruxel.Debug { msg = "trigger" } }
 }; handlers { new { name = "private handler"; noLog = true; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "synthetic-handler-path-marker" } } } } } } }
} }
''')
        failure = subprocess.run([str(cli), 'apply', str(handler), '--local', '--agent', str(agent),
                                  '--state-dir', str(base / 'handler-state')], stdin=subprocess.DEVNULL,
                                 capture_output=True, text=True, timeout=30)
        if failure.returncode == 0 or 'synthetic-handler-path-marker' in (failure.stdout + failure.stderr):
            raise RuntimeError('noLog handler failure was not sanitized')
        for path in (base / 'state').rglob('*'):
            if path.is_file():
                data = path.read_bytes()
                if FAKE_VALUE.encode() in data or encoded.encode() in data or NO_LOG_VALUE.encode() in data or encoded_no_log.encode() in data or encoded_twice.encode() in data:
                    raise RuntimeError('synthetic secret was persisted in local agent state')
        print('PASS: synthetic lookup, transforms, facts and command results are redacted in stdout and JSONL; public diagnostic remains visible')


if __name__ == '__main__':
    main()
