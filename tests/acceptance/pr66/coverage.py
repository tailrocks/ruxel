#!/usr/bin/env python3
"""Check that every concrete Pkl operation has an agent dispatch arm.

This is a structural guard, not proof that a provider works. Controller-only
operations must be added here only when there is an explicit executable route.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCHEMA = ROOT / "schema/Ruxel.pkl"
AGENT = ROOT / "crates/ruxel/ruxel-agent/src/modules/mod.rs"
CONTROLLER = ROOT / "crates/ruxel/ruxel-engine/src/controller/operation.rs"
CONTROLLER_LOCAL = ROOT / "crates/ruxel/ruxel-engine/src/controller/local.rs"


def operation_kinds(source):
    alias = re.search(r"^typealias\s+Operation\s*=\s*([^\n]+)", source, re.M)
    if not alias:
        raise ValueError("Operation union not found")
    names = [name.strip() for name in alias.group(1).split("|")]
    if not names or any(not re.fullmatch(r"[A-Za-z_]\w*", name) for name in names):
        raise ValueError("unrecognized Operation union")
    if len(set(names)) != len(names):
        raise ValueError("duplicate Operation class")
    kinds = []
    for name in names:
        pattern = r'\bclass\s+' + re.escape(name) + r'\s*\{\s*fixed\s+kind\s*=\s*"([^"]+)"'
        matches = re.findall(pattern, source)
        if len(matches) != 1:
            raise ValueError(f"{name} must have one fixed literal kind")
        kinds.append(matches[0])
    if len(set(kinds)) != len(kinds):
        raise ValueError("duplicate operation kind")
    return set(kinds)


def strip_rust_comments(source):
    """Discard line and nested block comments while preserving string literals."""
    out = []
    i = 0
    while i < len(source):
        if source.startswith('//', i):
            end = source.find('\n', i)
            i = len(source) if end < 0 else end
        elif source.startswith('/*', i):
            i += 2
            depth = 1
            while i < len(source) and depth:
                if source.startswith('/*', i):
                    depth += 1
                    i += 2
                elif source.startswith('*/', i):
                    depth -= 1
                    i += 2
                else:
                    out.append('\n' if source[i] == '\n' else ' ')
                    i += 1
            if depth:
                raise ValueError("unterminated Rust block comment")
        elif source[i] == '"':
            start = i
            i += 1
            while i < len(source):
                if source[i] == '\\':
                    i += 2
                elif source[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            else:
                raise ValueError("unterminated Rust string")
            out.append(source[start:i])
        elif source[i] == 'r' and (raw := re.match(r'r(#{0,255})"', source[i:])):
            start = i
            i += len(raw.group(0))
            end = source.find('"' + raw.group(1), i)
            if end < 0:
                raise ValueError("unterminated Rust raw string")
            i = end + 1 + len(raw.group(1))
            out.append(source[start:i])
        else:
            out.append(source[i])
            i += 1
    return ''.join(out)


def controller_kinds(operation, local):
    """Require uncommented admission and dispatch on both local retry paths."""
    operation = strip_rust_comments(operation)
    local = strip_rust_comments(local)
    admission = re.findall(
        r'if\s+matches!\s*\(\s*kind\s*,\s*((?:"[a-z_]+"\s*(?:\|\s*)?)+)\)'
        r'\s*\{\s*return\s+self\s*\.\s*local_retry\s*\(', operation, re.S)
    if len(admission) != 1:
        raise ValueError("controller local admission route not found or ambiguous")
    admitted_list = re.findall(r'"([a-z_]+)"', admission[0])
    if not admitted_list or len(set(admitted_list)) != len(admitted_list):
        raise ValueError("invalid controller local admission")
    functions = re.search(r'async\s+fn\s+local_once\s*\(.*?match\s+kind\s*\{(.*?)\n\s*_\s*=>'
                          r'.*?async\s+fn\s+local_retry\s*\((.*)', local, re.S)
    if not functions:
        raise ValueError("controller local dispatch or retry route not found")
    retry = functions.group(2)
    loop = re.search(r'for\s+attempt\s+in\s+1\s*\.\.\s*=\s*attempts\s*\{(.*?)\blet\s+scope\s*=\s*result_scope\s*\(', retry, re.S)
    if not loop or not re.search(r'\.\s*local_once\s*\(', loop.group(1)):
        raise ValueError("controller retry-loop local dispatch not found")
    if not re.search(r'\.\s*local_once\s*\(', retry[:loop.start()]):
        raise ValueError("controller non-retry local dispatch not found")
    arms = re.findall(r'^\s*"([a-z_]+)"\s*=>', functions.group(1), re.M)
    if not arms or len(set(arms)) != len(arms):
        raise ValueError("invalid controller local dispatch")
    if set(admitted_list) != set(arms):
        raise ValueError("controller local admission/dispatch mismatch")
    return set(arms)


def dispatch_kinds(source):
    # Limit extraction to execute's actual match, not is_implemented, comments,
    # tests or unrelated string literals. Fail closed if the format changes.
    match = re.search(r"let result = match module\s*\{(.*?)\n\s*_\s*=>", source, re.S)
    if not match:
        raise ValueError("agent dispatch match not found")
    arms = re.findall(r'^\s*((?:"[^"]+"\s*(?:\|\s*)?)+)\s*=>', match.group(1), re.M)
    kinds = {kind for arm in arms for kind in re.findall(r'"([^"]+)"', arm)}
    if not kinds:
        raise ValueError("agent dispatch contains no recognizable arms")
    return kinds


def admission_kinds(source):
    """Return a separate dispatch admission list when the agent has one."""
    if not re.search(r"\bfn\s+is_implemented\s*\(", source):
        if re.search(r"\bis_implemented\s*\(", source):
            raise ValueError("dispatch admission function not found")
        return None
    match = re.search(r"fn\s+is_implemented\s*\([^)]*\)[^{]*\{\s*matches!\s*\(\s*module\s*,(.*?)\)\s*\}", source, re.S)
    if not match:
        raise ValueError("unrecognized agent admission list")
    kinds = set(re.findall(r'"([^"]+)"', match.group(1)))
    if not kinds:
        raise ValueError("empty agent admission list")
    return kinds


def main():
    try:
        declared = operation_kinds(SCHEMA.read_text())
        agent = AGENT.read_text()
        implemented = dispatch_kinds(agent)
        admitted = admission_kinds(agent)
        controller = controller_kinds(CONTROLLER.read_text(), CONTROLLER_LOCAL.read_text())
    except (OSError, ValueError) as error:
        print(f"coverage error: {error}", file=sys.stderr)
        return 1
    if admitted is not None and admitted != implemented:
        print(f'agent admission/dispatch mismatch: dispatch-only={sorted(implemented - admitted)}, admission-only={sorted(admitted - implemented)}', file=sys.stderr)
        return 1
    missing = sorted(declared - implemented - controller)
    for kind in missing:
        print(f"no executable agent or controller dispatch for schema operation: {kind}", file=sys.stderr)
    if missing:
        return 1
    print(f"{len(declared)} schema operation kinds have dispatch arms; behavioral tests still required")
    return 0


if __name__ == "__main__":
    sys.exit(main())
