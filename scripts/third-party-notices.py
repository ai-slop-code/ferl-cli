#!/usr/bin/env python3
"""Collect bundled license/notice texts for a target's locked dependency graph."""

import json
from pathlib import Path
import subprocess
import sys

target, output = sys.argv[1:]
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target,
]))
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
included = set()
pending = [metadata["resolve"]["root"]]
while pending:
    ident = pending.pop()
    if ident in included:
        continue
    included.add(ident)
    pending.extend(d["pkg"] for d in nodes[ident]["deps"]
                   if any(k["kind"] != "dev" for k in d["dep_kinds"]))

parts = ["ferl third-party notices\n\n"
         "Bundled texts from locked runtime and build dependencies for " + target + ".\n"
         "Some packages offer alternative licenses; their upstream notices are retained.\n"]
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    if package["id"] not in included or package["source"] is None:
        continue
    root = Path(package["manifest_path"]).parent
    files = {p for p in root.rglob("*") if p.is_file() and p.name.lower().startswith(
        ("license", "licence", "copying", "notice", "copyright"))}
    if package["license_file"]:
        files.add(root / package["license_file"])
    if not files:
        raise SystemExit(f"missing bundled license text: {package['name']} {package['version']}")
    parts.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\n"
                 f"SPDX: {package['license']}\n")
    for path in sorted(files):
        parts.append(f"\n--- {path.relative_to(root)} ---\n{path.read_text()}\n")
Path(output).write_text("".join(parts))
