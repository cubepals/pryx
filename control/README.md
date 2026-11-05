# pryx control

Control places sandboxes on nodes, owns epochs and durable intent in Postgres, and keeps snapshots in an S3-compatible store. Nodes enroll and heartbeat over mutual TLS. Only an operator declares a host lost; silence changes health and stops new placements.

Run `bun install --frozen-lockfile` at the repository root, then `bun control/src/main.ts`. The daemon is built separately with `cargo build --locked`. A local Postgres can be started with:

```sh
docker run -d --name pryx-postgres -e POSTGRES_USER=pryx -e POSTGRES_PASSWORD=local-only -e POSTGRES_DB=pryx -p 127.0.0.1:5432:5432 postgres:17-alpine
export DATABASE_URL=postgres://pryx:local-only@127.0.0.1:5432/pryx
```

Supply a fleet CA certificate and its PKCS#8 private key as `PRYX_CA_CERT` and `PRYX_CA_KEY`, an operator token of at least 32 characters, and the store credentials. Keep private values in a secret store or a mode-0600 service environment file. Control applies its numbered migrations at startup. Every deployment has its own CA and database.

| Variable | Default | Meaning |
|---|---|---|
| DATABASE_URL | required | Postgres URL |
| PRYX_DEPLOYMENT | pryx | Stable deployment name |
| PRYX_NODE_LISTEN | 127.0.0.1:8443 | Private HTTPS node listener |
| PRYX_OPERATOR_LISTEN | 127.0.0.1:8081 | Private operator listener |
| PRYX_API_LISTEN | 127.0.0.1:8082 | Public API listener |
| PRYX_CA_CERT, PRYX_CA_KEY | required | Fleet CA PEMs; escaped newlines accepted |
| PRYX_OPERATOR_TOKEN | required | Operator bearer credential |
| PRYX_ENDPOINT_HOSTS | 127.0.0.1,localhost | Comma-separated certificate names and IPs |
| PRYX_JOIN_URL | first endpoint host and node listener port | HTTPS URL hosts join through |
| PRYX_PRYXD_BIN | /usr/local/bin/pryxd | Daemon handed to joining hosts; release signature beside it |
| PRYX_REGION_MAP | empty | Comma-separated source:destination mappings, or a JSON object |
| PRYX_PLACEMENT | balanced | balanced, binpack or spread |
| PRYX_HEADROOM_MB | 0 | Memory reserved for moves |
| PRYX_MEMORY_OVERCOMMIT | 4 | Placed memory / allocatable memory |
| PRYX_CPU_OVERCOMMIT | 1 | CPU admission multiplier |
| PRYX_CPU_PRESSURE | 0.8 | Balanced placement pressure boundary |
| PRYX_DISK_OVERCOMMIT | 1 | Disk admission multiplier |
| PRYX_CPU_MILLIS_PER_GB | 250 | CPU reservation when absent from a spec |
| PRYX_HEARTBEAT_SECONDS | 5 | Node heartbeat interval |
| PRYX_LEASE_SECONDS | 120 | Execution lease, after fences |
| PRYX_SUSPECT_SECONDS | 15 | Silence excludes new placements |
| PRYX_UNAVAILABLE_SECONDS | 45 | Silence marks observations unavailable |
| PRYX_NODE_CERT_DAYS | 30 | Node certificate lifetime |
| PRYX_RENEW_DAYS | 10 | Remaining lifetime that triggers renewal |
| PRYX_SUSPEND | off | on uses suspend where the node announces it |
| PRYX_UPGRADES | on | Offer a signed daemon to older nodes |
| S3_ENDPOINT, S3_BUCKET, S3_ACCESS_KEY, S3_SECRET_KEY | required | S3-compatible archive store |
| S3_RUNTIME_ENDPOINT | S3_ENDPOINT | Store address reachable from nodes |
| S3_REGION | auto | Signing region |

Upkeep runs every minute: health and probes, unfinished placements, archives, deleted copies, local-copy trimming, old events, TTL deletion and idle sleep. An exec, start, wake or edge session counts as activity. TTL is measured from creation; missing TTL means forever. Missing idle means no automatic sleep. Snapshots stop application writes before copying. Rest uploads a snapshot before releasing local data. Wake restores a resting sandbox under a new epoch.

For a host to join, set `PRYX_OPERATOR_API` to the operator listener and `PRYX_OPERATOR_TOKEN`, then run `bun cli/src/main.ts node token <region>`. Its `joinCommand` downloads the pinned CA, verifies its hash, fetches `join.sh` over that CA and installs Docker, nftables and pryxd. Append `--allow-image <trusted-prefix>` for each image prefix the host may run. Empty policy refuses every image. `node ls`, `node drain <id>`, `node events` and the lifecycle commands use the same authenticated operator API.

`bun run check` runs Biome, TypeScript, generated node-schema drift checks and unit tests. Set DATABASE_URL to a local test Postgres with permission to create databases so the database suites run. `bun run e2e` uses Docker, an S3 store, and debug/static daemons named by PRYXD_BIN and PRYXD_STATIC_BIN. Those full suites run in CI; a local unit pass says nothing about their result.

## Public API

`PRYX_API_LISTEN` defaults to `127.0.0.1:8082`. Use a trusted TLS reverse proxy for remote
API access. Set `PRYX_PROJECT` (default `default`) and `PRYX_PUBLIC_DOMAIN` (default `localhost`).
`pryx key create --name agent` uses the operator API and prints a new key once. Supply it as
`Authorization: Bearer <key>`; only its SHA256 is stored. `pryx key ls` lists metadata and
`pryx key revoke <id>` immediately revokes it. The operator token is never a public API key.

| Method | Path under /v1 | Result |
| --- | --- | --- |
| POST | /sandboxes | Create from exactly one image, snapshot or template; optional Idempotency-Key |
| GET | /sandboxes | Project sandboxes |
| GET, DELETE | /sandboxes/{id} | View, or delete data and snapshots (`existed`) |
| POST | /sandboxes/{id}/start, /stop, /sleep, /wake, /rest | Reached state; sleep/wake tier |
| POST | /sandboxes/{id}/exec | NDJSON stdout, stderr, exit; optional Idempotency-Key |
| GET | /sandboxes/{id}/logs | NDJSON; tail, since, follow |
| GET, PUT | /sandboxes/{id}/files/{path} | Base64 tar read; JSON dataBase64 write |
| POST, GET | /sandboxes/{id}/snapshots | Capture or list |
| DELETE | /sandboxes/{id}/snapshots/{snapshot} | Delete snapshot |
| POST | /snapshots/{snapshot}/sandboxes | Fork |
| GET | /sandboxes/{id}/endpoints | Private addresses and computed public hostname |
| GET | /sandboxes/{id}/events | Ledger; optional since timestamp |
| POST, GET | /templates | Warm an image on active nodes; list readiness |

The committed OpenAPI document is in `control/openapi/pryx.openapi.json`. Run
`bun run openapi:generate` after schema changes. Errors use `{error:{code,message,details}}`.
Create accepts the node's resources/storage/restart shapes, entrypoint, env, secrets and labels;
public ports use `{name,port,protocol,audience}`. Container is the supported isolation level.
The daemon's image policy and limits remain final authority. Defaults are 1024 MiB memory,
1 GiB declared storage mounted at /data, no ports, no TTL/idle timer, start true.

`ttl` is seconds from creation: an expired sandbox answers gone immediately and upkeep deletes it.
`idle` is seconds since activity: upkeep sleeps it only with no live edge session. Templates run
once on each active node, capture a snapshot and stop; upkeep retries unavailable copies and warms
new nodes. Template names are immutable: use a new name for another spec. Backing sandboxes are
excluded from ordinary listings. A template must have a ready copy before use. Snapshots preserve
persistent data, not process memory; snapshot consistency requires a stopped application.

Exec output is buffered by the node before its NDJSON events are emitted. An exec with an unknown
outcome remains fenced from automatic replay, including after a control restart. Inspect before
using a new key. Per-command env runs env(1) inside the image. File transport uses sh, realpath,
tar and base64 inside the container, only under its persistent mount; writes are limited to 48 KiB.

`PRYX_API_URL` and `PRYX_API_KEY` configure `pryx sandbox` and `pryx run`. Create accepts
`--image`, `--ttl`, `--idle`, `--stopped` or `--spec <json file>` for the complete create body.
`pryx run --image alpine:3.22 -- echo hello` creates a temporary shell image, executes and deletes
it, preserving the command's exit status. `--snapshot` first forks a stopped copy and rests it;
the snapshot and retaining sandbox ids are written to stderr. Remove that sandbox when finished.

## Edge protocol

Set `PRYX_EDGE_TOKEN` to the same secret on control and each edge. If absent, all /edge/v1
requests are refused. This token is independent of public keys and the operator token.
`PRYX_EDGE_PORT_RANGE` is `20000-20100` by default. Keep this API listener on a private network.
Every request names an edge in x-edge-id. GET /edge/v1/routes returns routes and the database's
monotonic x-routes-version; the edge sends x-routes-held only after applying all of a version.
ETags avoid unchanged bodies. POST /wake, /ping-wake, /idle, /sessions and /visits handle wake and
activity. The contract is control/openapi/edge.openapi.json; generated edge types are checked
for drift. See edge/README.md and docs/edge.md for fronts, network policy and measurements.

Public port views now include publicPort. Container-only private UDP still works; the public
edge accepts TCP. An unready or sleeping route names 0.0.0.0:0. A running public sandbox is
shown as starting while counted edges catch up. This display barrier never declares a node lost.

## Release binaries and bootstrap

`bun run release:build` compiles pryx, pryx-control and pryx-edge and packs
`pryx-control-assets.tar.gz`. Set `PRYX_ASSET_DIR` to the extracted control directory for a
compiled control process; without it, source runs use files beside the modules. The directory
contains migrations, join.sh and daemon.json. The release workflow also packs both SDKs and the
public/edge contracts before checksumming and attesting the assets. SDK versions match Cargo.toml.

`PRYX_BOOTSTRAP_API_KEY` is optional and has no default. Up generates it in control.env (0600);
control inserts only its hash once with name initial. Revocation survives control restarts. The
bootstrap key is never logged by control. Up records its one-time display before printing, so a
crash there may require a new operator-created key. Node enrollment retains its existing private
identity files; see docs/operations.md for that environment-file-only exception.

## Machine providers

Provider calls use only the operator listener; `pryx node shapes <region> --provider hetzner`,
`pryx node acquire <region> --provider hetzner --shape <catalog-name> --idempotency-key <key>`,
`pryx node acquisition <id>`, and `pryx node release <node>` expose their state.
`pryx node cancel-acquisition <id>` cancels an unused enrollment and returns its known rental.
Keep the same acquire key when retrying an uncertain answer.

| Provider | Environment and preparation |
| --- | --- |
| Hetzner | `PRYX_HETZNER_TOKEN`, `PRYX_HETZNER_NETWORK_ID`, `PRYX_HETZNER_FIREWALL_ID`, `PRYX_HETZNER_SSH_KEYS` (JSON array of registered key ids/names); optional `PRYX_HETZNER_IMAGE` (Ubuntu 24.04 by default). Create the private network/subnet, firewall and SSH key once. Allow private control/node/S3 traffic and required outbound bootstrap traffic. |
| Boat | `PRYX_BOAT_TOKEN`, `PRYX_BOAT_REGION` (logical grouping only), `PRYX_BOAT_SHAPES` (JSON shape/price catalog), `PRYX_BOAT_NETWORK_SETUP`, `PRYX_BOAT_PRIVATE_ADDRESS_COMMAND`. Supply your private-network bootstrap. Docker and systemd must prove usable before enrollment; an unproven VM is retained for inspection or explicit cancellation. |
| Manual | `PRYX_MANUAL_ADDRESS` (private IP), `PRYX_MANUAL_SHAPE` (JSON shape), optional `PRYX_MANUAL_REGION`, `PRYX_MANUAL_SSH_ADDRESS`, `PRYX_MANUAL_SSH_KEY` (path), `PRYX_MANUAL_SSH_USER`, `PRYX_MANUAL_SSH_PORT`. Install the SSH host key in known_hosts beforehand. This host is never released. |

Use a stable, backed-up `PRYX_PROVIDER_SEAL_KEY` (at least 32 characters) to derive pending
one-time enrollment secrets. Its fallback is the operator token; rotating that token without
retaining the seal key invalidates pending enrollments. Provider credentials remain in the
process environment. Set `PRYX_PROVIDER_ALLOW_IMAGES` to explicit comma-separated host policy
prefixes; it defaults to empty. Enrollment still requires the ordinary one-time token.

Automatic growth/shrink is off until `PRYX_AUTOSCALE=on`. Set `PRYX_AUTOSCALE_REGIONS` to a JSON
region-to-provider map. Defaults: `PRYX_REGION_MEMORY_FLOOR_MB=0`, `PRYX_NODE_IDLE_SECONDS=3600`,
`PRYX_REGION_MIN_NODES=1`. Two consecutive actual room refusals trigger one fitting rental;
a successful placement clears the streak. Pending acquisitions suppress additional growth.
Shrink drains, stores every live snapshot, retires, then returns only tracked managed hosts.
See [providers](../docs/providers.md) for safety, limitations and measurement status.

## Experimental store lease

`PRYX_STORE_LEASE=on` enables conditional-store authority only for nodes that advertise
store-lease after explicit host configuration. Term/renewal settings are
`PRYX_STORE_LEASE_SECONDS=30` and `PRYX_STORE_LEASE_RENEW_SECONDS=10`. Control first probes the
store's conditional writes, grants ephemeral URLs before starts, refreshes grants on upkeep,
and displaces a copy only after an expired-object CAS fence. It keeps the node active and restores
a ready stored snapshot elsewhere under a new epoch. Missing data/capacity stays blocked.
[store-lease](../docs/store-lease.md) documents the required cgroupfs/watchdog/non-suspending host
policy, maintenance effects, measured-versus-modeled costs, and CI-only fault experiments.
