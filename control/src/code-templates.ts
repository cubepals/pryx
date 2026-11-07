// SPDX-License-Identifier: FSL-1.1-ALv2
/** Code-template versions refer only to a project's completed snapshot. Build files execute
 * through the SDK on a sandbox; control never opens a caller's local filesystem. */
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import { z } from 'zod'
import type { Db } from './database.ts'
import { SandboxError, type Sandboxes } from './sandboxes.ts'
import { jsonb, lock, one, rows } from './sql.ts'
export const versionSchema = z
  .object({
    snapshot: z.uuid(),
    start: z.string().min(1).optional(),
    plan: z.record(z.string(), z.unknown()).default({}),
  })
  .strict()
export class CodeTemplates {
  constructor(
    private db: Db,
    private boxes: Sandboxes,
  ) {}
  async register(project: string, name: string, body: z.infer<typeof versionSchema>) {
    const owner = await one<{ sandbox: string }>(
      this.db,
      sql`SELECT s.sandbox FROM sandbox_snapshots s JOIN sandboxes b ON b.id=s.sandbox WHERE s.id=${body.snapshot} AND b.project=${project}`,
    )
    if (!owner) throw new SandboxError('not_found', 'No such snapshot')
    return this.boxes.exclusive(owner.sandbox, async () => {
      const snapshot = await this.boxes.snapshotRecord(body.snapshot, project)
      return this.db.transaction(async (tx) => {
        await lock(tx, `code-template:${project}:${name}`)
        await tx.execute(
          sql`INSERT INTO templates(id,project,name,spec) VALUES(${randomUUID()},${project},${name},${jsonb({ snapshot: body.snapshot })}) ON CONFLICT(project,name) DO NOTHING`,
        )
        const template = await one<{ id: string }>(
          tx,
          sql`SELECT id FROM templates WHERE project=${project} AND name=${name}`,
        )
        if (!template) throw new SandboxError('not_found', 'No such template')
        const row = await one<{ version: number }>(
          tx,
          sql`SELECT COALESCE(max(version),0)+1 AS version FROM template_versions WHERE template=${template.id}`,
        )
        const version = row?.version ?? 1
        await tx.execute(
          sql`INSERT INTO template_versions(template,version,snapshot,start,plan) VALUES(${template.id},${version},${body.snapshot},${body.start ?? null},${jsonb(body.plan)})`,
        )
        await tx.execute(
          sql`UPDATE sandboxes SET internal=true WHERE id=(SELECT sandbox FROM sandbox_snapshots WHERE id=${snapshot.id}) AND project=${project}`,
        )
        return { id: template.id, name, version, snapshot: body.snapshot, start: body.start }
      })
    })
  }
  async versions(project: string, name: string) {
    return rows(
      this.db,
      sql`SELECT v.version,v.snapshot,v.start,v.created_at AS "createdAt" FROM template_versions v JOIN templates t ON t.id=v.template WHERE t.project=${project} AND (t.id=${name} OR t.name=${name}) ORDER BY version DESC`,
    )
  }
}
