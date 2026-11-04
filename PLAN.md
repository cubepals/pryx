# The plan: from blocklyd to pryx

This file is the whole build, in order. Each section is one milestone and would normally be one
pull request; run in one task, each milestone is one or more commits on this branch, in order,
finished before the next begins.

## How to run this plan

- Read AGENTS.md first; it governs every milestone.
- Work autonomously from K1 through K8. Never stop to ask a question. Where a brief leaves a
  choice, take the simplest option that keeps every constraint, and record it in `docs/decisions.md`
  (one line per decision: milestone, the choice, why). Where something cannot be done here (no
  Docker, no network to a provider, no secret), do everything that doesn't depend on it, write
  what is missing in `docs/decisions.md`, and go on.
- If `cargo` is missing, install it first and use the toolchain the repository pins:
  `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none`,
  then `. "$HOME/.cargo/env"`, then `rustup toolchain install` in the repository root. If `bun` is
  missing, `curl -fsSL https://bun.sh/install | bash` and put `$HOME/.bun/bin` on PATH. If the
  network refuses these, say so in `docs/decisions.md` and continue with what can be checked.
- Commit at the end of each milestone, and more often inside large ones, with subjects of 50
  characters or fewer that say the change, no motive, no `@`. Do not open pull requests from
  inside the task; the branch is pushed when the task ends.
- Run the checks each brief names before its commit. Docker-backed and end-to-end suites may not be
  runnable here; never claim one passed when it did not run. The final message lists, per
  milestone, what was built, what ran, what did not, and every decision recorded.
- Where a brief says "two repositories are checked out", cubepals/cubepals is beside this one at
  `/workspace/cubepals`; if it is not there, clone it read-only from
  `https://github.com/cubepals/cubepals` into that path. Copy from it; never change it.
- Where a brief says "pull request" or "CI", read it as: the commit for that milestone, and the
  workflow files you add or change, which run once the branch is pushed.

## K1: blocklyd becomes pryxd

```
Goal

Rename this daemon from blocklyd to pryxd everywhere a person or a host sees a name, while keeping
the node protocol v1 and the on-disk format v1 exactly as they are, so that a control plane written
against blocklyd 0.3.x keeps working against pryxd with no change but the binary's name.

Context

This repository is the former cubepals/blocklyd, with its history. Read AGENTS.md first, then
README.md, docs/protocol.md, Cargo.toml, src/main.rs, src/lib.rs, deploy/blocklyd.service,
deploy/daemon.json, Dockerfile, examples/blocklyd.toml, .github/workflows/ci.yml and release.yml,
src/upgrade/trust.rs, src/cli/, src/fleet/, tests/formats.rs and tests/fixtures/.

Names a person or host sees, which change to pryx or pryxd: the crate and binary name in Cargo.toml
and the clap `name` and `about` in src/main.rs; the systemd unit (deploy/pryxd.service, its
Description and Documentation, and the paths in it); the configuration directory /etc/pryxd and
file pryxd.toml; the state directory default /var/lib/pryxd; the binary's install path
/usr/local/bin/pryxd and bin/pryxd and bin/pryxd.prev under the state directory (src/cli/upgrade.rs,
src/upgrade/); the Dockerfile's outputs (/pryxd, and the unit's name under /deploy) and image
labels; the release workflow's asset names (pryxd, pryxd.sig, pryxd.service) and the image name
ghcr.io/cubepals/pryxd; the release signature's statement in src/upgrade/trust.rs and release.yml
("pryxd release" in place of "blocklyd release": no node exists that verifies the old statement);
the release key secret's name (PRYXD_RELEASE_KEY) in release.yml; the debug-only environment
variable PRYXD_TEST_VERSION, read beside the old BLOCKLYD_TEST_VERSION, which a control plane's
test suite still sets; the README, CONTRIBUTING.md, SECURITY.md, SUPPORT.md, the issue templates,
the examples file (examples/pryxd.toml), the journal and log lines, the metrics' namespace
(pryxd_… in place of blocklyd_…), and every comment and doc string.

Names that are format v1 and do NOT change, each pinned by tests/formats.rs and tests/fixtures/:
the blocklyd-epoch and blocklyd-protocol headers; the blocklyd.* container labels, including
blocklyd.record; workload.json and the record's fields; the bk1. token prefix; the fleet endpoint's
paths (/fleet/v1/enroll, /heartbeat, /renew, /blocklyd, /blocklyd.sha256, /join.sh); the
daemon.json contents; the spec digest; the features list. Add one short section to
docs/protocol.md, "Format names", saying these are the names format v1 was first published with
and stay until protocol v2. Add one line to the README's License section saying pryxd 0.1.0
continues blocklyd 0.3.x, which was published as cubepals/blocklyd under the same licence, and
that 0.2.2 and earlier were published under AGPL-3.0-only as part of cubepals/cubepals.

Set `version` in Cargo.toml to 0.1.0: this pull request is a release.

The `contract` job in ci.yml checks out cubepals/cubepals at main and runs its
apps/control/src/infra/fleet/fleet-runtime.e2e.test.ts against this build. That suite sets
BLOCKLYD_BIN to the binary's path and BLOCKLYD_TEST_VERSION on the daemon it starts: give the job
the new binary path (target/debug/pryxd) under the variable name the suite reads, and keep reading
BLOCKLYD_TEST_VERSION in debug builds as an alias of PRYXD_TEST_VERSION. The job must stay green
without any change in cubepals/cubepals.

Constraints

- Within protocol v1 a change only adds (AGENTS.md). This pull request adds nothing to the
  protocol and removes nothing from it.
- tests/formats.rs and the fixtures under tests/fixtures/ keep passing unchanged: that is the
  proof the format is intact. If a test there has to change, the rename went too far; undo that
  part.
- The structure check (CI's Structure step) and `cargo deny` keep passing; the baseline does not
  grow.
- Docker-backed tests (tests/docker.rs) run in CI, not here. Say so in the pull request.
- The pull request's title is 50 characters or fewer, states the change and no motive, and
  contains no `@`.

Done when

- `cargo build --locked` makes target/debug/pryxd; `pryxd --version` prints 0.1.0; `cargo test
  --locked`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` and
  `cargo deny --locked check` pass locally.
- `grep -rn blocklyd --include=*.rs --include=*.md --include=*.toml --include=*.yml --include=*.json --include=*.service --include=*.sh .`
  lists only: the format names above and their tests and fixtures, the protocol.md "Format
  names" section, the README's history line, the BLOCKLYD_TEST_VERSION alias, and git history.
  Paste that grep's output in the pull request.
- CI is green on the pull request, the `contract` job included.
- The pull request body lists every renamed path and asset, and says which suites ran locally and
  which ran only in CI.
```

## K2: the game leaves the daemon

```
Goal

Make pryxd's defaults, examples and words those of a general sandbox daemon: nothing in the crate,
its docs or its examples assumes a Minecraft server, and a host runs no image until its policy
names which.

Context

Read AGENTS.md, README.md, docs/protocol.md, src/minecraft.rs, src/protocol/spec.rs,
src/protocol/spec/validate.rs, src/config.rs, src/doctor/, src/cli/join.rs, examples/pryxd.toml,
src/protocol/transfer.rs, src/manager.rs and src/manager/archive.rs, src/tarball.rs, src/lib.rs,
src/main.rs, and `grep -rni 'minecraft\|world\|server' src docs examples README.md`.

What changes:

- src/minecraft.rs holds three port constants (25565, 25575, 24454) for the tests' example specs.
  Replace it with src/examples.rs holding generic example ports for tests (an HTTP port, a second
  TCP port, a UDP port), named for what they are in the examples, and update every test that used
  the old module. The protocol never named these ports; nothing on the wire changes.
- The default of `[workloads] allowed_images` becomes empty: a host whose policy names no prefix
  refuses every spec with 422 invalid_request, field `image`, problem "no image prefix is allowed
  on this host". `pryxd join` takes `--allow-image <prefix>` (repeatable) and writes them into the
  configuration it makes; `pryxd doctor` warns when the list is empty. examples/pryxd.toml shows the
  key with a comment and a generic example prefix.
- Words. Where a comment, doc string, log line, error message or doc page says "server" meaning
  the workload, it says "sandbox"; where it says "world" meaning the workload's data, it says
  "data". The protocol's own vocabulary, "workload", stays as it is in paths, types and
  docs/protocol.md. Test fixtures and test data may keep any file names they have.
- docs/protocol.md: the section "How Blockly's MinecraftRuntime maps onto it" becomes "How a
  control plane maps onto it", keeping the table of verbs, with the Cubepals-specific handle
  description reduced to one sentence that says a control plane keeps the node, epoch and host
  ports in whatever handle it issues. The example spec in the PUT section uses a generic image and
  generic ports. The comment in src/protocol/transfer.rs about the caller pausing the workload's
  own writes keeps its meaning with a generic example.
- The README's opening describes pryxd as a sandbox node daemon in two sentences; the rest of the
  README keeps its sections.

Constraints

- Nothing on the wire or on disk changes: tests/formats.rs and tests/fixtures/ pass unchanged, and
  openapi/ is regenerated and shows no change.
- Within protocol v1 a change only adds. The `--allow-image` option and the doctor warning are
  the daemon's own; no new protocol field.
- `cargo test --locked`, fmt, clippy and `cargo deny` pass locally; Docker-backed tests run in CI;
  the `contract` job stays green with no change in cubepals/cubepals.
- Title under 50 characters, no motive, no `@`.

Done when

- `grep -rni minecraft src docs examples README.md deploy` prints only the README's history line
  and docs/protocol.md's "Format names" section. Paste the grep's output in the pull request.
- A spec sent to a host with an empty `allowed_images` is refused as described, with a test.
- `pryxd join --allow-image` is tested, and `examples/pryxd.toml` documents it.
- CI green, the `contract` job included.
```

## K3: the control service

```
Goal

Add `control/`, pryx's control service: a TypeScript service on Bun with Postgres that nodes
enroll with and heartbeat to, that places sandboxes on nodes under epochs, keeps their snapshots
in an S3-compatible store, moves and rebuilds them, runs upkeep, and offers an operator API with a
`pryx node` command line. It is moved from Cubepals' fleet runtime, which already does all of
this inside Cubepals' control plane; here it stands alone and speaks of sandboxes, not servers.

Context

Two repositories are checked out: this one (`pryx`) and `cubepals/cubepals` (`cubepals`), for
reading and copying from, never for changing. Read pryx's AGENTS.md first. Then in cubepals:
docs/fleet.md (how the fleet runtime works: parts, trust, state, placement, epochs, lease,
storage, moves, failure semantics, networking, upgrades), docs/fleet-operations.md,
apps/control/src/infra/fleet/ whole (the adapter: fleet-runtime.ts and fleet-runtime/*, registry.ts
and registry/*, placement.ts, node-endpoint.ts, node-client.ts, ca.ts, join-token.ts,
join-routes.ts, join.sh, operator-api.ts, operator-upgrades.ts, store.ts, sql.ts, wire.ts,
health.ts, handle.ts, fleet-logs.ts, daemon.json, the two OpenAPI copies and generated/), its
tests (*.test.ts) and the two end-to-end suites (fleet-runtime.e2e.test.ts, fleet-two-node.e2e.test.ts,
endpoint.e2e.ts), packages/db/src/schema/fleet.ts and the migrations 0040_fleet.sql,
0042_fleet_running_admission.sql and 0043_fleet_enrollment_key.sql, scripts/fleet.ts (the operator
CLI), tools/openapi/openapi.ts (how TypeScript types are generated from an OpenAPI document and
checked for drift), apps/control/src/app/ports/runtime.ts (the port the adapter implements, which
control/ replaces with its own model), and whatever the adapter imports from apps/control/src/app/
and infra/ (the archive store's presigned uploads in infra/s3, the config schema's FLEET_* keys in
config/schema.ts, the upkeep job's registration in infra/pg/jobs.ts). Cubepals' .github/workflows
show how its CI runs those suites (Postgres as a service, an S3 store in a container, a pryxd built
from source, BLOCKLYD_SUDO=1).

What control/ is:

- Layout: control/src/main.ts (the composition root: configuration from the environment,
  Postgres, the node endpoint, the operator API, upkeep on an interval), control/src/registry/
  (nodes, tokens, enrollment, heartbeats, renewal, lifecycle, upgrades), control/src/placement.ts,
  control/src/ledger.ts, control/src/admission.ts, control/src/rehoming.ts,
  control/src/transfers.ts, control/src/archives.ts, control/src/store.ts (S3 presigned PUT and
  multipart, moved from Cubepals' infra/s3), control/src/node-endpoint.ts, control/src/ca.ts,
  control/src/node-client.ts, control/src/workload-api.ts, control/src/operator-api.ts,
  control/src/sandboxes.ts (the model: SandboxSpec, Sandbox, SandboxState, the four verbs),
  control/src/durations.ts, control/src/schema/ (Drizzle schema and migrations, renumbered from
  0001; the tables keep their fleet_* names), control/src/generated/ (types generated from this
  repository's openapi/ by control/tools/openapi.ts, with a `check` that fails on drift),
  control/test/ (unit tests) and control/e2e/ (the two suites and the endpoint suite, moved and
  made to start a pryxd from target/debug/pryxd). cli/src/node.ts is `pryx node …`, moved from
  scripts/fleet.ts.
- Configuration from the environment, documented in control/README.md: DATABASE_URL,
  PRYX_NODE_LISTEN, PRYX_CA_KEY, PRYX_OPERATOR_TOKEN, PRYX_REGION_MAP, PRYX_PLACEMENT, PRYX_*_SECONDS
  and the rest of what Cubepals' FLEET_* keys configured, each with the default Cubepals' schema
  gives it; S3_ENDPOINT, S3_BUCKET, S3_ACCESS_KEY, S3_SECRET_KEY, S3_RUNTIME_ENDPOINT.
- The model replaces Cubepals' MinecraftRuntime port: a SandboxSpec is the node protocol's
  WorkloadSpec (image, entrypoint, env, secrets, resources, storage, ports with audience `edge` or
  `control`, stop, restart, labels, resumesOn) plus control's own fields, `ttl` (seconds; absent
  means forever) and `idle` (seconds without a session or an exec before control puts it to
  sleep; absent means never), which control keeps and the node never sees. A Sandbox is what
  control knows: id, project, state, node, epoch, endpoints, usage as last reported, issues, times.
  The states: creating, stopped, starting, running, sleeping, resting, restoring, failed. The
  verbs: make (ensure placed, started or not), start, stop, sleep (suspend where the node lists
  the feature, stop otherwise, and say which), wake (resume, boot or restore, and say which),
  snapshot, fork, rest, delete, exec, logs. Each verb is the operation the fleet runtime already
  performs for the matching port method (the table "How a control plane maps onto it" in
  docs/protocol.md); keep that code and its tests, renamed.
- Upkeep (every minute, as Cubepals' runtime-upkeep job): health changes, probes of silent nodes,
  uploads, tidying, deleted snapshots' copies, local-copy trimming, half-finished placements,
  abandoned first placements, events older than 90 days, and now TTL (delete past it) and idle
  (sleep past it). Activity for idle is an exec, a start, a wake, or a session the edge reports
  (the edge comes in a later pull request; leave the session input in place).
- Events: fleet_events stays the ledger; control/src/events.ts reads it per sandbox for the API
  that comes next.
- Words: "server" becomes "sandbox", "world" becomes "data", in names, comments and docs.
  "fleet", "node", "placement", "epoch", "fence", "lease" stay.

Constraints

- Every file moved from cubepals/cubepals gets this repository's SPDX header
  (SPDX-FileCopyrightText: 2026 The Cubepals Authors; SPDX-License-Identifier: FSL-1.1-ALv2) and
  no other change in the move commit; `Move: …` commits move code and nothing else, and the
  renaming and rewiring come in later commits of the same pull request, so the diff of each is
  readable.
- Nothing imports from cubepals/cubepals at build time; control/ depends only on its own files
  and on packages from npm. No pg-boss: upkeep is a plain interval. No Minecraft, no product,
  no billing.
- The node protocol does not change in this pull request; control sends nothing a node doesn't
  announce in `features`.
- Tests move with the code and keep passing; the unit suites run on a real Postgres (CI provides
  one). The end-to-end suites run in CI only; say so in the pull request.
- A `check` script at the root runs biome, the TypeScript typecheck and every workspace's unit
  tests; `e2e` runs the end-to-end suites. CI gets a `control` job for `check` and an `e2e` job
  modeled on Cubepals' fleet workflow and this repository's `contract` job (Postgres service, an
  S3 store container, pryxd built from source, sudo). The `contract` job stays as it is and green.
- The structure rules apply to TypeScript as to Rust: files under 800 lines, a header comment, no
  cycles, and CI's Structure step now runs Cubepals' check-structure.ts over control/ as well.
- Title under 50 characters, no motive, no `@`.

Done when

- `bun run check` passes locally.
- In CI: `e2e` passes, with the moved end-to-end suites taking a pryxd through enrollment over
  mutual TLS, placement, start, snapshot, upload, restore, a daemon killed and back, a node
  declared lost and a rebuild, and a self-upgrade; and the two-node suite passes.
- `pryx node ls`, `pryx node token`, `pryx node drain`, `pryx node events` work against a running
  control in the end-to-end suite.
- control/README.md documents every environment variable with its default, how to run control
  on one machine with a local Postgres, and how a host joins (the join line, join.sh, the token).
- The pull request body lists what moved from where, what was renamed, and what was left out and
  why; and says which suites ran locally and which ran only in CI.
```

## K4: the public API, keys, the CLI and the TypeScript SDK

```
Goal

Give control a public HTTP API, /v1, with API keys, an OpenAPI document and a generated
TypeScript SDK, and a `pryx sandbox` command line over it, so that a program, a CI job or an agent
can make a sandbox, run commands in it, snapshot and fork it, put it to sleep and wake it, rest it
and delete it, without knowing nodes exist.

Context

Read AGENTS.md, control/README.md, control/src/sandboxes.ts (the model from the previous pull
request), control/src/operator-api.ts (how an authenticated HTTP API is served today),
docs/protocol.md (the conventions the node protocol follows: JSON bodies, idempotent verbs,
Idempotency-Key on exec, conditional PUT, one error envelope with `code` and `details`, a version
in the path), control/tools/openapi.ts (generation and drift check), and in cubepals:
tools/openapi/openapi.ts and apps/control/src/infra/boat/client.ts for how a generated client is
wrapped.

The API, served by control on PRYX_API_LISTEN, authenticated with `Authorization: Bearer <key>`:

- Keys: `pryx key create --name <name>` makes one, printed once, stored as its sha256 with its
  name, project and creation time in a `keys` table; `pryx key ls` and `pryx key revoke`. One
  project per deployment in v1; the project is a column so that more can come later.
- POST /v1/sandboxes: from `image`, or `snapshot` (a fork), or `template`; with resources,
  storage, ports (name, port, protocol, audience: `public` or `private`), env, secrets, ttl,
  idle, isolation (`container` in v1; a node announces what it provides and placement filters on
  it), restart, labels, and `start: true|false`. Returns 201 with the sandbox. Idempotent on an
  Idempotency-Key header, as the node's exec is.
- GET /v1/sandboxes, GET /v1/sandboxes/{id}, DELETE /v1/sandboxes/{id} (data too; 200 with
  `existed`).
- POST /v1/sandboxes/{id}/start | stop | sleep | wake | rest. Each answers the state it reached and,
  for sleep and wake, which tier (`suspended` or `stopped`; `resumed`, `booted` or `restored`).
- POST /v1/sandboxes/{id}/exec: command, timeout, env; the answer streams stdout and stderr as
  newline-delimited JSON events and ends with the exit code; Idempotency-Key honoured as the node
  honours it.
- GET /v1/sandboxes/{id}/logs?tail=&since=&follow=.
- PUT and GET /v1/sandboxes/{id}/files/{path}: over exec and tar in this version.
- POST and GET /v1/sandboxes/{id}/snapshots, DELETE /v1/sandboxes/{id}/snapshots/{snapshot},
  POST /v1/snapshots/{snapshot}/sandboxes (a fork: a new sandbox restored from it).
- GET /v1/sandboxes/{id}/endpoints: for each port, where it is reachable now from the private
  network (host:port), and, for public ports, the hostname the edge will serve it on (the edge
  comes in the next pull request; the hostname is computed from PRYX_PUBLIC_DOMAIN now).
- GET /v1/sandboxes/{id}/events?since= : the ledger's events for that sandbox.
- POST /v1/templates, GET /v1/templates: an image pulled on every active node and snapshotted
  after one start, so a sandbox from a template starts from the snapshot.
- Errors: the node protocol's envelope. Codes include invalid_request (422, with fields),
  not_found (404), no_room (503, no node can take it), not_running, not_stopped (409),
  idempotency_key_reused (422), unauthorized (401), forbidden (403), gone (410, past its ttl).
- The OpenAPI document is written by control from its zod schemas to control/openapi/pryx.openapi.json
  and committed; control/tools/openapi.ts checks it for drift in CI; sdk/typescript is generated
  from it with openapi-typescript and wrapped in a small client (`@cubepals/pryx`), with its own
  README and tests; cli/ uses the SDK for `pryx sandbox create|ls|get|exec|logs|snapshot|fork|sleep|wake|rest|rm`,
  `pryx run --image <image> [--snapshot] -- <cmd…>` (create, exec, delete, exit with the command's
  code), `pryx key …`, and keeps `pryx node …`. `bun build --compile` makes the CLI one binary;
  release.yml publishes it with the daemon.

Constraints

- The public API is additive within v1 from this pull request on; its OpenAPI document is
  committed with every change to it.
- Keys are compared by their hash; a key is printed once and never logged. Nothing in the API
  reveals nodes but their ids in a sandbox's view and the operator API.
- The node protocol does not change.
- Unit tests on a real Postgres; the end-to-end suite extends control/e2e/ to take a sandbox
  through make, exec, logs, snapshot, fork, sleep, wake, rest and delete through the API and then
  through the CLI, in CI.
- Title under 50 characters, no motive, no `@`.

Done when

- `bun run check` passes; in CI, `e2e` passes with the new suite.
- `control/openapi/pryx.openapi.json` is committed, and the drift check passes.
- `pryx run --image <a small image> -- echo hello` prints hello and exits 0 against the
  end-to-end control, and `pryx sandbox …` covers every verb.
- sdk/typescript has a README with a complete example (make, exec, snapshot, fork, delete).
- control/README.md documents the API's authentication, every endpoint in a table, and ttl, idle
  and templates.
```

## K5: the edge

```
Goal

Add `edge/`, pryx's edge: the one way in to a sandbox from the public network. It routes a
connection to the sandbox by TLS server name, by HTTP Host or by port, holds the connection while
a sleeping sandbox wakes and then hands it over as if the client had just dialled, limits
connections per address and per sandbox, reports sessions and idle, and closes connections whose
sandbox moved. Control serves it the edge protocol, /edge/v1.

Context

Two repositories are checked out: `pryx` and `cubepals/cubepals`, the latter for reading and
copying from. Read pryx's AGENTS.md and control/README.md. In cubepals: docs/architecture.md §12
(edge routing, wake, idle and presence: the protocol, the control side, the edge side),
docs/fleet.md §11 (networking and the edge, including the design of the relay that closes stale
connections, under "Stale connections"), packages/contracts/src/edge.ts, edge-poll.ts and
proxy-header.ts (the contract: routes with versions, wake, ping-wake, idle, sessions, visits),
apps/control/src/app/edge/service.ts, routed.ts, inputs.ts (the control side: routes from handles,
the wake, routed-by-every-edge), and apps/edge/: agent.ts, poll.ts, router.ts, front.ts, limits.ts,
hold.ts, mappings.ts, metrics.ts, visits.ts, drain.ts, and their tests. Those files are shaped by
one game's protocol; what moves is the contract and the mechanics around it, not the game.

What edge/ is:

- A Bun service on hosts with public addresses. It polls GET /edge/v1/routes from control every
  second with x-edge-id and x-routes-held, applies each list one route at a time, says back the
  version it holds every route of, and asks again at once after applying; control serves
  /edge/v1 (routes, wake, ping-wake, idle, sessions, visits) on its API listener for edges,
  authenticated with an edge token (PRYX_EDGE_TOKEN), and reads a sandbox as running only once
  every counted edge holds the route's version (move routed.ts).
- Routes: one per public port of a sandbox: hostname (`<sandbox id or alias>.<PRYX_PUBLIC_DOMAIN>`
  for TLS and HTTP fronts; a port number for port fronts), destination host:port on the private
  network, state absent while it runs, `asleep` when a connection should wake it, `restarting`
  when it comes back by itself, `waking`, and the sandbox's epoch. A sandbox whose address isn't
  stable routes to 0.0.0.0:0 while it isn't running.
- Fronts, one listener each, with a greeter interface (read the first bytes, name the
  destination, say what may be sent to the client while it waits):
  - TLS on 443 and on any port a spec marks `tls`: the destination is the SNI of the ClientHello;
    the ClientHello is kept and replayed once the sandbox answers; nothing is sent to the client
    while it waits.
  - HTTP on 80: the Host header; the request is kept and replayed; nothing is sent while it waits.
  - Port: one sandbox per public port from PRYX_EDGE_PORT_RANGE; no greeting; bytes kept and
    replayed.
- The hold: a connection to a route that isn't running goes to the hold, which asks POST
  /edge/v1/wake once per sandbox for every connection waiting (again when the answer is
  `starting`), keeps the bytes the client sent up to a bound, and connects and replays when the
  wake answers `ready` with the destination; a `denied` wake closes the connection. The hold's
  longest wait is configuration (PRYX_EDGE_HOLD_SECONDS), chosen from the measurements below.
- Measurements, before the default is chosen: how long each of curl, Node's fetch, Python's
  requests, Go's net/http, Chrome and Safari wait for a TLS handshake and for an HTTP response with
  nothing sent, and how long OpenSSH's client waits for a banner, each measured against a listener
  that stays silent, with n, p50 and p90, written to docs/edge.md with the commands used. The
  default hold is set below the smallest of them, and docs/edge.md says which client bounds it.
- Limits, per address (an IPv6 address by its /64) and per destination: connections still
  greeting, new connections a second, open at once; hostnames no route names count together as one
  destination. The numbers are configuration with the defaults Cubepals' limits.ts has, and
  refusals are counted in /metrics.
- Stale connections: the edge remembers each spliced connection's destination; when a route names
  another destination for the same sandbox with a higher epoch, it closes the connections to the
  old one; a route with an older epoch than the edge has seen for that sandbox is ignored.
- Sessions: connect and disconnect per connection to POST /edge/v1/sessions; control counts them
  as activity for idle. Idle: a hint to POST /edge/v1/idle when a sandbox's last connection
  closes; control decides.
- The edge's own /metrics: routes held, version lag, holds, wakes by outcome, refusals by limit,
  stale closes.

Constraints

- Nothing in edge/ or control/ knows any application protocol but TLS's ClientHello, HTTP/1.1's
  request line and headers, and raw bytes. No game, no PROXY protocol in this pull request (the
  spec's `proxyProtocol` stays absent).
- The edge protocol's OpenAPI document is written to control/openapi/edge.openapi.json and
  committed, generated types checked for drift, and additive from here on.
- A sandbox is reachable from the public network only through the edge: the node's isolation
  rules already keep anything else out; edge/README.md says what the private network must allow.
- Unit tests for the greeters, the hold, limits and stale closes; the end-to-end suite in CI
  takes a sandbox running a small HTTP server through: a request to its hostname while it runs;
  `pryx sandbox sleep`, then a request that wakes it and completes; a route change with a higher
  epoch that closes the first connection. TLS in the suite uses a test CA.
- Title under 50 characters, no motive, no `@`.

Done when

- `bun run check` passes; in CI, `e2e` passes with the edge suite.
- docs/edge.md holds the measurements table and the chosen default, and edge/README.md documents
  every environment variable, the fronts, and how to run one edge beside control on one machine.
- A sleeping sandbox wakes on an HTTPS request to its hostname and the request completes, shown
  in the end-to-end suite's log in the pull request.
```

## K6: one box, the docs, the GitHub Action, the Python SDK

```
Goal

Make pryx something a person installs on one Linux host in minutes and uses from CI the same day:
`pryx up` sets up one machine with everything; the docs say how it works and how to run it; a
GitHub Action runs a workflow step inside a sandbox; and a Python SDK matches the TypeScript one.

Context

Read AGENTS.md, README.md, control/README.md, edge/README.md, docs/protocol.md, docs/edge.md,
cli/, sdk/typescript/, .github/workflows/, and in cubepals: docs/fleet-operations.md (the
operations guide the fleet has today: adding a node, day to day, a node that stops answering,
declaring a node lost, a lost node that comes back, retiring, certificates, upgrading, disk, two
hosts with one identity, what to watch) and apps/control/src/infra/fleet/join.sh.

What is added:

- `pryx up` (cli/src/up.ts): on a Linux host with systemd, as root: installs Docker from Docker's
  own repository and nftables if missing, writes pryxd's daemon.json, installs the pryxd of the same
  release, runs Postgres as a container with a volume, writes control's and the edge's
  configuration (a fleet CA made on the spot, an operator token, an edge token, a first API key
  printed once, PRYX_PUBLIC_DOMAIN from `--domain`), runs control and the edge as systemd units
  from the release's binaries, makes an enrollment token and joins the local pryxd to it, and
  prints what it made and how to undo each part. `pryx up --check` says what it would do.
  Idempotent: run again, it changes nothing that is already right.
- docs/: docs/operations.md adapted from Cubepals' fleet-operations.md with the game removed and
  `pryx node` in place of `fleet.ts`; docs/api.md generated from control/openapi/pryx.openapi.json;
  docs/architecture.md: the three parts, the four verbs, the epoch, the sleep tiers and what each
  costs, the isolation levels and which exist, and the failure table (what fails, what pryx does,
  what a caller sees) drawn from docs/fleet.md §10 and docs/protocol.md; README.md: a quickstart
  with `pryx up`, `pryx run`, and the Action.
- action/action.yml: a composite GitHub Action, `cubepals/pryx/action`, with inputs `url`, `key`,
  `image`, `run`, `snapshot` (keep a snapshot of the sandbox at the end, output its id) and
  `from-snapshot`; it downloads the CLI of the release the action's ref names, verifies its
  SHA256SUMS, and runs `pryx run`. An example workflow in the README.
- sdk/python: generated from control/openapi/pryx.openapi.json (openapi-python-client or the
  generator the repository already uses, pinned), package `cubepals-pryx`, with a README example
  matching the TypeScript one and tests against the end-to-end control.

Constraints

- `pryx up` never prints a secret it made except the first API key, once, and never stores one in
  the clear outside the units' environment files with mode 0600.
- Nothing new in the node protocol or the public API.
- CI gets a job that runs `pryx up` on a fresh ubuntu-24.04 runner from the build's artifacts and
  then the Action's example workflow against it; say in the pull request how long the job takes.
- Title under 50 characters, no motive, no `@`.

Done when

- On a fresh ubuntu-24.04 runner in CI, `pryx up --domain pryx.test` finishes, `pryx run --image
  <a small image> -- echo hello` prints hello, and the Action's example passes against it.
- docs/ has operations.md, api.md and architecture.md, and README.md's quickstart works as
  written.
- sdk/python's tests pass in CI, and its README example works.
```

## K7: machine providers

```
Goal

Let control rent machines, join them as nodes, and return them, through one provider interface,
so a fleet grows when sandboxes need room and shrinks when they don't. Providers in this pull
request: Hetzner Cloud, Fly Machines, Boat, and a host of your own (an address and an SSH key).

Context

Two repositories are checked out: `pryx` and `cubepals/cubepals`, the latter for reading and
copying from. Read pryx's AGENTS.md, control/README.md, control/src/registry/ (enrollment,
tokens, lifecycle: drain, retire, lost), control/src/placement.ts (the reasons a placement is
refused), the operator API and `pryx node`. In cubepals: scripts/fleet.ts (`add` and `remove`:
how a Hetzner host is made, given its join line through cloud-init, waited for, and taken back),
scripts/fleet-hetzner.ts, infra/terraform/modules/fleet-node/ (the host's firewall, private
network and cloud-init), apps/control/src/infra/fleet/join.sh and join-token.ts (what a joining
host fetches and runs), docs/fleet-operations.md §2 (adding a node), and
apps/control/src/infra/boat/ (client.ts, boat-runtime/sandboxes.ts, sandbox-scripts.ts: how a
Boat VM is made from Boat's API and how Docker is driven inside it). For Fly Machines, read Fly's
Machines API documentation and whether a Machine can run a Docker daemon (Fly's remote builders
do); for Boat, its API documentation; nothing is assumed about either until measured.

What to build, in control/src/providers/:

- The interface, `Provider`: `acquire(region, shape)` rents one host of a shape (vCPU, memory
  GB, disk GB) in a region, runs the join line on it (cloud-init where the provider offers it,
  SSH as root where it doesn't), and returns the host's provider id and private address;
  `release(host)` returns it to the provider; `list()` returns what exists under pryx's label
  (so upkeep can find a host the registry lost); `shapes(region)` returns what can be rented with
  its price per hour and per month as the provider states it. Every call is idempotent on a key
  control supplies, so a lost answer never makes two hosts.
- providers/hetzner.ts: Hetzner Cloud's API directly (no Terraform): server, private network,
  firewall, cloud-init with the join line, as the Terraform module does today. providers/fly.ts:
  a Fly Machine with Docker inside, sized to the shape, on Fly's private network, joined over
  SSH or its exec API. providers/boat.ts: a Boat VM, joined over Boat's command endpoint.
  providers/manual.ts: a host you name, joined over SSH, never released. providers/fake.ts for
  the tests.
- Rules, in control/src/autoscale.ts, all configuration with these defaults: grow a region by one
  host of the smallest shape that fits when placement refuses a sandbox for room twice in a row
  there, or when its allocatable memory left falls under PRYX_REGION_MEMORY_FLOOR_MB; shrink when
  a node has run no sandbox for PRYX_NODE_IDLE_SECONDS, every sleeping sandbox on it has its
  newest snapshot uploaded, and the region keeps PRYX_REGION_MIN_NODES: drain, rest what sleeps,
  retire, release. Never release a node that is lost, quarantined, holds a copy no store has, or
  was joined by hand. Each acquire and release is an event with its reason and the shape's
  price.
- Operators: `pryx node acquire <region> --provider <name> --shape <shape>`, `pryx node release
  <node>`, `pryx node shapes <region> --provider <name>`; the operator API gains the same.
- Configuration per provider from the environment (PRYX_HETZNER_TOKEN, PRYX_FLY_TOKEN and
  PRYX_FLY_ORG, PRYX_BOAT_TOKEN), documented in control/README.md with what each provider needs
  set up once by hand (a private network, a firewall, an SSH key).
- docs/providers.md: each provider, what it needs, what it costs to join a host (measured: from
  acquire to the node's first healthy heartbeat, n, p50, p90), and for Fly whether and how
  Docker runs inside a Machine, with the measurements. If Docker does not run inside a Fly
  Machine, say so and leave providers/fly.ts out rather than ship something unproven.

Constraints

- A provider decides nothing: it rents and returns. Trust is unchanged: a new host starts from
  the one-time token, never from the provider's identity.
- Nothing new in the node protocol or the public API.
- Unit tests with the fake provider cover the rules; the end-to-end suite in CI runs against
  Hetzner Cloud with a token from a repository secret, makes one of the smallest shape, waits
  for the node to join, places a sandbox, deletes it, and sees the node released; it is skipped,
  not failed, when the secret is absent, and says so.
- Secrets never printed, never in the ledger; a provider's token appears nowhere but the
  environment.
- Title under 50 characters, no motive, no `@`.

Done when

- `bun run check` passes; in CI, the Hetzner end-to-end run passes (or is skipped for want of
  the secret, visibly).
- docs/providers.md holds the measurements table for every provider shipped, and the Fly answer.
- `pryx node acquire` and `pryx node release` work against the fake provider in the suite and
  against Hetzner in the end-to-end run.
```

## K8: Rivet and celld integrations, and the store lease

```
Goal

Meet two open-source projects natively: Rivet (actors, workflows and agent VMs; rivet.dev) and
celld (self-hosted Workers and Durable Objects on S3; celld.dev). Ship a RivetKit package that
gives an actor a pryx sandbox, a pryx backend for Rivet's sandboxes if Rivet has an interface for
one, a pryx backend for celld's Containers service or a working prototype of one, and, taken
from celld's design, a lease in the object store that lets a node fence itself, behind a node
feature that is off by default. Two pull requests are fine: the integrations, then the lease.

Context

Read pryx's AGENTS.md, control/README.md, sdk/typescript/, docs/protocol.md (fences, the
execution lease, "After the host restarts"), and in cubepals/cubepals docs/fleet.md §6 and §7
(why a host is never declared lost by a timer, and what a fence that doesn't depend on the node
would need). Then Rivet: its docs (rivet.dev/docs, the Actors, agentOS and Platform sections)
and the source at github.com/rivet-dev/rivet and rivet-dev/actors: find whether Rivet exposes a
provider or driver interface for the VMs or sandboxes its agents use, and how a RivetKit actor is
written, slept and woken. Then celld: celld.dev and github.com/denoland/celld: how a cell's
ownership is taken by a conditional bucket write and renewed, how hibernation and restore work,
and what its Containers service expects of a backend. Say in the pull request what each
interface is, with links to the code, before building on it; where one doesn't exist, say so and
build the half that doesn't depend on it.

What to build:

1. integrations/rivet/: `@cubepals/pryx-rivet`, a RivetKit package: an actor whose state is the
   session's memory and whose actions are make, exec, snapshot, fork, sleep, wake and delete of
   one pryx sandbox through the SDK; the actor's sleep lets the sandbox sleep, its wake wakes it,
   its destruction deletes it; one example app (a coding agent whose memory is the actor and whose
   shell is the sandbox) with a README. If Rivet exposes a provider interface for its sandboxes or
   agentOS VMs, implement it over pryx's API in the same package, so a Rivet deployment can run
   those on pryx hosts, with a test against Rivet's self-hosted binary in CI.
2. integrations/celld/: the backend celld's Containers service calls, over pryx's API: a
   container that starts on first use and sleeps when idle is a pryx sandbox with idle set. If
   celld has no backend interface yet, write the adapter as a proposal (a design document in the
   package, and a prototype against celld's current code that runs a Worker's container on pryx
   end to end in CI).
3. The store lease, in pryxd and control, behind the node feature `store-lease`, off unless the
   host's configuration turns it on: each running sandbox holds a lease object in the object
   store that its node renews by a conditional write every PRYX_STORE_LEASE_RENEW_SECONDS for
   PRYX_STORE_LEASE_SECONDS; a node that cannot renew a lease within its term stops that sandbox
   itself and reports why; control may place the sandbox elsewhere only once the lease has lapsed
   and the node has not renewed it, under a new epoch, with the event saying so. Where the feature
   is off, nothing changes: a host is declared lost by an operator, as today. Measure what the
   renewals cost against the store (requests per sandbox per hour) and how long from a cut node
   to the sandbox stopped (n, p50, p90), and write both to docs/store-lease.md.

Constraints

- Nothing in pryxd, control or the edge depends on either integration; each package can be
  removed without a trace. The lease is a node feature announced in `features` before control
  sends anything for it; within protocol v1 a change only adds.
- No secret of Rivet's or celld's in the repository; test deployments are made in CI from their
  public releases.
- Unit tests for the package, the adapter and the lease; the end-to-end runs in CI.
- Title under 50 characters, no motive, no `@`.

Done when

- The RivetKit example app runs a command in a pryx sandbox from an actor, in CI, against
  Rivet's self-hosted binary.
- A celld Worker starts, uses and sleeps a container on pryx, in CI, or the pull request carries
  the proposal and the prototype with the reason there is no interface yet.
- A node whose connection to the store is cut stops its leased sandbox within the lease, shown in
  the suite's log, and docs/store-lease.md holds the measurements.
- The pull request says, with links, what each project's interface is or isn't.
```
