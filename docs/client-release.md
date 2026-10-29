# Releasing ferl CLI

## Agreed policy

Owner decisions recorded on 2026-10-29:

| Item | Decision |
| --- | --- |
| Source and homepage | Public `https://github.com/ai-slop-code/ferl-cli` |
| Homebrew tap | Public `https://github.com/ai-slop-code/homebrew-tap`, `Formula/ferl.rb` |
| Downloads | GitHub Releases in the CLI repository |
| License | MIT, copyright 2026 Lubos Babjak |
| Versions | Independent CLI SemVer, starting at `0.1.0`; tags `v<version>` |
| Archives | `ferl-<version>-<target>.tar.gz`; immutable once published, with SHA-256 checksums |
| Mac signing | No Developer ID signing or notarization for the first release |
| CI and publishing | GitHub Actions on hosted runners; pushing a version tag runs checks, publishes archives, then updates the tap |
| Installation | Archives and Homebrew both ship in the first release |
| Evidence | Workflow logs/artifacts and `docs/client-implementation-status.md` |

These decisions supersede the earlier pending distribution proposals in the plan.
Publishing a release tag is the maintainer's release action; no additional approval
workflow or separately assigned QA role is required.

## First-release requirements

The CLI must pass formatting, warning-denied Clippy, offline tests and MSRV tests.
All four archives must build, contain the MIT license and dependency notices, and
pass checksum, linkage and native help/version checks. Homebrew must pass style,
audit, install/test/upgrade/uninstall on the two hosted Mac architectures, preserving
profile contents, modes and ownership. The upgrade rehearsal uses an explicitly
local `0.0.0` formula pointing at the candidate binary, then upgrades the formula
to the real version; it does not claim a previous public release exists.

The existing local-server integration evidence remains relevant. The following
additional checks are **non-blocking follow-ups for 0.1.0**, by owner decision:

- AC18: exhaustive native live workflows and minimum-OS checks. Current hosted
  runners cover both CPUs on Linux and macOS; macOS 13/Linux 4.18 floor verification
  remains open. Build targets are not a claim that every OS floor has been tested.
- AC02/AC03: additional real two-account and externally rotated-key scenarios.
  Existing automated profile-isolation tests and single-account live tests remain.

Other outstanding checks, including AC10's memory/aggregate-bound measurement and
remaining clipboard/terminal checks, retain their recorded status. See the
[acceptance record](client-implementation-status.md). Owner-reported manual profile
setup and clipboard success are recorded there.

## One-time GitHub setup

1. Create the two approved public repositories with default branch `main`. The tap
   needs an initial commit (a short README is sufficient) so Actions can check it out.
2. Push the CLI source, including `.github/workflows/`. Allow GitHub Actions in both
   repositories under the organization's policy.
3. Set the CLI repository Actions secret **`HOMEBREW_TAP_TOKEN`** to a fine-grained
   GitHub token with access only to `ai-slop-code/homebrew-tap` and **Contents: read
   and write**. The organization may require approval of that token. Enter it in
   GitHub's secret UI; never place it in source, a command argument, or a chat.
   The normal Actions `GITHUB_TOKEN` publishes CLI release assets.
4. Run the branch checks and inspect the first hosted results. The prepared workflow
   files are not evidence that hosted checks have already passed.

No ferl server API key is needed by these workflows. Tests use local fixtures.

## Cut a release

1. Set the version in `Cargo.toml`, update `Cargo.lock`, and write
   `docs/releases/<version>.md`. Review remaining acceptance items.
2. Commit and push to `main`. Check the CI result.
3. Create and push the matching tag, for example:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

The tag workflow runs the same checks, builds the exact candidate assets, verifies
their downloaded bytes through a draft release, publishes it, then commits the
formula to the tap. The manifest includes the source revision, compiler, client
specification hash and both server contract hashes. Published fixes get a new
version; never replace an archive under an existing version.

If tap publication fails after the release succeeds, fix the credential/access
issue and rerun the failed job. It verifies existing assets before updating the tap.
If an upload left an incomplete **unpublished draft**, remove that draft and rerun
the failed job using the retained build artifacts. Do not delete a published release
as a repair step. CI artifacts are retained for 14 days.

## Installation checks

After the first release is published, the Homebrew command is:

```sh
brew install ai-slop-code/tap/ferl
```

For direct downloads, choose the OS/CPU archive in the CLI's GitHub Releases,
verify it against `SHA256SUMS`, extract it, and put `ferl` in a directory on `PATH`.
Unsigned downloads need an actual browser-download/Gatekeeper check on a Mac;
the hosted file-URL Homebrew rehearsal does not establish that experience.

Uninstalling removes the executable, not profiles under `~/.config/ferl` (or an
explicitly selected configuration path).
