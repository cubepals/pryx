// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Hash-only public keys; raw credentials leave this module only at creation. */
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Hono } from 'hono'
import { StatusCodes } from 'http-status-codes'
import type { Db } from './database.ts'
import { OPERATOR_BASE } from './operator-api.ts'
import { one, rows } from './sql.ts'

export const hashKey = (key: string) => createHash('sha256').update(key).digest('hex')
export async function createKey(db: Db, project: string, name: string) {
  const key = `pryx_${randomBytes(32).toString('base64url')}`
  const id = randomUUID()
  await db.execute(
    sql`INSERT INTO keys (id, hash, project, name) VALUES (${id}, ${hashKey(key)}, ${project}, ${name})`,
  )
  return { id, name, project, key }
}
export const authenticate = (db: Db, key: string) =>
  one<{ project: string }>(
    db,
    sql`SELECT project FROM keys WHERE hash = ${hashKey(key)} AND revoked_at IS NULL`,
  )
export function withKeys(app: Hono, db: Db, project: string) {
  app.post(`${OPERATOR_BASE}/keys`, async (c) => {
    const body = await c.req.json().catch(() => null)
    if (typeof body?.name !== 'string' || !body.name.trim() || body.name.length > 128)
      return c.json(
        { error: { code: 'invalid_request', details: [{ field: 'name', problem: '1 to 128 characters' }] } },
        StatusCodes.UNPROCESSABLE_ENTITY,
      )
    return c.json(await createKey(db, project, body.name), StatusCodes.CREATED)
  })
  app.get(`${OPERATOR_BASE}/keys`, async (c) =>
    c.json({
      keys: await rows(
        db,
        sql`SELECT id, name, project, created_at AS "createdAt", revoked_at AS "revokedAt" FROM keys WHERE project = ${project} ORDER BY created_at`,
      ),
    }),
  )
  app.delete(`${OPERATOR_BASE}/keys/:id`, async (c) => {
    const revoked = await one(
      db,
      sql`UPDATE keys SET revoked_at = COALESCE(revoked_at, now()) WHERE id = ${c.req.param('id')} AND project = ${project} RETURNING id`,
    )
    return c.json({ existed: Boolean(revoked) })
  })
}

export async function bootstrapKey(db: Db, project: string, key: string) {
  await db.execute(
    sql`INSERT INTO keys(id,hash,project,name) VALUES(${randomUUID()},${hashKey(key)},${project},'initial') ON CONFLICT(hash) DO NOTHING`,
  )
}
