# ferl CLI

`ferl` is a small, scriptable command-line client for an existing ferl server. It
creates short links, lists and searches your own links, deletes individual links, and
manages named server/account profiles. It is a separate project from the server and
uses only the server's HTTP API.

Version `0.1.0` is being prepared for release. Local automated and live-server tests
pass; manual profile setup and clipboard copy have also been confirmed. See the
[verification record](docs/client-implementation-status.md) for tested platforms
and remaining work.

## Quick start

```sh
cargo build --locked --release
./target/release/ferl profile

./target/release/ferl 'https://example.com/docs?edition=2#intro' --title 'Team docs' --tag Work
./target/release/ferl list --query 'Team docs'
./target/release/ferl delete 'returned-code'
```

The profile wizard asks for your server origin and an existing account's API key
(hidden while you type). Shortening prints the short URL and copies it to the
clipboard in an interactive terminal. Deletion asks for confirmation. Replace
`returned-code` with the code from a link you want to delete.

The full command reference, output contracts, exit codes, and examples are in
[docs/client-usage.md](docs/client-usage.md).

## Installation

For now, build from source as above. The release will provide downloadable binaries
for macOS and Linux on ARM64 and x86_64 through
[GitHub Releases](https://github.com/ai-slop-code/ferl-cli/releases).

Once the first release and tap are published, Mac installation will be:

```sh
brew install ai-slop-code/tap/ferl
```

Binary users need no Rust installation. Mac builds target macOS 13+; Linux builds
are static musl executables targeting kernel 4.18+. Minimum-OS testing is still
open. Mac releases are unsigned. Linux clipboard copy optionally uses `wl-copy`,
`xclip`, or `xsel`; headless usage works without them.

## Building

The toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml) (Rust 1.90.0 with
rustfmt and clippy). The declared minimum supported Rust version is 1.85
(`rust-version` in `Cargo.toml`). Dependencies are pinned by `Cargo.lock`; always build
with `--locked`.

If Homebrew's `cargo` appears before rustup's on your `PATH`, put `$HOME/.cargo/bin`
first so the pinned toolchain is used.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo +1.85.0 test --locked   # MSRV check (requires the 1.85.0 toolchain)
```

Ordinary `cargo test` is fully offline. It starts loopback mock servers, runs the
binary with an isolated configuration, and does not need the live server or any
credentials.

### Cross-compiling the four targets

```sh
scripts/package.sh            # builds, inspects, archives, and checksums all four targets
```

The Apple targets use the platform linker with `MACOSX_DEPLOYMENT_TARGET=13.0` (set in
`.cargo/config.toml`). The static Linux musl targets use `zig cc` as a build-time C
compiler and linker through the wrappers in `scripts/cross/`; use Zig 0.17.0 to build them.
The four-target packaging script runs on a Mac and includes the MIT license,
third-party notices, checksums, and a build manifest.
Cross-built artifacts are compilation evidence only. Native execution evidence must come
from running each binary on its own OS and CPU.

## Live integration tests (optional)

The live tests are ignored by default. Run them only against a server you are allowed
to use. They read `FERL_URL` and `FERL_API_KEY` from the environment, never print them,
and delete only the links they create:

```sh
(
  set +x; umask 077; set -a
  . "$HOME/.config/ferl-cli-local/profile.env"
  set +a
  cargo test --locked --test live_api -- --ignored --test-threads=1
)
```

## Repository layout

| Path | Contents |
| --- | --- |
| `src/` | The client: grammar (`cli.rs`), workflows (`commands.rs`, `profile_cli.rs`, `wizard.rs`), profiles and storage (`config.rs`, `profile.rs`, `storage.rs`), validation (`input.rs`, `origin.rs`, `secret.rs`), HTTP (`transport.rs`, `api.rs`), pagination (`list.rs`), output (`report.rs`, `table.rs`), clipboard and interruption (`clipboard.rs`, `cancel.rs`). |
| `tests/` | Offline unit-level integration suites (`cli_process`, `http_contract`, `tls_transport`, `pty_wizard`, `signals`, `schema`) and the ignored live suite (`live_api`). |
| `schemas/` | Published JSON Schemas for the stable v1 stdout Result and stderr error and warning records. |
| `scripts/` | Reproducible packaging, formula rehearsal, and cross-linker wrappers. |
| `packaging/homebrew/` | Formula template rendered with the exact release URLs and archive checksums. |
| `.github/workflows/` | Hosted checks, package smoke tests, and tag-triggered publication. |
| `spec/`, `docs/` | Business specification, implementation plan, imported server contract, and client documentation. |

## Releases and license

[MIT](LICENSE), copyright 2026 Lubos Babjak.

The [release guide](docs/client-release.md) covers GitHub setup, version tags,
Homebrew publication, and recovery from a failed release job.
