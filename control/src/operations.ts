// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Durable request identities: creates resume; uncertain execs never run twice automatically. */
import { createHash, randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Db } from './database.ts'
import { SandboxError } from './sandboxes.ts'
import { jsonb, one } from './sql.ts'

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical)
  if (value && typeof value === 'object')
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([k, v]) => [k, canonical(v)]),
    )
  return value
}
export function fingerprint(value: unknown) {
  return createHash('sha256')
    .update(JSON.stringify(canonical(value)))
    .digest('hex')
}
interface Operation {
  id: string
  fingerprint: string
  response: unknown
  state: string
}
export async function operation(
  db: Db,
  project: string,
  scope: string,
  key: string | undefined,
  body: unknown,
) {
  if (!key) return { id: randomUUID(), state: 'new', response: null } as Operation
  if (key.length > 256 || !key.trim())
    throw new SandboxError('invalid_request', 'Idempotency-Key is 1 to 256 characters')
  const digest = fingerprint(body)
  const row = await one<Operation>(
    db,
    sql`INSERT INTO api_operations(project, scope, key, id, fingerprint)
      VALUES (${project}, ${scope}, ${key}, ${randomUUID()}, ${digest})
      ON CONFLICT (project, scope, key) DO UPDATE SET key = api_operations.key
      RETURNING id, fingerprint, response, state`,
  )
  if (!row || row.fingerprint !== digest)
    throw new SandboxError('idempotency_key_reused', 'Idempotency-Key has another request')
  return row
}
export async function claimExec(db: Db, id: string, sandbox?: string) {
  const row = await one(
    db,
    sandbox === undefined
      ? sql`UPDATE api_operations SET state = 'executing' WHERE id = ${id} AND state = 'new' RETURNING id`
      : sql`WITH claimed AS (
          UPDATE api_operations SET state = 'executing' WHERE id = ${id} AND state = 'new' RETURNING id
        ), touched AS (
          UPDATE sandboxes SET activity_at = now() WHERE id = ${sandbox} AND deleted_at IS NULL
          AND EXISTS (SELECT 1 FROM claimed) RETURNING id
        ) SELECT claimed.id FROM claimed JOIN touched ON true`,
  )
  if (!row)
    throw new SandboxError(
      'operation_unknown',
      'Command is in progress or its outcome is unknown; inspect before using a new key',
    )
}
export async function complete(db: Db, id: string, response: unknown) {
  await db.execute(
    sql`UPDATE api_operations SET state = 'done', response = ${jsonb(response)} WHERE id = ${id}`,
  )
}
