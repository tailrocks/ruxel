#!/usr/bin/env python3
"""Verify one local agent handshake failure does not stop healthy hosts."""
import argparse
import json
import shutil
import subprocess
import sys
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-handshake-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        module = base / 'fixtures/two-plays.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture-a"] { address = "127.0.0.1" }; ["fixture-b"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture-a"; "fixture-b" } } } }
 plays {
  new { name = "first"; targets { "all" }; tasks { new { name = "first command"; operation = new Ruxel.Command { argv = new Listing { "/usr/bin/printf"; "first" } } } } }
  new { name = "second"; targets { "all" }; tasks { new { name = "second command"; operation = new Ruxel.Command { argv = new Listing { "/usr/bin/printf"; "second" } } } } }
 }
} }
''')
        for failed_index in (0, 1):
            log = base / f'launches-{failed_index}.jsonl'
            wrapper = base / f'agent-{failed_index}'
            wrapper.write_text('#!' + sys.executable + '\n' + '''import fcntl, json, os, sys
with open(LOG, "a+") as f:
    fcntl.flock(f, fcntl.LOCK_EX)
    f.seek(0)
    rows = [json.loads(line) for line in f]
    state = os.environ["RUXEL_STATE_DIR"]
    known = {row[1]: row[0] for row in rows}
    index = known.get(state, len(known))
    f.write(json.dumps([index, state]) + "\\n")
    f.flush()
    fcntl.flock(f, fcntl.LOCK_UN)
if index == FAILED_INDEX:
    sys.exit(23)
os.execv(AGENT, [AGENT])
'''.replace('LOG', repr(str(log))).replace('FAILED_INDEX', str(failed_index)).replace('AGENT', repr(str(agent))))
            wrapper.chmod(0o700)
            result = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(wrapper),
                                     '--state-dir', str(base / f'state-{failed_index}'), '--jobs', '1'],
                                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=25)
            require(result.returncode == 8, f'failed index {failed_index}: expected exit 8, got {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            failed = ('fixture-a', 'fixture-b')[failed_index]
            healthy = ('fixture-b', 'fixture-a')[failed_index]
            for task, output in [('first command', 'first'), ('second command', 'second')]:
                matches = [e for e in events if e.get('task') == task and 'result' in e]
                require(len(matches) == 1 and matches[0].get('host') == healthy
                        and matches[0]['result'].get('stdout') == output,
                        f'failed index {failed_index}: healthy host did not complete {task}')
            recaps = [e for e in events if 'recap' in e]
            require(len(recaps) == 2 and {e['host'] for e in recaps} == {healthy, failed},
                    f'failed index {failed_index}: missing or duplicate host recaps')
            require(next(e['recap']['failed'] for e in recaps if e['host'] == failed) > 0,
                    f'failed index {failed_index}: failed host recap did not fail')
            require(next(e['recap']['failed'] for e in recaps if e['host'] == healthy) == 0,
                    f'failed index {failed_index}: healthy host reported failure')
            launches = [json.loads(line) for line in log.read_text().splitlines()]
            require({row[0] for row in launches} == {0, 1} and len({row[1] for row in launches}) == 2
                    and sum(row[0] == failed_index for row in launches) == 1,
                    f'failed index {failed_index}: failed host was retried or a host was not launched')
    print('PASS handshake isolation: healthy host completes two plays in both host orderings; failed host not retried')


if __name__ == '__main__':
    main()
