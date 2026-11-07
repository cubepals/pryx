// SPDX-License-Identifier: FSL-1.1-ALv2
/** Operator project lifecycle and quota upkeep. Sandbox work uses its existing per-workload lock. */
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Queryable } from './database.ts'
import { createKey } from './keys.ts'
import { type ProjectAdmission, projectRow, projectUsage } from './project-admission.ts'
import { checkLimit, LIMIT_COLUMNS, type ProjectLimits } from './project-model.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'
import { jsonb, lock, rows } from './sql.ts'

export class Projects {
  constructor(
    readonly admission: ProjectAdmission,
    readonly boxes: Sandboxes,
  ) {}
  get db() {
    return this.admission.db
  }
  async get(id: string) {
    return {
      ...(await projectRow(this.db, id, this.admission.defaults)),
      usage: await projectUsage(this.db, id, this.admission.unit),
    }
  }
  async list() {
    return Promise.all(
      (
        await rows<{ id: string }>(
          this.db,
          sql`SELECT id FROM projects WHERE state<>'deleted' ORDER BY created_at,id`,
        )
      ).map((p) => this.get(p.id)),
    )
  }
  async setLimits(q: Queryable, id: string, limits: ProjectLimits) {
    const fields = Object.entries(limits).map(
      ([name, value]) =>
        sql`${sql.raw(LIMIT_COLUMNS[name as keyof ProjectLimits])}=${name === 'allowedImages' && value !== null ? jsonb(value) : value}`,
    )
    if (fields.length) await q.execute(sql`UPDATE projects SET ${sql.join(fields, sql`, `)} WHERE id=${id}`)
  }
  async create(name: string, limits: ProjectLimits = {}, key = false) {
    const id = randomUUID()
    await this.db.transaction(async (tx) => {
      await tx.execute(sql`INSERT INTO projects(id,name) VALUES(${id},${name})`)
      await this.setLimits(tx, id, { ...this.admission.defaults, ...limits })
    })
    return { ...(await this.get(id)), ...(key ? { key: await createKey(this.db, id, 'initial') } : {}) }
  }
  async set(id: string, limits: ProjectLimits, name?: string) {
    await this.db.transaction(async (tx) => {
      await lock(tx, `project:${id}`)
      await projectRow(tx, id)
      await this.setLimits(tx, id, limits)
      if (name !== undefined) await tx.execute(sql`UPDATE projects SET name=${name} WHERE id=${id}`)
    })
    return this.get(id)
  }
  async freeze(id: string) {
    return this.boxes.exclusive(`project-lifecycle:${id}`, () => this.freezeWork(id))
  }
  private async freezeWork(id: string) {
    await this.db.transaction(async (tx) => {
      await lock(tx, `project:${id}`)
      await projectRow(tx, id)
      await tx.execute(sql`UPDATE projects SET state='frozen' WHERE id=${id}`)
    })
    for (const box of await rows<{ id: string }>(
      this.db,
      sql`SELECT id FROM sandboxes WHERE project=${id} AND deleted_at IS NULL ORDER BY id`,
    ))
      await this.boxes.stop(box.id, undefined, true).catch(async (error: unknown) => {
        if (error instanceof SandboxError && error.code === 'gone')
          await this.boxes.delete(box.id, undefined, true)
        else throw error
      })
    return this.get(id)
  }
  async unfreeze(id: string) {
    return this.boxes.exclusive(`project-lifecycle:${id}`, async () => {
      await this.db.transaction(async (tx) => {
        await lock(tx, `project:${id}`)
        await projectRow(tx, id)
        await tx.execute(sql`UPDATE projects SET state='active' WHERE id=${id}`)
      })
      return this.get(id)
    })
  }
  async remove(id: string) {
    return this.boxes.exclusive(`project-lifecycle:${id}`, async () => {
      await this.freezeWork(id)
      await this.db.execute(
        sql`DELETE FROM template_versions WHERE template IN (SELECT id FROM templates WHERE project=${id})`,
      )
      await this.db.execute(sql`DELETE FROM webhooks WHERE project=${id}`)
      await this.db.execute(sql`UPDATE keys SET revoked_at=COALESCE(revoked_at,now()) WHERE project=${id}`)
      for (const box of await rows<{ id: string }>(
        this.db,
        sql`SELECT id FROM sandboxes WHERE project=${id} AND deleted_at IS NULL ORDER BY id`,
      ))
        await this.boxes.delete(box.id)
      await this.db.transaction(async (tx) => {
        await lock(tx, `project:${id}`)
        await tx.execute(
          sql`DELETE FROM template_copies WHERE template IN (SELECT id FROM templates WHERE project=${id})`,
        )
        await tx.execute(sql`DELETE FROM templates WHERE project=${id}`)
        await tx.execute(sql`DELETE FROM project_snapshot_reservations WHERE project=${id}`)
        await tx.execute(sql`UPDATE projects SET state='deleted' WHERE id=${id}`)
      })
      return { existed: true }
    })
  }
  async upkeep() {
    for (const pending of await rows<{ id: string; sandbox: string }>(
      this.db,
      sql`SELECT id,sandbox FROM project_snapshot_reservations`,
    ))
      await this.boxes.exclusive(pending.sandbox, () => this.admission.releaseSnapshot(pending.id))
    for (const project of await this.list()) {
      if (project.limits.expiresAt && Date.parse(project.limits.expiresAt) <= Date.now()) {
        await this.remove(project.id)
        continue
      }
      const usage = project.usage
      let over = false
      for (const [name, current] of [
        ['maxUnits', usage.units],
        ['maxRunning', usage.running],
        ['maxSandboxes', usage.sandboxes],
        ['maxSnapshots', usage.snapshots],
        ['maxSnapshotBytes', usage.snapshotBytes],
        ['maxEgressPorts', usage.egressPorts],
      ] as const) {
        try {
          checkLimit(project.limits, name, current)
        } catch {
          over = true
        }
      }
      if (over) await this.freeze(project.id)
    }
  }
}
