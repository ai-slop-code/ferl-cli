# ferl CLI usage reference

This reference describes release-one behaviour. The authoritative requirements are in
the [client business specification](../spec/cli-business-spec.md); this page is the
practical guide that matches the implementation.

## Commands

```text
ferl <https-url> [--title TEXT] [--tag TEXT]... [--copy | --no-copy]
ferl create <url> [--title TEXT] [--tag TEXT]... [--copy | --no-copy]
ferl list [--query TEXT] [--limit 1..200] [--cursor TOKEN | --cursor-file PATH]
          [--save-cursor PATH] [--all [--max-pages 1..10000]] [--columns LIST]
ferl delete <code-or-short-url> [--yes]
ferl profile                       # interactive menu (terminal only)
ferl profile add NAME --server ORIGIN [--public-origin ORIGIN] [--ca-bundle PATH]
              [--allow-http-loopback] KEY_SOURCE [--default] [--validate]
ferl profile list | show [NAME] | use [NAME] | check NAME [KEY_SOURCE]
ferl profile set NAME [--server ORIGIN KEY_SOURCE] [--public-origin ORIGIN | --clear-public-origin]
              [--ca-bundle PATH | --clear-ca-bundle] [--allow-http-loopback | --disallow-http-loopback]
ferl profile set-key NAME [KEY_SOURCE] [--validate]
ferl profile remove NAME [--yes]
ferl --help | --version
```

A destination must start with `http://` or `https://` (any letter case). Anything else
is a usage error, never a guessed command. `create` and the shorthand produce identical
requests. Only `--tag` may repeat. Use `--` before a deletion code that starts with `-`.

### Global options

| Option | Meaning |
| --- | --- |
| `--profile NAME` | Profile for a business command. Not accepted by `profile` subcommands. |
| `--config PATH` | Configuration file (overrides `FERL_CONFIG` and the default location). |
| `--output human\|json` | Output format (overrides `FERL_OUTPUT`). |
| `--non-interactive` | Never prompt. |
| `--timeout SECONDS` | Whole-command HTTP deadline, 0.1 to 3600 (default 30). |
| `--connect-timeout SECONDS` | Connection deadline, 0.1 to 3600 (default 5), not above `--timeout`. |
| `--read-retries 0..2` | Retries for transient read failures only (default 2). |
| `--key-stdin`, `--key-file PATH`, `--key-env NAME` | One key source (see below). |
| `--no-color` | Accepted; output is never colored. |

Transport options apply only to commands that perform HTTP. Supplying them elsewhere is
a usage error.

## Profiles and keys

Profiles live in `$XDG_CONFIG_HOME/ferl/config.toml` (when `XDG_CONFIG_HOME` is an
absolute path) or `$HOME/.config/ferl/config.toml`. The companion credential file is
`config.toml.credentials.toml` in the same directory. Both are owner-only (0600) inside
an owner-only directory (0700). The client refuses group- or other-accessible files,
symlinks, and non-owned files and never repairs them automatically.

Selection: `--profile` > `FERL_PROFILE` > the saved default. A missing or unknown profile
is exit 3. The client never falls back to another profile, and never sends an anonymous
request after an authentication failure.

Keys are never accepted as argument values:

- `--key-stdin` reads one line through EOF (one terminal LF or CRLF is removed).
- `--key-file PATH` reads an owner-only regular file.
- `--key-env NAME` reads the named environment variable.

Keys must be a single run of visible ASCII. Keys saved by `profile add` and `profile
set-key` are stored in the private credential file. A key supplied to a business command
is used in memory only and is never saved. A business-command key override requires an
explicit `--profile`, so an ambient default cannot receive an automation secret.

### Interactive use

`ferl profile` opens a menu (add, switch default, inspect, replace key, edit connection
settings, remove, exit) when stdin, stdout, and stderr are terminals. Prompts and menus
go to stderr. Key entry is hidden. Summaries show `key: <redacted>`. Cancelling exits 130
without changes. With no profiles, the menu starts at Add. `ferl profile use` without a
name opens the switch selector only in a terminal.

### Scripted use

```sh
printf '%s\n' "$WORK_KEY" | ferl --non-interactive profile add work \
  --server https://api.example.com --public-origin https://s.example.com \
  --key-stdin --default --validate
ferl profile use work
ferl profile set-key work --key-file /secure/replacement.key --validate
ferl profile check work
ferl profile remove work --yes
```

`--validate` calls `GET /api/v1/me` with the new key before anything is saved. A failure
leaves the previous state unchanged. Without `--validate`, profile commands make no
network requests.

Changing `--server` requires a key source in the same command. The profile gets a new
identity, and the old key is never carried to the new origin.

### Origins and loopback HTTP

- Server origins are `https://host[:port]` (or `http://` for a literal loopback address
  only, with `--allow-http-loopback`). A single trailing `/` is accepted. Paths such as
  `/api/v1` are rejected.
- `--public-origin` is only used to recognize short URLs passed to `delete`. The client
  never rewrites a returned short URL.
- `localhost` cannot use plaintext HTTP; use `127.0.0.1` or `[::1]`.

### Private CA bundles and proxies

`--ca-bundle PATH` adds the PEM certificates in that file to the public roots for this
profile. The file must contain only certificates and is read before any request. TLS
verification is never disabled.

HTTPS requests honour `https_proxy`/`HTTPS_PROXY`, then `all_proxy`/`ALL_PROXY`, with
`no_proxy`/`NO_PROXY` bypass (exact hosts, `.suffix`, optional `:port`, `*`). An empty
lowercase variable disables its uppercase pair. Only HTTP and HTTPS proxies (CONNECT) are
supported; SOCKS and PAC are not. `HTTP_PROXY` is ignored because plaintext API use is
loopback-only and always direct.

## Creating links

```sh
ferl 'https://example.com/docs?edition=2#intro' --title 'Team docs' --tag Work --tag 'reference,internal'
```

- The request body contains only `destination_url`, plus `title` and `tags` when given.
  Values are sent as supplied. A title is trimmed for local checks only.
- `--title` must have 1 to 200 characters after trimming, with no control characters.
- At most 20 `--tag` values, each up to 256 characters. The server canonicalizes and
  deduplicates them (`Work` and `work` become one tag).
- Human output prints exactly the server's `short_url` plus a newline. Nothing else is on
  stdout.
- Copying: in an interactive human session (terminals on stdin, stdout, and stderr),
  the short URL is copied automatically. `--copy` enables copying in scripts and JSON;
  `--no-copy` disables it. Copying never sends another request. A failed copy is a warning
  and the command still exits 0. macOS uses `pbcopy`. Linux uses `wl-copy` on Wayland
  sessions, or `xclip`/`xsel` on X11. Helpers receive the URL on stdin only.

## Listing and searching

`ferl list` requests one page of your own links (limit 50 by default, `scope=own` for
everyone including admins) and returns server order without modification.

```sh
ferl --profile work list --query 'docs & reference' --limit 50 --save-cursor ./next.cursor
ferl --profile work list --query 'docs & reference' --cursor-file ./next.cursor --save-cursor ./next.cursor
ferl --profile work --output json list --all --max-pages 100
```

- `--query` is literal text (1 to 256 characters after trimming). `%`, `_`, `&`, `+`,
  commas, and non-ASCII are data.
- `--columns` (human output only) selects and orders columns from: `short_code`,
  `short_url`, `destination_url`, `title`, `tags`, `enabled`, `active`, `visit_count`,
  `created_at`, `updated_at`, `owner_id`, `id`. Default: `short_url,destination_url,title,
  visit_count,created_at`. Duplicates are rejected.
- Dates display in UTC to the second with a `Z` designation. JSON keeps the server's
  RFC 3339 strings. Counts are exact integers, including values above 2^53.
- Null titles and owners display as `—`. Tags display as a JSON-style array.
- Terminal output is truncated to the terminal width at grapheme boundaries; piped
  output is never truncated. Control characters and bidirectional controls are escaped
  as `\u{…}`.
- `--cursor` takes a token; `--cursor-file` takes a file containing a JSON string.
  A JSON `null` in the file is a usage error (there is no continuation), and nothing is
  fetched.
- `--save-cursor PATH` atomically writes the next token as a JSON string (or `null`) with
  mode 0600, replacing the file, only after a valid result.
- Fetch-all (`--all`) follows cursors serially until `next_cursor` is `null`, stopping at
  `--max-pages` (default 100), the command deadline, or a 256 MiB aggregate bound. A
  later-page failure, or a bound, produces valid partial output with `complete: false`,
  `truncated: true`, and exit 12. The continuation is the one that requests the first
  page that was not retrieved. A repeated continuation stops the list with an error.
- An empty page with a continuation is not completion.
- Ctrl-C during a list gives partial output with `stop_reason: "cancelled"` and exit 130
  if any page was retrieved.

Own-scope listing is the only listing supported. Advanced filters and sorting are not
part of release one.

## Deleting links

```sh
ferl --profile work delete a1b2c3d4 --yes
ferl --profile work delete 'https://s.example.com/a1b2c3d4' --yes
```

- A target is either an exact code (`[a-z0-9-]{3,64}`) or a short URL on the profile's
  public origin with exactly one path segment and no query, fragment, trailing slash,
  encoding, or uppercase letters. UUIDs are rejected.
- Interactively a default-No confirmation names the profile, server origin, and code.
  Scripts, JSON mode, and redirected input must pass `--yes`. Without consent there is no
  request (exit 2).
- A successful deletion has empty stdout; the human message goes to stderr. `delete` does
  not check whether the link exists first.
- A repeated delete returns 404 (exit 6), which means "not found or not accessible", not
  proof that an earlier request failed or succeeded.

## Output contracts

### Human

Successful creation writes the short URL to stdout. Lists write a heading row and then
rows. Notices, prompts, and warnings go to stderr. Profile `show`, `list`, and `check`
write records to stdout; profile mutations write nothing to stdout.

### JSON (`--output json`)

Each successful data command writes one JSON object plus a newline to stdout:

```json
{"schema_version":1,"command":"list","profile":"work",
 "data":{"items":[…Link…],"next_cursor":null},
 "meta":{"pages_fetched":1,"complete":true,"truncated":false,"stop_reason":"end","request_id":"…"}}
```

Commands: `create`, `list`, `delete`, `profile.add`, `profile.list`, `profile.show`,
`profile.use`, `profile.set`, `profile.set-key`, `profile.check`, `profile.remove`. The
`Link` object has exactly the twelve fields in the API schema. Field order is not
contractual.

On failure, stderr contains JSON Lines: zero or more warning records, then one error
record. Nothing else is written to stderr. The schemas are in [../schemas/](../schemas/):
`ferl-result-v1`, `ferl-link-v1`, `ferl-error-v1`, and `ferl-warning-v1`.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success, including a page with more available, an empty complete list, and a committed create with a clipboard warning. |
| 1 | Unexpected internal failure. |
| 2 | Usage or local validation, missing consent, or a 400/413 rejection. |
| 3 | Missing or invalid profile, configuration, credential source, unsafe storage, or CA configuration. |
| 4 | 401: credentials missing or invalid. |
| 5 | 403: forbidden. |
| 6 | 404: not found or not accessible. |
| 7 | 409: conflict. |
| 8 | 429: rate limited. The wait is reported when the server supplies it. |
| 9 | Connection, DNS, TLS, or deadline failure with no possible request, or read retries exhausted before any page. |
| 10 | Server or intermediary failure for a read, or a definitive write rejection. |
| 11 | Protocol problem: unexpected status, redirect, malformed success body, or a 405/415. |
| 12 | Partial list: a later page failed or a bound was reached. |
| 13 | Uncertain mutation: the request may have been processed without an acknowledgement. Not retried. |
| 14 | Local output or cursor-file failure (for example a closed pipe). |
| 130 | Declined, or interrupted by Ctrl-C. |

Precedence: uncertain mutation (13), then interruption (130), then local output failure
(14), then partial list (12), then the underlying status.

Mutation outcomes (`mutation_outcome` in error records): `not_sent`, `rejected`,
`committed`, `unknown`, or `not_applicable`.

- If a create is not acknowledged (for example the connection drops after the request was
  written), the CLI reports exit 13 and does not retry. Check with
  `ferl list --query` using a recognizable part of the destination. Duplicate
  destinations create separate links.
- If a create is acknowledged but the body or the output fails, the result is
  `committed` (exit 11 for an unusable body, or 14 for an output failure).

## Transport behaviour

- HTTPS only, except literal loopback HTTP when the profile allows it. Certificates and
  hostnames are verified. Redirects are never followed; they are reported as exit 11.
- Each request is sent on a fresh connection with `Connection: close`. There are no
  cookies, no caching, and no automatic retries except for GET operations.
- GET retries apply to connection failures, resets, timeouts, and 502/503/504 responses:
  up to two retries (about 250 ms, then 1 s, plus up to 100 ms of jitter), within the
  remaining deadline. A valid `Retry-After` on those responses is honoured when it fits.
- POST and DELETE are never retried by the client.
- The default deadline is 30 seconds of wall time for the whole command, including all
  pages and retries. The connection deadline is 5 seconds.

## Files and environment

| Name | Use |
| --- | --- |
| `FERL_CONFIG` | Configuration file path (after `--config`). |
| `FERL_PROFILE` | Default profile name for business commands (after `--profile`). |
| `FERL_OUTPUT` | `human` or `json` (after `--output`). |
| `XDG_CONFIG_HOME`, `HOME` | Configuration location. |
| `https_proxy`, `HTTPS_PROXY`, `all_proxy`, `ALL_PROXY`, `no_proxy`, `NO_PROXY` | HTTPS proxy selection. |
| `WAYLAND_DISPLAY`, `DISPLAY` | Linux clipboard helper selection. |

No environment variable supplies a key implicitly. Pass one through `--key-env` (for example
`ferl --profile work --key-env WORK_KEY list`).

## Known limitations in release one

- Custom codes, advanced list filters and sorts, editing, detail, tag, analytics, and
  account commands, anonymous creation, and key rotation are intentionally absent.
- The interactive wizard is a menu-driven convenience; scripts should use the explicit
  commands.
- Clipboard behaviour is tested with helper fixtures and the macOS pbcopy path; Linux
  helper behaviour requires native desktop runs (see the status document).
