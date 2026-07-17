#!/usr/bin/env python3
"""Build/verify closed-surface feature -> executable evidence traceability."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

RENDER_PREFIXES = (
    "filter:", "jinja-", "lookup:", "template:", "template-file:",
)
CONTROLLER_MODULES = {"assert", "debug", "fail", "pause", "set_fact"}
PLAY_PREFIXES = ("document:", "play-key:", "shape:play-vars", "template:play-vars")


def read_json(path: Path):
    return json.loads(path.read_text())


def capture_tasks(path: Path) -> set[str]:
    tasks = set()
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        task = record.get("task_name") or record.get("task")
        if task:
            tasks.add(str(task).split(" : ")[-1])
    return tasks


def capture_for(fixture: str, capture_root: Path) -> Path:
    stem = Path(fixture).stem
    name = "check-semantics" if stem == "check-semantics" else f"bless-{stem}"
    return capture_root / f"{name}.jsonl"


def render_locations(capture_root: Path) -> tuple[set[str], set[str]]:
    path = capture_root / "render-parity.jsonl"
    locations = set()
    playbooks = set()
    for line in path.read_text().splitlines():
        record = json.loads(line)
        playbook = record.get("playbook")
        task = record.get("task")
        if playbook and task:
            locations.add(f"{playbook}#{task}")
            playbooks.add(playbook)
        if record.get("kind") == "template_file":
            locations.add(Path(record["src"]).name)
    return locations, playbooks


def build(required_path: Path, fixture_path: Path, capture_root: Path):
    required = read_json(required_path)
    fixtures = read_json(fixture_path)
    render, rendered_playbooks = render_locations(capture_root)
    cache: dict[Path, set[str]] = {}
    entries = {}
    missing = []

    for feature in sorted(required["features"]):
        candidates = []
        for location in sorted(fixtures.get("locations", {}).get(feature, [])):
            fixture, _, task = location.partition("#")
            if feature.startswith(RENDER_PREFIXES) and not task and fixture in rendered_playbooks:
                candidates.append({
                    "fixture_task": f"{fixture}#(play vars)",
                    "oracle": "tools/oracle/captures/render-parity.jsonl",
                    "ruxel_assertion": "crates/ruxel-core/tests/render_parity.rs",
                    "state_assertion": "not-applicable: controller rendering",
                })
                continue
            if feature.startswith(RENDER_PREFIXES) and location in render:
                candidates.append({
                    "fixture_task": location,
                    "oracle": "tools/oracle/captures/render-parity.jsonl",
                    "ruxel_assertion": "crates/ruxel-core/tests/render_parity.rs",
                    "state_assertion": "not-applicable: controller rendering",
                })
                continue
            if not fixture.endswith((".yml", ".yaml")):
                continue
            capture = capture_for(fixture, capture_root)
            if not capture.exists():
                continue
            tasks = cache.setdefault(capture, capture_tasks(capture))
            structural = (
                feature.startswith(PLAY_PREFIXES)
                or feature.startswith("shape:task-key.block")
                or feature.startswith("shape:task-key.when")
                or feature == "task-key:block"
            )
            if task and task not in tasks and not structural:
                continue
            module = feature.removeprefix("module:") if feature.startswith("module:") else ""
            state = (
                "not-applicable: controller-only result"
                if module in CONTROLLER_MODULES
                else "tools/fixtures/state-snapshot.sh"
            )
            candidates.append({
                "fixture_task": location if task else f"{fixture}#(play)",
                "oracle": str(capture),
                "ruxel_assertion": "tools/oracle/compare_results.py",
                "state_assertion": state,
            })
        if candidates:
            entries[feature] = candidates
        else:
            missing.append(feature)

    return {
        "schema": 1,
        "required_manifest": str(required_path),
        "entries": entries,
        "missing": missing,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("required", type=Path)
    parser.add_argument("fixtures", type=Path)
    parser.add_argument("captures", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    evidence = build(args.required, args.fixtures, args.captures)
    rendered = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.verify:
        if not args.output.exists() or args.output.read_text() != rendered:
            raise SystemExit("evidence manifest is stale; regenerate it")
        if evidence["missing"]:
            raise SystemExit(f"evidence incomplete: {len(evidence['missing'])} features")
    else:
        args.output.write_text(rendered)
        print(f"wrote {args.output}: {len(evidence['entries'])} mapped, "
              f"{len(evidence['missing'])} missing")


if __name__ == "__main__":
    main()
