#!/bin/sh
# Exercises a disposable local tap on a fresh GitHub-hosted Mac runner.
set -eu
[ "${GITHUB_ACTIONS:-}" = true ] || { echo "run this on a disposable CI runner" >&2; exit 1; }
DIST=$(cd "$1" && pwd)
ROOT=$(cd "$(dirname "$0")/.." && pwd)
export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
TAP=ferl-ci/rehearsal
FORMULA=$TAP/ferl
WORK=$(mktemp -d "$RUNNER_TEMP/ferl-homebrew.XXXXXX")
chmod 700 "$WORK"
export FERL_CONFIG="$WORK/config.toml"
cleanup() {
  brew uninstall --force "$FORMULA" >/dev/null 2>&1 || true
  brew untap "$TAP" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT
brew tap-new --no-git "$TAP"
FILE="$(brew --repo "$TAP")/Formula/ferl.rb"
python3 "$ROOT/scripts/render-formula.py" "$DIST" "$FILE" --local --rehearsal-version 0.0.0
brew install "$FORMULA"
# A public sentinel, not an account credential. This is an entirely offline profile.
export FERL_TEST_KEY=ferl-ci-synthetic-key
ferl --non-interactive profile add rehearsal --server https://example.invalid --key-env FERL_TEST_KEY --default
snapshot() {
  python3 - "$FERL_CONFIG" <<'PY'
import hashlib, json, os, sys
result = []
for path in [sys.argv[1], sys.argv[1] + '.credentials.toml']:
    stat = os.stat(path)
    result.append([stat.st_mode, stat.st_uid, hashlib.sha256(open(path, 'rb').read()).hexdigest()])
print(json.dumps(result))
PY
}
snapshot > "$WORK/before"
python3 "$ROOT/scripts/render-formula.py" "$DIST" "$FILE" --local
brew style --formula "$FILE"
brew upgrade "$FORMULA"
brew test "$FORMULA"
snapshot > "$WORK/after-upgrade"
cmp "$WORK/before" "$WORK/after-upgrade"
brew uninstall "$FORMULA"
snapshot > "$WORK/after-uninstall"
cmp "$WORK/before" "$WORK/after-uninstall"
# Audit the actual release formula, with its immutable HTTPS download URLs.
python3 "$ROOT/scripts/render-formula.py" "$DIST" "$FILE"
# This disposable tap's formula was just generated from the candidate archives; trust
# that exact content so Homebrew will load it for audit.
brew trust --formula "$FORMULA"
brew audit --strict "$FORMULA"
echo "Homebrew install/upgrade/uninstall preserved profile bytes, modes, and ownership."
