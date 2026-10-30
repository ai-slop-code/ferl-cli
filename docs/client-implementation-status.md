# ferl CLI: implementation status and acceptance record

Status date: 2026-10-29. This record tracks the local release-one client implementation
against the [client business specification](../spec/cli-business-spec.md) and the
[implementation plan](cli-implementation-plan.md). It separates verified behaviour from
behaviour that awaits a named environment check, and from remaining release
prerequisites. Version 0.1.0 is published (§11). Minimum-OS floors, Linux desktop
clipboard helpers, and the Mac browser-download check remain follow-ups as described
in §11; sections before §10 record the pre-release state.

## 1. Baseline (M0 / PR02 checkpoint)

| Item | Value | Evidence |
| --- | --- | --- |
| Server contract revision | `0d4cdcc499f02edaedef70ffb332012935a8af9e` | [API handoff](cli-api-handoff.md) §2 |
| OpenAPI SHA-256 | `45677d0d0c0afda4687fada791ce492682ad546f2fbc612113890cffecbb6b5e` | Matches the handoff and business spec §1. |
| Server business specification SHA-256 | `c9fd70817b687df1e44dc6d3c4eb64e68af5b92b91fe2264903f7d0e05352c36` | Matches `spec/server-business-logic.md` and client business spec §1. The earlier mismatch report incorrectly compared the client specification with the server specification; there is no contract-hash discrepancy. |
| Client specification snapshot used for candidate packaging | `67e39811db4020d3fe9d8f4e556d341eaadc8bdf119eff6e271a20b57cada1fa` | Historical packaging input, recorded as `contract.business_spec_sha256` in the existing candidate manifest. This is a client-specification hash, distinct from the imported server contract above. Later documentation corrections do not change the existing archives. |
| Server business logic | `spec/server-business-logic.md`, read in full | Joint contract with OpenAPI. |
| Live setup workflow | Owner-supplied local API and private environment file | Current commands are in the [README](../README.md#live-integration-tests-optional). |
| Host used for this evidence | Apple M5 (arm64), macOS 26.7.1 | `uname -m`, `sw_vers` |

Engineering decisions made within the baseline (no scope change):

- **D-IMPL-01 — MSRV and dependency versions.** Edition 2024, MSRV `1.85` (`rust-version`),
  build pin `1.90.0` (`rust-toolchain.toml`). `time` is pinned to 0.3.41 and `uuid` to
  1.18.1 because newer releases require Rust 1.88 and 1.89. The MSRV-aware resolver selects
  the remaining versions. No locked crate declares a `rust-version` above 1.85.
- **D-IMPL-02 — HTTP client.** A small HTTP/1.1 client over `rustls` (ring provider) and
  `webpki-root-certs` replaces an earlier `ureq` prototype. Reason: the `http` crate that
  `ureq` uses limits URIs to 65,534 bytes, which cannot carry the 87,387-byte continuation
  cursors the release must support (business spec §8). The new client sends one request per
  connection, never follows redirects, stores no cookies or cache, and implements proxy
  CONNECT tunnels, bounded bodies, and chunked decoding.
- **D-IMPL-03 — Private CA bundles** extend the public Mozilla roots (business spec §10.1
  "adds private trust anchors"). Bundles must contain only PEM certificates.
- **D-IMPL-04 — Local canonicalization.** Titles and tags use only unambiguous checks
  (trim, scalar bounds, controls). Full case folding and NFC are left to the server, as
  business spec §9.1 permits. Titles and tags are sent as supplied.
- **D-IMPL-05 — Interruption.** `signal-hook` sets a SIGINT flag. `ctrlc` was replaced
  because its macOS dependency chain linked Objective-C runtime and Foundation frameworks,
  which the spec does not allow. Binaries now link only `libSystem` and `libiconv`.
- **D-IMPL-06 — Linux cross-builds** use `zig cc` as a build-time C compiler and linker
  (`scripts/cross/`). They produce compilation evidence only.
- **D-IMPL-07 — Connections.** Each HTTP operation uses a fresh connection. Only GET
  operations retry (transient connect, reset, timeout, and 502/503/504), so the transport
  never replays a POST or DELETE.

## 2. Build and dependency evidence

Toolchains: rustup-managed `1.90.0` (build pin, with rustfmt and clippy) and `1.85.0`
(MSRV, clippy). Targets for `1.90.0`: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`. Homebrew's `cargo` 1.99.1 is
not used for release evidence.

Dependency licenses for the 138 third-party crates in the lockfile are MIT, Apache-2.0,
or dual/tri-licensed combinations (84 MIT OR Apache-2.0; others below). Notable items for
the licensing review (PR03): `webpki-root-certs` (CDLA-Permissive-2.0, root-store data),
Unicode-3.0 (ICU data crates via `idna`), ISC (`ring`'s ISC parts, `untrusted`), BSD-3
(`subtle`), and `r-efi` (MIT OR Apache-2.0 OR LGPL-2.1-or-later, used only on UEFI targets,
not in these binaries). No crate is linked under LGPL for these targets.

## 3. Final verification run

All commands are run from the repository root with `$HOME/.cargo/bin` first on `PATH`.

| Check | Command | Result |
| --- | --- | --- |
| Formatting | `cargo +1.90.0 fmt --check` | Passed |
| Lints (all targets, warnings denied) | `cargo +1.90.0 clippy --locked --all-targets -- -D warnings` | Passed |
| Offline test suite | `cargo +1.90.0 test --locked` | Passed: 76 unit, 22 process, 37 HTTP fixture, 2 PTY wizard, 3 signal, 4 TLS/proxy, 6 schema; 8 live tests ignored by default |
| MSRV build and tests | `cargo +1.85.0 check --locked --all-targets` and `cargo +1.85.0 test --locked` | Passed (same counts) |
| Live API suite (explicit) | sourced `profile.env` in a subshell, then `cargo +1.90.0 test --locked --test live_api -- --ignored --test-threads=1` | Passed: 8 of 8 |
| Four release builds | `scripts/package.sh` | Passed; linkage checks run during packaging |
| Formula rehearsal | `scripts/rehearse-formula.sh` | Syntax passed; offline version/help passed on arm64. `brew style` could not complete (Homebrew's bundled Ruby cannot verify rubygems.org certificates in this environment). |

## 4. Initial artifacts (historical pre-policy candidate, not published)

`scripts/package.sh` produced `dist/ferl-0.1.0/` with one archive per target, `SHA256SUMS`,
and `manifest.json` (CLI version 0.1.0, JSON schema 1, compiler `rustc 1.90.0`, contract
hashes above, `license: PENDING`, `signing: NOT_APPLIED`). Archives extract to a single
relative directory containing `ferl` (mode 0755), `NOTICE.txt`, root ownership, and no
AppleDouble entries.

This inventory predates the owner's MIT/signing/repository decisions. Updated,
licensed candidates are in `dist/approved-0.1.0/`; see §10. The original archives
are retained as historical local evidence.

| Archive | SHA-256 |
| --- | --- |
| `ferl-0.1.0-aarch64-apple-darwin.tar.gz` | `5f139821d67a0e25f023300123dc60cd5739d94584c57d92700030a2496ccd29` |
| `ferl-0.1.0-x86_64-apple-darwin.tar.gz` | `52240c6bf42512af0f9676e3c38a938da0161a7da146660831df1430a1d32a7b` |
| `ferl-0.1.0-aarch64-unknown-linux-musl.tar.gz` | `250faeee00ad129717f80347a75defbba628262a2f0d6b7f63d9a9a81b095b6b` |
| `ferl-0.1.0-x86_64-unknown-linux-musl.tar.gz` | `a41deaf5a54e20ae5141c9df4bdc6f1285e4a758b44cf8b8ade6a9ffddf672c3` |

Inspection results (AC19):

- Mach-O (both Mac targets): `LC_BUILD_VERSION` platform macOS, `minos 13.0`; only
  `/usr/lib/libSystem.B.dylib` and `/usr/lib/libiconv.2.dylib` are linked.
- ELF (both musl targets): `statically linked`; no interpreter string.
- Checksums verified with `shasum -a 256 -c SHA256SUMS`.
- Archives are not signed or notarized, and do not yet include an approved license file.

## 5. Native and live evidence

| Environment | Evidence | Status |
| --- | --- | --- |
| Apple Silicon Mac, macOS 26.7.1 (arm64), **native** | Extracted `ferl-0.1.0-aarch64-apple-darwin.tar.gz` and ran `--version`, validated `profile add --validate` with the local key on `FERL_API_KEY`, `profile check`, `list --query cli-demo`, `create` with title and tag, `delete` by bare code (0), repeated delete (6), in an isolated temporary configuration. | Native execution on macOS 26.7.1. **Not** the macOS 13 floor. |
| Intel Mac binary on the same Mac | `x86_64-apple-darwin` archive run under Rosetta: `--version`, profile add/list of `cli-demo`. | **Emulated** smoke only; not native Intel evidence. |
| macOS 13 floor (Intel and Apple Silicon) | — | Blocked (no machine). |
| Linux ARM64 and x86_64, kernel 4.18 floor, GNU and musl userspace | — | Blocked (no machines). Musl binaries are static and compile; they have not been executed. |
| Live local API at `http://127.0.0.1:18080` | `profile check`, `list` (`cli-demo` present, visit count unchanged at 5), create/list/delete with query and fragment preserved, duplicate destinations independent, cursor traversal of 3 links at limit 1, deleted custom-code reservation (409) and reserved `api` (409), wrong-key 401 with no creation, fault-proxy dropped create acknowledgement (one POST, exit 13, committed link found by list, cleanup). | Passed. Every per-run link was deleted; the residual check returned zero. |
| Owner manual test, reported 2026-10-29 | Owner reports manual profile setup works nicely and the link reaches the clipboard. | User-confirmed setup and clipboard success. Exact binary, OS/CPU, and byte-for-byte paste-back were not specified; this report does not establish the remaining platform matrix. |

The live suite and manual checks used the credential file only through
`FERL_API_KEY`; no key value was printed, logged, or committed.

## 6. Acceptance criteria (AC01–AC20)

Status key: **Verified** = automated or recorded evidence at the level the criterion
requires; **Partial** = some evidence, part of the criterion not yet exercised;
**Pending** = awaiting an environment or approval; **Blocked** = cannot be executed here.

| AC | Result | Evidence and remaining gap |
| --- | --- | --- |
| AC01 scope and parsing | Verified | `cli_process::invalid_invocations_fail_locally_with_zero_requests` (zero mock requests); help/version/no-argument tests; JSON usage errors. |
| AC02 wizard and selection | Partial | Owner reports successful manual profile setup (2026-10-29). PTY wizard add with hidden key and validation; PTY wizard cancellation exits 130 with no profile committed (`pty_commands`); menu inspect and exit; default and `--profile`/`FERL_PROFILE` precedence with distinct servers and keys; default cleared on removal. Real two-account live workflow is an owner-approved non-blocking 0.1.0 follow-up (PR08). |
| AC03 persistence and recovery | Partial | Private file and symlink refusal, unsafe mode not repaired, unknown schema rejected, credential binding enforced, concurrent writers keep all six adds. Deterministic interruption at each atomic rename (before and after) for add, set-key, server change, and removal leaves every present profile usable with its own key, never another profile's (`profile::interruption_tests`, test-only hooks). This found and fixed a server-change window in which the prior key was pruned before the config switched. External key rotation on a real disposable account is an owner-approved non-blocking 0.1.0 follow-up (PR08). |
| AC04 automation | Verified | Missing env and unsafe key file exit 3; two key sources and override without `--profile` exit 2; runtime key overrides never persist; delete without consent makes no request; JSON never prompts. |
| AC05 create metadata | Verified | Exact request bodies; shorthand equals create; title and tag bounds (200/201, 20/21) unit-tested; live duplicate destinations independent and server canonicalization (`Work` → `work`) observed. |
| AC06 URL preservation | Partial | Query and fragment preserved in the request and the live response; returned `short_url` printed unchanged; destination validation corpus unit-tested (whitespace, user information, schemes, ports, length); private, IPv6, and IDN destinations accepted without fetching. Remaining: a live server whose public origin differs from its API origin (not available; the local server returns API-origin short URLs). |
| AC07 list and search | Partial | Own scope, exact `q` encoding, default limit, server order preserved, and no detail calls are tested. Golden human table for default columns, a null title, and an exact 2^53+1 count is pinned in `human_table_golden…`; layout and truncation are unit-tested. |
| AC08 integer and schema fidelity | Verified | Counts 9007199254740993 and i64::MAX round-trip; overflow, fractions, negatives, offset timestamps, and missing fields rejected; management tokens never emitted; `schemas/` validated against real output (`schema`). |
| AC09 pagination | Verified | 401 links at limit 200 in server order (`fetch_all_follows_cursors…`); empty page with continuation; 87,387-byte cursor through file, request, and output; repeated cursors stop; default one page advertises continuation; live traversal. A changed-context cursor on a later page returns the server 400 as a partial result (exit 12, stop reason `error`, no restart from page one); on the first page it is exit 2 with empty stdout (`http_contract`). |
| AC10 partial and bounded output | Verified | Page-two failure gives valid partial JSON with exit 12; first-page failure has empty stdout; `--max-pages` bound; repeated cursor stop; SIGINT during a later page gives exit 130 with partial output and removes staging. Measured on the release binary (§11): 200 MiB spool peak RSS 6–18 MiB on all four hosted runners; 256 MiB aggregate bound stops with exit 12 and 540 accepted pages; staging dir 0700, file 0600; no staging left behind. |
| AC11 delete targeting | Verified | Raw-input corpus (aliases, suffixes, encodings, dot segments, UUIDs, foreign origins) unit-tested; bare code and public short URL each send exactly one bodyless DELETE; empty 204 consumed; consent required without a TTY. Interactive confirmation has no PTY test yet. |
| AC12 delete consequences | Verified | Live: duplicate destinations, delete one, the other remains; repeated delete exit 6; deleted code reserved (409) through the authorized HTTP boundary. |
| AC13 error handling | Verified for list rejections | Mapped and tested: 401, 404, 409, 429 with Retry-After, 500, 503, redirects, malformed bodies, unstructured responses. Fixtures cover 400, 403, 405, 413 and 415 with exit codes 2, 5, 11, 2 and 11, one attempt each, and no stdout. Stderr goldens for 401 and 429 (with Retry-After) are pinned in `stderr_golden…`. |
| AC14 uncertainty | Verified | Dropped create ack: one POST, exit 13, unknown outcome, not replayed (fixture and live fault proxy); dropped delete ack: one DELETE, exit 13; 201 with unusable body: exit 11, committed; closed output pipe after commit: exit 14, committed, no second POST; later 404 is never a success. |
| AC15 transport policy | Verified | Certificate chain and hostname verification (untrusted and mismatched names fail before any request); private CA bundle succeeds; CONNECT proxy used and `NO_PROXY` bypass; `http_proxy` ignored; redirects not followed and no credential to the redirect target; `localhost` rejected for plaintext; GET attempts ≤ 3, POST/DELETE exactly 1; 0.5 s deadline honoured. |
| AC16 clipboard | Partial | Owner reports the link reaches the clipboard (2026-10-29). A disposable hosted macOS round trip (`clipboard_desktop`, Apple Silicon and Intel) confirms that explicit `--copy` leaves exactly the returned URL on the clipboard. Helper runner unit-tested (exact bytes on stdin without a shell, fall-through, missing helper nonfatal, slow helper bounded to 2 s). Linux Wayland and X11 helper checks are deferred by owner decision as a 0.1.0 follow-up. |
| AC17 streams and terminal safety | Partial | No prompts or progress on stdout; JSON stderr lines parse; sentinel key scans; control and bidi escaping and TTY grapheme truncation unit-tested. PTY tests cover hostile titles, a null title and a 2^53+1 count in a 100-column terminal, and interactive delete (default No: zero requests; confirmation: one DELETE). Remaining: stored golden transcripts. |
| AC18 four native runtimes | Non-blocking follow-up | Owner explicitly removed the missing exhaustive native/floor checks as a 0.1.0 release blocker (2026-10-29). Existing native evidence is macOS 26.7.1 (arm64); see §5. Prepared CI adds current-platform checks on all four CPUs/OS combinations; first hosted execution is pending. |
| AC19 binary packaging | Verified for candidate | Four archives with LICENSE and target-specific notices; safe paths, executable mode, checksums and manifest verified in hosted smoke jobs; static musl linkage (no interpreter or DT_NEEDED) and macOS 13.0 deployment target with only libSystem and libiconv checked during packaging. Mac signing and notarization are declined for 0.1.0. |
| AC20 Homebrew lifecycle | Verified for candidate (file URL rehearsal) | Hosted `macos-15` (ARM64) and `macos-15-intel` runs passed style, install, upgrade from a 0.0.0 rehearsal, test, uninstall, and profile byte/mode/owner comparison; `brew audit --strict` on the release formula with HTTPS URLs and exact checksums. Public tap installation is not yet verified (§11). |

## 7. Milestones

| Milestone | State | Notes |
| --- | --- | --- |
| M0 baseline and interface review | Done | §1. |
| M1 feasibility and contract foundation | Done (local) | Toolchains, MSRV-compatible lockfile, four target builds, static musl, Mac 13 deployment target, large cursors transported (§1 D-IMPL-02). |
| M2 profiles, storage, shared contracts | Done (local) | Profiles, key sources, storage and binding, wizard, JSON/error contracts. |
| M3 release-one link workflows | Done (local) | Create (metadata, clipboard policy), list (columns, cursors, fetch-all), delete (strict targets, consent). |
| M4 failure hardening and integration | Partial | Deterministic HTTP, TLS, PTY, signal, and filesystem suites and the live suite pass. Pending: AC10 memory bound and remaining AC16/AC17 native/PTY checks. AC18 and extra two-account/key-rotation live scenarios are non-blocking follow-ups. |
| M5 packaging and local tap rehearsal | Partial | Archives, checksums, manifest, and formula rehearsal done. Pending: `brew style`, tap-based install and upgrade. |
| M6 publication | Setup pending | Policies and tag-triggered publication approved. Local workflows prepared. Approved repository setup, tap secret, first hosted results and remaining acceptance checks precede the first release tag. |

## 8. Remaining release prerequisites

| ID | Status |
| --- | --- |
| PR03 client repository owner, name, source-host policy, license | Decided: public `ai-slop-code/ferl-cli`; MIT, copyright 2026 Lubos Babjak. Local LICENSE and Cargo metadata added. |
| PR04 tap owner, formula name, homepage, artifact host, immutable URL convention | Decided: public `ai-slop-code/homebrew-tap`, formula `ferl`, CLI repository homepage, GitHub Releases under `releases/download/v<version>/`. |
| PR05 version and tag policy, signing and notarization policy | Decided: independent SemVer from `0.1.0`, `v` tags, immutable archives/checksums, unsigned Mac release. |
| PR06 CI provider, native runners, minimum-OS machines | Decided: GitHub Actions hosted runners. Local workflows prepared; hosted execution pending. Missing AC18 floors are non-blocking. |
| PR07 publishing method, credentials, Apple identities | Decided: automated tag publication and tap update. Scoped `HOMEBREW_TAP_TOKEN` needs GitHub setup; no Apple identity needed. |
| PR08 additional real accounts, a second server, private-CA fixture | Local API and deterministic private-CA fixtures available. Extra two-account/key-rotation live scenarios are non-blocking follow-ups. |
| PR09 acceptance reviewer and evidence location | Maintainer release-tag action; evidence in this repository and Actions logs/artifacts. |
| PR11 Homebrew compatibility environments on both Mac architectures | Hosted `macos-15` (ARM64) and `macos-15-intel` selected; lifecycle workflow prepared, execution pending. |

## 9. Open items and blockers, for the release decision

1. Set up the approved GitHub repositories, enable Actions and configure the scoped
   tap secret. Review first hosted build/test and Homebrew lifecycle results (AC20).
2. AC10 spool peak-memory measurement at 200 MiB and the 256 MiB aggregate-bound run.
3. Remaining platform-specific clipboard evidence (AC16); owner-confirmed success
   is recorded in §5. Check actual unsigned Mac downloads and their installation UX.
4. Remaining local acceptance gaps in §6, including rename fault injection, terminal
   rendering, interactive delete confirmation and wizard cancellation through a PTY.
5. Non-blocking 0.1.0 follow-ups: AC18 exhaustive native/minimum-OS verification and
   AC02/AC03 additional two-account/external-key-rotation live scenarios.

Resolved: license, repository/tap/download identity, version/tag/signing policy,
CI provider and publication method. The reported contract-hash discrepancy was a
comparison against the wrong file; both imported server contract hashes match (§1).
The [release guide](client-release.md) is the concise setup and release reference.

## 10. Owner-approved release setup and local verification

Completed locally on 2026-10-29:

- Added MIT `LICENSE` (Lubos Babjak) and Cargo description/license/repository metadata.
- Updated package generation to include LICENSE, README, target-specific locked
  dependency license/notice texts, normalized archive timestamps/ownership/modes,
  and correctly distinguished client/server specification hashes.
- Prepared `.github/workflows/check.yml` and `release.yml`: four current native
  host test jobs, four-target packaging, native archive smoke, two-Mac Homebrew
  lifecycle rehearsal, tag-triggered release and checksum-bound tap update.
- Added checksum-verifying formula rendering and an isolated, CI-only Homebrew
  install/upgrade/uninstall test with synthetic profile preservation checks.
- Updated README, spec and plan to reflect D08/D10/D11. The short operational
  reference is `docs/client-release.md`.

| Local check | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo test --locked` | Passed: 150 offline tests; 8 live tests ignored, previous live evidence retained |
| `actionlint` 1.7.12 | Both workflow files passed; older 1.7.7 did not recognize GitHub's current `macos-15-intel` label |
| Shell/Python syntax | Packaging, formula rehearsal, CI Homebrew script and Python helper syntax passed |
| Updated four-target packaging | Passed; local output `dist/approved-0.1.0/` |
| Archive inspection | Four checksums, safe relative paths, regular files/directories, executable mode, normalized ownership, exact MIT license, dependency notices, and separate contract hashes passed |
| Formula rendering | Approved HTTPS URLs and Ruby syntax passed; deliberately modified archive rejected before writing a formula |
| Local Mac formula rehearsal | Ruby syntax and native ARM64 archive help/version passed; `brew style` still blocked by Homebrew's Ruby/gem certificate environment |

Hosted workflows, Homebrew lifecycle execution and public publication have not run.
Read-only GitHub checks could not resolve either approved repository through the
current credentials. No remote repository, release or publishing secret was created.

## 11. Release v0.1.0 verification (2026-10-29)

| Item | Result |
| --- | --- |
| Tag and source | `v0.1.0` at commit `68134c6ace76d6f18a6ffeb2ee1494aeb21f8350` (manifest `source_revision`). |
| Release | [v0.1.0](https://github.com/ai-slop-code/ferl-cli/releases/tag/v0.1.0), published (not draft) from `Release` run `40975018640`. |
| Candidate checks | `Check` runs for the same source: `40973742085` (all 4 test jobs, 4 smoke jobs, package job green). The release run's check reused the same workflow and also passed. |
| Public assets | Four archives, `SHA256SUMS`, `manifest.json`, `ferl.rb`. Anonymous download bytes are identical to the run's retained artifact. `SHA256SUMS` verifies. |
| Manifest | CLI `0.1.0`, JSON schema `1`, MIT, `signing: NOT_APPLIED`, `rustc 1.90.0`, OpenAPI and server-spec hashes match the server contract record. |
| Native archive (arm64) | Extracted the public `aarch64-apple-darwin` archive on macOS 26.7.1 (arm64). `--version` is `ferl 0.1.0`; `--help` runs. Packaged-binary smoke against the approved local API, isolated configuration: profile add and check, create with a title, list (one match), delete by exact code, zero residual. |
| Homebrew tap | Verified. The first publish attempt updated the release, then the tap push failed (HTTP 403) because `HOMEBREW_TAP_TOKEN` lacked write access to the tap. After the owner replaced the token with an organization token, rerunning only the failed publish job (`40975018640`) verified the published bytes and pushed `ai-slop-code/homebrew-tap` commit `fc5d6e6` (`github-actions[bot]`, `ferl v0.1.0`). `Formula/ferl.rb` is byte-identical to the release asset. |

Gates resolved or deferred by owner decision (2026-10-29):

- Unsigned Mac browser-download and Gatekeeper behavior: the owner will run the check after the tag, on a user-assisted Mac, and record the result. This moves the handoff gate from before to after the tag.
- Linux Wayland and X11 clipboard helper smoke tests (AC16): deferred as a 0.1.0 follow-up.
- AC18 exhaustive native and minimum-OS checks, and the additional AC02/AC03 two-account and externally rotated-key scenarios: approved non-blocking follow-ups.

Public installation (`public-install` workflow, manual run `40977320727`, hosted runners):
`brew install ai-slop-code/tap/ferl` on `macos-15` (arm64) and `macos-15-intel` (x86_64)
installed `ferl 0.1.0`, the expected architecture, passed `brew test`, and uninstalled; a
synthetic profile's config and credential files were byte-identical, with the same modes
and owners, after uninstall. No account or network business operation ran at install time.

Open follow-up: a private CA generated with OpenSSL 1.1.1w was rejected by the rustls
trust-anchor parser on the hosted macOS runners. The cause was not diagnosed; the
TLS fixtures now require OpenSSL 3, matching the other runners.
