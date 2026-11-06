#!/usr/bin/env python3
"""Verify per-host facts survive plays without exposing noLog values."""
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SENTINEL = 'synthetic-sensitive-cross-play-638251-'
HOSTS = {'fixture-a', 'fixture-b'}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ruxel', type=Path, required=True)
    parser.add_argument('--agent', type=Path, required=True)
    args = parser.parse_args()
    cli, agent = args.ruxel.resolve(strict=True), args.agent.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ruxel-pr66-cross-play-') as temp:
        base = Path(temp)
        (base / 'schema').mkdir()
        (base / 'fixtures').mkdir()
        payload = base / 'payload'
        payload.mkdir()
        shutil.copyfile(ROOT / 'schema/Ruxel.pkl', base / 'schema/Ruxel.pkl')
        module = base / 'fixtures/facts.pkl'
        module.write_text('''import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture-a"] { address = "127.0.0.1" }; ["fixture-b"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture-a"; "fixture-b" } } } }
 plays {
  new { name = "set"; targets { "all" }; tasks { new { name = "set private fact"; noLog = true
   operation = new Ruxel.SetFact { values { ["private_value"] = new Ruxel.Call { `function` = "concat"; args { new Ruxel.Literal { value = SECRET }; new Ruxel.Ref { path { "inventoryHost" } } } } } }
  } } }
  new { name = "consume"; targets { "all" }; tasks { new { name = "consume private fact"; noLog = true
   operation = new Ruxel.Copy { dest = new Ruxel.Call { `function` = "concat"; args { new Ruxel.Literal { value = PREFIX }; new Ruxel.Ref { path { "inventoryHost" } } } }; content = new Ruxel.Ref { path { "vars"; "private_value" } } }
  } } }
 }
} }
'''.replace('PREFIX', json.dumps(str(payload) + '/')).replace('SECRET', json.dumps(SENTINEL)))
        def snapshot():
            return {p.name: p.read_bytes() for p in payload.iterdir() if p.is_file()}
        def run(mode, ordinal):
            log = base / f'events-{ordinal}.jsonl'
            result = subprocess.run([str(cli), mode, str(module), '--local', '--agent', str(agent),
                                     '--state-dir', str(base / 'state'), '--jobs', '2', '--log-file', str(log)],
                                    stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=25)
            require(log.is_file() and log.stat().st_size > 0, f'{mode}: missing event log')
            require(SENTINEL not in result.stdout + result.stderr + log.read_text(), f'{mode}: private fact leaked in output')
            require(result.returncode == 0, f'{mode}: exit {result.returncode}: {result.stderr}\n{result.stdout}')
            events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            recaps = [e for e in events if 'recap' in e]
            require(len(recaps) == 2 and {e['host'] for e in recaps} == HOSTS
                    and all(e['recap']['failed'] == 0 for e in recaps), f'{mode}: host recaps incorrect')
            for task in ('set private fact', 'consume private fact'):
                rows = [e for e in events if e.get('task') == task and 'result' in e]
                require(len(rows) == 2 and {e['host'] for e in rows} == HOSTS
                        and all(e['result'].get('censored') is True for e in rows), f'{mode}: noLog result missing')
            return recaps
        first = run('apply', 'fresh')
        require(snapshot() == {host: (SENTINEL + host).encode() for host in HOSTS}
                and all(e['recap']['changed'] > 0 for e in first), 'fresh cross-play facts incorrect')
        stable = run('apply', 'converged')
        require(snapshot() == {host: (SENTINEL + host).encode() for host in HOSTS}
                and all(e['recap']['changed'] == 0 for e in stable), 'cross-play facts did not converge')
        (payload / 'fixture-a').write_text('drift')
        for mode in ('check', 'diff'):
            before = snapshot()
            recaps = run(mode, mode)
            require(snapshot() == before and any(e['recap']['changed'] > 0 for e in recaps),
                    f'{mode}: changed payload or failed to predict drift')
        repaired = run('apply', 'repair')
        require(snapshot() == {host: (SENTINEL + host).encode() for host in HOSTS}
                and any(e['recap']['changed'] > 0 for e in repaired), 'cross-play fact drift not repaired')
        for path in (base / 'state').rglob('*'):
            if path.is_file():
                require(SENTINEL.encode() not in path.read_bytes(), 'private fact leaked in agent state')
    print('PASS cross-play per-host facts: noLog, convergence, drift and check/diff')


if __name__ == '__main__':
    main()
