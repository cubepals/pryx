// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Sandbox intent and lifecycle over the fleet engine; the node receives only its own spec. */

import { randomUUID } from 'node:crypto'
import { setTimeout as pause } from 'node:timers/promises'

import { isDeepStrictEqual } from 'node:util'
import { sql } from 'drizzle-orm'
import type pg from 'pg'
import nodeSchema from '../../openapi/node-reads.openapi.json'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { decodeHandle, decodeSnapshot } from './handle.ts'
import type { NodeClient } from './node-client.ts'
import { getNode } from './registry.ts'
import type { FleetRuntime } from './runtime-engine.ts'
import {
  type ProgressSink,
  type RuntimeHandle,
  type RuntimeSpec,
  runtimeKey,
  type SnapshotHandle,
} from './runtime-model.ts'
import { event, jsonb, one, rows } from './sql.ts'
import type { WorkloadReport, WorkloadSpec } from './wire.ts'
import { TRANSFER_MS } from './workload-api.ts'

export type SandboxSpec = WorkloadSpec & { ttl?: number; idle?: number }
export type SandboxState =
  | 'creating'
  | 'stopped'
  | 'starting'
  | 'running'
  | 'sleeping'
  | 'resting'
  | 'restoring'
  | 'failed'
export interface Sandbox {
  id: string
  project: string
  state: SandboxState
  node: string | null
  epoch: number | null
  endpoints: Array<{ name: string; host: string; port: number }>
  usage: unknown
  issues: unknown[]
  createdAt: string
  updatedAt: string
  expiresAt: string | null
}
export interface SandboxRow {
  id: string
  project: string
  spec: SandboxSpec
  region: string
  start_requested: boolean
  source_snapshot: string | null
  past_ttl?: boolean
  handle: RuntimeHandle | null
  state: SandboxState
  created_at: string
  updated_at: string
  activity_at: string
  expires_at: string | null
  deleted_at: string | null
  expired: boolean
  usage: unknown
}
export class SandboxError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}
export function runtimeSpec(spec: SandboxSpec): RuntimeSpec {
  const defaults = nodeSchema.components.schemas.WorkloadSpec.properties
  return {
    image: spec.image,
    ...(spec.entrypoint ? { entrypoint: spec.entrypoint } : {}),
    env: spec.env ?? {},
    secrets: spec.secrets ?? {},
    resources: spec.resources,
    storage: spec.storage,
    ports: (spec.ports ?? []).map((p) => ({
      name: p.name,
      port: p.containerPort,
      protocol: p.protocol ?? 'tcp',
      audience: p.audience,
    })),
    stop: {
      signal: spec.stop?.signal ?? (defaults.stop.default.signal as 'SIGTERM'),
      timeoutSeconds: spec.stop?.timeoutSeconds ?? defaults.stop.default.timeoutSeconds,
    },
    restart: spec.restart,
    labels: spec.labels ?? {},
    ...(spec.resumesOn ? { resumesOn: spec.resumesOn } : {}),
  }
}

export class Sandboxes {
  constructor(
    readonly db: Db,
    readonly engine: FleetRuntime,
    private readonly locks: pg.Pool,
    private readonly nodes: NodeClient,
  ) {}

  /** Separate lock connections prevent waiting callers from exhausting the engine's query pool. */
  async exclusive<T>(id: string, work: () => Promise<T>): Promise<T> {
    const client = await this.locks.connect()
    try {
      await client.query('SELECT pg_advisory_lock(hashtextextended($1, 0))', [`sandbox:${id}`])
      return await work()
    } finally {
      try {
        await client.query('SELECT pg_advisory_unlock(hashtextextended($1, 0))', [`sandbox:${id}`])
      } finally {
        client.release()
      }
    }
  }

  async record(id: string, project?: string): Promise<SandboxRow> {
    const row = await one<SandboxRow>(
      this.db,
      sql`SELECT *, expires_at <= now() AS past_ttl FROM sandboxes WHERE id = ${id}`,
    )
    if (!row || (project !== undefined && row.project !== project))
      throw new SandboxError('not_found', 'No such sandbox')
    if (row.past_ttl && !row.deleted_at) throw new SandboxError('gone', 'Sandbox TTL elapsed')
    if (row.deleted_at)
      throw new SandboxError(
        row.expired ? 'gone' : 'not_found',
        row.expired ? 'Sandbox TTL elapsed' : 'No such sandbox',
      )
    return row
  }
  async get(id: string, project?: string): Promise<Sandbox> {
    return this.view(await this.record(id, project))
  }
  async list(project: string): Promise<Sandbox[]> {
    return Promise.all(
      (
        await rows<SandboxRow>(
          this.db,
          sql`SELECT * FROM sandboxes WHERE project = ${project} AND deleted_at IS NULL AND (expires_at IS NULL OR expires_at > now()) ORDER BY created_at`,
        )
      ).map((row) => this.view(row)),
    )
  }
  async view(row: SandboxRow): Promise<Sandbox> {
    const ref = row.handle ? decodeHandle(row.handle) : null
    const observed = ref?.node
      ? await one<{ report: WorkloadReport }>(
          this.db,
          sql`SELECT report FROM fleet_observations WHERE workload = ${row.id} AND node_id = ${ref.node} AND epoch = ${ref.epoch}`,
        )
      : null
    return {
      id: row.id,
      project: row.project,
      state: row.state,
      node: ref?.node ?? null,
      epoch: ref?.epoch ?? null,
      endpoints: ref
        ? Object.keys(ref.ports).map((name) => ({
            name,
            ...this.engine.endpoint(row.handle as RuntimeHandle, name, 'control'),
          }))
        : [],
      usage: row.usage,
      issues: observed?.report.issues ?? [],
      createdAt: row.created_at,
      updatedAt: row.updated_at,
      expiresAt: row.expires_at,
    }
  }
  private sink(id: string, steps: string[] = []): ProgressSink {
    return {
      step: async (name) => {
        steps.push(name)
      },
      handle: async (handle) => {
        await this.db.execute(
          sql`UPDATE sandboxes SET handle = ${handle}, updated_at = now() WHERE id = ${id}`,
        )
      },
    }
  }
  private async state(id: string, state: SandboxState, activity = false): Promise<void> {
    await this.db.execute(
      sql`UPDATE sandboxes SET state = ${state}, updated_at = now(), activity_at = CASE WHEN ${activity} THEN now() ELSE activity_at END WHERE id = ${id}`,
    )
    await event(this.db, `sandbox.${state}`, { workload: id })
  }
  async make(
    project: string,
    spec: SandboxSpec,
    options: {
      id?: string
      region?: string
      start?: boolean
      snapshot?: string
    } = {},
  ): Promise<Sandbox> {
    const id = options.id ?? randomUUID()
    await this.db.execute(sql`INSERT INTO sandboxes (id, project, spec, region, expires_at, start_requested, source_snapshot)
      VALUES (${id}, ${project}, ${jsonb(spec)}, ${options.region ?? 'default'}, CASE WHEN ${spec.ttl ?? null}::double precision IS NULL THEN NULL ELSE now() + (${spec.ttl ?? null} * interval '1 second') END, ${options.start !== false}, ${options.snapshot ?? null}) ON CONFLICT (id) DO NOTHING`)
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      if (!isDeepStrictEqual(row.spec, spec))
        throw new SandboxError('idempotency_key_reused', 'Sandbox id has another spec')
      if (row.state !== 'creating' && row.state !== 'failed') return this.view(row)
      try {
        if (row.source_snapshot) {
          const snap = await this.snapshotRecord(row.source_snapshot, project)
          const handle =
            row.handle ??
            (await this.engine.adopt(runtimeKey(id), { regionKey: row.region }, runtimeSpec(spec)))
          await this.sink(id).handle(handle)
          const restored = await this.engine.restore(
            handle,
            { kind: 'snapshot', snapshot: snap.handle },
            runtimeSpec(spec),
            this.sink(id),
          )
          if (options.start !== false) await this.sink(id).handle(await this.engine.start(restored))
        } else {
          await this.engine.ensureProvisioned(
            runtimeKey(id),
            { regionKey: row.region },
            runtimeSpec(spec),
            this.sink(id),
            null,
            undefined,
            options.start !== false,
          )
        }
        await this.state(id, options.start === false ? 'stopped' : 'running', true)
        return this.get(id, project)
      } catch (error) {
        await this.state(id, 'failed')
        throw error
      }
    })
  }
  async start(id: string, project?: string) {
    return this.wake(id, project)
  }
  async wake(
    id: string,
    project?: string,
  ): Promise<{ sandbox: Sandbox; tier: 'resumed' | 'booted' | 'restored' }> {
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      const steps: string[] = []
      const restoring = row.state === 'resting'
      if (restoring) {
        const [snap] = await this.snapshots(id, row.project)
        if (!snap || !row.handle) throw new SandboxError('not_found', 'No snapshot to restore')
        await this.state(id, 'restoring')
        const handle = await this.engine.restore(
          row.handle,
          { kind: 'snapshot', snapshot: snap.handle },
          runtimeSpec(row.spec),
          this.sink(id, steps),
        )
        await this.sink(id).handle(await this.engine.start(handle))
      } else {
        await this.state(id, 'starting')
        await this.engine.ensureProvisioned(
          runtimeKey(id),
          { regionKey: row.region },
          runtimeSpec(row.spec),
          this.sink(id, steps),
        )
      }
      await this.state(id, 'running', true)
      return {
        sandbox: await this.get(id, row.project),
        tier: restoring
          ? 'restored'
          : steps.includes('resuming') && !steps.includes('booting')
            ? 'resumed'
            : 'booted',
      }
    })
  }
  async stop(id: string, project?: string): Promise<Sandbox> {
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      if (row.handle && row.state !== 'resting') await this.engine.stop(row.handle)
      await this.state(id, row.state === 'resting' ? 'resting' : 'stopped')
      return this.get(id, row.project)
    })
  }
  async sleep(id: string, project?: string): Promise<{ sandbox: Sandbox; tier: 'suspended' | 'stopped' }> {
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      const tier = row.handle && row.state !== 'resting' ? await this.engine.sleep(row.handle) : 'stopped'
      await this.state(id, row.state === 'resting' ? 'resting' : 'sleeping')
      return { sandbox: await this.get(id, row.project), tier }
    })
  }
  async exec(id: string, command: string[], timeout: number, project?: string) {
    const row = await this.exclusive(id, async () => {
      const row = await this.record(id, project)
      if (row.state !== 'running' || !row.handle)
        throw new SandboxError('not_running', 'Sandbox is not running')
      await this.touch(id)
      return row
    })
    return this.engine.exec(row.handle as RuntimeHandle, command, timeout)
  }
  async touch(id: string): Promise<void> {
    await this.db.execute(
      sql`UPDATE sandboxes SET activity_at = now() WHERE id = ${id} AND deleted_at IS NULL`,
    )
  }
  async snapshot(id: string, project?: string) {
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      return this.capture(row)
    })
  }
  private async capture(row: SandboxRow, resume = true) {
    if (!row.handle || row.state === 'resting')
      throw new SandboxError('not_stopped', 'Sandbox has no local data')
    const running = row.state === 'running'
    if (running || row.state === 'sleeping') await this.engine.stop(row.handle)
    await this.state(row.id, 'stopped')
    const snap = await this.engine.snapshot(row.handle)
    const id = decodeSnapshot(snap.snapshot).archive
    await this.db.execute(
      sql`INSERT INTO sandbox_snapshots (id, sandbox, handle, spec) VALUES (${id}, ${row.id}, ${snap.snapshot}, ${jsonb(row.spec)}) ON CONFLICT (id) DO NOTHING`,
    )
    if (running && resume) await this.sink(row.id).handle(await this.engine.start(row.handle))
    if (running && resume) await this.state(row.id, 'running')
    return { id, handle: snap.snapshot, sizeBytes: snap.sizeBytes, at: snap.at }
  }
  async snapshots(id: string, project?: string) {
    await this.record(id, project)
    return rows<{ id: string; handle: SnapshotHandle; spec: SandboxSpec }>(
      this.db,
      sql`SELECT id, handle, spec FROM sandbox_snapshots WHERE sandbox = ${id} AND deleted_at IS NULL ORDER BY created_at DESC`,
    )
  }
  async snapshotRecord(id: string, project: string) {
    const row = await one<{
      id: string
      handle: SnapshotHandle
      spec: SandboxSpec
    }>(
      this.db,
      sql`SELECT s.id, s.handle, s.spec FROM sandbox_snapshots s JOIN sandboxes b ON b.id = s.sandbox WHERE s.id = ${id} AND b.project = ${project} AND s.deleted_at IS NULL`,
    )
    if (!row) throw new SandboxError('not_found', 'No such snapshot')
    return row
  }
  async deleteSnapshot(id: string, snapshot: string, project?: string) {
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      const snap = await this.snapshotRecord(snapshot, row.project)
      const held = await one<{ sandbox: string }>(
        this.db,
        sql`SELECT sandbox FROM sandbox_snapshots WHERE id = ${snapshot}`,
      )
      if (held?.sandbox !== id) throw new SandboxError('not_found', 'No such snapshot')
      await this.engine.deleteSnapshot(snap.handle)
      await this.db.execute(sql`UPDATE sandbox_snapshots SET deleted_at = now() WHERE id = ${snapshot}`)
      return { existed: true }
    })
  }
  async rest(id: string, project?: string): Promise<Sandbox> {
    return this.exclusive(id, async () => {
      const row = await this.record(id, project)
      if (row.state === 'resting') return this.view(row)
      const snap = await this.capture(row, false)
      await this.engine.stop(row.handle as RuntimeHandle)
      const began = Date.now()
      while (true) {
        const result = await this.engine.upload(snap.id)
        const archive = await one<{ status: string }>(
          this.db,
          sql`SELECT status FROM fleet_archives WHERE id = ${snap.id}`,
        )
        if (result === 'ready' || archive?.status === 'ready') break
        if (result === 'failed' || Date.now() - began >= TRANSFER_MS)
          throw new SandboxError('not_stopped', 'Snapshot upload did not complete; local data is retained')
        await pause(SECOND_MS)
      }
      await this.sink(id).handle(await this.engine.release(row.handle as RuntimeHandle))
      await this.state(id, 'resting')
      return this.get(id, row.project)
    })
  }
  async delete(id: string, project?: string, expired = false): Promise<{ existed: boolean }> {
    return this.exclusive(id, async () => {
      const row = await one<SandboxRow>(this.db, sql`SELECT * FROM sandboxes WHERE id = ${id}`)
      if (!row || row.deleted_at || (project !== undefined && row.project !== project))
        return { existed: false }
      await this.engine.destroy(row.handle ?? runtimeKey(id))
      await this.db.execute(
        sql`UPDATE sandboxes SET deleted_at = now(), expired = ${expired}, updated_at = now(), spec = spec - 'secrets' WHERE id = ${id}`,
      )
      await this.db.execute(
        sql`UPDATE sandbox_snapshots SET deleted_at = now(), spec = spec - 'secrets' WHERE sandbox = ${id}`,
      )
      await event(this.db, expired ? 'sandbox.expired' : 'sandbox.deleted', {
        workload: id,
      })
      return { existed: true }
    })
  }
  async relocate(id: string): Promise<void> {
    await this.exclusive(id, async () => {
      const row = await this.record(id)
      if (!row.handle || row.state === 'resting') return
      const ref = decodeHandle(row.handle)
      const node = ref.node ? await getNode(this.db, ref.node) : null
      let from: SnapshotHandle | undefined
      if (node?.lifecycle === 'lost') {
        const snapshot = await one<{ handle: SnapshotHandle }>(
          this.db,
          sql`SELECT s.handle FROM sandbox_snapshots s JOIN fleet_archives a ON a.id = s.id WHERE s.sandbox = ${id} AND s.deleted_at IS NULL AND a.status = 'ready' ORDER BY s.created_at DESC LIMIT 1`,
        )
        if (!snapshot) throw new SandboxError('not_found', 'A lost node has no uploaded snapshot to rebuild')
        from = snapshot.handle
      } else await this.engine.stop(row.handle)
      const handle = await this.engine.relocate(
        row.handle,
        { regionKey: row.region },
        runtimeSpec(row.spec),
        this.sink(id),
        from,
      )
      await this.sink(id).handle(handle)
      if (row.state === 'running') await this.sink(id).handle(await this.engine.start(handle))
    })
  }
  async upkeep(): Promise<void> {
    await this.engine.upkeep()
    const pending = await rows<SandboxRow>(
      this.db,
      sql`SELECT * FROM sandboxes WHERE deleted_at IS NULL AND state IN ('creating','failed') AND (expires_at IS NULL OR expires_at > now())`,
    )
    for (const row of pending)
      await this.make(row.project, row.spec, {
        id: row.id,
        region: row.region,
        start: row.start_requested,
      }).catch(() => undefined)
    const placed = await rows<SandboxRow>(
      this.db,
      sql`SELECT * FROM sandboxes WHERE deleted_at IS NULL AND handle IS NOT NULL AND state NOT IN ('resting', 'creating')`,
    )
    for (const row of placed) {
      const ref = decodeHandle(row.handle as RuntimeHandle)
      const node = ref.node ? await getNode(this.db, ref.node) : null
      if (node?.lifecycle === 'lost' || node?.lifecycle === 'draining') {
        await this.relocate(row.id).catch(async () => {
          await event(this.db, 'sandbox.relocation_failed', {
            workload: row.id,
          })
        })
      }
      const address = this.engine.nodeAddress(row.handle as RuntimeHandle)
      if (address && row.state === 'running') {
        try {
          const usage = await this.nodes.call(
            address,
            'GET',
            `/v1/workloads/${encodeURIComponent(row.id)}/stats`,
          )
          await this.db.execute(sql`UPDATE sandboxes SET usage = ${jsonb(usage)} WHERE id = ${row.id}`)
        } catch {
          /* A missing report leaves the last sample intact. */
        }
      }
    }
    const expired = await rows<{ id: string }>(
      this.db,
      sql`SELECT id FROM sandboxes WHERE expires_at <= now() AND deleted_at IS NULL`,
    )
    for (const row of expired)
      await this.delete(row.id, undefined, true).catch((error) =>
        console.error('TTL cleanup failed', row.id, error instanceof Error ? error.name : 'error'),
      )
    const idle = await rows<{ id: string }>(
      this.db,
      sql`SELECT b.id FROM sandboxes b WHERE b.deleted_at IS NULL AND b.state = 'running' AND b.spec->>'idle' IS NOT NULL AND b.activity_at + ((b.spec->>'idle')::double precision * interval '1 second') < now() AND NOT EXISTS (SELECT 1 FROM sandbox_sessions s WHERE s.sandbox = b.id AND s.seen_at > now() - interval '2 minutes')`,
    )
    for (const row of idle)
      await this.sleep(row.id).catch((error) =>
        console.error('Idle sleep failed', row.id, error instanceof Error ? error.name : 'error'),
      )
  }
}
