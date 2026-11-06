#!/usr/bin/env python3
"""Exercise nested loop labels and registered results through the real agent."""
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
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-loops-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        labels = base / 'labels.jsonl'
        proxy = base / 'agent-proxy'
        proxy_error = base / 'proxy-error.log'
        # Pass the agent protocol through unchanged; record only TaskStart's
        # item_label. JSONL controller events do not expose that wire field.
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
        module = base / 'fixtures/loops.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
function literal(v: Ruxel.Scalar): Ruxel.Literal = new { value = v }
function ref(p: Listing<String>): Ruxel.Ref = new { path = p }
function call(n: String, a: Listing<Ruxel.Expr>): Ruxel.Call = new { `function` = n; args = a }
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "nested loops"; targets { "all" }
  vars { ["parents"] = new Ruxel.ListExpr { items {
   new Ruxel.MapExpr { entries { ["name"] = literal("alpha"); ["children"] = new Ruxel.ListExpr { items { literal("one"); literal("two") } } } }
   new Ruxel.MapExpr { entries { ["name"] = literal("beta"); ["children"] = new Ruxel.ListExpr { items { literal("three") } } } }
  } } }
  tasks {
   new { name = "pairs"; register = "pair_results"
    `loop` = call("subelements", new Listing { ref(new Listing { "vars"; "parents" }); literal("children") })
    loopLabel = call("concat", new Listing { ref(new Listing { "item"; "0"; "name" }); literal("/"); ref(new Listing { "item"; "1" }) })
    operation = new Ruxel.Command { argv = new Ruxel.ListExpr { items { literal("/usr/bin/printf"); literal("%s:%s"); ref(new Listing { "item"; "0"; "name" }); ref(new Listing { "item"; "1" }) } } }
   }
   new { name = "registered assertion"; `when` = call("not", new Listing { ref(new Listing { "results"; "pair_results"; "skipped" }) })
    operation = new Ruxel.Assert { that = call("eq", new Listing { ref(new Listing { "results"; "pair_results"; "results"; "2"; "stdout" }); literal("beta:three") }) }
   }
  }
 } }
} }
''')
        expected_labels = ['alpha/one', 'alpha/two', 'beta/three']
        for mode in ('apply', 'apply', 'check', 'diff'):
            labels.unlink(missing_ok=True)
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(proxy),
                                     '--state-dir', str(base / 'state')], stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=20)
            require(result.returncode == 0, f'{mode} exited {result.returncode}: {result.stderr}\n{result.stdout}\n'
                    + (proxy_error.read_text() if proxy_error.exists() else ''))
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            pair_events = [e['result'] for e in events if e.get('task') == 'pairs' and 'result' in e]
            require(len(pair_events) == 1, f'{mode}: missing loop result')
            pairs = pair_events[0].get('results', [])
            require(len(pairs) == 3, f'{mode}: incorrect iteration count')
            observed_labels = [json.loads(line) for line in labels.read_text().splitlines()]
            require(observed_labels == expected_labels, f'{mode}: wrong wire labels: {observed_labels}')
            checks = [e['result'] for e in events if e.get('task') == 'registered assertion' and 'result' in e]
            require(len(checks) == 1, f'{mode}: missing registered assertion')
            if mode in ('check', 'diff'):
                require(all(item.get('skipped') is True for item in pairs) and checks[0].get('skipped') is True,
                        f'{mode}: command or dependent assertion not skipped')
            else:
                require([item.get('stdout') for item in pairs] == ['alpha:one', 'alpha:two', 'beta:three'],
                        f'{mode}: incorrect nested loop output')
                require(checks[0].get('failed') is False, f'{mode}: registered result assertion failed')
    print('PASS nested loops: expanded pairs, wire labels, registered results and check/diff skips')


if __name__ == '__main__':
    main()
