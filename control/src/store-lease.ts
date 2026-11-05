// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Optional per-placement store authority. Expiry alone is insufficient: a winning conditional fence precedes displacement. */
import { setTimeout as pause } from 'node:timers/promises'
import { sql } from 'drizzle-orm'
import contract from '../../openapi/node-reads.openapi.json'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import type { components as NodeWrites } from './generated/node-writes.ts'
import type { NodeClient } from './node-client.ts'
import { getNode } from './registry.ts'
import type { Sandboxes } from './sandboxes.ts'
import { event, one, rows } from './sql.ts'
import type { LeaseBucket, LeaseObject } from './store-lease-bucket.ts'
import { address } from './workload-api.ts'
export const STORE_LEASE_DEFAULTS = { seconds: 30, renewSeconds: 10 }
export class StoreLeaseAuthority {
  constructor(
    readonly db: Db,
    readonly boxes: Sandboxes,
    readonly nodes: Pick<NodeClient, 'call'>,
    readonly bucket: LeaseBucket,
    readonly deployment: string,
    readonly seconds: number,
    readonly renewSeconds: number,
  ) {
    if (
      seconds < contract.components.schemas.StoreLeaseGrant.properties.seconds.minimum ||
      renewSeconds <= 0 ||
      renewSeconds * 2 >= seconds - contract.components.schemas.StoreLeaseGrant['x-renew-margin-seconds']
    )
      throw new Error('Invalid store lease term/renewal')
  }
  key(workload: string) {
    return `${this.deployment}/store-leases/v1/${encodeURIComponent(workload)}.json`
  }
  async beforeStart(nodeId: string, workload: string, epoch: number) {
    const node = await getNode(this.db, nodeId)
    if (!node?.features.includes('store-lease')) return
    const identity = await this.nodes.call<NodeWrites['schemas']['StoreLeaseIdentity']>(
      address(node),
      'GET',
      `/v1/workloads/${workload}/store-lease`,
      { epoch, timeoutMs: 10 * SECOND_MS },
    )
    if (!identity || typeof identity.session !== 'string' || !identity.session)
      throw new Error('Node has no lease session identity')
    const key = this.key(workload),
      first = await this.bucket.read(key)
    let read = first
    // Planned handoffs also honor the old published term. Fencing/deleting its copy stops renewal.
    const waitUntil = performance.now() + this.seconds * SECOND_MS
    while (
      read.object &&
      read.object.epoch < epoch &&
      read.object.expiresAtMs > read.now &&
      performance.now() < waitUntil
    ) {
      await pause(SECOND_MS / 5)
      read = await this.bucket.read(key)
    }
    let object = read.object
    if (
      object &&
      (object.version !== 1 ||
        object.workload !== workload ||
        !Number.isSafeInteger(object.expiresAtMs) ||
        object.expiresAtMs < 0 ||
        !Number.isSafeInteger(object.epoch))
    )
      throw new Error('Invalid lease object')
    if (object?.node === nodeId && object.epoch === epoch && object.session === identity.session) {
      if (object.expiresAtMs <= read.now) throw new Error('Store lease expired: recover under a new epoch')
    } else {
      if (object && (object.expiresAtMs > read.now || object.epoch >= epoch))
        throw new Error('Another placement still holds store authority')
      object = {
        session: identity.session,
        version: 1,
        workload,
        node: nodeId,
        epoch,
        generation: crypto.randomUUID(),
        expiresAtMs: read.now + this.seconds * SECOND_MS,
      }
      if (!(await this.bucket.compare(key, read.etag, object)))
        throw new Error('Store authority changed; retry placement')
    }
    await this.db.execute(
      sql`INSERT INTO store_lease_placements(workload,node_id,epoch,generation,session) VALUES(${workload},${nodeId},${epoch},${object.generation},${identity.session}) ON CONFLICT(workload,epoch) DO NOTHING`,
    )
    await this.nodes.call(address(node), 'POST', `/v1/workloads/${workload}/store-lease`, {
      epoch,
      body: {
        ...(await this.bucket.links(key)),
        session: identity.session,
        generation: object.generation,
        seconds: this.seconds,
        renewSeconds: this.renewSeconds,
      },
      timeoutMs: 10 * SECOND_MS,
    })
  }
  async tick() {
    const placements = await rows<{
      workload: string
      node_id: string
      epoch: string
      generation: string
      session: string
    }>(
      this.db,
      sql`
      SELECT l.* FROM store_lease_placements l JOIN fleet_placements p ON p.workload=l.workload AND p.epoch=l.epoch AND p.node_id=l.node_id
      JOIN sandboxes b ON b.id=l.workload WHERE p.state IN ('placed','placing','displaced') AND b.deleted_at IS NULL`,
    )
    for (const p of placements) {
      let fenced = false
      await this.boxes
        .exclusive(p.workload, async () => {
          const current = await one(
            this.db,
            sql`SELECT 1 FROM fleet_placements WHERE workload=${p.workload} AND node_id=${p.node_id} AND epoch=${p.epoch} AND state IN ('placed','placing','displaced')`,
          )
          if (!current) return
          const read = await this.bucket.read(this.key(p.workload)),
            object = read.object
          if (
            !object ||
            object.version !== 1 ||
            object.workload !== p.workload ||
            object.node !== p.node_id ||
            object.epoch !== Number(p.epoch) ||
            object.session !== p.session
          )
            return
          if (!Number.isSafeInteger(object.expiresAtMs) || object.expiresAtMs < 0) return
          if (object.expiresAtMs > read.now) {
            await this.beforeStart(p.node_id, p.workload, Number(p.epoch)).catch(() => {})
            return
          }
          const generation = `fenced:${p.generation}`
          if (object.generation !== generation) {
            if (object.generation !== p.generation) return
            const marker: LeaseObject = { ...object, generation, expiresAtMs: 0 }
            if (!(await this.bucket.compare(this.key(p.workload), read.etag, marker))) return
          }
          // Retrying after a crash recognizes the exact marker. The proof and displacement commit together.
          await this.db.transaction(async (t) => {
            const changed = await one(
              t,
              sql`UPDATE fleet_placements SET state='displaced',updated_at=now() WHERE workload=${p.workload} AND node_id=${p.node_id} AND epoch=${p.epoch} AND state IN ('placed','placing') RETURNING workload`,
            )
            await t.execute(
              sql`INSERT INTO store_lease_fences(workload,node_id,epoch,generation) VALUES(${p.workload},${p.node_id},${p.epoch},${generation}) ON CONFLICT DO NOTHING`,
            )
            if (changed)
              await event(t, 'placement.store_lease_expired', {
                node: p.node_id,
                workload: p.workload,
                epoch: Number(p.epoch),
                data: {
                  object: this.key(p.workload),
                  reason: 'Expired lease conditionally fenced in object store',
                },
              })
          })
          fenced = true
        })
        .catch(() => {})
      if (fenced) await this.boxes.relocate(p.workload).catch(() => {}) // A missing stored snapshot or target leaves it displaced and retained.
    }
  }
}
