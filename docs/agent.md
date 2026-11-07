# The sandbox agent

`pryx-agent` runs as PID 1, as the image's user and within the sandbox's resource limits.
The node mounts its static binary read-only; images need no installed shell or file utilities
for agent operations. The original image entrypoint remains a supervised child. SDK-created
sandboxes use the agent itself to stay alive.

Control owns creation, placement, sleep, wake, network changes, snapshots and deletion. Running
commands, processes, terminals, data and runtime port forwarding go directly through the edge to
the agent. A Postgres regression counts actual queries: after token warmup, these SDK operations
make zero control calls and zero SQL queries. Token renewal and lifecycle still contact control.

The node announces `agent`, `files` and `network-policy` before control sends their new fields.
Every supported node runs the agent. The v1 node wire retains its published endpoints.

## Capabilities and routing

`POST /v1/sandboxes/{id}/token` requires the project's API key and returns a sandbox capability,
expiry, epoch, public key and agent base URL. Capabilities expire within five minutes, are bound
to one sandbox and epoch, and enforce project execution and spare-port limits. The node's
private Ed25519 key stays in the container's private bootstrap environment; records, labels,
responses and logs never contain it. Child process environments omit the bootstrap entry.

The SDK caches its capability, renews it before expiry, and invalidates it on lifecycle refresh.
HTTP uses `Authorization: Bearer TOKEN`. Websockets offer `pryx` and `pryx-token.TOKEN` as
subprotocols; the negotiated subprotocol is `pryx`. TLS verification remains enabled.

The edge verifies capabilities locally before wake. The only private API on
`https://SANDBOX.PUBLIC_DOMAIN` is `/_pryx/`. A local one-box installation also accepts
`http://localhost/sandboxes/SANDBOX/_pryx/`. Unexposed application ports return 404.
Ready requests do not ask control for permission again. Route-only edge capabilities can list
port assignments but cannot run commands or read data.

## HTTP and websocket API

All paths below start with `/_pryx`. The committed Rust schema is
[agent/openapi.json](../agent/openapi.json); node capabilities remain in the node schema.

| Operation | Path |
| --- | --- |
| Start or list processes | `POST` or `GET /processes` |
| Process metadata | `GET /processes/{id}` |
| Wait for completion | `GET /processes/{id}/wait` |
| Write or close stdin | `POST` or `DELETE /processes/{id}/stdin` |
| Signal or resize | `POST /processes/{id}/signal` or `/resize` |
| Output | `GET /processes/{id}/output` (NDJSON), `/ws` (websocket) |
| Interactive PTY | websocket `/terminal` |
| Read, write or delete data | `GET`, `PUT` or `DELETE /files/content?path=PATH` |
| List or stat data | `GET /files/list?path=PATH`, `/files/stat?path=PATH` |
| Create directory or move | `POST /files/mkdir`, `/files/move` |
| Watch a directory | websocket `/files/watch?path=PATH` |
| Transfer from or to an HTTP URL | `POST /files/upload`, `/files/download` |
| Issue a short file URL | `POST /files/token` |
| Expose or list ports | `POST` or `GET /ports` |
| Unexpose | `DELETE /ports/{port}` |
| Health | `GET /health` |

Starting a process accepts `command` as an argv array or shell string, `env`, `cwd`, `stdin`,
`pty`, `cols`, `rows` and an optional timeout in seconds. Supply `Idempotency-Key`; the SDK
generates one. A reused key with a different body is refused. Requests never retry a mutation.
Processes outlive disconnected clients. Completion waits for the actual process and output,
including commands that close stdout and stderr early. Signals address the process group.

Output events carry base64 byte chunks, followed by an exit result. Detached replay is bounded
to 1 MiB; a slow subscriber has a bounded queue and is detached after backpressure times out.
Results expose truncation. SDK collected output and line buffers are bounded; use `stream()`
for large or indefinitely long output. PTYs provide a controlling terminal and support resize.

Files stream without a total body-size cap; JSON requests are limited to 1 MiB. Paths are
relative to persistent data, opened with descriptor-relative walks that refuse parent symlinks,
special files and traversal. A watch first sends `ready`, then `change` messages; overflow is
explicit. URL transfers accept only HTTP and HTTPS destinations without user information.
They resolve the hostname before connecting, refuse every non-public answer (including
loopback, private, link-local and multicast addresses), and pin connections to that first
answer set. Redirects and transfer-client proxy overrides are disabled; HTTPS certificate
and hostname verification remain enabled. This applies to container, process and Wasm agents.
`files.url(path,
'GET' | 'PUT')` returns a capability URL bound to that path and method for at most one minute,
and never beyond the parent token. Treat the URL as a credential until it expires.

## Ports, activity and network

`sparePorts` reserves edge-only host bindings at creation, defaulting to three. Projects reserve
their quota for these bindings up front. `expose(port)` assigns a spare and returns
`https://SANDBOX-PORT.PUBLIC_DOMAIN`; no restart or spec update occurs. Exhaustion returns
409 `no_spare_port`. Unexpose closes existing forwards and frees the slot. Assignments survive
agent restart in an atomic checkpoint. A target header checked by the agent prevents a stale
hostname from reaching a reused slot; it is removed before the application sees the request.
On the process tier, the agent also proves the connected application socket belongs to its
supervisor tree before sending request bytes. Foreign sockets are refused even for the same
UID. This proxy check does not isolate direct guest networking; Internet-enabled process
workloads must be mutually trusted as described in [process-tier.md](process-tier.md).

Activity stays in agent memory and is checkpointed once per second. Existing node heartbeats
carry activity and assignments to control and edge polling. Idle upkeep honors recent activity
and active streams; the sandbox's absolute TTL still applies.

`network: 'none'` uses an internal Docker bridge with inter-container communication disabled;
agent access from the node and edge remains available. `box.network('internet' | 'none')`
changes the policy while retaining persistent data. It replaces compute, so running processes
and terminals end. The SDK invalidates the old capability. Hostname policy is described below.

The node's file feature supplies read, write, list and stat while compute is stopped or sleeping.
The SDK selects that path automatically. Interactive operations use the agent and may wake a
sleeping sandbox. MCP uses the same capability boundary.

## Validation and measurements

Public exec and logs forward through the edge to the agent. Entrypoint output is replayed from
a bounded persistent logbook; a slow live reader receives an explicit gap. Private activity,
port and log checkpoints are hidden from normal listings and watch events. Stopped file writes
use the sandbox's configured owner, so the agent can update them after start.

The supervisor disables process dumps and same-user proc inspection before loading its private
material. Signing keys live outside sandbox data under the node state directory, mode 0600,
and survive compute replacement within the same epoch. A placement change uses a new key.
The fleet join script installs the trusted agent before starting the node. Control serves its
installed agent from the Rust-owned default path, or PRYX_AGENT_BIN when configured.

For the HTTPS sandbox hostnames, configure PRYX_EDGE_AGENT_TLS_CERT and
PRYX_EDGE_AGENT_TLS_KEY with paths to a trusted wildcard certificate and its private key.
The edge terminates TLS only for agent and exposed-port hostnames; ordinary TLS routes remain
pass-through. A platform TLS terminator can instead forward to the HTTP front. Localhost SDK
URLs use the HTTP alias. TLS verification is never disabled. Health reads wait for the agent
after wake before a user mutation is forwarded.

Rust tests execute real local processes, PTYs, websocket streams, notifications, safe file walks,
file capabilities and spare-slot reuse. The Docker acceptance suite exercises the TypeScript,
synchronous Python and asynchronous Python handles through a real node and edge. Container
acceptance is run in CI. [speed.md](checks/speed.md) records measured cohorts and their limits.

## Field API version 2

Authenticated health announces `apiVersion: 2`. SDKs check it before every new route family,
cache that answer per capability and invalidate it after lifecycle changes. New routes are:

| Operation | Path |
| --- | --- |
| Readiness: kind port/log/exit, timeout seconds | `POST /processes/{id}/ready` |
| Kill every running owned process | `POST /processes/kill-all` |
| Open a cwd/env shell session | `POST /shells` |
| Run a command with timeout | `POST /shells/{id}/run` |
| Close the shell | `DELETE /shells/{id}` |
| Glob find, regex grep/replace, chmod | `POST /files/find`, `/grep`, `/replace`, `/chmod` |
| Stream a directory tar | `GET` or `PUT /files/archive?path=PATH` |

Readiness never selects an outside address. Process listeners must belong to the supervisor;
VM probes use that VM network. Log waits search bytes per stream, including split UTF-8,
with a 1 MiB capture bound. Timeout and early exit have distinct structured codes. One-shot
SDK stdin writes bounded chunks then closes once; partial delivery is an uncertain mutation.

Shells use one pipe-driven read/eval process, serialize commands, encode each frame and capture
independent random stdout/stderr sentinels. This preserves cwd, env, functions, exact exit codes
and separate streams. Thirty-sample measurements compare fresh commands and persistent sessions
in [K16 evidence](checks/k16.md). There are at most 32 sessions; output is bounded to 1 MiB per
stream. A timeout or framing failure closes the session. Intentional exit returns its real code.

Searches refuse links/special files, scan at most 100,000 entries and 64 MiB total, with 1 MiB
per text file and response. Regexes have a linear-time engine and a 1 MiB compile bound.
Replace treats replacement text literally and retains ordinary file modes. Tar transfers preserve
files and empty directories, refuse links/devices/traversal and special permission bits, cap
packed and expanded bytes at 256 MiB and entries at 100,000. Two archive slots bound anonymous
on-disk spools; download retains its slot until the body ends. Blocking file work is tracked and
drained before VM stop/snapshot. Local SDK disk walks also hold no-follow parent descriptors.
Sync hashes content and upserts changes; it never deletes remote data.

Git tokens stay in the one-call process environment and a host-scoped helper, never a URL or
credential file. Hooks, global/system config, redirects and trace environment settings are
disabled for helper calls; results redact the token and refuse incomplete parsed stdout.
On wasm, a transient loopback bridge adapts Git HTTP to the VM fetch stack, retains the original
HTTPS remote and terminates after the call. VM network policy and upstream TLS remain in force.
Container hostname egress is enforced by the host bridge; process and wasm refuse it explicitly.

Wasm Git clones keep their object database in the VM’s persistent guest filesystem via Git’s
separate-git-dir option; checkout files stay in persistent data. This avoids the host mount’s
read-only object-write limitation without relaxing permissions. Both are retained by stop and
snapshot. Git network helpers have a five-minute default and per-call timeout overrides and disable automatic background
maintenance/GC. Virtual Git HTTP request bodies are bounded by the 16 MiB guest frame limit.
