# Local ferl server for CLI development

A local ferl server is running in Podman for CLI integration testing. It uses a
new, dedicated PostgreSQL database and volume; it does not use or expose the
server repository's retained Stage 01 PostgreSQL fixture.

## Connection and test account

- API and short-link origin: `http://127.0.0.1:18080`
- Readiness: `GET /api/health/ready`
- Test admin username: `ferl_cli_admin`
- Private shell profile: `~/.config/ferl-cli-local/profile.env` (mode `0600`)
- Synthetic sample link: code `cli-demo`, title `ferl CLI test link`, tags `cli`
  and `test`, destination `https://example.com/ferl-cli-demo`

Load the private profile into a shell for manual API tests:

```sh
set -a
. "$HOME/.config/ferl-cli-local/profile.env"
set +a

curl --fail-with-body "$FERL_URL/api/v1/me" \
  -H "Authorization: Bearer $FERL_API_KEY"

curl --fail-with-body "$FERL_URL/api/v1/links?q=CLI" \
  -H "Authorization: Bearer $FERL_API_KEY"
```

The profile contains an API key: do not commit, print, or share it. The CLI has
not selected a profile-file format yet; this shell file is only a local test
convenience and should not predetermine the client's configuration design.

## Podman resources

| Resource | Name | Purpose |
| --- | --- | --- |
| Network | `ferl-cli-net` | Private server-to-PostgreSQL traffic. |
| PostgreSQL container | `ferl-cli-postgres` | PostgreSQL 17.6; no host port is published. |
| Persistent database volume | `ferl-cli-postgres-data` | Retains the CLI test database across container restarts. |
| Server container | `ferl-cli-server` | Packaged server, published only on host loopback port 18080. |
| Podman secrets | `ferl-cli-db-password`, `ferl-cli-database-url`, `ferl-cli-cursor-key` | Runtime secrets; values are not stored in this repository. |

The server process uses the previously verified
`aarch64-unknown-linux-gnu` release binary, mounted read-only in the pinned
Debian Bookworm arm64 Rust runtime image. The artifact was verified before
extraction; it was not rebuilt for this setup. From the `ferl-cli` project root,
the running container mounts the verified executable from the sibling checkout
at `../ferl/target/ferl-cli-local-runtime/bin/ferl-server`; keep that extracted
file in place while using or restarting this local service.

- Server repository source revision:
  `25f077f6fde044d41a665e100309e421bcdb7cb9`
- Archive:
  `../ferl/target/release-readiness/36effbe1b5e845dab684df60c98bf51e/clean-checkout-final/packages/ferl-server-0.1.0-aarch64-unknown-linux-gnu.tar.gz`
- SHA-256: `f968bd5babd6098b202ae505a45ecf3f59dcfedcf9ffccd42acd234384e773d7`
- Target: `aarch64-unknown-linux-gnu`; server listens on container port 8080.

This is a local development instance, not a production deployment. It uses
loopback HTTP for convenience and an isolated Podman bridge without a trusted
proxy. Do not copy this local transport configuration to a public deployment.
No ferl server Docker/OCI image has been published; the future GHCR image is a
separate planned deliverable.

## Start and stop

The containers and credentials persist across stops. If the Podman machine is
not running, start it first. Then start PostgreSQL and wait for readiness before
starting ferl:

```sh
podman machine start podman-machine-default  # only if stopped
podman start ferl-cli-postgres  # only if stopped

until podman exec ferl-cli-postgres pg_isready -q -t 1 -U ferl_cli -d ferl_cli; do
  sleep 1
done

podman start ferl-cli-server  # only if stopped
curl --fail-with-body http://127.0.0.1:18080/api/health/ready
```

The server checks database/schema readiness after startup. If it stays unready,
inspect its logs with `podman logs ferl-cli-server`; do not rerun migration or
bootstrap as an automatic restart action.

Stop both containers without deleting the database:

```sh
podman stop ferl-cli-server ferl-cli-postgres
```

The database has already been migrated and bootstrapped. Ordinary restart uses
neither `migrate` nor `bootstrap-admin`; the bootstrap result is one-time, and
the saved admin key is needed for API tests. The sample link is synthetic and
can be deleted through the API, but its short code remains reserved in this
local database.
