#!/usr/bin/env python3
"""Prove local host isolation, bounded overlap, and a barrier between plays."""
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-hosts-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        log = base / 'intervals.jsonl'
        helper = base / 'instrument.py'
        helper.write_text('''import fcntl, json, os, sys, time
with open(sys.argv[1], "a+") as f:
    def event(phase):
        fcntl.flock(f, fcntl.LOCK_EX)
        f.write(json.dumps([phase, os.getpid(), time.monotonic_ns()]) + "\\n")
        f.flush()
        fcntl.flock(f, fcntl.LOCK_UN)
    event("start")
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.seek(0)
        starts = sum(json.loads(line)[0] == "start" for line in f)
        fcntl.flock(f, fcntl.LOCK_UN)
        if starts >= 2:
            break
        time.sleep(0.02)
    time.sleep(0.1)
    event("end")
''')
        hosts = {f'fixture-{i}' for i in range(1, 5)}
        host_rows = '\n'.join(f'[{json.dumps(h)}] {{ address = "127.0.0.1"; vars {{ ["token"] = new Ruxel.Literal {{ value = {json.dumps(h)} }} }} }}' for h in sorted(hosts))
        argv = '; '.join(map(json.dumps, (sys.executable, str(helper), str(log))))
        source = '''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { HOSTS }; groups { ["all"] { hosts { HOSTNAMES } } } }
 plays {
  new { name = "first"; targets { "all" }; tasks {
   new { name = "instrument"; changedWhen = new Ruxel.Literal { value = false }; operation = new Ruxel.Command { argv = new Listing { ARGV } } }
   new { name = "set distinct fact"; operation = new Ruxel.SetFact { values { ["answer"] = new Ruxel.Ref { path { "vars"; "token" } } } } }
   new { name = "read distinct fact"; operation = new Ruxel.Debug { msg = new Ruxel.Ref { path { "vars"; "answer" } } } }
  } }
  new { name = "second"; targets { "all" }; tasks { new { name = "second play"; operation = new Ruxel.Debug { msg = "barrier" } } } }
 }
} }
'''.replace('HOSTS', host_rows).replace('HOSTNAMES', '; '.join(map(json.dumps, sorted(hosts)))).replace('ARGV', argv)
        module = base / 'fixtures/hosts.pkl'
        module.write_text(source)
        result = subprocess.run([str(cli), 'apply', str(module), '--local', '--agent', str(agent),
                                 '--state-dir', str(base / 'state'), '--jobs', '2'], stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=20)
        require(result.returncode == 0, f'apply failed: {result.stderr}\n{result.stdout}')
        events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        facts = [event for event in events if event.get('task') == 'read distinct fact']
        require(len(facts) == len(hosts) and {e['host'] for e in facts} == hosts,
                'missing or duplicate per-host fact results')
        require(all(e['result'].get('msg') == e['host'] for e in facts), 'host facts crossed boundaries')
        first = [i for i, e in enumerate(events) if e.get('task') == 'read distinct fact']
        second = [i for i, e in enumerate(events) if e.get('task') == 'second play']
        require(len(second) == len(hosts) and max(first) < min(second), 'play barrier violated')
        intervals = [json.loads(line) for line in log.read_text().splitlines()]
        require(len(intervals) == 2 * len(hosts), 'missing instrumentation events')
        active = set()
        peak = 0
        for phase, pid, _ in sorted(intervals, key=lambda x: x[2]):
            if phase == 'start':
                require(pid not in active, 'duplicate process start')
                active.add(pid)
                peak = max(peak, len(active))
            else:
                require(phase == 'end' and pid in active, 'unmatched process end')
                active.remove(pid)
        require(not active and peak == 2, f'expected overlap bounded by two jobs; observed peak {peak}')
    print('PASS host isolation: distinct facts, observed peak two jobs, play barrier')


if __name__ == '__main__':
    main()
