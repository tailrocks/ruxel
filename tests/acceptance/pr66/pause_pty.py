#!/usr/bin/env python3
"""Exercise interactive pause and interruption with a local agent and a PTY."""
import argparse
import json
import os
import pty
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
PROMPT_TIMEOUT = 10


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read_prompt(stream, timeout):
    deadline = time.monotonic() + timeout
    pending = b''
    lines = []
    os.set_blocking(stream.fileno(), False)
    try:
        while True:
            remaining = deadline - time.monotonic()
            require(remaining > 0, 'pause prompt timed out')
            ready, _, _ = select.select([stream], [], [], remaining)
            require(ready, 'pause prompt timed out')
            chunk = os.read(stream.fileno(), 65536)
            require(chunk, 'process ended before pause')
            pending += chunk
            while b'\n' in pending:
                line, pending = pending.split(b'\n', 1)
                event = json.loads(line)
                lines.append(event)
                if event.get('pause') == 'synthetic pause prompt':
                    return lines, pending
    finally:
        os.set_blocking(stream.fileno(), True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-pause-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        marker = base / 'marker'
        module = base / 'fixtures/pause.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "pause"; targets { "all" }; tasks {
  new { name = "pause"; operation = new Ruxel.Pause { prompt = "synthetic pause prompt" } }
  new { name = "after pause"; operation = new Ruxel.Copy { dest = DEST; content = "continued" } }
 } } }
} }
'''.replace('DEST', json.dumps(str(marker))))
        for case in ('enter', 'eof', 'interrupt', 'check', 'diff'):
            marker.unlink(missing_ok=True)
            master, slave = pty.openpty()
            attrs = termios.tcgetattr(slave)
            attrs[3] &= ~termios.ECHO
            termios.tcsetattr(slave, termios.TCSANOW, attrs)
            mode = case if case in ('check', 'diff') else 'apply'
            process = subprocess.Popen([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                        '--state-dir', str(base / ('state-' + case))],
                                       stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       start_new_session=True)
            os.close(slave)
            lines = []
            pending = b''
            try:
                if mode == 'apply':
                    lines, pending = read_prompt(process.stdout, PROMPT_TIMEOUT)
                    time.sleep(0.2)
                    require(process.poll() is None and not marker.exists(), f'{case}: pause did not block')
                    if case == 'interrupt':
                        os.kill(process.pid, signal.SIGINT)
                    else:
                        os.write(master, b'\n' if case == 'enter' else b'\x04')
                stdout, stderr = process.communicate(timeout=10)
                lines.extend(json.loads(line) for line in (pending + stdout).splitlines() if line)
                if case == 'interrupt':
                    require(process.returncode in (-signal.SIGINT, 130), f'interrupt: exit {process.returncode}: {stderr!r}')
                    require(not marker.exists(), 'interrupt: post-pause task executed')
                else:
                    require(process.returncode == 0, f'{case}: exit {process.returncode}: {stderr!r}')
                    task = [e['result'] for e in lines if e.get('task') == 'pause' and 'result' in e]
                    require(len(task) == 1, f'{case}: missing pause result')
                    recap = [e['recap'] for e in lines if e.get('host') == 'fixture' and 'recap' in e]
                    require(len(recap) == 1 and recap[0].get('failed') == 0, f'{case}: missing or failed recap')
                    if mode == 'apply':
                        require(task[0].get('skipped') is not True and task[0].get('changed') is False
                                and task[0].get('user_input') == '' and marker.read_text() == 'continued',
                                f'{case}: incorrect pause result or continuation')
                    else:
                        require(task[0].get('skipped') is True and not marker.exists(), f'{case}: pause or preview mutated')
            finally:
                os.close(master)
                cleanup_deadline = time.monotonic() + 2
                while True:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                        break
                    except ProcessLookupError:
                        break
                    except PermissionError:
                        # macOS can briefly retain a group containing only orphan zombies.
                        if time.monotonic() >= cleanup_deadline:
                            raise
                        time.sleep(0.02)
                process.wait(timeout=5)
                process.stdout.close()
                process.stderr.close()
    print('PASS local PTY pause: enter, EOF, interrupt, check, diff')


if __name__ == '__main__':
    main()
