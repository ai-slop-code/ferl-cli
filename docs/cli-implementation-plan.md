# ferl CLI: phased implementation and release plan

Date: 2026-10-29. This is the implementation plan, updated with the owner's release
decisions. See the [acceptance record](client-implementation-status.md) for actual
completion and the [release guide](client-release.md) for the current release process.

Companion requirement source: [client business specification](../spec/cli-business-spec.md).
The imported [OpenAPI](../spec/openapi.yaml) and
[server business specification](../spec/server-business-logic.md) remain the joint
server contract. This plan implements the client's agreed subset through HTTP.

The [README](../README.md) provides current build and live-test commands. The
[release guide](client-release.md) describes repository setup and publication.

## 1. Delivery objective and scope control

Deliver a standalone Rust `ferl` executable for four native OS/CPU combinations.
An existing account holder must be able to:

1. Add and switch named profiles with an interactive wizard or scripted commands.
2. Shorten one URL, optionally adding a title/tags; receive the server's short URL
   and automatic interactive clipboard copy.
3. List/search own links with selectable human columns or stable JSON, including
   explicit continuation and bounded fetch-all.
4. Delete one code or associated short URL with documented confirmation semantics.

The owner clarified title/tag options and column selection into this release,
and custom codes/advanced filters out. Deferred command families are listed in
business spec §2; no implementation phase below silently adds them. Profile
validation uses the existing `getCurrentAccount`, not a new account command family.

The planning sequence is specification → decision/dependency gate → client
implementation → verification → packaging rehearsal → separately authorized
publication. File delivery of these two documents completes the analysis output,
not the implementation or any distribution prerequisite.

## 2. Prerequisites and accountable decisions

The following decisions and setup tasks are tracked separately from test results.

| ID | Prerequisite / decision | Accountable role | Gate |
| --- | --- | --- | --- |
| PR01 | Core scope, refinements, clipboard, private files, platform/transport baseline | Product owner | Confirmed in business spec D01–D07. |
| PR02 | Review analyst-proposed exact grammar, schemas/exits, profile binding, retry/size limits, and toolchain direction | Product owner + client implementer | M0 sign-off before freezing public interfaces. |
| PR03 | Public `ai-slop-code/ferl-cli`; MIT, copyright 2026 Lubos Babjak | Owner | Decided. Repository setup remains. |
| PR04 | Public `ai-slop-code/homebrew-tap`, `Formula/ferl.rb`; CLI repository homepage and GitHub Releases | Owner | Decided. Versioned archive URLs under `releases/download/v<version>/`. |
| PR05 | Independent SemVer from `0.1.0`, `v` tags, immutable archives/checksums; unsigned Mac release | Owner | Decided. No Apple identity required for 0.1.0. |
| PR06 | GitHub Actions hosted runners; local workflow preparation approved | Maintainer | Decided. First hosted runs pending. AC18 floor/exhaustive native checks are non-blocking for 0.1.0. |
| PR07 | Automated tag release and tap update | Maintainer | Decided. Actions token publishes releases; scoped `HOMEBREW_TAP_TOKEN` still needs configuration. |
| PR08 | Reachable isolated existing server, synthetic accounts/keys, separate-profile fixtures, private-CA fixture, and permission for destructive synthetic tests | Server/test-environment owner | Partially satisfied: owner supplied localhost API, private enabled-admin credential and `cli-demo`; readiness/account/search checks passed. Additional real account/server/CA fixtures and full integration evidence remain pending. |
| PR09 | Maintainer reviews evidence and pushes release tag; records in repository and Actions | Maintainer | Tag publication is the release action; no separate approval workflow. |
| PR10 | Dependency/compiler/MSRV/license compatibility, static-musl and Mac deployment-target proof | Client implementer | M1 gate before feature build-out. |
| PR11 | Hosted Apple Silicon and Intel Mac Homebrew lifecycle runs | Maintainer | CI environments selected; first passing results pending. Homebrew remains first-release work. |

The original handoff supplies contract files; the owner's subsequent local testing
handoff additionally supplies the API and private credential described below.
Repository setup and publishing credentials still need execution; their policies
and identities are now approved. Additional PR08 two-account/key-rotation live
scenarios are owner-approved non-blocking follow-ups for 0.1.0.
Completed backend implementation is not an outstanding task here. The client does
not require database access or a cursor encryption key.

### Available local integration environment

As of 2026-10-29, `http://127.0.0.1:18080` is reachable. Readiness, authenticated
`GET /api/v1/me`, and own-scope search for `cli-demo` returned HTTP 200. The expected
account `ferl_cli_admin` is enabled with admin role. Its key remains private in
`~/.config/ferl-cli-local/profile.env` (verified current-user ownership, regular
file, mode 0600). The developer handoff documents credential-safe test setup and
the required loopback-HTTP profile option.

This satisfies basic local connectivity/authentication prerequisites for M2–M4.
The supplied ARM64 server package is not four-platform client execution evidence.
Additional real-account/server/CA fixtures, native runtime machines, an acceptance
reviewer and release infrastructure remain separate requirements.

## 3. Measurable acceptance criteria

Every applicable row must have an automated result or recorded native/manual
evidence. Success claims must include the CLI source/build revision, contract
hashes, target, OS/kernel version, and execution date. Expected codes refer to
business spec §7.4; an uncertainty/partial-output override takes precedence there.

Owner-approved 0.1.0 exceptions: AC18's missing exhaustive native/floor evidence
and AC02/AC03's additional two-account/externally rotated-key live scenarios do
not block publication. Preserve their gaps as follow-ups; other AC gaps retain
their requirements. The criteria below describe the full verification scope.

| ID | Observable acceptance criterion | Required evidence |
| --- | --- | --- |
| AC01 — scope and parsing | Help exposes only approved commands/options. URL shorthand and `create` produce identical requests. Deferred flags/unknown subcommands, duplicate scalars, extra URLs, blank required values and invalid option combinations exit 2 before any HTTP. Help/version work with no config/network. | Process-level parser/CLI fixtures with HTTP request count zero on invalid/local-only cases. |
| AC02 — wizard and selection | Starting `ferl profile` with empty config adds a profile through hidden key entry; subsequent wizard use switches default. With two servers and two accounts, explicit flag > `FERL_PROFILE` > default selects exactly the expected origin/key/CA for every request. Profile list/show never reveal keys. Removing default clears selection; no auto-fallback. | PTY wizard tests plus distinct sentinel-key HTTP fixtures and two-profile real-server workflow. |
| AC03 — persistence and recovery | Config/credentials are private and UUID/origin-bound. Symlinks, unsafe mode/ACL, unknown schema and inconsistent credential bindings exit 3. Failure injected before/after each local rename leaves either a usable prior/new profile or a diagnosable state, never another profile's key. Concurrent writers do not lose committed updates. Externally rotated key replacement preserves link access. | Filesystem fault/concurrency tests, platform permission checks, and real-server old/new key scenario with externally prepared keys. |
| AC04 — automation | Missing key/profile is exit 3; missing delete consent is exit 2 with zero requests. `--key-stdin`, private key-file, and explicit key-env sources work without echo/prompt; runtime overrides require an explicit profile and never persist. JSON mode never blocks for a prompt. | Non-TTY/closed-stdin subprocess tests with deadlines; config comparison before/after credential overrides. |
| AC05 — minimal/metadata create | POST contains only required destination plus supplied title/tags; omitted options produce no null/extra members, and no `short_code`. Title trimming/null omission and Unicode tag examples round-trip correctly; comma-containing tags stay one value, 21 inputs fail even if identical. Two identical destinations create two independent links. | Wire-body/schema fixtures plus real-server create/list checks. Local bounds test 200/201 title scalars and 20/21 tag inputs; server-authoritative normalization cases. |
| AC06 — URL preservation | Query and fragment survive creation; stdout and clipboard use exact returned `short_url` even when public/API origins differ. Allow internal, localhost, private IPv4, IPv6 and IDN destinations without destination network access. Reject whitespace, empty/nonempty user-info, invalid ports, non-HTTP schemes, and >8,192-byte input/serialization. | URL boundary/fuzz corpus, resolver/request spy, multibyte length cases, real-server round trips. |
| AC07 — list/search/columns | Default/admin-key listing sends only own scope. Literal search values containing `%`, `_`, backslash, `&`, `+`, comma, Unicode are encoded exactly once. Server ordering is unchanged; selected columns use returned fields with no detail/analytics calls. Null titles, disabled links, and enabled-vs-active differences are displayed correctly. | Exact query fixtures, API request-count assertion, golden human output, own-scope real-server examples. |
| AC08 — integer/schema fidelity | JSON has all 12 Link fields, required nulls/empty arrays, UTC strings and exact counts including `9007199254740993` and `9223372036854775807`. Count overflow, missing fields and malformed success JSON cannot become success data. Unexpected management tokens are never emitted. | Checked-in client JSON Schemas, typed round-trip/schema/golden tests and malformed-response fixtures. |
| AC09 — pagination correctness | A 401-link unchanged dataset at limit 200 returns all 401 once in server order and terminates only at null. An empty page with non-null cursor continues. Large 87,387-byte token round-trips through JSON file/request/output without truncation. Changed-context cursor returns 400 without restarting. Default one-page output advertises continuation. | Multi-page synthetic HTTP fixtures and real-server small-limit traversal; actual server tokens where available, large-token transport fixture for capacity. |
| AC10 — partial/bounded output | A page-two failure after valid page one emits parseable partial JSON, exit 12, retained items and retryable continuation; first-page failure leaves stdout empty. Page/deadline/256 MiB aggregate bounds stop honestly. Repeated cursor does not loop. Cancellation gives 130 unless write uncertainty applies. | Deterministic page/fault/time tests; output schema validation; spool cleanup and request-count checks. Peak resident memory below 128 MiB for a 200 MiB synthetic list spool. |
| AC11 — delete targeting | Bare code and correctly associated short URL cause exactly one bodyless DELETE to `/api/v1/links/{code}`. No preflight/public redirect calls occur. Reject foreign origin, suffixes, encoded/uppercase aliases, trailing/extra/dot segments and UUIDs locally. Consume empty 204 without JSON parsing. | Raw-input security corpus, HTTP capture, PTY confirmation and noninteractive `--yes` tests. |
| AC12 — delete consequences | Create duplicate destinations, delete one, and observe the other still listed. Repeated delete returns 404/exit 6, described as unknown or inaccessible. A test fixture using the existing API confirms the deleted code stays reserved, without adding a custom-code CLI option. | Real-server HTTP-boundary workflow with isolated synthetic data; reservation assertion via authorized test harness, not direct database access. |
| AC13 — error handling | Cover 400/401/403/404/409/413/415/429/500/503, global 405 and unexpected statuses. Preserve safe code/status/request ID and valid Retry-After. Handle HTML/empty proxy errors, invalid JSON, missing/mismatched IDs, and unknown codes with sanitized stderr and documented exit. 401/403 never trigger anonymous/profile fallback. | Table-driven HTTP fixtures per applicable operation and stdout/stderr/exit goldens; request count assertions. Unsupported create-code behavior tested as a response fixture, not an exposed flag. |
| AC14 — uncertainty | Drop a create response after real commit but before acknowledgement: exactly one POST and exit 13, no claim that nothing changed. Drop delete acknowledgement: one DELETE and exit 13. Later 404 is not retroactive success. Received 201 with bad body reports committed/protocol failure; output failure after success reports committed/14. | Fault-injecting HTTP proxy over an isolated real server, list-based observation, transport fixtures for pre-dispatch vs possible-dispatch failures. |
| AC15 — transport policy | Verify cert/hostname, explicit private CA, no API redirect following, literal-loopback HTTP opt-in and proxy bypass, proxy environment precedence, and zero key leakage to redirect/destination/other profile hosts. Default GET attempts ≤3; POST/DELETE attempts exactly 1. Default HTTP work ends within 30 s plus 1 s cancellation/scheduling tolerance. | TLS/proxy fixture servers, credential sinks, controllable clocks for backoff and wall-clock subprocess deadline tests. |
| AC16 — clipboard | On both Macs, successful interactive shorten copies exactly the returned URL and it remains available after process exit. Linux Wayland and X11 helper paths work; headless/missing helper emits warning, retains short URL, exit 0. Non-TTY/JSON does not copy by default; explicit copy/opt-out works. URL data is never shell-interpreted. Clipboard phase completes in ≤2 s plus 0.5 s scheduling tolerance. | Native clipboard smoke evidence, helper spies, headless and slow/missing-helper process tests. Linux helper matrix is tested across available native desktop environments; core CLI acceptance still runs on both Linux CPUs. |
| AC17 — stream/terminal safety | No prompts/progress on stdout; JSON stderr is valid JSON Lines. Control/OSC/ANSI/bidi content is rendered as data; no secrets appear in output, errors or helper argv. TTY truncation preserves grapheme boundaries; non-TTY human output and JSON preserve full values. | PTY/non-TTY golden tests and sentinel scans over captured diagnostics; pipe/write-failure tests. |
| AC18 — four native runtimes | Each actual packaged binary runs help/version, profile add/check, create with metadata, list/search/continuation and delete against an isolated real server on its own native OS/CPU. Verify macOS 13 floor on Apple Silicon and Intel; Linux kernel 4.18 floor on both CPUs plus contemporary GNU and musl userspaces. Cross-compilation/emulation alone is insufficient. | Four signed-off target reports with native machine/VM architecture and OS/kernel details; minimum-floor evidence and current-userland smoke runs. |
| AC19 — binary packaging | Four archives extract safely, executable mode is preserved, checksums/manifest match, compiler/contract versions are recorded and license is approved/included. Linux has no dynamic ELF interpreter/DT_NEEDED entries; Mac links only allowed system libraries with 13.0 deployment target. Users need no Rust/system OpenSSL. | Artifact inventory, ELF/Mach-O inspection, clean-environment smoke tests and checksum verification; signing/notarization evidence if policy approved. |
| AC20 — Homebrew lifecycle | Authorized formula selects the correct immutable Mac archive/checksum. Formula syntax/audit/offline version/help tests pass; install and upgrade work on native ARM64 and Intel. Existing profile/default/key files are byte-identical across package upgrade/uninstall, with mode/ownership preserved. | Local tap rehearsal first; actual approved tap checks only after prerequisites. Native install/upgrade/uninstall transcripts with secret contents excluded. |

## 4. Implementation approach and meaningful test layers

### 4.1 Proposed client structure

Use small Rust modules with clear responsibilities, chosen to support the above
observable requirements rather than coupling to server code:

- CLI/parser and explicit interactive profile wizard.
- Typed config/profile resolution and private-file credential store.
- HTTP transport and a typed adapter for just `getCurrentAccount`, `createLink`,
  `listLinks`, `deleteLink` plus their required schemas.
- Domain input validation and deletion-target resolution.
- Pagination coordinator, result/error classification and output staging.
- Human/JSON renderer and clipboard adapter.

Candidate dependencies: `clap` for grammar, `serde`/`serde_json` and TOML tooling,
`url` for standards-based URL work, `reqwest` with Rust TLS (and carefully disabled
automatic redirect/retry behavior), maintained Unicode utilities, secure prompt
input, and temporary-file/locking support. Choose exact versions only after the
M1 MSRV/static-linking review. No OpenAPI generator is required: hand-written typed
models for the small subset are acceptable when tested against the pinned schemas.
Do not import server crates or generate all 22 command families.

Proposed edition/MSRV/initial build compiler: 2024 / 1.85.0 / 1.90.0, as explained
in business spec §11. Pin toolchain and dependency lockfile during M1, verify both
MSRV and release compiler builds, and review the maintained release compiler before
public distribution. Rust TLS reduces libc/OpenSSL packaging variability; static
musl reduces Linux runtime installation dependencies. Keep platform-specific
filesystem/clipboard code behind narrow interfaces.

### 4.2 Verification layers

1. **Contract baseline checks:** hash imported files, resolve local OpenAPI refs,
   validate relevant request/response examples, and maintain operation-to-CLI
   traceability. Contract schemas are authoritative for wire shapes; business
   invariants such as byte bounds and unknown-vs-inaccessible need separate cases.
2. **Unit/property tests:** raw URL/path validation, origin comparison, Unicode
   length/canonicalization boundaries, query encoding, precise integers, precedence,
   result/error classification, cursor-file formats, and failure/cancellation states.
   Include generated raw-path/control-character cases where parser normalization
   might conceal invalid input.
3. **Subprocess/PTY tests:** test the built command as users invoke it. Assert
   stdout, stderr, exit status, files and HTTP request count, including absent
   terminals and secrets in hostile error paths. PTY tests cover wizard navigation,
   hidden entry, default-No confirmations and clipboard auto-mode detection.
4. **Deterministic HTTP fixtures:** valid/status/error/schema responses, exact
   serialized requests, long cursors, retry timing, large counts, proxy/TLS failures,
   and dropped/truncated responses. Spy endpoints prove that credentials never
   leave the selected origin. Stub timing is not a substitute for wall-clock tests.
5. **Real-server integration:** run the actual CLI with synthetic accounts/data
   against an existing isolated server at the pinned boundary. Test profile
   validation, metadata create, duplicate destinations, list/search, live
   continuation, authorization failures, deletion and external key replacement.
   Provisioning/fixture preparation belongs to the environment owner or an authorized
   HTTP test harness; it does not become CLI features or require direct database
   access. Keep the client out of server bootstrap/migration/deployment design.
6. **Native/package evidence:** actual target binaries on native OS/CPU machines
   or native-architecture VMs, including minimum runtime floors, followed by both
   Mac formula lifecycle runs. A Linux ARM cross-build or Rosetta/QEMU run may help
   development but cannot satisfy native execution acceptance.

Use a fault proxy that forwards a mutation and deliberately discards its successful
response to reproduce after-commit uncertainty without changing backend code.
Inspect state through existing HTTP operations. Keep cleanup and test permissions
limited to the synthetic data owned by that test run; do not require production
credentials. Rate-limit tests should respect fixture-account budgets, use fixtures
for exhaustive rejection matrices, and avoid relying on fixed production limits.

Future-option tests belong to their later feature work. Existing endpoints may be
used by an authorized fixture harness to prepare data (e.g. disabled links) without
adding corresponding client commands or reopening backend acceptance work.

### 4.3 Required checks when implementation exists

- `cargo fmt --check`, Clippy with warnings denied for client-owned targets, and
  meaningful unit/process/contract tests using the committed dependency lockfile.
- MSRV and selected build compiler checks; four target builds; dependency license
  compatibility review before redistributing archives.
- Fixture transport/security and real-server workflow suites; every AC row linked
  to evidence or an explicitly unresolved blocker.
- Package checksum/layout/dynamic-linkage inspection; native runtime and Mac
  formula lifecycle evidence; reviewed release manifest and pending-decision list.

These are future checks, not claims that Cargo manifests, test runners, or hosted
workflows exist now. Local reproducible scripts can precede CI authorization;
creating a hosted workflow still requires PR06/PR07.

## 5. Phased milestones and exit gates

Indicative effort is engineering days for one experienced Rust implementer with
part-time product/QA/release review. Ranges exclude waiting for decisions, hardware,
signing, credentials or test servers, and are estimates rather than a delivery date.
Independent local test/packaging work can overlap after interfaces stabilize.

| Milestone | Work and deliverables | Dependencies | Exit gate | Estimate |
| --- | --- | --- | --- | --- |
| M0 — specification and decisions | Deliver business spec and plan; record confirmed scope and open proposals/prerequisites. Review grammar, schemas, storage and release boundaries. | Handoff + owner clarifications supplied. | Analysis documents delivered; PR02 decisions recorded before freezing interfaces. No client code required for this milestone. | Analysis delivery now; review timing owner-dependent. |
| M1 — feasibility and contract foundation | Establish local Rust project after M0 review; pin compiler/MSRV/deps; typed subset models, schema fixtures and transport skeleton; prove static musl, Mac deployment floor, large-cursor request capacity and integer fidelity. | PR02; PR10 resolved during phase. PR03 before remote creation only. | Locked build plan, four compile artifacts, MSRV/dependency report, core contract/transport fixtures. Document native tests still pending. | 3–5 days. |
| M2 — profiles and interactive shell experience | Private storage/binding, atomic recovery, key sources/precedence, profile add/use/show/list/set/set-key/check/remove, wizard, hidden entry and validation; stable error/JSON scaffolding. | M1. | AC01–AC04 local/process fixtures; no key leakage; offline and noninteractive paths verified. Real key replacement can wait for PR08. | 4–6 days. |
| M3 — release-one link workflows | Create with title/tags; human/JSON output and clipboard; own-scope list/search, columns/cursors/fetch-all; strict delete URL parsing, consent and empty 204. Add CLI schema artifacts and user examples. | M2 + stable HTTP adapter. | AC05–AC12 and AC16–AC17 deterministic cases pass; command-to-operation request inventory is exactly the approved subset. | 5–7 days. |
| M4 — failure hardening and real-server acceptance | Implement/verify retry deadlines, partial output, permissions/concurrency faults, TLS/proxy isolation, uncertain writes and native real-server flows. Gather current-OS evidence and resource measurements. | M3; selected CI and available live API. | AC01–AC18 evidence, subject to the owner-approved 0.1.0 exceptions above; no cross-build is labeled native success. | Original estimate: 5–8 days plus access wait. |
| M5 — packaging and local tap rehearsal | Produce four archives/manifest/checksums; finalize license/version/signing policy; prepare prebuilt-binary formula template, then real URLs only after identities approved. Rehearse both-Mac install/upgrade/uninstall and signed-download validation if applicable. | M4; PR03–PR05, PR10/PR11; PR07 for signing. | AC19–AC20 pass against candidate artifacts; release manifest reviewed; no public publication implied by rehearsal. | 3–5 days plus policy/access wait. |
| M6 — authorized publication | Publish approved immutable artifacts, verify retrieval/checksums, update the approved tap, run installed-package smoke checks and release notes. | M5; all identity/license/CI/publisher/signing prerequisites satisfied and explicit release approval. | Release owner records four artifact URLs, final hashes, native evidence, tap revision and published installation instructions. | 1–2 days plus approval wait. |

Estimated implementation/release execution after M0: **21–33 engineering days**,
subject to the dependency/runner findings. No remote milestone is marked complete
by the handoff or by creation of these local planning files.

## 6. Packaging and Homebrew work breakdown

### Binary work package

1. Compile release binaries with generic CPU baselines, deterministic inputs and
   recorded compiler/target information. Inspect Linux static linkage and macOS
   deployment target/system-library dependencies.
2. Run native workflow acceptance using the candidate executable itself. Keep a
   target report for each of:
   `aarch64-apple-darwin`, `x86_64-apple-darwin`,
   `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`.
3. Apply the approved Mac signing/notarization procedure, if adopted. Verify the
   final downloaded executable/archive behavior on clean Mac environments; do not
   assume a Linux checksum proves Mac Gatekeeper acceptance.
4. Package `ferl-<version>-<target>.tar.gz` with executable, approved license,
   notices and README. Generate final SHA-256/manifest from the exact signed or
   unsigned artifacts that will be distributed; never checksum an earlier build.
5. Freeze the candidate asset set and verify extraction, version/help, and native
   smoke tests. Do not mutate already released assets; fixes get a new version.

### Formula work package

PR04 is resolved. The formula is rendered from the packaged archives using
`scripts/render-formula.py`; the following identities are approved:

```text
tap repository: ai-slop-code/homebrew-tap
formula file: Formula/ferl.rb
homepage: https://github.com/ai-slop-code/ferl-cli
version: <APPROVED_CLI_VERSION>
license: MIT
ARM Mac url: <IMMUTABLE_ARTIFACT_BASE>/ferl-<version>-aarch64-apple-darwin.tar.gz
ARM Mac sha256: <FINAL_ARM_ARCHIVE_SHA256>
Intel Mac url: <IMMUTABLE_ARTIFACT_BASE>/ferl-<version>-x86_64-apple-darwin.tar.gz
Intel Mac sha256: <FINAL_INTEL_ARCHIVE_SHA256>
```

Version and checksums are generated from the exact candidate assets.
Formula behavior is specified in business spec §11: architecture selection,
macOS Ventura floor (subject to Homebrew's support policy), `bin.install "ferl"`,
offline help/version test and no install-time account setup. Do not describe the
archives as bottles or add `bottle` metadata for upstream binaries.

After identities and asset locations are approved, fill the immutable URLs and
final SHA-256 values. Test with a local/controlled tap first on both architectures;
run formula syntax, audit where applicable, install, version/help, upgrade and
uninstall. Use synthetic profiles to verify config/credential preservation without
printing their secrets. Final public tap publication follows binary publication
and checksum verification. Only then write real user installation commands.

### Publication controls and recovery

The owner selected GitHub Actions and GitHub Releases with tag-triggered
publication. Build/test jobs precede publication; configure the scoped tap token
through GitHub. The [release guide](client-release.md) gives the concrete setup and
recovery steps. No signing identity is needed for the approved unsigned release.

If candidate tests fail, stop that release and retain evidence for repair. If a
published release has a defect, publish a new immutable version and update/revert
the tap according to the approved release policy; never silently replace a
versioned archive/checksum. Document config-schema compatibility for downgrade.
Installing/upgrading/uninstalling the binary never deletes user profile files.

## 7. Release evidence and definition of done

The final evidence package should contain:

- Source revision, CLI/JSON/config schema versions, imported server contract
  revision and both verified hashes.
- Approved decision log, resolved prerequisite register, compiler/MSRV/dependency
  report, target runtime matrix and approved license notices.
- AC01–AC20 traceability with pass/fail, test identifiers and execution environments;
  native minimum-floor and real-server reports for all four targets.
- Captured HTTP method/path/schema assertions, retry-count/uncertain-mutation tests,
  integer/large-cursor cases, private-storage/secret-redaction/PTY checks, and
  clipboard/headless evidence.
- Four final artifacts and SHA-256 manifest, signature/notarization results under
  the approved policy, and two-Mac Homebrew install/upgrade evidence.
- Release-owner approval, actual immutable download URLs and tap revision, and
  accurate installation/runtime/clipboard/config/error/pagination documentation.

Release one is done when its scoped client behavior meets the updated release
requirements, all four artifacts are verifiable, and the Homebrew lifecycle passes.
AC18 and the additional AC02/AC03 live scenarios remain documented follow-ups.
Before publication authority exists, the furthest truthful state is a verified
local/candidate release with explicit remaining prerequisites.

## 8. Planning risks and next decisions

| Risk / dependency | Planned response |
| --- | --- |
| Minimum-floor or exhaustive native live environments unavailable | AC18 is an owner-approved non-blocking 0.1.0 follow-up. Current hosted runners add native checks; keep the remaining coverage gaps explicit. |
| Dependency MSRV or musl incompatibility | Prove at M1; pin compatible versions or explicitly revise the client proposal before building features. |
| Homebrew stops supporting the selected core Mac floor | Verify current formula/OS compatibility at M5; retain the core native-binary floor and document the actual tap support range after owner review. |
| Headless Linux has no clipboard | Owner-approved nonfatal warning, documented optional helpers, deterministic test coverage. Core create/list/delete remains usable. |
| Proxy cannot carry large legal cursor URLs | Client preserves tokens and reports intermediary failures; verify transport capacity in fixtures and known deployment path. No cursor rewriting or backend change. |
| Lost mutation acknowledgement | Single-attempt writes, explicit outcome classification, HTTP fault tests and honest reconciliation guidance. |
| Runtime deadline or result bound interrupts fetch-all | Valid partial JSON, nonzero exit, accepted-page counts and opaque continuation; scripts can resume with the same profile/query. |
| GitHub setup or tap credential unavailable | Identities and policies are decided. Complete repository/secret setup before the first tag-triggered release. |

Release identity, license, version/signing policy and CI/publication are decided
in business spec §13. Next steps are repository/secret setup, first hosted runs,
and remaining local verification listed in the acceptance record.
No further choice about custom-code/title/tag/advanced-filter release scope is open:
the owner already resolved that boundary.
