#!/usr/bin/env python3

import json
import pathlib
import subprocess
import sys
import tomllib

root = pathlib.Path(__file__).resolve().parents[2]
config = tomllib.loads((root / ".velnor/config.toml").read_text())
checks = [check for check in config["checks"] if check["id"] == "cargo-deny"]
if len(checks) != 1:
    sys.exit("expected exactly one cargo-deny check")
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
    cwd=root,
))
members = set(metadata["workspace_members"])
manifests = {
    pathlib.Path(package["manifest_path"]).relative_to(root).as_posix()
    for package in metadata["packages"] if package["id"] in members
}
declared = {path for path in checks[0]["inputs"] if path.endswith("/Cargo.toml")}
if manifests != declared:
    sys.exit(f"cargo-deny manifest inputs differ: missing={sorted(manifests - declared)}, extra={sorted(declared - manifests)}")
print("cargo-deny manifest inputs match workspace members")
