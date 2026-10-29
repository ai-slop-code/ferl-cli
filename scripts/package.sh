#!/bin/sh
# Reproducible release-candidate packaging for the four native targets.
#
# Builds each target from the locked dependency graph, inspects linkage, stages one
# archive per target, and writes SHA256SUMS plus a machine-readable manifest. It does
# not upload or publish anything. Policy: MIT, unsigned, immutable versioned releases.
# This four-target cross-build script runs on an Apple Silicon Mac with Zig 0.17.0.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
export COPYFILE_DISABLE=1

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
DIST="${FERL_DIST:-$ROOT/dist/ferl-$VERSION}"
TARGETS="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl"
OPENAPI_SHA=$(shasum -a 256 spec/openapi.yaml | cut -d ' ' -f 1)
SPEC_SHA=$(shasum -a 256 spec/cli-business-spec.md | cut -d ' ' -f 1)
SERVER_SPEC_SHA=$(shasum -a 256 spec/server-business-logic.md | cut -d ' ' -f 1)
RUSTC_VERSION=$(rustc -V)
SOURCE_REVISION=$(git rev-parse HEAD 2>/dev/null || echo "unversioned-local-build")
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git show -s --format=%ct HEAD 2>/dev/null || echo 0)}
if [ -n "${FERL_RELEASE_TAG:-}" ] && [ "$FERL_RELEASE_TAG" != "v$VERSION" ]; then
  echo "release tag must match Cargo.toml: v$VERSION" >&2
  exit 1
fi

rm -rf "$DIST"
mkdir -p "$DIST/stage"

for target in $TARGETS; do
  cargo build --locked --release --target "$target"
  name="ferl-$VERSION-$target"
  stage="$DIST/stage/$name"
  mkdir -p "$stage"
  cp "target/$target/release/ferl" "$stage/ferl"
  chmod 755 "$stage/ferl"
  cp LICENSE README.md "$stage/"
  python3 scripts/third-party-notices.py "$target" "$stage/THIRD_PARTY_NOTICES.txt"
  cat > "$stage/NOTICE.txt" <<EOF
ferl $VERSION ($target).
License: MIT. Copyright (c) 2026 Lubos Babjak. See LICENSE and THIRD_PARTY_NOTICES.txt.
Signing: no Developer ID signing or notarization. Archives include SHA-256 integrity checks.
Runtime: no installed Rust, OpenSSL, or libc is required for the Linux musl builds (static). macOS builds require macOS 13.0 or later.
Clipboard: pbcopy on macOS; optional wl-copy, xclip, or xsel on Linux desktops.
CLI JSON schema: 1. Contract: OpenAPI 1.0.0 sha256 $OPENAPI_SHA.
Business specification sha256 (as packaged): $SPEC_SHA.
Server business specification sha256: $SERVER_SPEC_SHA.
Compiler: $RUSTC_VERSION.
Source: $SOURCE_REVISION.
EOF
  case "$target" in
    *-apple-darwin)
      otool -l "$stage/ferl" | grep -A4 LC_BUILD_VERSION | grep -q "minos 13.0" \
        || { echo "unexpected macOS deployment target for $target" >&2; exit 1; }
      if otool -L "$stage/ferl" | tail -n +2 | awk '{print $1}' | \
        grep -Ev '^/usr/lib/(libSystem\.B\.dylib|libiconv\.2\.dylib)$'; then
        echo "unexpected macOS library dependency for $target" >&2
        exit 1
      fi
      ;;
    *-linux-musl)
      file "$stage/ferl" | grep -q "statically linked" \
        || { echo "$target is not statically linked" >&2; exit 1; }
      ;;
  esac
  python3 - "$stage" "$DIST/$name.tar.gz" "$SOURCE_DATE_EPOCH" <<'PY'
import gzip, pathlib, sys, tarfile
stage, archive, epoch = pathlib.Path(sys.argv[1]), sys.argv[2], int(sys.argv[3])
with open(archive, 'wb') as output:
    with gzip.GzipFile(filename='', mode='wb', fileobj=output, mtime=epoch) as compressed:
        with tarfile.open(fileobj=compressed, mode='w', format=tarfile.USTAR_FORMAT) as tar:
            for path in [stage, *sorted(stage.iterdir())]:
                info = tar.gettarinfo(str(path), str(path.relative_to(stage.parent)))
                info.uid = info.gid = 0
                info.uname = info.gname = 'root'
                info.mtime = epoch
                info.mode = 0o755 if path.is_dir() or path.name == 'ferl' else 0o644
                if path.is_file():
                    with path.open('rb') as content:
                        tar.addfile(info, content)
                else:
                    tar.addfile(info)
PY
done

(cd "$DIST" && shasum -a 256 ./*.tar.gz | sed 's#  \./#  #' > SHA256SUMS)

python3 - "$DIST" "$VERSION" "$RUSTC_VERSION" "$OPENAPI_SHA" "$SPEC_SHA" "$SERVER_SPEC_SHA" "$SOURCE_REVISION" $TARGETS <<'PY'
import hashlib, json, os, sys
dist, version, rustc, openapi, spec, server_spec, source, *targets = sys.argv[1:]
artifacts = []
for target in targets:
    name = f"ferl-{version}-{target}.tar.gz"
    path = os.path.join(dist, name)
    with open(path, "rb") as handle:
        digest = hashlib.sha256(handle.read()).hexdigest()
    artifacts.append({
        "target": target,
        "archive": name,
        "bytes": os.path.getsize(path),
        "sha256": digest,
    })
manifest = {
    "package": "ferl",
    "cli_version": version,
    "cli_json_schema_version": 1,
    "release_candidate": not bool(os.environ.get("FERL_RELEASE_TAG")),
    "license": "MIT",
    "signing": "NOT_APPLIED",
    "compiler": rustc,
    "source_revision": source,
    "client_spec_sha256": spec,
    "contract": {
        "server_revision": "0d4cdcc499f02edaedef70ffb332012935a8af9e",
        "openapi_sha256": openapi,
        "server_business_spec_sha256": server_spec,
    },
    "artifacts": artifacts,
}
with open(os.path.join(dist, "manifest.json"), "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2)
    handle.write("\n")
PY

rm -rf "$DIST/stage"
echo "packaged $DIST"
ls -l "$DIST"
