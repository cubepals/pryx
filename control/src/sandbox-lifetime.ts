// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public lifetime mutations serialize with lifecycle operations and preserve project ceilings.
 * Metrics are the last node observation and retained heartbeat samples, never estimates. */

import { sql } from 'drizzle-orm'
import { z } from 'zod'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { checkLimit } from './project-model.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'
import { event, jsonb, one, rows } from './sql.ts'
export const lifetimeSchema = z
  .object({
    keepAlive: z.number().positive().optional(),
    ttl: z.number().positive().optional(),
    idle: z.number().positive().optional(),
  })
  .strict()
  .refine((v) => Object.keys(v).length === 1)
export class SandboxLifetime {
  constructor(
    private db: Db,
    private boxes: Sandboxes,
  ) {}
  async change(id: string, project: string, body: { keepAlive?: number; ttl?: number; idle?: number }) {
    return this.boxes.exclusive(id, async () => {
      const row = await this.boxes.record(id, project)
      const update = async (
        q: import('./database.ts').Queryable,
        limits: import('./project-model.ts').ProjectLimits,
      ) => {
        const ttl = body.ttl ?? body.keepAlive
        if (ttl !== undefined) checkLimit(limits, 'maxTtl', 0, ttl)
        if (body.idle !== undefined) checkLimit(limits, 'maxIdle', 0, body.idle)
        if (
          ttl !== undefined &&
          limits.expiresAt &&
          Date.now() + ttl * SECOND_MS > Date.parse(limits.expiresAt)
        )
          throw new SandboxError('forbidden', 'Lifetime would exceed project expiry')
        const spec = {
          ...row.spec,
          ...(body.ttl === undefined ? {} : { ttl: body.ttl }),
          ...(body.idle === undefined ? {} : { idle: body.idle }),
        }
        await q.execute(
          sql`UPDATE sandboxes SET spec=${jsonb(spec)},activity_at=now(),updated_at=now(),expires_at=CASE WHEN ${ttl ?? null}::double precision IS NULL THEN expires_at WHEN ${body.keepAlive !== undefined} THEN CASE WHEN expires_at IS NULL THEN NULL ELSE GREATEST(expires_at,now()+(${ttl ?? 0}*interval '1 second')) END ELSE now()+(${ttl ?? 0}*interval '1 second') END WHERE id=${id} AND project=${project}`,
        )
        await event(q, 'sandbox.lifetime', { workload: id })
      }
      if (this.boxes.projects) await this.boxes.projects.serial(project, update)
      else await update(this.db, {})
      return this.boxes.get(id, project)
    })
  }
  async metrics(id: string, project: string, since?: string) {
    await this.boxes.record(id, project)
    if (since)
      return {
        samples: await rows(
          this.db,
          sql`SELECT at,metrics FROM sandbox_metrics WHERE sandbox=${id} AND at>=${since}::timestamptz ORDER BY at LIMIT 2000`,
        ),
      }
    const row = await one<{ at: string; metrics: unknown }>(
      this.db,
      sql`SELECT o.observed_at AS at, o.report->'metrics' AS metrics FROM fleet_observations o JOIN fleet_placements p ON p.node_id=o.node_id AND p.workload=o.workload AND p.epoch=o.epoch WHERE p.workload=${id} AND o.report->'metrics' IS NOT NULL AND o.report->'metrics' <> 'null'::jsonb ORDER BY o.observed_at DESC LIMIT 1`,
    )
    return (
      row ??
      (await one<{ at: string; metrics: unknown }>(
        this.db,
        sql`SELECT at,metrics FROM sandbox_metrics WHERE sandbox=${id} ORDER BY at DESC LIMIT 1`,
      )) ?? { at: null, metrics: null }
    )
  }
}
