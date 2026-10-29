#!/usr/bin/env python3
"""Render a formula from the exact packaged archives; no network access."""

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("dist", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--local", action="store_true", help="use file URLs for rehearsal")
parser.add_argument("--rehearsal-version", help="local-only version for upgrade testing")
args = parser.parse_args()
if args.rehearsal_version and not args.local:
    parser.error("--rehearsal-version requires --local")
manifest = json.loads((args.dist / "manifest.json").read_text())
version = manifest["cli_version"]
base = (args.dist.resolve().as_uri() if args.local else
        f"https://github.com/ai-slop-code/ferl-cli/releases/download/v{version}")
values = {"VERSION": args.rehearsal_version or version,
          "ARTIFACT_VERSION": version, "ARTIFACT_BASE": base}
for arch, key in [("aarch64", "ARM_SHA256"), ("x86_64", "INTEL_SHA256")]:
    name = f"ferl-{version}-{arch}-apple-darwin.tar.gz"
    artifact = next(a for a in manifest["artifacts"] if a["archive"] == name)
    digest = hashlib.sha256((args.dist / name).read_bytes()).hexdigest()
    if digest != artifact["sha256"]:
        raise SystemExit(f"checksum mismatch: {name}")
    values[key] = digest
text = (ROOT / "packaging/homebrew/ferl.rb.template").read_text()
for key, value in values.items():
    text = text.replace(f"<{key}>", value)
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(text)
