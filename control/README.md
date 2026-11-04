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
