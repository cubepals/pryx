// SPDX-License-Identifier: FSL-1.1-ALv2
/** Durable lifecycle deliveries retain sanitized payloads, leases and bounded retries. Signing
 * secrets derive from an operator key and an immutable identity; listings never return them. */
import { createHmac, randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { z } from 'zod'
import type { Db } from './database.ts'
import { MINUTE_MS, SECOND_MS } from './durations.ts'
import { SandboxError } from './sandboxes.ts'
import { jsonb, one, rows } from './sql.ts'
import { deliver } from './webhook-target.ts'
export const webhookKinds = [
  'created',
  'running',
  'sleeping',
  'woken',
  'resting',
  'deleted',
  'failed',
  'ttl_expired',
  'idle_slept',
] as const
export const webhookSchema = z
  .object({
    url: z.url().refine((value) => {
      const u = new URL(value)
      return ['http:', 'https:'].includes(u.protocol) && !u.username && !u.password && !u.hash
    }),
    kinds: z.array(z.enum(webhookKinds)).min(1).max(webhookKinds.length),
  })
  .strict()
export const webhookViewSchema = webhookSchema.extend({ id: z.uuid(), createdAt: z.string().optional() })
export const webhookCreatedSchema = webhookViewSchema.extend({ secret: z.string() })
export class Webhooks {
  constructor(
    private db: Db,
    private sealKey: string,
    private operatorHosts: readonly string[] = [],
    private send = deliver,
  ) {}
  private secret(id: string) {
    return `whsec_${createHmac('sha256', this.sealKey).update(`pryx-webhook-v1:${id}`).digest('base64url')}`
  }
  async create(project: string, body: z.infer<typeof webhookSchema>) {
    if (!this.sealKey) throw new SandboxError('unsupported', 'Webhook signing is not configured')
    const id = randomUUID()
    await this.db.execute(
      sql`INSERT INTO webhooks(id,project,url,kinds,secret) VALUES(${id},${project},${body.url},${jsonb([...new Set(body.kinds)])},'derived-v1')`,
    )
    return { id, ...body, secret: this.secret(id) }
  }
  list(project: string) {
    return rows(
      this.db,
      sql`SELECT id,url,kinds,created_at AS "createdAt" FROM webhooks WHERE project=${project} ORDER BY created_at`,
    )
  }
  async delete(project: string, id: string) {
    const result = await this.db.execute(sql`DELETE FROM webhooks WHERE id=${id} AND project=${project}`)
    return { existed: !!result.rowCount }
  }
  async deadletters(project: string, id: string) {
    if (!(await one(this.db, sql`SELECT 1 FROM webhooks WHERE id=${id} AND project=${project}`)))
      throw new SandboxError('not_found', 'No such webhook')
    return rows(
      this.db,
      sql`SELECT id,payload::jsonb,attempts,dead_at AS "deadAt",last_error AS "lastError" FROM webhook_deliveries WHERE webhook=${id} AND dead_at IS NOT NULL ORDER BY dead_at DESC LIMIT 100`,
    )
  }
  async tick() {
    const due = await rows<{ id: string; webhook: string; url: string; payload: string; attempts: number }>(
      this.db,
      sql`WITH due AS(SELECT d.id FROM webhook_deliveries d WHERE delivered_at IS NULL AND dead_at IS NULL AND next_at<=now() AND (lease_until IS NULL OR lease_until<now()) ORDER BY next_at FOR UPDATE SKIP LOCKED LIMIT 8)
   UPDATE webhook_deliveries d SET lease_until=now()+interval '1 minute',attempts=attempts+1 FROM due,webhooks w WHERE d.id=due.id AND w.id=d.webhook RETURNING d.id,d.webhook,w.url,d.payload,d.attempts`,
    )
    await Promise.all(
      due.map(async (delivery) => {
        const timestamp = String(Math.floor(Date.now() / SECOND_MS))
        const signature = createHmac('sha256', this.secret(delivery.webhook))
          .update(`${timestamp}.${delivery.payload}`)
          .digest('hex')
        let error = 'network_failed'
        try {
          const status = await this.send(
            delivery.url,
            delivery.payload,
            { 'pryx-id': delivery.id, 'pryx-timestamp': timestamp, 'pryx-signature': `v1=${signature}` },
            this.operatorHosts,
          )
          if (status >= StatusCodes.OK && status < StatusCodes.MULTIPLE_CHOICES) {
            await this.db.execute(
              sql`UPDATE webhook_deliveries SET delivered_at=now(),lease_until=NULL,last_error=NULL WHERE id=${delivery.id}`,
            )
            return
          }
          error = `http_${status}`
        } catch {}
        const delay = Math.min(60 * MINUTE_MS, 2 ** (delivery.attempts - 1) * MINUTE_MS)
        await this.db.execute(
          sql`UPDATE webhook_deliveries SET lease_until=NULL,last_error=${error},next_at=now()+(${delay / SECOND_MS}*interval '1 second'),dead_at=CASE WHEN attempts>=5 THEN now() ELSE NULL END WHERE id=${delivery.id}`,
        )
      }),
    )
  }
}
