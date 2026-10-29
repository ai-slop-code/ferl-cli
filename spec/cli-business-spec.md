# ferl CLI: client business specification

Status: release-one client specification. Implementation and verification are tracked
in the [acceptance record](../docs/client-implementation-status.md). Release decisions
were updated with the owner on 2026-10-29; see [release guide](../docs/client-release.md).

## 1. Purpose, authority, and contract baseline

The separate Rust `ferl` CLI lets an existing account holder configure named
server/account profiles, shorten URLs, find links, and delete individual links.
Its primary users are terminal users and automation authors. A successful first
release makes these workflows predictable on Linux and macOS, on ARM64 and x86_64.

Authority, in order of subject matter:

- Owner-confirmed client decisions, including the clarifications recorded in §13,
  determine client scope and experience.
- [openapi.yaml](openapi.yaml) and
  [server-business-logic.md](server-business-logic.md) jointly define the existing
  HTTP contract. Neither supersedes the other.
- [The handoff](../docs/cli-api-handoff.md) explains client implications.
- This document specifies client behavior; the
  [implementation plan](../docs/cli-implementation-plan.md) defines delivery gates.

Imported baseline, verified against the supplied files:

| Item | Value |
| --- | --- |
| Server revision recorded by handoff | `0d4cdcc499f02edaedef70ffb332012935a8af9e` |
| OpenAPI version | `1.0.0`, OpenAPI `3.1.0` |
| OpenAPI SHA-256 | `45677d0d0c0afda4687fada791ce492682ad546f2fbc612113890cffecbb6b5e` |
| Server business specification (`server-business-logic.md`) SHA-256 | `c9fd70817b687df1e44dc6d3c4eb64e68af5b92b91fe2264903f7d0e05352c36` |

No conflict was identified in the first-release operations reviewed here. Future
conflicts must be reported with exact operation/schema and business-section
references before changing client behavior. Contract updates require deliberate
review; the CLI version, server package version, and API version are independent.

“Must” below is a client acceptance requirement under this planning baseline.
Detailed analyst-selected defaults are proposals, distinguished from owner decisions
in §13. They do not imply approval of remote repositories or publishing access.

## 2. Release-one boundary

### Included

| Capability | Release-one extent |
| --- | --- |
| Profiles | Add, inspect, list, select a default, edit connection settings, replace a locally saved key, validate, and remove named profiles; interactive add/switch wizard and explicit noninteractive equivalents. |
| Shorten | One authenticated URL per invocation; generated code; optional caller-supplied title and repeated tags. Explicit `create` command and URL shorthand. |
| Clipboard | Automatically copy the server-returned short URL during interactive human-output creation; explicit copy/opt-out controls and nonfatal failure notice. |
| List/search | Own links for both users and admins; literal text search; one page by default, explicit continuation, and bounded opt-in fetch-all. |
| Presentation | Fixed default table columns, selectable/reorderable list columns, stable JSON, exact integer counts, readable and sanitized terminal output. |
| Delete | One bare code or associated public short URL; terminal confirmation and explicit noninteractive consent. |
| Connectivity | Verified HTTPS, explicit private-CA bundle, explicitly enabled loopback-only HTTP development, bounded deadlines and read retries. |
| Distribution | Four native executable targets, downloadable archives, and a prebuilt-binary Homebrew tap plan for both Mac architectures. |

An admin key has its existing server authority: it still lists only its own links
in this client release, but can delete any known authorized code. Local profile
selection is not an authorization boundary enforced by the server.

### Deferred

- Custom-code creation; advanced filters (`scope`, owner, enabled, creation range,
  tag filters/modes) and configurable sort/order. Internally selecting `scope=own`
  is included; exposing `--scope` is deferred.
- Link detail commands, editing, disabling/re-enabling, bulk operations, batch
  shortening, and interactive link pickers for deletion.
- Dedicated tag discovery/management commands and per-link analytics commands.
  Displaying returned tags and `visit_count` is included and needs no extra calls.
- Account administration, server-side key/token rotation, anonymous creation,
  anonymous-token management, login/OAuth, and automatic account provisioning.
- OS credential-store integration, clipboard history, self-update, shell
  completions, Linuxbrew, Windows, GUI, and server-health command families.

The full API inventory is reference material. No client path requires database
access, server internals, deployment commands, or the server cursor encryption key.

## 3. Command and flag grammar

### 3.1 Commands

Angle brackets mean required values; brackets mean optional syntax. These are CLI
arguments, not literal request JSON. `KEY_SOURCE` is defined below.

```text
ferl [GLOBAL] <destination-url> [--title <text>] [--tag <text>]... [COPY]
ferl [GLOBAL] create <destination-url> [--title <text>] [--tag <text>]... [COPY]
ferl [GLOBAL] list [--query <text>] [--limit <1..200>]
                  [--cursor <token> | --cursor-file <path>]
                  [--save-cursor <path>] [--all [--max-pages <n>]]
                  [--columns <column,...>]
ferl [GLOBAL] delete <code-or-short-url> [--yes]

ferl [LOCAL_GLOBAL] profile
ferl [LOCAL_GLOBAL] profile add <name> [--server <origin>]
    [--public-origin <origin>] [--ca-bundle <path>]
    [--allow-http-loopback] [KEY_SOURCE] [--default] [--validate]
ferl [LOCAL_GLOBAL] profile list
ferl [LOCAL_GLOBAL] profile show [<name>]
ferl [LOCAL_GLOBAL] profile use [<name>]
ferl [LOCAL_GLOBAL] profile set <name>
    [--server <origin> KEY_SOURCE]
    [--public-origin <origin> | --clear-public-origin]
    [--ca-bundle <path> | --clear-ca-bundle]
    [--allow-http-loopback | --disallow-http-loopback]
ferl [LOCAL_GLOBAL] profile set-key <name> [KEY_SOURCE] [--validate]
ferl [LOCAL_GLOBAL] profile check <name> [KEY_SOURCE]
ferl [LOCAL_GLOBAL] profile remove <name> [--yes]
ferl --help | ferl <command> --help
ferl --version

COPY       = --copy | --no-copy
KEY_SOURCE = --key-stdin | --key-file <path> | --key-env <variable-name>
```

`GLOBAL` includes `LOCAL_GLOBAL`, `--profile <name>`, and optional `KEY_SOURCE`.
`LOCAL_GLOBAL` includes `--config <path>`, `--output <human|json>`,
`--non-interactive`, `--no-color`, `--timeout <seconds>`,
`--connect-timeout <seconds>`, and `--read-retries <0..2>`.
Transport flags affect only commands performing HTTP. Supplying an inapplicable
flag is a usage error, not an ignored setting. Profile commands target their named
profile; they reject `--profile` to avoid competing selectors. `profile show`
without a name uses the configured default, not `FERL_PROFILE`.

Parsing rules:

- Subcommands are exact, case-sensitive words. The shorthand is recognized only
  when the positional argument starts with an HTTP(S) scheme, case-insensitively;
  unknown words are errors, never destinations or commands inferred from spelling.
- Exactly one destination or deletion target is accepted. No destination stdin,
  URL-file input, raw-JSON body option, or arbitrary HTTP command in release one.
- `--` ends option parsing, permitting a bare deletion code such as `-abc`.
  Flags may occur before the final `--`, in the applicable command context.
- Repeated scalar options, duplicate column names, unknown options, incompatible
  options, and extra positionals fail locally with exit 2 before HTTP.
  Only `--tag` repeats; a comma within its value is data.
- `--title` is a string; omission means no title. There is no null/clear-title
  option at creation: omission already produces that outcome. Blank is invalid.
- `--query` maps to `q`; it is not a positional search language. Shell users must
  quote URLs/search text containing `&`, `#`, spaces, or shell metacharacters.
- `--columns` is allowed only for human list output. Accepted names, in any selected
  order: `short_code`, `short_url`, `destination_url`, `title`, `tags`, `enabled`,
  `active`, `visit_count`, `created_at`, `updated_at`, `owner_id`, `id`.
- `--max-pages` requires `--all` and is an integer 1–10,000; default 100.
  `--all` means all pages, never administrator `scope=all`.
- Missing required values can be prompted for only in the explicitly interactive
  profile workflows. Shortening/listing never silently open a wizard.
- No arguments prints concise help and exits 0, without network or clipboard use.
  `--help` and `--version` work without a config file, key, network, or clipboard.

### 3.2 Interactive profile wizard

Interactive means stdin, stdout, and stderr are terminals, human output is
selected, and `--non-interactive` is absent. Redirecting/piping any standard stream
disables prompts and automatic clipboard copy. JSON output always disables prompts.
`profile` and `profile use` without an explicit name require interactive mode;
otherwise they exit 2 with the available noninteractive forms.

`ferl profile` opens a small menu: Add profile, Switch default profile, Inspect
profiles, Replace saved key, Edit connection settings, Remove profile, Exit.
With no profiles it starts Add. Existing profile names are shown with the default
marked; API keys are never displayed. Switching shows the saved server origin
and profile name before applying the selection. `profile use` with no name opens
only the switch selector. Menu cancellation exits 130 without changes.

Add collects name, server origin, an optional distinct public origin, optional CA
bundle, hidden API key, whether to validate, and whether to make default. Validation
defaults to yes in the wizard. All input is summarized with the key redacted before
save; key entry has echo disabled. A validation failure leaves the previous local
state intact; the user may explicitly choose Save unvalidated or revise/cancel.
An offline saved profile is a supported state, not evidence of valid credentials.

`profile add NAME` prompts for missing server/key in interactive mode. Scripts must
supply both. Noninteractive `--validate` checks before saving and fails without
committing the proposed local change. Without `--validate`, add is local-only.
`profile set-key` prompts securely if needed; scripts require a key source.
The edit menu uses the same validations and update rules as `profile set`.

`profile use NAME` sets a persisted default locally, without prompting or HTTP.
The first profile becomes default automatically; later adds preserve the default
unless `--default` is supplied. Removing the default clears it rather than silently
selecting another profile. A subsequent business command must select one explicitly
or set a default. Removal confirms in terminals and requires `--yes` in scripts;
it removes local configuration and the local key, not a server account or links.

## 4. Profiles, credentials, and precedence

### 4.1 Files and profile schema

Use UTF-8 TOML. Default directory on both operating systems:
`$XDG_CONFIG_HOME/ferl` if that environment value is an absolute nonempty path;
otherwise `$HOME/.config/ferl`. An invalid explicitly supplied XDG path fails with
exit 3. No usable home/config location is also exit 3.

Config resolution is `--config PATH` > `FERL_CONFIG` > default
`config.toml`. Explicit paths are resolved against the current directory; no
shell expansion is performed by the client. There is no implicit project-local
configuration or merge of multiple config files.

Proposed stored shape (illustrative origins and UUID, no real credential):

```toml
schema_version = 1
default_profile = "work"

[profiles.work]
id = "4804ab6a-5e2d-4b0a-a1a3-ff61715414b2"
server_origin = "https://api.example.com"
public_origin = "https://s.example.com"
ca_bundle = "/absolute/path/to/company-ca.pem"
allow_http_loopback = false
```

Each profile requires a locally generated stable UUID and `server_origin`.
`public_origin` and `ca_bundle` are optional; absent public origin means the server
origin is the associated public origin. `allow_http_loopback` defaults false.
Profile names match `[a-z][a-z0-9_-]{0,31}` and are unique/case-sensitive. A name
need not equal the server account username. Credentials remain opaque; local IDs
are not server account IDs or authorization assertions.

The companion credential file is `<config-filename>.credentials.toml` in the same
directory (default `config.toml.credentials.toml`). It has `schema_version = 1`
and a `credentials` table keyed by profile UUID; each entry has `server_origin`
and `api_key`. The binding must match the selected profile before use. Never
copy a key automatically when adding a profile, even for the same server.

Private-file storage is plaintext with owner-only access, not encryption:

- Create client-owned directories with mode 0700, both files and temporary files
  with 0600; verify current-user ownership and reject symlinks for these files.
  Check effective access, including ACLs, rather than mode bits alone. An explicitly
  selected existing directory must also satisfy the private-directory policy.
- Fail with exit 3 on unsafe permissions; do not silently repair existing user
  files. Explain the affected path and required access, without file contents.
- Lock writers, stage complete files, fsync and atomically rename. Individual-file
  atomic replacement is required; do not claim a two-file transaction.
- Add/connection change allocates a new profile UUID and writes its bound credential
  entry before committing the config reference. Existing config remains usable
  until commit. Remove commits config removal before cleaning its credential entry.
  Crashes may leave unreferenced entries, which are never selected and are removed
  during a later successful locked write. Key replacement updates one credential
  file atomically. Readers use a consistent locked snapshot.
- Never overwrite an existing profile via `add`. Reject unknown config versions
  rather than resetting or silently migrating them. Keep no plaintext backup copies
  or HTTP-response cache; users control any exported result files.

`profile set` requires at least one change. Changing `server_origin` requires a
new explicitly supplied key source in the same update, resets public-origin/CA/
HTTP exceptions unless supplied again, and never carries the old credential to
the new origin. Other changes preserve the profile UUID and key. Store CA paths
as absolute paths at configuration time; read the selected file for each process.

### 4.2 Resolution and secret entry

| Setting | Precedence / behavior |
| --- | --- |
| Config location | `--config` > `FERL_CONFIG` > XDG/home rule. One config only. |
| Business-command profile | `--profile` > `FERL_PROFILE` > saved default. Missing/unknown selection is exit 3; never choose another profile on failure. |
| Credential | One explicit `KEY_SOURCE` > that profile's stored credential. A business-command override requires explicit `--profile`, so an ambient default cannot redirect an automation secret. |
| Server, public association, CA, HTTP permission | The selected profile only. Changed through profile commands; no ambient server/key/CA merge or per-request host override. |
| Output | `--output` > `FERL_OUTPUT` > `human`. Only `human` and `json`; invalid explicit/env value fails with exit 2. |
| Deadlines/retries | Explicit transport flags > fixed defaults in §10. No saved or ambient transport overrides in release one. |
| Color | `--no-color`, presence of `NO_COLOR`, or non-TTY disables color. Baseline output is uncolored, even in a TTY. |
| Proxy | Explicit environment behavior in §10; cannot alter the selected authenticated origin. |

`KEY_SOURCE` alternatives are mutually exclusive. `--key-stdin` reads one line
through EOF; `--key-file` reads a current-user-owned regular file without group/
other access; `--key-env NAME` reads that explicitly named environment variable.
Strip one terminal LF or CRLF only; reject empty values, multiple lines, whitespace
or characters unsafe in a Bearer header rather than trimming arbitrary secret
content. Do not decode tokens or establish validity from a prefix. Bound local
secret input to 65,536 bytes and fail with a local-input error on excess.

For add/set-key, these sources are saved in the private credential file. For a
business command or `profile check`, an override stays in memory and never updates
the profile. There is no `--api-key <plaintext>` option or automatic `FERL_API_KEY`
override. Business commands never prompt for missing keys. A missing env value,
missing/unsafe file, or unavailable saved key is exit 3 before HTTP.

External key rotation is handled with `profile set-key`; no rotation request is
made. Optional validation uses the newly supplied key. An explicit `profile check`
always verifies against the current server; no cached role grants authority.

No debug output, error echo, prompt, subprocess argument, crash message under
client control, or profile listing may contain the key. Redact authorization,
proxy credentials, and known secrets before diagnostic rendering. Avoid body/query
logging entirely. Profile show/list reports credential presence and storage kind,
never plaintext, a prefix, or a hash.

## 5. Concrete workflows

Origins and keys below are illustrative; an administrator supplies the real origin
and an existing account key. Distribution URLs are intentionally not specified.

### Interactive setup, switching, and shortening

```text
$ ferl profile
Add profile / Switch default profile / ...
Name: work
Server origin: https://api.example.com
Public short-link origin (optional): https://s.example.com
API key: [hidden entry]
Validate now? [Y/n]
Make default? [Y/n]
Saved profile work.                         # stderr

$ ferl 'https://example.com/docs?edition=2#intro' \
    --title 'Team docs' --tag Work --tag 'reference,internal'
https://s.example.com/a1b2c3d4              # stdout: actual returned URL
Copied short URL to clipboard.             # stderr, on clipboard success

$ ferl profile use
Select work or personal; selected profile becomes the default.
$ ferl --profile work list --query 'Team docs' \
    --columns short_url,title,tags,visit_count
$ ferl --profile work delete 'https://s.example.com/a1b2c3d4'
Permanently delete code a1b2c3d4 using profile work at https://api.example.com?
This also removes its visits; the code cannot be reused. [y/N]
Deleted a1b2c3d4.                          # stderr after 204
```

Wizard/prompt text is indicative; stream placement and behavior are normative.
The bare-code equivalent is `ferl --profile work delete a1b2c3d4`.

### Noninteractive and local key replacement

```sh
# /secure/work.key is an existing owner-only file supplied by the user.
ferl --non-interactive profile add work \
  --server https://api.example.com --public-origin https://s.example.com \
  --key-file /secure/work.key --default --validate

ferl --non-interactive --profile work --output json create \
  'https://example.com/docs?edition=2#intro' --title 'Team docs' --tag Work

ferl --non-interactive --profile work --output json list \
  --query 'docs & reference' --limit 50 --save-cursor ./next.cursor
ferl --non-interactive --profile work --output json list \
  --query 'docs & reference' --cursor-file ./next.cursor --save-cursor ./next.cursor

ferl --non-interactive --profile work --output json list --all --max-pages 100
ferl --non-interactive --profile work delete a1b2c3d4 --yes
ferl --non-interactive profile set-key work --key-file /secure/replacement.key --validate

# WORK_KEY is injected by the automation environment, never passed as an argument.
ferl --non-interactive --profile work --key-env WORK_KEY --output json list
```

Continue with `next.cursor` only when the preceding result says another page exists.
Every script checks the exit status; list `complete=false` has defined page semantics
in §8. A wizard or delete without consent must never wait for a prompt in scripts.

## 6. Exact HTTP mapping

All four operations use exactly one `Authorization: Bearer <account-key>` and
`Accept: application/json`. Only creation sends a JSON body and
`Content-Type: application/json`. GET/DELETE have zero body bytes; do not send
`{}`, `null`, or whitespace. Fixed API paths are appended to the selected origin.

| CLI action | Existing operation | Request schema / parameters | Required success | Declared operation errors |
| --- | --- | --- | --- | --- |
| URL shorthand / `create` | `createLink`, `POST /api/v1/links` | `CreateLink`: required `destination_url`; optional `title`, `tags`; omit `short_code` | 201, `LinkCreation` | 400, 401, 403, 409, 413, 415, 429, 503, 500 |
| `list`, including query and subsequent pages | `listLinks`, `GET /api/v1/links` | `Scope=own`, `Limit`, optional `LinkSearch` (`q`), optional `Cursor`; no body | 200, `LinkCollection` | 400, 401, 403, 503, 500 |
| `delete` | `deleteLink`, `DELETE /api/v1/links/{code}` | Path `Code` → `ShortCode`; no body | 204, no response body (`Deleted` response) | 400, 401, 403, 404, 503, 500 |
| `profile check`; add/set-key/wizard validation when requested | `getCurrentAccount`, `GET /api/v1/me` | No query or body | 200, `Account` | 400, 401, 403, 503, 500 |
| Other profile/help/version/clipboard actions | Local only | No server profile or clipboard API | Local result | Local errors |

Schema identifiers refer to `components.schemas`; named parameters refer to
`components.parameters`. Global unsupported-path 404 and unsupported-method 405
with `Allow` are handled too; they are not additions to every operation's declared
response set. Unexpected statuses remain reportable, never coerced into success.

Request example:

```json
{"destination_url":"https://example.com/docs?edition=2#intro","title":"Team docs","tags":["Work","reference,internal"]}
```

Serialize optional absent values by omission. No title supplied means omit `title`;
no tags means omit `tags`. Never send null tags, a custom-code member, ownership,
enabled state, IDs, or display options. The UTF-8 serialized body must be at most
65,536 bytes. Use a structured serializer to avoid duplicate JSON fields.

`list` sends `scope=own` and `limit` (default 50); omit `q` and `cursor` unless
provided. Omit `sort`/`order` to use creation-time descending then UUID descending.
Use a query encoder exactly once; do not concatenate raw text into URLs. No unknown
or repeated scalar query parameters are allowed. `--columns`, `--all`, deadlines,
and cursor-file paths are strictly local settings.

The client calls no detail, analytics, health, root, public GET/HEAD, or external
destination route to implement these workflows. Deletion has no preliminary GET.
Profile validation is the only reason to call `/api/v1/me`; it is not an account
command family or a prerequisite before each business request.

Authoritative references: OpenAPI paths `/api/v1/me`, `/api/v1/links`, and
`/api/v1/links/{code}`; business §§4, 6, 7, 11.2–11.4, 12, 13.

## 7. Output and stream contracts

### 7.1 Human output

- Successful creation writes exactly the returned `short_url` followed by LF to
  stdout. Do not reconstruct it from the profile origin. Clipboard notices go to
  stderr. There is no banner, label, or color around the URL.
- List stdout is a table with headings. Default columns, in order:
  `short_url,destination_url,title,visit_count,created_at`. `--columns` replaces this
  list. Null title/owner displays `—`; tags display as a JSON-style array of strings.
  Empty results retain headings and report `0 links` on stderr.
- Show dates in UTC, with a `Z` designation and second precision in the table;
  retain the original RFC 3339 string/precision in JSON. Counts are decimal
  integers, not rounded or abbreviated. `enabled` and `active` are separate columns.
- TTY tables fit the detected width (fallback 120); truncate long cells with an
  ellipsis at Unicode grapheme boundaries. Non-TTY human output has no color or
  truncation. JSON is the supported machine schema; aligned table spacing is not.
- Page counts, continuation/completion notices, prompts, profile-write summaries,
  and successful human deletion messages go to stderr. Successful delete and
  profile mutations have empty stdout. Profile show/list/check have human stdout
  records; the interactive profile menu itself uses stderr.
- Render all untrusted data as text: escape C0/C1 controls, ESC, newlines, tabs,
  and bidirectional formatting controls in terminal tables/diagnostics. Never
  interpret ANSI, OSC hyperlinks, HTML, Markdown, or shell fragments from input
  or the server. JSON escapes data without dropping characters. A conforming
  `short_url` contains no literal terminal controls; reject a violating response
  instead of printing it as a successful creation.

### 7.2 Stable JSON v1

`--output json` emits one UTF-8 JSON object plus LF on stdout for a successful
data-producing command. Profile mutations and deletion also emit their defined
local result object. Object key order/indentation is not contractual. No progress,
prompts, or human text is mixed into JSON stdout. Partial list results are the
explicit exception to success-only stdout, defined in §8.

All objects use this envelope; every listed field is required unless stated:

```text
Result = {
  schema_version: 1,
  command: "create" | "list" | "delete" | "profile.add" | "profile.list" |
           "profile.show" | "profile.use" | "profile.set" | "profile.set-key" |
           "profile.check" | "profile.remove",
  profile: string | null,
  data: command-specific object,
  meta: command-specific object
}
```

Shorthand creation reports `command="create"`. `profile` is the selected/targeted
profile name, or null for profile listing. Help/version remain textual, with
`ferl --version` producing `ferl <CLI-version>` plus LF; they are not Result objects.

| Command | `data` | `meta` |
| --- | --- | --- |
| create | `{link: Link}` (account-owned `LinkCreation`) | `{request_id: string|null, clipboard: "copied"|"skipped"|"failed"}` |
| list | `{items: Link[], next_cursor: string|null}` | `{pages_fetched: integer, complete: boolean, truncated: boolean, stop_reason: "end"|"page"|"page_limit"|"timeout"|"error"|"size_limit"|"cancelled", request_id: string|null}` |
| delete | `{short_code: string, deleted: true}` | `{request_id: string|null}` |
| profile.list | `{items: ProfileView[], default_profile: string|null}` | `{}` |
| profile.show | `{profile: ProfileView}` | `{}` |
| profile.check | `{account: Account}` | `{request_id: string|null}` |
| profile.add / set / set-key | `{profile: ProfileView, validation: "passed"|"not_run"}` | `{request_id: string|null}` |
| profile.use | `{default_profile: string}` | `{}` |
| profile.remove | `{name: string, removed: true, default_profile: string|null}` | `{}` |

`ProfileView` fields are `name`, `id`, `server_origin`, `public_origin`
(effective associated origin), `ca_bundle` (string|null), `allow_http_loopback`
(boolean), `is_default` (boolean), `credential_storage` (constant `"private_file"`),
and `credential_present` (boolean). It contains no secret. Profile listing sorts
names ascending; link results always retain server ordering. Successful profile
validation returns the existing `Account` schema where specified and never a key.

`Link` is exactly the OpenAPI representation: all 12 fields are present.

| Field | JSON type / meaning |
| --- | --- |
| `id` | UUID string |
| `short_code` | Exact `ShortCode` string |
| `short_url`, `destination_url` | Server-returned `StoredUrl` strings |
| `title` | String or null, never replaced with destination/code |
| `tags` | Canonical string array, including empty array |
| `owner_id` | UUID string or null |
| `enabled`, `active` | Independent booleans with the server's meanings |
| `visit_count` | Integer in `0..=9223372036854775807` |
| `created_at`, `updated_at` | RFC 3339 UTC strings ending `Z` |

Use exact integer deserialization/serialization (e.g. Rust `i64` with range checks),
never floating-point intermediate values. JSON consumers must use an integer-safe
parser for counts above 2^53. Title/tag options do not change the response schema.
`Account` has `id`, `username`, `role`, `enabled`, `created_at`, `updated_at` as in
OpenAPI. Account-owned creation must omit `management_token`, not emit null.

Unknown response properties may be ignored for forward compatibility, but only
documented fields are emitted by CLI JSON v1; required fields/types/nullability
must be validated. Explicitly reject an anonymous creation shape in this
account-only workflow and never leak an unexpected secret-bearing field. A
received 201 still acknowledges commit even if its representation is unusable.

Example single-page completion:

```json
{"schema_version":1,"command":"list","profile":"work","data":{"items":[],"next_cursor":null},"meta":{"pages_fetched":1,"complete":true,"truncated":false,"stop_reason":"end","request_id":"72ad8f48-40e4-4a78-9bac-3cd81aba6000"}}
```

CLI JSON schema version is independent of API/CLI SemVer. Preserve field meanings
through schema v1; additive envelope fields can be introduced with release notes,
whereas breaking changes require a schema-version decision. Checked-in client
JSON Schema artifacts and schema tests are an implementation deliverable, not a
change to the server OpenAPI document.

### 7.3 Errors and warnings

Human errors go to stderr with a concise cause and useful next step. Include HTTP
status, server code, and request ID when available. Never print raw HTML, arbitrary
response bodies, full request/destination URLs, search text, authorization values,
or library debug structures. Explicit profile output and deletion consent may
show the selected origin; diagnostic context otherwise uses command/profile and
route template. Validate/escape IDs and messages before display. For a known
contract error use its safe message, additionally applying local secret/control
redaction; for unrecognized/unstructured responses use a client-generated message.

In JSON mode stderr is JSON Lines: zero or more warning records followed, on
failure, by one error record. It never contains prose. Error schema:

```text
{
  schema_version: 1, type: "error", command: string, profile: string|null,
  exit_code: integer,
  error: {kind: string, code: string, message: string},
  http_status: integer|null, request_id: string|null,
  retry_after_seconds: integer|null,
  mutation_outcome: "not_applicable"|"not_sent"|"rejected"|"committed"|"unknown",
  partial: boolean, pages_fetched: integer
}
```

`kind` is `usage`, `configuration`, `authentication`, `authorization`, `not_found`,
`conflict`, `rate_limit`, `transport`, `server`, `protocol`, `partial`, `uncertain`,
`local_io`, `cancelled`, or `internal`. `code` is the server code if safely parsed,
otherwise a stable client code such as `client.invalid_arguments`,
`client.clipboard_unavailable`, `client.unexpected_response`, or
`client.mutation_outcome_unknown`. Unknown safe server codes are retained as text;
no closed-enum crash. A partial-list error keeps the underlying server/client code
and status while its kind/exit code identifies partial completion.

Warning schema is `{schema_version:1,type:"warning",command:string,
profile:string|null,code:string,message:string}`. Clipboard failure is a warning;
the result's clipboard status records it. All string values are JSON-escaped.
Use `X-Request-ID` when present, else the error-body ID. If both exist but disagree,
report a protocol warning and use the header ID, without pretending they matched.
Missing IDs are null; do not invent a server ID. No success is changed into an
uncertain mutation merely because a diagnostic header is absent.

### 7.4 Exit codes

| Exit | Meaning |
| --- | --- |
| 0 | Requested operation succeeded, including one requested page with more available, empty complete list, and committed creation with clipboard warning. |
| 1 | Unexpected internal client failure; sanitized diagnostic. |
| 2 | Usage/local validation, missing noninteractive consent, or structured API 400/413 rejection. |
| 3 | Missing/invalid profile, config, credential source, unsafe storage, or CA configuration. |
| 4 | HTTP 401 authentication required/invalid credentials. |
| 5 | HTTP 403 forbidden. |
| 6 | HTTP 404 unknown or inaccessible; not proof of global nonexistence. |
| 7 | HTTP 409 conflict, including `short_code_unavailable`. |
| 8 | HTTP 429; show positive whole-second `Retry-After` when valid. |
| 9 | DNS/connection/TLS/timeout failure with no possible sent mutation, or exhausted read transport retries before any page. |
| 10 | HTTP server/intermediary failure for a read or definitively rejected write. |
| 11 | Protocol/unexpected response, including 405, 415, refused 3xx, malformed success JSON, or unexpected success status. |
| 12 | Incomplete fetch-all or later-page failure after accepted pages; includes page/size/deadline bounds. |
| 13 | Uncertain mutation outcome after possible dispatch without an acknowledged success or definitive rejection. |
| 14 | Local output/cursor-file I/O failure (including broken pipe); acknowledged mutations remain committed. |
| 130 | User declines/cancels or SIGINT; after possible mutation dispatch without acknowledgement, exit 13 takes precedence. |

Interpretation precedence: uncertain mutation (13) > interrupted operation (130,
unless the write is uncertain) > local output failure (14) > partial list (12) >
underlying HTTP/transport result. A completed write followed by output failure is
14 with `mutation_outcome=committed`; clipboard failure alone remains 0. If stderr
is itself unwritable, return the appropriate nonzero exit without promising an
error record. Hard termination/power loss cannot guarantee final output.

401/403 never cause anonymous retry, fallback credentials, or profile switching.
503 is never converted to an empty list/404/401. 404 on a repeated delete is exit 6,
not a synthetic successful deletion. The shared error vocabulary is not a promise
that every operation returns every status.

## 8. Pagination and bounded results

The default request fetches one page with limit 50 (range 1–200), and exits 0 if
that page is valid. `complete` is true only when the received `next_cursor` is
explicitly null. When another page exists, set `complete=false`, `truncated=false`,
`stop_reason="page"`; stderr clearly says more results are available. An empty
page with a non-null cursor is not completion.

`--cursor` passes a nonempty opaque cursor unchanged. `--cursor-file PATH` accepts
a UTF-8 JSON string containing the token; it also recognizes JSON null as “there
is no continuation” and fails with exit 2 without fetching page one. `--save-cursor`
atomically writes the next token as a JSON string or null, mode 0600, after a
valid result; it deliberately replaces the named file. This avoids shell argument
limits, newline ambiguity, and copying long tokens out of a table. Input/output
may be the same file: read it before any replacement. No cursor file is written
implicitly. JSON stdout always includes the continuation; human output recommends
`--save-cursor` or JSON when another page exists, rather than dumping a huge token.

Cursors must support at least 87,387 ASCII bytes end to end, including JSON-file
input, encoded request URL, response handling, and saved continuation. Do not
decode, synthesize, truncate, log, or treat them as credentials. A 414 or intermediary
request-length limit is reported honestly; the client cannot fix a proxy limit
by editing a cursor or changing the server contract.

Every page retains the selected profile/key snapshot, origin, scope, search, and
ordering. The same query must be supplied when manually resuming; cursor files
contain only the token, not trusted context. Limit can change. An incompatible
cursor remains the server's 400; never silently restart page one. A repeated
non-null cursor is a client-detected protocol error to prevent a loop. Do not
deduplicate/re-sort links across pages: live changes can move results, and the CLI
cannot promise a frozen snapshot or global totals.

`--all` follows cursors serially to explicit null, preserving order. It is bounded
by `--max-pages` (100 default; max 10,000), the whole-command HTTP deadline (§10),
and 256 MiB of aggregate encoded result data. Per-response JSON is bounded to
16 MiB, sufficient for the documented maximum link page and cursor. If a page
cannot fit the aggregate bound, keep only previously accepted full pages and the
cursor used to request the excluded page. Do not claim those bounds are server
constraints.

Validate a complete page before accepting its items. Accumulate in private
temporary spool storage (0600, private directory) to keep memory bounded; this is
ephemeral output staging, not a response cache. Remove staging on normal exit or
handled cancellation. No automatic resumable job/session or persistent result
history. Measure peak memory and cleanup in acceptance tests.

On termination:

- Reaching null: output all accepted items, `next_cursor=null`, `complete=true`,
  `truncated=false`, `stop_reason="end"`; exit 0.
- First request fails before any accepted page: stdout empty; error on stderr;
  use the underlying exit code.
- Later request fails, or fetch-all hits a bound: output a valid partial Result
  (or human table) with accepted pages, `complete=false`, `truncated=true`, the
  appropriate stop reason, and a stderr error; exit 12. The cursor is the last
  accepted page's next token, so it requests the first unaccepted page again.
- SIGINT between/during reads: finish a valid partial envelope where output is
  still writable, use `stop_reason="cancelled"`, and exit 130. Zero accepted pages
  means empty stdout. SIGKILL cannot guarantee this finalization.

For partial output `meta.request_id` identifies the last accepted page; stderr's
error ID identifies the failed attempt. `pages_fetched` counts accepted pages,
not retry attempts. `--save-cursor` saves the same continuation as the Result;
with no accepted page leave any existing file intact. Cursor-save failure is
exit 14 even if the list data can still be emitted; diagnose the local failure.

## 9. URL, title, tag, deletion, and clipboard rules

### 9.1 Inputs and origins

Destination validation follows `DestinationInput` and business §6.2: explicit
absolute HTTP(S) authority, host, valid port; no user info (even empty `@`),
surrounding whitespace, literal ASCII whitespace/control, or malformed URL.
Check both submitted and standards-serialized lengths against 8,192 UTF-8 bytes.
Scheme is case-insensitive. Preserve submitted query/fragment and parameter order;
send the original validated value and accept the server's canonical serialization.
Do not silently trim invalid destination input or strip tracking data. A robust URL
parser must be preceded by raw-input checks where the parser would normalize away
invalid whitespace/user-info. Private, loopback, internal, IPv6, and IDN destinations
are valid; no destination DNS/fetch/title discovery or reachability test.

Profile origins have HTTP(S) scheme/host/optional valid port, no user info, query,
fragment, or non-root path. Accept one trailing `/` and normalize it away; reject
`/api/v1` and other path prefixes rather than guessing. Canonicalize scheme, host,
IDN, IPv6 spelling and default port using standards-based origin comparison.
Authenticated requests always go to the selected `server_origin` and fixed paths.
The profile's public origin is only an explicit deletion-URL association, never
an instruction to move credentials or rewrite the returned short URL.

Titles: trim surrounding Unicode whitespace for local validation; require 1–200
Unicode scalar values afterward and no control characters. Send the supplied
string and use the server's returned title. Omission means no title, not the
literal string `null`. Reject blank input before consuming creation allowance.

Tags: `--tag Work --tag 'work' --tag 'reference,internal'` sends three array elements.
Check at most 20 input elements before deduplication, each at most 256 scalar
values. Full validation applies Unicode trim, full case-fold, NFC, then 1–64
scalars without controls; never substitute ASCII lowercasing. Send original tag
values and retain returned canonical/deduplicated UTF-8-byte-sorted tags. If local
Unicode normalization libraries cannot guarantee the same tables, do only the
unambiguous local checks and leave canonical acceptance to the server, with tests
for the shared examples; do not reject server-valid names through an approximation.

Search: trim surrounding Unicode whitespace, require 1–256 scalar values without
controls, then send as `q`. The server applies full case-fold/NFC matching across
destination, short code, or title. `%`, `_`, backslash, comma, `&`, `+`, and Unicode
are literal data. No regex, wildcard, fuzzy search, client-side filtering, or
hidden tag syntax.

### 9.2 Deletion targets

Accept either an exact bare `[a-z0-9-]{3,64}` code or an absolute HTTP(S) URL with
exactly one raw path segment `/<code>` at the selected profile's associated public
origin. With an explicit public origin, only that origin is accepted as a short
URL; API-origin URLs are not an additional implicit alias. A bare code always
uses the selected API origin.

Reject user info, uppercase/percent-encoded codes, dot segments, backslashes,
encoded path separators, trailing slash, extra segments, whitespace, query, and
fragment suffixes. Reject even an empty `?` or `#` suffix. The strict suffix rule
is a client input choice; the public redirect API's query behavior is unchanged.
Inspect raw path syntax before parser normalization; never lowercase or percent-
decode a code into validity. UUIDs and destinations are not alternative link IDs.

Origin mismatch fails locally with exit 2 and no network request. Explain that the
user must select the associated profile or configure its correct public origin;
do not auto-switch profiles or learn an association from an input/returned URL.
Allow existing codes such as `assets`, `health`, and `api-sale` under the same syntax.
`api` passes the syntax check but cannot identify an allocated code; let the API
answer rather than inventing alternate routing.

Before dispatch, interactive deletion asks a default-No question naming profile,
server origin, and code, with the permanent-deletion/code-reservation consequence.
`--yes` suppresses this question. JSON/noninteractive/non-TTY execution requires
`--yes` or exits 2 without HTTP. Declining is exit 130. Consent does not cause a
preflight GET and is not a promise that the object exists or belongs to the caller.

On 204, report that one code deleted. Tags/visits/token are removed by the server;
the code stays reserved and independent links to the same destination survive.
There is no undelete or delete-by-destination operation.

### 9.3 Clipboard

Copy only a validated, server-returned `short_url`, without its output LF. Auto
copy requires interactive human mode with terminal stdout. `--copy` explicitly
enables it in scripts/JSON; `--no-copy` disables it. Neither flag changes HTTP.
Never copy destination URLs, credentials, JSON objects, or a reconstructed URL.

Proposed OS adapters use `/usr/bin/pbcopy` on macOS, and `wl-copy` when a Wayland
session is available, otherwise `xclip -selection clipboard` or `xsel --clipboard
--input` under X11. Resolve executable helpers directly, pass URL bytes on stdin,
and never interpolate data into a shell command. On Linux a missing/unusable helper
is nonfatal and reported; a clipboard desktop/session/helper is optional for core
CLI use. Document that these are native binary runtime conveniences, not Rust or
server dependencies. Do not invent an internal persistent clipboard service.

Complete clipboard work within two seconds after receiving success. Where an X11
helper must retain clipboard ownership, use its documented clipboard-owner mode
without keeping the CLI process waiting. Test persistence after CLI exit. Exit 0
means server creation succeeded even if clipboard copy failed; stderr warning and
JSON `clipboard="failed"` explain the ancillary failure. No extra POST is ever
issued to repair clipboard/output problems. Clipboard success notices are human
stderr only; JSON records the status without a success warning.

## 10. Transport, retries, and mutation outcomes

### 10.1 TLS, HTTP, proxies, and deadlines

- Verify certificate chain and hostname using Rust TLS with maintained public roots;
  an explicit per-profile PEM CA bundle adds private trust anchors. Do not depend
  on ambient `SSL_CERT_FILE`/`SSL_CERT_DIR` or silently import another profile's
  trust. Reject an unreadable/malformed bundle before HTTP. Never offer an
  `--insecure` bypass. TLS failure is distinct from credential rejection.
- HTTP is allowed only when `allow_http_loopback=true` and the configured host is
  a literal loopback IP (`127.0.0.0/8` or `::1`). `localhost` and other DNS names
  must use HTTPS or be replaced with an explicit loopback address for development;
  this avoids treating a hostname as proof of a local peer. Bypass proxies for this
  HTTP development path. HTTP destinations being shortened are still permitted
  regardless of API transport policy.
- Disable all automatic API redirects, including same-origin ones. Report 3xx
  as a protocol error and never follow `Location`; credentials stay at the selected
  API origin. Disable cookies, persistent HTTP caching, and implicit authentication
  mechanisms. Do not send credentials to the public short URL or destination.
- Support HTTPS API routing through `https_proxy` then `HTTPS_PROXY`, with
  `all_proxy` then `ALL_PROXY` fallback. `no_proxy` then `NO_PROXY` bypass rules
  accept comma-separated exact hosts/IPs, domain suffixes beginning `.`, optional
  ports, and `*`; document/test matching. Empty lowercase value disables its
  uppercase counterpart; invalid selected proxy settings fail locally. Support
  HTTP/HTTPS proxy URLs and HTTPS CONNECT; SOCKS/PAC are deferred. Ignore `HTTP_PROXY`
  and `http_proxy` because release-one plaintext API use is loopback-only and
  always direct. Proxy authorization is distinct and never receives the account key.
- Default connect deadline: 5 seconds for DNS/TCP/proxy/TLS establishment, included
  in the overall deadline. Default HTTP operation deadline: 30 seconds of wall time
  across attempts, retry waits, and all list pages. Start it after prompts and local
  input validation. Both flags accept decimal seconds from 0.1 to 3,600, with
  connect timeout no greater than overall timeout; zero never means unlimited.
  Cancellation interrupts active I/O/backoff. Clipboard's separate two-second
  local budget does not extend the HTTP operation deadline.

### 10.2 Read retry policy

`--read-retries` defaults to 2, hence at most 3 attempts for a business GET.
Retry transient connect/reset/timeout errors and HTTP 502/503/504 only. Use bounded
backoff of 250 ms then 1 s with at most 100 ms positive jitter. All waits/attempts
must fit the remaining command deadline; do not reset that deadline per page.
If a retryable read response supplies a valid positive-second `Retry-After`, wait
at least that value instead; if it cannot fit, fail immediately with the observed
error and wait advice. A missing 503 header creates no assumed server wait.

Do not retry invalid certificates, malformed successful JSON, ordinary 4xx,
401/403, incompatible cursors, or 500 by default. A read's valid page is accepted
only once, after a complete response; retries preserve its exact query and profile.
Retain the final attempt's request ID in an error. Optional diagnostics may count
attempts but must not log full URLs, keys, bodies, or cursors. The only read
operations here are `listLinks` and `getCurrentAccount`.

### 10.3 Writes and uncertain outcomes

Never automatically retry POST creation or DELETE, including redirects, connection
failures, 429, 5xx, and library-level retry facilities. A new user invocation is a
new attempt. No idempotency key, request-ID lookup, or transactional clipboard API
exists. Duplicate destinations create independent links; failed eligible attempts
can consume the account's rolling creation allowance (default 60/60 seconds).

| Observation | Classification / required behavior |
| --- | --- |
| Local failure or transport proves no HTTP request was dispatched (e.g. DNS/TLS handshake failure) | `not_sent`; relevant local/transport exit. |
| Complete expected 201 or 204 status received | `committed`. An unreadable/malformed 201 body is exit 11 with committed outcome; it is not permission to create again. 204 needs no JSON parser. |
| Conforming structured 400/401/403/404/409/413/415/429 rejection | `rejected`; corresponding exit, with authoritative server meaning. Creation allowance may still have been charged except on auth/rate rejection. |
| Conforming `503 service_unavailable` admission rejection, or create `503 code_generation_unavailable` | Definitive rejection under the documented semantics; exit 10, no retry. |
| `storage_unavailable`, `internal_error`, unstructured intermediary failure, unknown response semantics, timeout/reset after possible dispatch, or cancellation without acknowledgement | `unknown`; exit 13 for mutation. Preserve HTTP status/code/ID when available. |

Unexpected success statuses/redirects without contractual outcome evidence also
remain unknown for a dispatched mutation. For reads they are ordinary protocol
errors. Do not infer a rejection from an arbitrary proxy's HTML 4xx/5xx page.
The HTTP client must conservatively distinguish pre-dispatch failure from possible
dispatch; when that distinction is unavailable, choose unknown.

An uncertain-create message says the link may have been created and the CLI did
not retry. Suggest inspecting own links with `list --query` using a recognizable
destination substring, reviewing destination/time/code in JSON. Search is not
exact equality, not an idempotency proof, and concurrent or duplicate creations
make attribution uncertain. Do not add a detail command for reconciliation.

An uncertain-delete message says the code may have been deleted and no replay
occurred. A later list can help inspection but absence or a later 404 cannot prove
the earlier request succeeded, especially with authority changes/admin-owned scope.
Every later invocation reports its own observation; no hidden outcome journal.

For 429, report the positive whole-second wait; invalid/missing headers remain
null with a protocol warning. No automatic or opt-in create-rate retry is included.
After a committed response, failed local output or clipboard handling must explicitly
retain the committed outcome. In particular, never tell the user that no link was
created solely because it could not be copied or printed.

## 11. Native runtime and distribution baseline

The four-platform build matrix, macOS 13+ deployment target, and Linux static-musl/
kernel 4.18+ baseline are retained. The owner made exhaustive native workflows and
minimum-OS verification (AC18) non-blocking for 0.1.0. Document tested environments
and outstanding checks explicitly; a build target alone is not runtime evidence.

| Platform | Native Rust target | Runtime requirement / verification |
| --- | --- | --- |
| macOS ARM64 | `aarch64-apple-darwin` | macOS 13.0+, native Apple Silicon; system libraries; pbcopy clipboard. |
| macOS x86_64 | `x86_64-apple-darwin` | macOS 13.0+, native Intel; system libraries; pbcopy clipboard. |
| Linux ARM64 | `aarch64-unknown-linux-musl` | Kernel 4.18+, ARMv8-A baseline; static musl executable; no installed libc/OpenSSL/Rust requirement. |
| Linux x86_64 | `x86_64-unknown-linux-musl` | Kernel 4.18+, generic x86-64 baseline (no x86-64-v2/AVX requirement); static musl executable. |

Proposed client toolchain: Rust edition 2024, MSRV `1.85.0`, initial development
build pin `1.90.0`. Edition 2024 is available at the MSRV; the separate build pin
provides a concrete reproducible starting point without inheriting the server's
compiler. Milestone M1 must validate dependency MSRVs and pin compatible versions
in `Cargo.lock`; if the proposed MSRV cannot be met, record an explicit client
decision before feature implementation. Before public release, review/pin an
appropriate maintained stable build compiler and rerun the four-platform gates;
do not claim the initial pin is the latest 2026 compiler. Binary users need no Rust
installation. Compiler pin/MSRV changes are documented separately from OS support.

Use Rust TLS to avoid a system OpenSSL dependency. Linux builds must demonstrate
no dynamic ELF interpreter/DT_NEEDED entries. macOS uses only approved system
libraries; set deployment target 13.0. Do not compile with build-machine-specific
CPU tuning. Ordinary OS resolver/network support remains necessary; optional Linux
clipboard helpers and a desktop session are the only clipboard runtime additions.

Approved archive pattern:
`ferl-<version>-<rust-target>.tar.gz`, containing `ferl`, the approved `LICENSE`,
third-party notices, and a brief README stating runtime floors, CLI/JSON/config
versions and imported contract revision. Include per-release `SHA256SUMS` and a
machine-readable manifest with target, size, checksum, compiler, source revision,
and contract hashes. Archive paths must be relative, deterministic, and executable
mode preserved. Build four independent executables, not a script invoking Rust or
a server image. No universal Mac binary is required.

Use independent CLI SemVer starting at `0.1.0`, `v<version>` tags and immutable
release archives with SHA-256 checksums. The first release has no Developer ID
signing or notarization. Check downloaded-binary installation behavior and document
it accurately. SHA-256 files provide integrity checks. License: MIT, copyright
2026 Lubos Babjak.

### Homebrew tap plan

The approved `ai-slop-code/homebrew-tap` repository contains `Formula/ferl.rb`.
It selects immutable **prebuilt upstream archives** by `on_macos` and
`on_arm`/`on_intel`, supplies each actual SHA-256, requires macOS Ventura (13) or
newer, declares license/homepage/version, and installs only `ferl` into `bin` plus
approved documentation. This is an upstream-binary formula, not a claim to provide
Homebrew-built bottles. Linuxbrew is outside the commitment.

The formula must not build a server, create accounts/profiles, run network business
operations at install time, or obtain/store API keys. Its offline `test do` checks
`ferl --version` and `ferl --help`; native install tests additionally verify profile
files survive upgrade and uninstall. Homebrew formula parsing/audit/install/upgrade
must run on both Mac architectures. Core CLI macOS-floor testing and compatibility
with Homebrew's then-current supported macOS versions are separate gates; disclose
any tap-specific runtime limitation instead of claiming all macOS 13 installations
are supported by the external package manager.

The approved public source/homepage is `https://github.com/ai-slop-code/ferl-cli`;
its GitHub Releases host the archives. GitHub Actions on hosted runners performs
checks and tag-triggered publication, followed by the tap update using final
checksums. Repository setup, first hosted execution and the scoped tap publishing
secret remain setup work; their identities and policies are decided. Archives and
Homebrew both ship in the first release. See the [release guide](../docs/client-release.md).

## 12. Acceptance and change control

Acceptance criteria AC01–AC20 and verification evidence are defined in the
[implementation plan](../docs/cli-implementation-plan.md#3-measurable-acceptance-criteria).
Each retains its evidence requirement, with these owner-approved 0.1.0 exceptions:
AC18's missing native/floor checks and AC02/AC03's additional two-account/externally
rotated-key live scenarios are non-blocking follow-ups. Other gaps remain open.
Native runtime and Homebrew evidence cannot be replaced by cross-compilation or
the server's existing results.

Review changes to public command grammar, JSON/exit schemas, credential resolution,
write retry behavior, supported runtimes, and release policy explicitly. New API
families and deferred options need a later scope decision; availability in the
server schema alone does not approve them.

## 13. Short decision log and remaining owner inputs

| ID | Authority / status | Decision |
| --- | --- | --- |
| D01 | Owner, handoff | Separate CLI; Rust planning baseline; named profiles, shorten, list/search, delete; Linux/macOS on both CPUs; native binaries and Mac tap. |
| D02 | Owner, clarification | Include creation title/tags and output-column selection; defer custom codes and advanced filters. This resolves the initially overlapping “minimal only” and refinement selections. |
| D03 | Owner, clarification | `ferl profile` walks through adding and switching profiles; include automatic clipboard copy. |
| D04 | Owner, approved baseline | URL shorthand plus `create`, profile commands, `list --query`, code/URL deletion, one-page default, opt-in fetch-all, JSON, terminal delete confirmation and script `--yes`. |
| D05 | Owner, approved baseline | Private credential files; defer OS keychain integration. |
| D06 | Owner, approved baseline | macOS 13+, static-musl Linux/kernel 4.18+; verified TLS/private CA; explicit loopback HTTP; 5 s connect/30 s operation; bounded GET retries; no automatic mutation retries. |
| D07 | Owner, approved clarification | Clipboard auto-copy only in interactive human mode; explicit `--copy`/`--no-copy`; nonfatal clipboard failure; optional Linux helpers. |
| D08 | Owner, resolved 2026-10-29 | Public `ai-slop-code/ferl-cli` and `ai-slop-code/homebrew-tap`; GitHub Releases; formula `ferl`; MIT, copyright Lubos Babjak. |
| D09 | Owner, subsequent test-environment handoff | Local API at `http://127.0.0.1:18080`, private enabled-admin credential and synthetic `cli-demo` are available for testing. Readiness/account/search connectivity was verified; usage is documented in the [README live-test instructions](../README.md#live-integration-tests-optional). |
| D10 | Owner, release clarification 2026-10-29 | Simple self-hosted project; archives and Homebrew in release one. AC18 and additional AC02/AC03 live fixtures are non-blocking follow-ups. Report actual test coverage. |
| D11 | Owner, release clarification 2026-10-29 | `0.1.0`, independent SemVer, `v` tags, immutable archives/checksums; unsigned Mac release. GitHub Actions hosted checks and automated tag publication/tap update are approved. |
| P01 | Analyst proposal | Exact profile TOML/credential layout and binding, flag/env precedence, strict delete URL parsing, private temp staging, clipboard helper order. |
| P02 | Analyst proposal | JSON v1 envelopes, exit-number assignments, fixed column defaults, cursor JSON files, pagination/size bounds, exact retry/backoff/proxy rules. |
| P03 | Analyst proposal | Rust edition/MSRV/build pin, module/dependency direction, target packaging and prebuilt-binary formula structure. |
| P04 | Resolved by owner, D11 | Version/tag/immutable-archive policy approved; Developer ID signing/notarization declined for the first release. |

The owner resolved the release-policy questions above. Remaining practical setup:
create or connect the approved repositories, enable Actions, supply the scoped tap
publishing secret through GitHub, and review first hosted results. Pushing a version
tag is the maintainer's release action. Track execution gaps in the acceptance
record rather than reopening settled policy questions.
