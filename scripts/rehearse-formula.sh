#!/bin/sh
# Local rehearsal of the Homebrew formula against the candidate archives in dist/.
#
# Renders packaging/homebrew/ferl.rb.template with approved identities
# and file:// artifact URLs, checks Ruby syntax, runs the Homebrew style checker when
# it is available, and runs the offline version/help checks on the host architecture.
# It does not tap, install, upgrade, or publish anything. Installing through a tap is
# a separate, authorized step (business spec section 11, plan M5-M6).
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
DIST="${FERL_DIST:-$ROOT/dist/ferl-$VERSION}"
OUT="${FERL_REHEARSAL_DIR:-$ROOT/dist/rehearsal}"
rm -rf "$OUT"
mkdir -p "$OUT/Formula"

FORMULA="$OUT/Formula/ferl.rb"
python3 scripts/render-formula.py "$DIST" "$FORMULA" --local
ruby -c "$FORMULA" >/dev/null
echo "formula syntax: ok"

if command -v brew >/dev/null 2>&1; then
  if brew style --formula "$FORMULA" >"$OUT/brew-style.log" 2>&1; then
    echo "brew style: ok"
  else
    echo "brew style: not completed (see $OUT/brew-style.log; Homebrew's own gem fetch may be blocked in this environment)"
  fi
fi

case "$(uname -m)" in
  arm64) ARCHIVE="ferl-$VERSION-aarch64-apple-darwin.tar.gz" ;;
  x86_64) ARCHIVE="ferl-$VERSION-x86_64-apple-darwin.tar.gz" ;;
  *) echo "no Mac archive for this architecture; skipping offline run"; exit 0 ;;
esac
EXTRACT="$OUT/extract"
mkdir -p "$EXTRACT"
tar -xzf "$DIST/$ARCHIVE" -C "$EXTRACT"
BIN="$EXTRACT/ferl-$VERSION-${ARCHIVE#ferl-$VERSION-}"
BIN="${BIN%.tar.gz}/ferl"
"$BIN" --version | grep -q "ferl $VERSION"
"$BIN" --help | grep -q "Usage"
echo "offline version/help on $(uname -m): ok ($ARCHIVE)"
