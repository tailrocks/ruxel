#!/usr/bin/env python3
"""Exercise registered retry results using a disposable local marker."""
import argparse
import json
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-retry-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        marker = base / 'marker'
        quoted = shlex.quote(str(marker))
        command = f'test -f {quoted} && exit 0; touch {quoted}; exit 1'
        # JSON strings are valid Pkl string literals and escape arbitrary temp paths.
        source = '''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
  inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
  plays { new { name = "retry"; targets { "all" }; tasks { new {
    name = "retry"; operation = new Ruxel.Shell { cmd = COMMAND }; register = "fixture_retry"
    until = new Ruxel.Call { `function` = "eq"; args { new Ruxel.Ref { path { "results"; "fixture_retry"; "rc" } }; new Ruxel.Literal { value = 0 } } }
    retries = 3; delaySeconds = 1
    changedWhen = new Ruxel.Literal { value = false }; failedWhen = new Ruxel.Literal { value = false }
  } } } }
} }
'''.replace('COMMAND', json.dumps(command))
        module = base / 'fixtures/retry.pkl'
        module.write_text(source)
        result = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(agent),
                                 '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=15)
        if result.returncode:
            raise RuntimeError(f'apply exited {result.returncode}: {result.stderr}\n{result.stdout}')
        events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        results = [e.get('result') for e in events if e.get('task') == 'retry']
        if not marker.is_file() or len(results) != 1 or results[0].get('rc') != 0:
            raise RuntimeError('retry did not create its marker and return success')
    print('PASS registered retry: initial failure followed by successful retry')


if __name__ == '__main__':
    main()
