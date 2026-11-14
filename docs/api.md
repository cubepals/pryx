# Public API

Authenticate with `Authorization: Bearer <key>`. Create, list and revoke keys with `pryx key` on the private operator listener. The project is chosen by the key. See control/README.md for configuration and sdk/typescript/README.md or sdk/python/README.md for examples.

| Method | Endpoint | Request schema | Success |
| --- | --- | --- | --- |
| GET | /v1/project | — | 200 |
| POST | /v1/sandboxes/{id}/ssh | SshRegister | 200 |
| DELETE | /v1/sandboxes/{id}/ssh | — | 200 |
| POST | /v1/sandboxes/{id}/token | — | 200 |
| POST | /v1/sandboxes/{id}/network | — | 200 |
| GET | /v1/sandboxes/{id}/data/content | — | 200 |
| PUT | /v1/sandboxes/{id}/data/content | — | 204 |
| GET | /v1/sandboxes/{id}/data/list | — | 200 |
| GET | /v1/sandboxes/{id}/data/stat | — | 200 |
| POST | /v1/sandboxes | MakeRequest | 201 |
| GET | /v1/sandboxes | — | 200 |
| POST | /v1/sandboxes/{id}/lifetime | LifetimeRequest | 200 |
| GET | /v1/sandboxes/{id}/metrics | — | 200 |
| POST | /v1/templates/{name}/versions | TemplateVersionRequest | 201 |
| GET | /v1/templates/{name}/versions | — | 200 |
| POST | /v1/webhooks | WebhookRequest | 201 |
| GET | /v1/webhooks | — | 200 |
| DELETE | /v1/webhooks/{id} | — | 200 |
| GET | /v1/webhooks/{id}/deadletters | — | 200 |
| GET | /v1/sandboxes/{id} | — | 200 |
| DELETE | /v1/sandboxes/{id} | — | 200 |
| POST | /v1/sandboxes/{id}/start | — | 200 |
| POST | /v1/sandboxes/{id}/stop | — | 200 |
| POST | /v1/sandboxes/{id}/sleep | — | 200 |
| POST | /v1/sandboxes/{id}/wake | — | 200 |
| POST | /v1/sandboxes/{id}/rest | — | 200 |
| POST | /v1/sandboxes/{id}/exec | ExecRequest | 200 |
| GET | /v1/sandboxes/{id}/logs | — | 200 |
| PUT | /v1/sandboxes/{id}/files/{path} | — | 200 |
| GET | /v1/sandboxes/{id}/files/{path} | — | 200 |
| POST | /v1/sandboxes/{id}/snapshots | — | 201 |
| GET | /v1/sandboxes/{id}/snapshots | — | 200 |
| PUT | /v1/sandboxes/{id}/file-chunks/{path} | — | 200 |
| GET | /v1/sandboxes/{id}/file-chunks/{path} | — | 200 |
| DELETE | /v1/sandboxes/{id}/snapshots/{snapshot} | — | 200 |
| POST | /v1/snapshots/{snapshot}/sandboxes | MakeRequest | 201 |
| GET | /v1/sandboxes/{id}/endpoints | — | 200 |
| GET | /v1/sandboxes/{id}/events | — | 200 |
| POST | /v1/templates | TemplateRequest | 201 |
| GET | /v1/templates | — | 200 |

Requests and responses are defined in [the OpenAPI document](../control/openapi/pryx.openapi.json). Exec and logs use application/x-ndjson; exec forwards live agent stdout/stderr chunks, a final exit event, and an error event for a failure after headers. File writes take JSON dataBase64; reads return a base64 tar archive. File paths are relative to persistent storage.

Use a stable Idempotency-Key for retried create or exec requests. Reusing one for different input is refused. An exec with an uncertain outcome is never replayed automatically. Lifecycle operations return the reached sandbox and, for sleep/wake, its tier.

TTL expires access immediately and upkeep deletes data. Idle is evaluated from activity and live sessions. Templates warm each active node once and retain a stopped snapshot. Public endpoints include their edge hostname/port; private endpoints are reachable only on the private network.

Errors have the node envelope `{error:{code,message,details}}`. Invalid fields answer 422, missing keys 401, forbidden operations 403, missing resources 404, state conflicts 409, expired TTL 410 and unavailable placement 503. API v1 evolves additively.

## Direct sandbox operations

POST /v1/sandboxes/{id}/token returns a short-lived sandbox capability and agent URL. SDKs cache it and send running processes, terminals, streamed files and runtime expose requests through the edge directly to /_pryx/. The project key is never sent to the sandbox. See [the agent API](agent.md) for its contract, file URLs, spare ports and network policy.
The /data/content, /data/list and /data/stat routes use the node files feature while compute is stopped or sleeping. Raw content streams as application/octet-stream; JSON requests stay bounded.

## Projects and limits

An operator creates a project with POST /operator/v1/projects on the private operator listener.
The body accepts name, key:true and any project limit; the response includes the project,
effective limits, usage, and one new raw key only when requested. GET on the collection lists
projects; GET/PATCH/DELETE /{id} views, changes or deletes one. POST /{id}/freeze stops its
sandboxes and refuses public writes; /unfreeze permits new work without restarting them.
GET /{id}/usage and public GET /v1/project show units and counts against effective limits.

`pryx project create --max-units 5 --max-running 3 --max-ttl 2h --expires-in 24h` creates a
project; `pryx key create --project <id>` prints a key once. Project ls/get/set/freeze/unfreeze/rm
use the same operator listener. Set accepts --max-* flags, --allowed-images comma-separated
prefixes and --expires-in. A value of unlimited resets a numeric/image limit to inheritance.

Units default to 512 MiB and 250 CPU millis (PRYX_UNIT_MEMORY_MB/PRYX_UNIT_CPU_MILLIS).
Create accepts units (default one) or explicit resources rounded upward to whole units.
cpuReservationMillis records admission CPU, not an enforced ceiling. Container units retain their quotas. Process and Wasm units reserve CPU without a quota; explicit cpuMillis ceilings or cpuWeight requests on those tiers return 422 with the resource field identified. Use container isolation when those controls are required.
Node listings include unitsFree and regional totals after host and running reservations.
Project limits are maxUnits, maxSandboxes, maxRunning (including suspension with memory held),
maxUnitsPerSandbox, maxTtl, maxIdle, maxExecTimeout, maxSnapshots, maxSnapshotBytes,
maxEgressPorts, allowedImages and expiresAt. A refusal is 403 limit_exceeded with
details:{name,limit,current}. Unknown command outcomes retain their compute reservation until
a confirmed stop; they never let a retry exceed quota. Snapshot bytes are measured before
publication. Lower aggregate limits freeze excess usage; expiry deletes owned data and revokes
keys. Lists and all resource routes are project-scoped; foreign IDs answer 404.

Each limit has PRYX_PROJECT_<UPPER_SNAKE_NAME> as its deployment default, for example
PRYX_PROJECT_MAX_UNITS and PRYX_PROJECT_ALLOWED_IMAGES. Missing defaults are unlimited.
Non-null project columns override defaults; null restores inheritance. maxTtl/maxIdle also
supply omitted sandbox timers. Public image permissions always remain bounded by host policy.

## Isolation and software

Creation accepts isolation: container, process, wasm or vm. A node advertises its available tier;
placement and template warming require that tier, while project usage counts all tiers in units.
Container accepts host-permitted images. Process and wasm refuse images with a field-specific
422 invalid_request. Packages is an array of agentOS registry names, available only for wasm
and sent only after the node advertises wasm-packages. Names are not URLs or host paths.
The no-engine image default and measured comparison are in [tiers.md](tiers.md);
[agentos.md](agentos.md) describes persistence, software distribution and the agentOS boundary.

## SDK handles and live commands

The TypeScript and Python SDK READMEs contain executable handle examples. Commands start on
call: run returns a native promise/task or sync result, stream iterates output with lines/result.
Each command receives one idempotency key; mutations never retry automatically. A stream ending
without its exit has an unknown outcome. Public exec and logs forward agent frames into their
published v1 records; failures after exec headers use an error event.

`PUT /v1/sandboxes/{id}/file-chunks/{path}` accepts offset (a 32 KiB boundary) and dataBase64
(up to 32 KiB decoded); GET returns a base64 tar member named data. These v1 JSON representations
remain for HTTP callers; SDKs use binary streams directly. Control forwards binary bytes to the
agent, with stopped data served by the node. Positional JSON writes spool existing bytes to a
private control temp file before replacement, keeping RAM bounded. Concurrent writers must be
serialized by callers; multi-request writes are not atomic. Decoded JSON file reads use the node-owned JSON memory bound; binary streams and chunk reads have no total file bound. No sandbox command or archive
extraction is involved. Paths remain relative, without dot segments or symlink components.

## Field surface

Create accepts env, labels, project-unique name, metadata as a JSON object, absolute workdir, numeric user, idle and autoDelete. The requested user must equal the operator container UID policy; process/wasm refuse user selection. autoDelete requires idle and deletes after idle instead of retaining compute/data. GET sandboxes filters by name, label subset and state. Names remain reserved until deletion. SDK id/name lookup wakes sleeping/resting data unless wake:false.

POST /v1/sandboxes/{id}/lifetime takes exactly one positive keepAlive, ttl or idle in seconds, bounded by project ceilings and expiry. keepAlive extends access and touches activity; ttl replaces the deadline from now. GET /metrics returns the last node CPU, memory, disk and network sample without polling; since returns at most one day of minute samples. Unavailable counters are null. SDK waitUntil polls public state; events uses NDJSON with stream=true, optional since timestamp and follow=true. Plain v1 events still returns its published JSON shape.

The SDK process handle adds waitForPort (HTTP or TCP, path/status/timeout), waitForLog, waitForExit and logs. A deadline is wait_timeout; exit before readiness is process_exited. run/spawn accept one-shot stdin text or bytes. Sandbox processes and killAll inspect/stop owned processes. shell opens a serialized stateful pipe session with separate stderr and exact exit codes; close ends it. See agent.md for all versioned routes and budgets.

Files add exists, find glob, grep and literal replacement at regex matches, chmod, caller-disk upload/download with onProgress, tar uploadDir/downloadDir and SHA-256 upsert sync. CLI cp uses these methods. Git clone/status/diff/branch/checkout/add/commit/push/pull/log return parsed values with raw output. Clone supports path/branch/depth/token; commit supports author, diff supports staged, log supports limit. Tokens use an HTTPS host-scoped one-call credential helper, never a persisted credential file. Git clone/push/pull and their transient VM bridge default to five minutes with per-call timeout overrides, disable background maintenance and unset all trace destinations. Wasm clone metadata uses the persistent guest filesystem while checkout data stays mounted. Repo creation clones before returning and retains the sandbox identity if preparation fails.

pryx.template builds from image/isolation with copy pairs, env/workdir, run commands and ready; only a successful ready publishes a snapshot version. Creating from a version runs start in the sandbox data directory unless an explicit workdir is set. POST /v1/templates/{name}/versions registers an owned completed snapshot and start/plan; GET lists immutable versions. A referenced snapshot and hidden builder cannot be deleted while versions use them. SDK rebuild uses the same local context. CLI templates lists; template build -f pryx.template.ts imports its default plan.

Container egress:{allow:[hosts],deny:[hosts]} requires the announced egress-hostnames node feature and root-installed firewall. Deny overrides allow; a wildcard matches only subdomains. The bridge redirects HTTP/TLS to host gateways, resolves and pins public destinations at connect time, refuses raw IPs/non-public answers and blocks other new outbound protocols. TLS passes through with certificate checks intact. Process/wasm refuse this policy with the tier named. The request cannot configure firewall readiness or relax host policy.

## Lifecycle webhooks

POST /v1/webhooks takes url and kinds; GET lists, DELETE /{id} removes, GET /{id}/deadletters inspects exhausted delivery. Kinds are created, running, sleeping, woken, resting, deleted, failed, ttl_expired and idle_slept. Creation returns secret once; listing never does. Payload is {id,event,sandboxId,at}, with no sandbox env, metadata or secret values. A lifecycle transaction enqueues delivery atomically.

Verify pryx-signature as v1= plus lowercase hex HMAC-SHA256 using the full UTF-8 creation secret over pryx-timestamp + a literal dot + the exact raw body bytes. Compare in constant time, reject stale timestamps, and deduplicate pryx-id across retries. The timestamp is Unix seconds. Delivery resolves all addresses once, refuses non-public DNS, pins the chosen destination with original HTTPS SNI/Host, verifies TLS and never follows redirects. An operator may explicitly allow an owned private receiver with PRYX_WEBHOOK_ALLOW_HOSTS; projects cannot.

Delivery has a ten-second DNS/connect/response deadline, five total attempts, one/two/four/eight-minute delays, durable leases and dead letters. Delivery IDs remain stable and each attempt gets a fresh timestamp/signature. Receivers should return 2xx after durable acceptance. CLI webhook create URL --kinds created,deleted, webhook ls, webhook rm ID and webhook deadletters ID operate on the same scoped registrations.

CLI sandbox adds ps, spawn, kill-all, wait-port, wait-log, wait-exit, shell (interactive or --command), exists, find, grep, replace, chmod, sync, git and keep-alive/ttl/idle/metrics/wait. exec/run/spawn --stdin read caller input once. Create flags include --name, --labels, --metadata, --env, --workdir, --user, --auto-delete and --egress, or --spec JSON. sandbox ls accepts --name/--labels/--state and events accepts --since/--follow. Git uses repo by default; clone --path changes only that SDK handle.
