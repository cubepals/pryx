# K10: projects, units and limits

This is milestone K10 of PLAN.md. Everything under "How to run this plan" in PLAN.md applies to
it: read AGENTS.md first, work autonomously, never ask, record choices in `docs/decisions.md`,
commit with short subjects and no `@`, never claim a check passed that did not run.

```
Goal

Let one pryx serve many people who never see each other, with every limit a number in
configuration: a key belongs to a project, a project sees only its own sandboxes, a host is
split into units of a configured size, and each project gets a configured number of units, of
sandboxes, of runs at once, and of time. Nothing about who the people are, how they sign up or
what they pay: that is whatever sits in front of the API.

Context

Read AGENTS.md, PLAN.md (K3 for placement and admission, K4 for keys and the `project` column
that exists with one project per deployment, K7 for the host reservations placement accounts
for), control/README.md, control/src/ (keys, placement, admission, the public API, the operator
API), control/openapi/pryx.openapi.json, cli/src/, and docs/api.md.

What control gains:

- Projects. A `projects` table: id, name, created, limits (below), state (active, frozen,
  deleted). Every key belongs to one project; every sandbox, snapshot, template and event belongs
  to the project of the key that made it; a request from another project answers 404 as if the
  thing did not exist. Existing keys move to a default project in the migration, so nothing
  changes for a deployment that never makes a second one. Deleting a project deletes its
  sandboxes and snapshots and revokes its keys; freezing a project stops and refuses everything
  but reads, reversibly.

- Units. A unit is the smallest sandbox a host is split into: PRYX_UNIT_MEMORY_MB and
  PRYX_UNIT_CPU_MILLIS (defaults 512 and 250), and a sandbox asks for a whole number of units
  (`units: 2` in the spec, default 1) or, as today, explicit resources, which placement rounds
  up to units. Placement already reserves memory and CPU per sandbox; it now counts in units, and
  a node reports how many units it has free, after the daemon's own reservation. `pryx node ls`
  shows units free per node and per region.

- Limits per project, each a column with a deployment default from the environment, every one
  optional and unlimited when absent:
    maxUnits (units running at once across the project), maxSandboxes (existing, any state),
    maxRunning (running or sleeping with memory held), maxUnitsPerSandbox, maxTtl, maxIdle,
    maxExecTimeout, maxSnapshots, maxSnapshotBytes, maxEgressPorts (public ports),
    allowedImages (prefixes; absent means the host's policy alone), expiresAt (the project and
    everything in it is deleted after it).
  A request past a limit answers 403 limit_exceeded with the limit's name, the limit, and the
  current count. Limits are enforced in control at admission, never trusted from a client.
  Expired or over-limit projects are tidied by upkeep.

- The operator API and CLI: `pryx project create --name <n> [--max-units 5 --max-running 3
  --max-ttl 2h --expires-in 24h …]`, `pryx project ls|get|set|freeze|unfreeze|rm`,
  `pryx key create --project <id>`. The same over HTTP on the operator listener, so a website or a
  script can make a project and its key in one call (POST /operator/v1/projects with `key: true`
  returns the project and a key printed once), and can read a project's usage against its limits
  (GET /operator/v1/projects/{id}/usage).

- The public API: GET /v1/project answers the caller's project, its limits and its usage, so an
  SDK can show "3 of 5 units used". Nothing else in /v1 changes shape; every list is scoped.

- Measure, in CI's one-box job, how many units a 16 GB host reports free with the defaults, and
  that 50 one-unit sandboxes from 10 projects place, run, and are refused past each project's
  maxUnits, with the refusal's body shown in the job's log.

Constraints

- Public API additive within v1; OpenAPI committed; `pryx.api.*` and the SDK handle from K9 keep
  working; a deployment with one project behaves exactly as before.
- Every limit is a number in configuration or on the project; none is hard-coded. No sign-up, no
  accounts, no billing, no web page: the operator API is the only way a project is made.
- A project never sees another's anything: list, get, exec, logs, files, snapshots, events,
  endpoints, templates. Tests prove it for every endpoint.
- Unit tests on a real Postgres for scoping and every limit; the end-to-end suite in CI covers the
  measurement above.
- Title under 50 characters, no motive, no `@`.

Done when

- `pryx project create --max-units 5 --max-running 3 --expires-in 24h` then `pryx key create
  --project <id>` gives a key that can make sandboxes up to those limits and is refused past
  them, with the refusal naming the limit.
- Two projects on one box cannot see each other, proven by tests for every endpoint.
- `pryx node ls` shows units free; docs/api.md and control/README.md document projects, units,
  every limit and its environment default.
- `bun run check` passes; CI green.
```
