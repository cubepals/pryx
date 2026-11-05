// SPDX-License-Identifier: FSL-1.1-ALv2
/** Atomic project quota reservations. Locks cover database decisions, never a running command or node transfer. */
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Db, Queryable, Tx } from './database.ts'
import {
  checkLimit,
  exceeded,
  LIMIT_COLUMNS,
  ProjectError,
  type ProjectLimits,
  type ProjectSpec,
  UNIT_DEFAULTS,
  type UnitSize,
  unitsFor,
} from './project-model.ts'
import { lock, one, toDate } from './sql.ts'

export async function ensureProject(db: Queryable, id: string) {
  await db.execute(sql`INSERT INTO projects(id,name) VALUES(${id},${id}) ON CONFLICT DO NOTHING`)
}
export async function projectRow(q: Queryable, id: string, defaults: ProjectLimits = {}) {
  const row = await one<
    { id: string; name: string; state: 'active' | 'frozen' | 'deleted'; createdAt: string } & ProjectLimits
  >(
    q,
    sql`SELECT id,name,state,created_at AS "createdAt",${sql.join(
      Object.entries(LIMIT_COLUMNS).map(
        ([name, column]) =>
          sql`${sql.raw(column)}${sql.raw(name === 'allowedImages' || name === 'expiresAt' ? '' : '::float8')} AS ${sql.raw(`"${name}"`)}`,
      ),
      sql`, `,
    )} FROM projects WHERE id=${id}`,
  )
  if (!row || row.state === 'deleted') throw new ProjectError('not_found', 'No such project')
  const { id: project, name, state, createdAt, ...limits } = row
  for (const name of Object.keys(defaults) as (keyof ProjectLimits)[])
    if (limits[name] === null) Object.assign(limits, { [name]: defaults[name] })
  if (limits.expiresAt) limits.expiresAt = toDate(limits.expiresAt).toISOString()
  return { id: project, name, state, createdAt, limits }
}
export async function projectUsage(q: Queryable, project: string, unit: UnitSize = UNIT_DEFAULTS) {
  return (await one<{
    units: number
    sandboxes: number
    running: number
    snapshots: number
    snapshotBytes: number
    egressPorts: number
  }>(
    q,
    sql`
    SELECT (SELECT COALESCE(sum(COALESCE((spec->>'units')::int,GREATEST(1,ceil((spec->'resources'->>'memoryMb')::float8/${unit.memoryMb}),ceil(COALESCE((spec->'resources'->>'cpuMillis')::float8,0)/${unit.cpuMillis})))),0)::float8 FROM sandboxes WHERE project=${project} AND deleted_at IS NULL AND quota_running) AS units,
      (SELECT count(*)::int FROM sandboxes WHERE project=${project} AND deleted_at IS NULL) AS sandboxes,
      (SELECT count(*)::int FROM sandboxes WHERE project=${project} AND deleted_at IS NULL AND quota_running) AS running,
      (SELECT count(*)::int FROM sandbox_snapshots s JOIN sandboxes b ON b.id=s.sandbox WHERE b.project=${project} AND s.deleted_at IS NULL) AS snapshots,
      (SELECT COALESCE(sum(s.size_bytes),0)::float8 FROM sandbox_snapshots s JOIN sandboxes b ON b.id=s.sandbox WHERE b.project=${project} AND s.deleted_at IS NULL) AS "snapshotBytes",
      ((SELECT count(*)::int FROM sandboxes b CROSS JOIN LATERAL jsonb_array_elements(b.public_ports) p WHERE b.project=${project} AND b.deleted_at IS NULL AND p->>'audience'='public') + (SELECT COALESCE(sum(COALESCE((b.spec->>'sparePorts')::int,0)),0)::int FROM sandboxes b WHERE b.project=${project} AND b.deleted_at IS NULL)) AS "egressPorts"`,
  )) as {
    units: number
    sandboxes: number
    running: number
    snapshots: number
    snapshotBytes: number
    egressPorts: number
  }
}
export class ProjectAdmission {
  constructor(
    readonly db: Db,
    readonly unit: UnitSize = UNIT_DEFAULTS,
    readonly defaults: ProjectLimits = {},
  ) {}
  async active(q: Queryable, id: string) {
    const project = await projectRow(q, id, this.defaults)
    if (project.limits.expiresAt && Date.parse(project.limits.expiresAt) <= Date.now())
      exceeded('expiresAt', project.limits.expiresAt, new Date().toISOString())
    if (project.state !== 'active') throw new ProjectError('project_frozen', 'Project is frozen')
    return project
  }
  async write(project: string) {
    await this.active(this.db, project)
  }
  async serial<T>(project: string, action: (tx: Tx, limits: ProjectLimits) => Promise<T>) {
    return this.db.transaction(async (tx) => {
      await lock(tx, `project:${project}`)
      return action(tx, (await this.active(tx, project)).limits)
    })
  }
  async create(
    project: string,
    id: string,
    spec: ProjectSpec,
    start: boolean,
    ports: number,
    insert: (tx: Queryable, spec: ProjectSpec) => Promise<unknown>,
    autoSpares = false,
  ) {
    return this.serial(project, async (tx, limits) => {
      const prepared = { ...spec }
      if (limits.maxTtl === 0) exceeded('maxTtl', 0, 0)
      if (prepared.ttl === undefined && limits.maxTtl !== null && limits.maxTtl !== undefined)
        prepared.ttl = limits.maxTtl
      if (prepared.idle === undefined && limits.maxIdle !== null && limits.maxIdle !== undefined)
        prepared.idle = limits.maxIdle
      if (await one(tx, sql`SELECT 1 FROM sandboxes WHERE id=${id}`)) return prepared
      const units = unitsFor(prepared, this.unit)
      checkLimit(limits, 'maxUnitsPerSandbox', 0, units)
      checkLimit(limits, 'maxTtl', 0, prepared.ttl ?? 0)
      checkLimit(limits, 'maxIdle', 0, prepared.idle ?? 0)
      if (limits.allowedImages && !limits.allowedImages.some((prefix) => prepared.image.startsWith(prefix)))
        exceeded('allowedImages', limits.allowedImages, prepared.image)
      const usage = await projectUsage(tx, project, this.unit)
      if (autoSpares && typeof limits.maxEgressPorts === 'number')
        prepared.sparePorts = Math.min(
          prepared.sparePorts ?? 0,
          Math.max(0, limits.maxEgressPorts - usage.egressPorts - ports),
        )
      checkLimit(limits, 'maxSandboxes', usage.sandboxes, 1)
      checkLimit(limits, 'maxEgressPorts', usage.egressPorts, ports + (prepared.sparePorts ?? 0))
      if (start) {
        checkLimit(limits, 'maxUnits', usage.units, units)
        checkLimit(limits, 'maxRunning', usage.running, 1)
      }
      await insert(tx, prepared)
      return prepared
    })
  }
  async start(project: string, id: string, spec: ProjectSpec) {
    await this.serial(project, async (tx, limits) => {
      const row = await one<{ quota_running: boolean }>(
        tx,
        sql`SELECT quota_running FROM sandboxes WHERE id=${id}`,
      )
      checkLimit(limits, 'maxUnitsPerSandbox', 0, unitsFor(spec, this.unit))
      if (!row?.quota_running) {
        const usage = await projectUsage(tx, project, this.unit)
        checkLimit(limits, 'maxUnits', usage.units, unitsFor(spec, this.unit))
        checkLimit(limits, 'maxRunning', usage.running, 1)
        await tx.execute(sql`UPDATE sandboxes SET quota_running=true WHERE id=${id}`)
      }
    })
  }
  async stopped(id: string) {
    await this.db.execute(sql`UPDATE sandboxes SET quota_running=false WHERE id=${id}`)
  }
  async exec(project: string, timeout: number) {
    const { limits } = await this.active(this.db, project)
    checkLimit(limits, 'maxExecTimeout', 0, timeout)
  }
  async reserveSnapshot(project: string, sandbox: string) {
    return this.serial(project, async (tx, limits) => {
      const usage = await projectUsage(tx, project, this.unit)
      const pending = await one<{ count: number }>(
        tx,
        sql`SELECT count(*)::int AS count FROM project_snapshot_reservations WHERE project=${project}`,
      )
      checkLimit(limits, 'maxSnapshots', usage.snapshots + (pending?.count ?? 0), 1)
      const id = randomUUID()
      await tx.execute(
        sql`INSERT INTO project_snapshot_reservations(id,project,sandbox) VALUES(${id},${project},${sandbox})`,
      )
      return id
    })
  }
  async snapshot(
    project: string,
    reservation: string,
    bytes: number,
    insert: (tx: Queryable) => Promise<unknown>,
  ) {
    return this.serial(project, async (tx, limits) => {
      const usage = await projectUsage(tx, project, this.unit)
      checkLimit(limits, 'maxSnapshots', usage.snapshots, 1)
      checkLimit(limits, 'maxSnapshotBytes', usage.snapshotBytes, bytes)
      await insert(tx)
      await tx.execute(sql`DELETE FROM project_snapshot_reservations WHERE id=${reservation}`)
    })
  }
  async releaseSnapshot(id: string) {
    await this.db.execute(sql`DELETE FROM project_snapshot_reservations WHERE id=${id}`)
  }
}
