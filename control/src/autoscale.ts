// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Region growth and safe host return; providers rent hosts but never decide placement or data safety. */
import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { Db } from './database.ts'
import { HOUR_S } from './durations.ts'
import type { Thresholds } from './health.ts'
import {
  HOST_CPU_MILLIS,
  HOST_DISK_FLOOR_GB,
  HOST_MEMORY_DIVISOR,
  HOST_MIN_MEMORY_MB,
} from './host-defaults.ts'
import { cpuRequest } from './placement.ts'
import type { Acquisition, ProviderLedger } from './provider-ledger.ts'
import { ProviderError, type Shape } from './providers/provider.ts'
import { drain, getNode, nodeSummaries, retire, toNodeView } from './registry.ts'
import type { Sandboxes, SandboxRow, SandboxSpec } from './sandboxes.ts'
import { event, one, rows } from './sql.ts'
export interface AutoscaleConfig {
  enabled: boolean
  regions: Record<string, string>
  memoryFloorMb: number
  idleSeconds: number
  minNodes: number
  cpuMillisPerGb: number
}
export const AUTOSCALE_DEFAULTS = { memoryFloorMb: 0, idleSeconds: HOUR_S, minNodes: 1 }
interface Need {
  memoryMb: number
  cpuMillis: number
  diskGb: number
}
export function smallestShape(shapes: Shape[], need: Need) {
  return shapes
    .filter(
      (s) =>
        s.memoryGb * 1024 - Math.max(HOST_MIN_MEMORY_MB, (s.memoryGb * 1024) / HOST_MEMORY_DIVISOR) >=
          need.memoryMb &&
        s.cpus * 1000 - HOST_CPU_MILLIS >= need.cpuMillis &&
        s.diskGb - HOST_DISK_FLOOR_GB >= need.diskGb,
    )
    .sort((a, b) => a.memoryGb - b.memoryGb || a.cpus - b.cpus || a.diskGb - b.diskGb)[0]
}
export class Autoscale {
  constructor(
    readonly db: Db,
    readonly boxes: Sandboxes,
    readonly hosts: ProviderLedger,
    readonly config: AutoscaleConfig,
    readonly thresholds: Thresholds,
  ) {}
  async refused(region: string, spec: SandboxSpec) {
    const required = {
      memoryMb: spec.resources.memoryMb,
      cpuMillis: spec.resources.cpuMillis ?? cpuRequest(spec.resources.memoryMb, this.config.cpuMillisPerGb),
      diskGb: spec.storage.sizeGb,
    }
    await this.db.execute(
      sql`INSERT INTO autoscale_regions(region,refusals,required) VALUES(${region},1,${JSON.stringify(required)}::jsonb) ON CONFLICT(region) DO UPDATE SET refusals=autoscale_regions.refusals+1,required=excluded.required`,
    )
  }
  async succeeded(region: string) {
    await this.db.execute(sql`UPDATE autoscale_regions SET refusals=0 WHERE region=${region}`)
  }
  async release(nodeId: string, reason = 'operator') {
    const request = await one<Acquisition>(
      this.db,
      sql`SELECT * FROM provider_acquisitions WHERE node_id=${nodeId} AND state<>'released'`,
    )
    if (!request) throw new ProviderError('refused', 'Hand-joined and owned hosts are never released')
    return this.boxes.exclusive(`provider-region:${request.region}`, async () => {
      const a = await one<Acquisition>(
        this.db,
        sql`SELECT * FROM provider_acquisitions WHERE node_id=${nodeId} AND state<>'released'`,
      )
      if (!a || !this.hosts.provider(a.provider).managed)
        throw new ProviderError('refused', 'Hand-joined and owned hosts are never released')
      const node = await getNode(this.db, nodeId)
      if (node?.lifecycle === 'retired' && a.state === 'releasing' && a.host) {
        await this.hosts.provider(a.provider).release(a.host, `release:${a.id}`)
        await this.db.execute(sql`UPDATE provider_acquisitions SET state='released' WHERE id=${a.id}`)
        await event(this.db, 'provider.release', {
          node: nodeId,
          data: { provider: a.provider, host: a.host.id, shape: a.shape, reason: 'retry' },
        })
        return { state: 'released' }
      }
      if (!node || node.lifecycle === 'lost' || node.duplicate_seen_at)
        throw new ProviderError('refused', 'Lost or quarantined nodes are never released')
      const summary = (await nodeSummaries(this.db, this.thresholds)).find((n) => n.id === nodeId)
      if (node.lifecycle !== 'retired' && summary?.derivedHealth !== 'healthy')
        throw new ProviderError('refused', 'Node must be healthy before releasing its data')
      const remaining = (await nodeSummaries(this.db, this.thresholds)).filter(
        (n) =>
          n.id !== nodeId &&
          n.region_key === a.region &&
          n.lifecycle === 'active' &&
          n.derivedHealth === 'healthy' &&
          !n.duplicate_seen_at,
      ).length
      if (remaining < this.config.minNodes)
        throw new ProviderError('refused', 'Region minimum healthy node count would be violated')
      if (node.lifecycle === 'active') await drain(this.db, nodeId, 'autoscale')
      await this.db.execute(
        sql`INSERT INTO provider_node_state(node_id,release_requested) VALUES(${nodeId},true) ON CONFLICT(node_id) DO UPDATE SET release_requested=true`,
      )
      const live = await rows<SandboxRow>(
        this.db,
        sql`SELECT b.* FROM sandboxes b JOIN fleet_placements p ON p.workload=b.id WHERE p.node_id=${nodeId} AND p.state<>'released' AND b.deleted_at IS NULL`,
      )
      if (live.some((b) => !['sleeping', 'stopped', 'resting'].includes(b.state)))
        return { state: 'draining' }
      for (const box of live) await this.boxes.rest(box.id)
      const archives = await rows<{ id: string; status: string }>(
        this.db,
        sql`SELECT id,status FROM fleet_archives WHERE node_id=${nodeId} AND status<>'deleted'`,
      )
      for (const archive of archives)
        if (archive.status !== 'ready') await this.boxes.engine.upload(archive.id)
      if (
        await one(
          this.db,
          sql`SELECT 1 FROM fleet_archives WHERE node_id=${nodeId} AND status NOT IN ('ready','deleted')`,
        )
      )
        return { state: 'waiting_for_snapshots' }
      if (
        await one(this.db, sql`SELECT 1 FROM fleet_placements WHERE node_id=${nodeId} AND state<>'released'`)
      )
        return { state: 'draining' }
      if (
        await one(
          this.db,
          sql`SELECT 1 FROM fleet_observations o WHERE o.node_id=${nodeId} AND NOT EXISTS(SELECT 1 FROM sandboxes b WHERE b.id=o.workload)`,
        )
      )
        throw new ProviderError('refused', 'Node reports a copy unknown to the sandbox ledger')
      await retire(this.db, nodeId, 'autoscale')
      await this.db.execute(sql`UPDATE provider_acquisitions SET state='releasing' WHERE id=${a.id}`)
      if (!a.host) throw new ProviderError('refused', 'No provider host identity recorded')
      await this.hosts.provider(a.provider).release(a.host, `release:${a.id}`)
      await this.db.execute(sql`UPDATE provider_acquisitions SET state='released' WHERE id=${a.id}`)
      await event(this.db, 'provider.release', {
        node: nodeId,
        data: { provider: a.provider, host: a.host.id, shape: a.shape, reason },
      })
      return { state: 'released' }
    })
  }
  async tick() {
    await this.hosts.reconcile()
    for (const a of await rows<Acquisition>(
      this.db,
      sql`SELECT * FROM provider_acquisitions WHERE state='releasing' AND node_id IS NOT NULL`,
    ))
      await this.release(a.node_id as string, 'retry').catch(() => {})
    const summaries = await nodeSummaries(this.db, this.thresholds)
    for (const n of summaries) {
      const a = await one<Acquisition>(
        this.db,
        sql`SELECT * FROM provider_acquisitions WHERE node_id=${n.id} AND state<>'released'`,
      )
      if (!a || !this.hosts.providers.has(a.provider)) continue
      if (a.state === 'waiting' && n.derivedHealth === 'healthy' && !n.duplicate_seen_at)
        await this.db.execute(
          sql`UPDATE provider_acquisitions SET state='ready',ready_at=COALESCE(ready_at,now()),error=NULL WHERE id=${a.id}`,
        )
      const running = await one(
        this.db,
        sql`SELECT 1 FROM sandboxes b JOIN fleet_placements p ON p.workload=b.id WHERE p.node_id=${n.id} AND p.state<>'released' AND b.deleted_at IS NULL AND b.state IN ('creating','starting','running','restoring')`,
      )
      await this.db.execute(
        sql`INSERT INTO provider_node_state(node_id,idle_since) VALUES(${n.id},CASE WHEN ${Boolean(running)} THEN NULL ELSE now() END) ON CONFLICT(node_id) DO UPDATE SET idle_since=CASE WHEN ${Boolean(running)} THEN NULL ELSE COALESCE(provider_node_state.idle_since,now()) END`,
      )
      const state = await one<{ release_requested: boolean; idle: boolean }>(
        this.db,
        sql`SELECT release_requested,idle_since < now()-(${this.config.idleSeconds}*interval '1 second') AS idle FROM provider_node_state WHERE node_id=${n.id}`,
      )
      if (
        state?.release_requested ||
        (this.config.enabled && state?.idle && this.hosts.provider(a.provider).managed)
      )
        await this.release(n.id, 'idle').catch(() => {})
    }
    if (!this.config.enabled) return
    for (const [region, providerName] of Object.entries(this.config.regions))
      await this.boxes.exclusive(`autoscale:${region}`, async () => {
        if (
          await one(
            this.db,
            sql`SELECT 1 FROM provider_acquisitions WHERE region=${region} AND state IN ('new','executing','uncertain','waiting')`,
          )
        )
          return
        const demand = await one<{ refusals: number; required: Need }>(
          this.db,
          sql`SELECT * FROM autoscale_regions WHERE region=${region}`,
        )
        const free = summaries
          .filter((n) => n.region_key === region && n.lifecycle === 'active' && n.derivedHealth === 'healthy')
          .map(toNodeView)
          .reduce((sum, n) => sum + Math.max(0, n.capacity.allocatableMemoryMb - n.running.memoryMb), 0)
        const reason =
          (demand?.refusals ?? 0) >= 2
            ? 'placement_refused'
            : free < this.config.memoryFloorMb
              ? 'memory_floor'
              : null
        if (!reason) return
        const provider = this.hosts.provider(providerName)
        if (!provider.managed) return
        const shape = smallestShape(
          await provider.shapes(region),
          demand?.required ?? { memoryMb: this.config.memoryFloorMb, cpuMillis: 0, diskGb: 0 },
        )
        if (!shape) return
        await this.hosts.acquire(
          region,
          providerName,
          shape.name,
          `autoscale:${region}:${randomUUID()}`,
          reason,
        )
        await this.succeeded(region)
      })
  }
}
