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
  'Requests and responses are defined in [the OpenAPI document](../control/openapi/pryx.openapi.json). Exec and logs use application/x-ndjson; exec emits stdout/stderr chunks and a final exit event after the node completes its bounded command. File writes take JSON dataBase64; reads return a base64 tar archive. File paths are relative to persistent storage.',
  '',
  'Use a stable Idempotency-Key for retried create or exec requests. Reusing one for different input is refused. An exec with an uncertain outcome is never replayed automatically. Lifecycle operations return the reached sandbox and, for sleep/wake, its tier.',
  '',
  'TTL expires access immediately and upkeep deletes data. Idle is evaluated from activity and live sessions. Templates warm each active node once and retain a stopped snapshot. Public endpoints include their edge hostname/port; private endpoints are reachable only on the private network.',
  '',
  'Errors have the node envelope `{error:{code,message,details}}`. Invalid fields answer 422, missing keys 401, forbidden operations 403, missing resources 404, state conflicts 409, expired TTL 410 and unavailable placement 503. API v1 evolves additively.',
)
const output = lines.join('\n') + '\n',
  path = new URL('docs/api.md', root)
if (process.argv[2] === 'generate') writeFileSync(path, output)
else if (readFileSync(path, 'utf8') !== output) throw new Error('API documentation drift')
