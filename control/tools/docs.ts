// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Generate the public endpoint reference from the committed OpenAPI document and reject documentation drift. */
import { readFileSync, writeFileSync } from 'node:fs'

const root = new URL('../../', import.meta.url)
const api = JSON.parse(readFileSync(new URL('control/openapi/pryx.openapi.json', root), 'utf8')) as {
  paths: Record<
    string,
    Record<
      string,
      {
        requestBody?: { content: Record<string, { schema: { $ref?: string } }> }
        responses: Record<string, unknown>
      }
    >
  >
}
const lines = [
  '# Public API',
  '',
  'Authenticate with `Authorization: Bearer <key>`. Create, list and revoke keys with `pryx key` on the private operator listener. The project is chosen by the key. See control/README.md for configuration and sdk/typescript/README.md or sdk/python/README.md for examples.',
  '',
  '| Method | Endpoint | Request schema | Success |',
  '| --- | --- | --- | --- |',
]
for (const [path, methods] of Object.entries(api.paths))
  for (const [method, op] of Object.entries(methods))
    lines.push(
      `| ${method.toUpperCase()} | ${path} | ${op.requestBody?.content['application/json']?.schema.$ref?.split('/').at(-1) ?? '—'} | ${Object.keys(
        op.responses,
      )
        .filter((s) => Number(s) >= 200 && Number(s) < 300)
        .join(', ')} |`,
    )
lines.push(
  '',
  'Requests and responses are defined in [the OpenAPI document](../control/openapi/pryx.openapi.json). Exec and logs use application/x-ndjson; exec forwards live stdout/stderr chunks on nodes advertising exec-stream, buffered output on older nodes, a final exit event, and an error event for a failure after headers. File writes take JSON dataBase64; reads return a base64 tar archive. File paths are relative to persistent storage.',
  '',
  'Use a stable Idempotency-Key for retried create or exec requests. Reusing one for different input is refused. An exec with an uncertain outcome is never replayed automatically. Lifecycle operations return the reached sandbox and, for sleep/wake, its tier.',
  '',
  'TTL expires access immediately and upkeep deletes data. Idle is evaluated from activity and live sessions. Templates warm each active node once and retain a stopped snapshot. Public endpoints include their edge hostname/port; private endpoints are reachable only on the private network.',
  '',
  'Errors have the node envelope `{error:{code,message,details}}`. Invalid fields answer 422, missing keys 401, forbidden operations 403, missing resources 404, state conflicts 409, expired TTL 410 and unavailable placement 503. API v1 evolves additively.',
)
lines.push(
  '',
  '## Direct sandbox operations',
  '',
  'POST /v1/sandboxes/{id}/token returns a short-lived sandbox capability and agent URL. SDKs cache it and send running processes, terminals, streamed files and runtime expose requests through the edge directly to /_pryx/. The project key is never sent to the sandbox. See [the agent API](agent.md) for its contract, file URLs, spare ports and network policy.',
  'The /data/content, /data/list and /data/stat routes use the node files feature while compute is stopped or sleeping. Raw content streams as application/octet-stream; JSON requests stay bounded. Older nodes retain the existing chunk/archive fallback.',
  '',
  '## Projects and limits',
  '',
  'An operator creates a project with POST /operator/v1/projects on the private operator listener.',
  'The body accepts name, key:true and any project limit; the response includes the project,',
  'effective limits, usage, and one new raw key only when requested. GET on the collection lists',
  'projects; GET/PATCH/DELETE /{id} views, changes or deletes one. POST /{id}/freeze stops its',
  'sandboxes and refuses public writes; /unfreeze permits new work without restarting them.',
  'GET /{id}/usage and public GET /v1/project show units and counts against effective limits.',
  '',
  '`pryx project create --max-units 5 --max-running 3 --max-ttl 2h --expires-in 24h` creates a',
  'project; `pryx key create --project <id>` prints a key once. Project ls/get/set/freeze/unfreeze/rm',
  'use the same operator listener. Set accepts --max-* flags, --allowed-images comma-separated',
  'prefixes and --expires-in. A value of unlimited resets a numeric/image limit to inheritance.',
  '',
  'Units default to 512 MiB and 250 CPU millis (PRYX_UNIT_MEMORY_MB/PRYX_UNIT_CPU_MILLIS).',
  'Create accepts units (default one) or explicit resources rounded upward to whole units.',
  'Node listings include unitsFree and regional totals after host and running reservations.',
  'Project limits are maxUnits, maxSandboxes, maxRunning (including suspension with memory held),',
  'maxUnitsPerSandbox, maxTtl, maxIdle, maxExecTimeout, maxSnapshots, maxSnapshotBytes,',
  'maxEgressPorts, allowedImages and expiresAt. A refusal is 403 limit_exceeded with',
  'details:{name,limit,current}. Unknown command outcomes retain their compute reservation until',
  'a confirmed stop; they never let a retry exceed quota. Snapshot bytes are measured before',
  'publication. Lower aggregate limits freeze excess usage; expiry deletes owned data and revokes',
  'keys. Lists and all resource routes are project-scoped; foreign IDs answer 404.',
  '',
  'Each limit has PRYX_PROJECT_<UPPER_SNAKE_NAME> as its deployment default, for example',
  'PRYX_PROJECT_MAX_UNITS and PRYX_PROJECT_ALLOWED_IMAGES. Missing defaults are unlimited.',
  'Non-null project columns override defaults; null restores inheritance. maxTtl/maxIdle also',
  'supply omitted sandbox timers. Public image permissions always remain bounded by host policy.',
)
lines.push(
  ...[
    '',
    '## SDK handles and live commands',
    '',
    'The TypeScript and Python SDK READMEs contain executable handle examples. Run values collect',
    'stdout/stderr and terminal status once, or expose live line/chunk iterators. Make/run generate',
    'idempotency keys unless supplied and never retry automatically. A stream ending without its',
    "exit is an unknown outcome. Public exec forwards a feature-capable node's NDJSON; older nodes",
    'retain their buffered path. Errors after stream headers use an additive error event.',
    '',
    '`PUT /v1/sandboxes/{id}/file-chunks/{path}` accepts offset (a 32 KiB boundary) and dataBase64',
    '(up to 32 KiB decoded); GET with offset returns a bounded base64 tar member named data.',
    'The SDKs hide this framing, loop to EOF, and create parent directories for writes. Paths are',
    "relative to persistent storage, without dot segments or symlinks in the write target's walk.",
    'Chunks run only through container exec and tar; none are extracted on the client or host.',
    'The existing files endpoints retain their original shapes and bound. Multi-chunk writes',
    'are not atomic; serialize concurrent writers. Running images need the documented shell tools.',
  ],
)
const output = lines.join('\n') + '\n',
  path = new URL('docs/api.md', root)
if (process.argv[2] === 'generate') writeFileSync(path, output)
else if (readFileSync(path, 'utf8') !== output) throw new Error('API documentation drift')
