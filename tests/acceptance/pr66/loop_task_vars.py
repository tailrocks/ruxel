#!/usr/bin/env python3
"""Exercise per-item task vars through the real local agent and wire labels."""
import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SENTINEL = 'synthetic-private-loop-item-748261'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-loop-vars-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        labels, proxy_error = base / 'labels.jsonl', base / 'proxy-error.log'
        proxy = base / 'agent-proxy'
        # Forward the real protocol unchanged; record only TaskStart.item_label.
        proxy.write_text('#!' + sys.executable + '\n' + '''import json, os, subprocess, sys, threading
sys.stderr = open(ERROR_LOG, "a", buffering=1)
def varint(buf, pos):
    value = shift = 0
    while True:
        b = buf[pos]; pos += 1; value |= (b & 127) << shift
        if b < 128: return value, pos
        shift += 7

def fields(buf):
    pos = 0
    while pos < len(buf):
        tag, pos = varint(buf, pos); number, wire = tag >> 3, tag & 7
        if wire == 0: value, pos = varint(buf, pos)
        elif wire == 2:
            size, pos = varint(buf, pos); value = buf[pos:pos+size]; pos += size
        elif wire == 1: value = buf[pos:pos+8]; pos += 8
        elif wire == 5: value = buf[pos:pos+4]; pos += 4
        else: raise ValueError("unsupported protobuf wire type")
        yield number, value
child = subprocess.Popen([AGENT], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
def forward_input():
    try:
        while True:
            data = sys.stdin.buffer.read1(65536)
            if not data: break
            child.stdin.write(data); child.stdin.flush()
    finally:
        child.stdin.close()
threading.Thread(target=forward_input, daemon=True).start()
while True:
    header = bytearray()
    while True:
        b = child.stdout.read(1)
        if not b:
            if header: raise EOFError("truncated frame header")
            os._exit(child.wait())
        header.extend(b)
        if b[0] < 128: break
    length, _ = varint(header, 0)
    body = child.stdout.read(length)
    if len(body) != length: raise EOFError("truncated frame body")
    for number, value in fields(body):
        if number == 3:
            for field, label in fields(value):
                if field == 2:
                    with open(LABELS, "a") as log: log.write(json.dumps(label.decode()) + "\\n")
    sys.stdout.buffer.write(header + body); sys.stdout.buffer.flush()
'''.replace('AGENT', repr(str(agent))).replace('LABELS', repr(str(labels))).replace('ERROR_LOG', repr(str(proxy_error))))
        proxy.chmod(0o700)
        module = base / 'fixtures/loop-vars.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
function lit(v: Ruxel.Scalar): Ruxel.Literal = new { value = v }
function ref(p: Listing<String>): Ruxel.Ref = new { path = p }
function call(n: String, a: Listing<Ruxel.Expr>): Ruxel.Call = new { `function` = n; args = a }
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "per-item vars"; targets { "all" }; tasks {
  new { name = "default loop"; vars {
   ["items"] = new Ruxel.ListExpr { items { lit("one"); lit("two"); lit("skip") } }
   ["prefix"] = lit("default-")
   ["part"] = ref(new Listing { "item" })
   ["rendered"] = call("concat", new Listing { ref(new Listing { "vars"; "prefix" }); ref(new Listing { "vars"; "part" }) })
  }
   `loop` = ref(new Listing { "vars"; "items" })
   `when` = call("ne", new Listing { ref(new Listing { "vars"; "part" }); lit("skip") })
   loopLabel = ref(new Listing { "vars"; "rendered" })
   environment { ["RUXEL_LOOP_VALUE"] = ref(new Listing { "vars"; "rendered" }) }
   operation = new Ruxel.Command { argv = new Listing { "/usr/bin/printenv"; "RUXEL_LOOP_VALUE" } }
  }
  new { name = "custom loop"; loopVar = "entry"; vars {
   ["items"] = new Ruxel.ListExpr { items { lit("red"); lit("green"); lit("skip") } }
   ["part"] = ref(new Listing { "entry" })
   ["rendered"] = call("concat", new Listing { lit("custom-"); ref(new Listing { "vars"; "part" }) })
  }
   `loop` = ref(new Listing { "vars"; "items" })
   `when` = call("ne", new Listing { ref(new Listing { "vars"; "part" }); lit("skip") })
   loopLabel = ref(new Listing { "vars"; "rendered" })
   environment { ["RUXEL_LOOP_VALUE"] = ref(new Listing { "vars"; "rendered" }) }
   operation = new Ruxel.Command { argv = new Listing { "/usr/bin/printenv"; "RUXEL_LOOP_VALUE" } }
  }
  new { name = "private loop"; noLog = true; vars { ["value"] = ref(new Listing { "item" }) }
   `loop` = new Ruxel.ListExpr { items { lit(SECRET) } }; loopLabel = ref(new Listing { "vars"; "value" })
   operation = new Ruxel.Command { argv = new Ruxel.ListExpr { items { lit("/usr/bin/printf"); ref(new Listing { "vars"; "value" }) } } }
  }
  new { name = "empty loop"; vars { ["never_evaluate"] = ref(new Listing { "item" }) }
   `loop` = new Ruxel.ListExpr { items {} }; operation = new Ruxel.Debug { msg = ref(new Listing { "vars"; "never_evaluate" }) }
  }
 } } }
} }
'''.replace('SECRET', json.dumps(SENTINEL)))
        for mode in ('apply', 'apply', 'check', 'diff'):
            labels.unlink(missing_ok=True)
            log = base / f'events-{mode}-{len(list(base.glob("events-*")))}.jsonl'
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(proxy),
                                     '--state-dir', str(base / 'state'), '--log-file', str(log)],
                                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20)
            require(log.is_file() and log.stat().st_size > 0, f'{mode}: missing event log')
            require(SENTINEL not in result.stdout + result.stderr + log.read_text(), f'{mode}: private item leaked')
            require(result.returncode == 0, f'{mode}: exit {result.returncode}: {result.stderr}\n{result.stdout}\n'
                    + (proxy_error.read_text() if proxy_error.exists() else ''))
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recaps = [e['recap'] for e in events if e.get('host') == 'fixture' and 'recap' in e]
            require(len(recaps) == 1 and recaps[0]['failed'] == 0, f'{mode}: failed recap')
            for task, values in [('default loop', ['default-one', 'default-two']), ('custom loop', ['custom-red', 'custom-green'])]:
                rows = [e['result'] for e in events if e.get('task') == task and 'result' in e]
                require(len(rows) == 1, f'{mode}: missing {task}')
                items = rows[0].get('results', [])
                require(len(items) == 3 and items[2].get('skipped') is True, f'{mode}: bad conditional loop')
                if mode in ('check', 'diff'):
                    require(all(item.get('skipped') is True for item in items), f'{mode}: command did not skip')
                else:
                    require([item.get('stdout') for item in items[:2]] == values, f'{mode}: task vars/env incorrect')
            private = [e['result'] for e in events if e.get('task') == 'private loop' and 'result' in e]
            require(len(private) == 1 and private[0].get('censored') is True, f'{mode}: noLog loop not censored')
            empty = [e['result'] for e in events if e.get('task') == 'empty loop' and 'result' in e]
            require(len(empty) == 1 and empty[0].get('skipped') is True and empty[0].get('results') == [],
                    f'{mode}: empty loop evaluated or failed')
            observed = [json.loads(line) for line in labels.read_text().splitlines()]
            require(observed == ['default-one', 'default-two', 'custom-red', 'custom-green', '[censored]'],
                    f'{mode}: incorrect labels: {observed}')
        for path in (base / 'state').rglob('*'):
            if path.is_file():
                require(SENTINEL.encode() not in path.read_bytes(), 'private item leaked in agent state')
    print('PASS per-item vars: default/custom loops, transitive vars, condition, env, labels, noLog, empty loop')


if __name__ == '__main__':
    main()
