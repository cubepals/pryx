// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * A data's copies as rows in `fleet_archives`: a snapshot taken on its node, kept while anything
 * can still restore from it, and deleted (in the store and on its node) once nothing does, now
 * where it can be and by upkeep where it can't. A copy on a node is trimmed only once a newer one
 * of its server is in the store.
 *
 * Sending a copy off its node (an upload, an export, a move's export) and the rows' `uploading`
 * state are `transfers.ts`'s; nothing else here talks to the store except to delete or read.
 */

import { randomUUID } from 'node:crypto'
import { type SQL, sql } from 'drizzle-orm'
import type { FleetStore } from './archive-transport.ts'
import type { Db, Queryable } from './database.ts'
import { decodeSnapshot } from './handle.ts'
import { getNode, type NodeRow } from './registry.ts'
import type { RestoreSource } from './runtime-model.ts'
import { event, one, rows } from './sql.ts'
import type { SnapshotResponse } from './wire.ts'
import type { WorkloadApi } from './workload-api.ts'

export interface ArchiveRow {
  id: string
  workload: string
  purpose: 'snapshot' | 'move'
  epoch: string
  node_id: string | null
  local_id: string | null
  local_deleted_at: string | null
  object_key: string | null
  status: 'creating' | 'local' | 'uploading' | 'ready' | 'failed' | 'deleted'
  sha256: string | null
  size_bytes: string | null
  consistency: string
  captured_at: string
  attempts: number
}

export class Archives {
  readonly #db: Db
  readonly #api: Pick<WorkloadApi, 'view' | 'snapshot' | 'dropSnapshot'>
  readonly #store: Pick<FleetStore, 'delete' | 'getUrl'>
  readonly #suspectSeconds: number
  readonly #log: (message: string, fields?: object) => void

  constructor(options: {
    db: Db
    api: Pick<WorkloadApi, 'view' | 'snapshot' | 'dropSnapshot'>
    store: Pick<FleetStore, 'delete' | 'getUrl'>
    suspectSeconds: number
    log: (message: string, fields?: object) => void
  }) {
    this.#db = options.db
    this.#api = options.api
    this.#store = options.store
    this.#suspectSeconds = options.suspectSeconds
    this.#log = options.log
  }

  async archiveOf(id: string): Promise<ArchiveRow | null> {
    return one<ArchiveRow>(this.#db, sql`SELECT * FROM fleet_archives WHERE id = ${id}`)
  }

  /**
   * A copy on the server's own node, made at once while the application holds its saving still,
   * recorded before the node is asked, so a copy the node made is never one nobody knows about.
   */
  async take(
    key: string,
    epoch: number,
    node: NodeRow,
  ): Promise<{ id: string; sizeBytes: number; at: Date }> {
    const id = randomUUID()
    const view = await this.#api.view(node, key)
    const quiesced = view.state === 'running'
    await this.#db.execute(sql`
      INSERT INTO fleet_archives (id, workload, purpose, epoch, node_id, status, consistency, captured_at)
      VALUES (${id}, ${key}, 'snapshot', ${epoch}, ${node.id}, 'creating',
              ${quiesced ? 'quiesced' : 'stopped'}, now())`)
    let made: SnapshotResponse
    try {
      made = await this.#api.snapshot(node, key, epoch, { id, quiesced })
    } catch (error) {
      await this.#db.execute(sql`
        UPDATE fleet_archives SET status = 'failed', error = ${(error as Error).message} WHERE id = ${id}`)
      throw error
    }
    const at = new Date(made.snapshot.createdAt)
    await this.#db.execute(sql`
      UPDATE fleet_archives SET status = 'local', local_id = ${made.snapshot.id},
        size_bytes = ${made.snapshot.sizeBytes}, captured_at = ${at} WHERE id = ${id}`)
    await event(this.#db, 'snapshot.local', {
      node: node.id,
      workload: key,
      epoch,
      data: {
        id,
        sizeBytes: made.snapshot.sizeBytes,
        method: made.snapshot.method,
        ms: made.snapshot.durationMs,
      },
    })
    return { id, sizeBytes: made.snapshot.sizeBytes, at }
  }

  /** Whether a copy can still be restored from: from the store, or from its node while it holds it. */
  async held(row: ArchiveRow): Promise<boolean> {
    if (row.status === 'deleted') return false
    if (row.status === 'ready') return true
    if (row.local_id === null || row.local_deleted_at !== null || row.node_id === null) return false
    const node = await getNode(this.#db, row.node_id)
    return node !== null && (node.lifecycle === 'active' || node.lifecycle === 'draining')
  }

  async deleteSnapshot(archiveId: string): Promise<void> {
    const row = await this.archiveOf(archiveId)
    if (row === null || row.status === 'deleted') return
    await this.#db.execute(
      sql`UPDATE fleet_archives SET status = 'deleted', deleted_at = now() WHERE id = ${row.id}`,
    )
    // The node's copy and the store's go now where they can; upkeep finishes what can't.
    await this.#dropCopies(row).catch((error) =>
      this.#log('snapshot copies left for upkeep', {
        id: row.id,
        error: (error as Error).message,
      }),
    )
  }

  /**
   * A snapshot's copies, in the store and on its node, each deleted now where it can be; upkeep
   * deletes the rest later. The store's keeps its key until the store has let it go. A node's stays
   * until the node can be asked: one declared lost or retired keeps it, and it goes once that node
   * is reinstated or beats again. True once its node holds no copy.
   */
  async #dropCopies(row: ArchiveRow): Promise<boolean> {
    const failed =
      row.object_key === null
        ? null
        : await this.deleteStored(row.id, row.object_key).then(
            () => null,
            (error: unknown) => error,
          )
    const local = row.local_id !== null && row.local_deleted_at === null && row.node_id !== null
    const node = !local
      ? null
      : await one<Pick<NodeRow, 'id' | 'api_address'>>(
          this.#db,
          sql`SELECT n.id, n.api_address FROM fleet_nodes n WHERE n.id = ${row.node_id} AND ${this.#askable()}`,
        )
    if (node !== null) {
      await this.#api.dropSnapshot(node, row.workload, row.local_id ?? '')
      await this.#db.execute(sql`UPDATE fleet_archives SET local_deleted_at = now() WHERE id = ${row.id}`)
    }
    if (failed !== null) throw failed
    return !local || node !== null
  }

  /** An archive's object, out of the store. Its key is forgotten only then, so upkeep retries a failure. */
  async deleteStored(id: string, objectKey: string): Promise<void> {
    await this.#store.delete(objectKey)
    await this.#db.execute(
      sql`UPDATE fleet_archives SET object_key = NULL WHERE id = ${id} AND object_key = ${objectKey} AND status = 'deleted'`,
    )
  }

  /**
   * Nodes (`n`) that can be asked to delete what the fleet no longer keeps: those in the fleet, and
   * one declared lost that beats again. Nothing of its sandboxes goes from that one, only copies of
   * snapshots the application deleted.
   */
  #askable(): SQL {
    return sql`(n.lifecycle IN ('active', 'draining') OR (n.lifecycle = 'lost' AND n.last_heartbeat_at > n.lost_at
      AND n.last_heartbeat_at > now() - make_interval(secs => ${this.#suspectSeconds})))`
  }

  /** A move's copy that no move will use: marked so, and gone from the store, now or by upkeep. */
  async discardExport(archive: { id: string; objectKey: string }): Promise<void> {
    await this.#db.execute(
      sql`UPDATE fleet_archives SET status = 'deleted', deleted_at = now() WHERE id = ${archive.id}`,
    )
    await this.deleteStored(archive.id, archive.objectKey).catch(() => undefined)
  }

  /**
   * A server's copies on a node that deleted its data went with it; the store's stay. A snapshot
   * the store never got is gone, so it is no longer offered (`held`), and each is recorded.
   */
  async forgetNodeCopies(key: string, nodeId: string): Promise<void> {
    const forgotten = await rows<{
      id: string
      status: ArchiveRow['status']
      error: string | null
    }>(
      this.#db,
      sql`UPDATE fleet_archives SET local_deleted_at = now()
          WHERE workload = ${key} AND node_id = ${nodeId} AND local_id IS NOT NULL AND local_deleted_at IS NULL
          RETURNING id, status, error`,
    )
    for (const row of forgotten)
      if (row.status !== 'ready' && row.status !== 'deleted')
        await event(this.#db, 'snapshot.lost_with_copy', {
          node: nodeId,
          workload: key,
          data: { id: row.id, status: row.status, error: row.error },
        })
  }

  /** Every copy of a destroyed server the store holds, deleted; the first failure stops it. */
  async deleteAllStored(key: string): Promise<void> {
    const archives = await rows<ArchiveRow>(
      this.#db,
      sql`SELECT * FROM fleet_archives WHERE workload = ${key} AND object_key IS NOT NULL AND status <> 'deleted'`,
    )
    for (const archive of archives) await this.#store.delete(archive.object_key ?? '')
  }

  /** Every copy of a destroyed server marked deleted, in the caller's transaction. */
  async markAllDeleted(t: Queryable, key: string): Promise<void> {
    await t.execute(sql`
        UPDATE fleet_archives SET status = 'deleted', deleted_at = now(),
          local_deleted_at = COALESCE(local_deleted_at, now())
        WHERE workload = ${key} AND status <> 'deleted'`)
  }

  /**
   * Where a restore reads from: a snapshot's copy on the node being filled when it is still there,
   * else the copy in the store; an archive by its URL.
   */
  async source(
    from: RestoreSource,
    nodeId: string,
  ): Promise<{ snapshot: string } | { url: string; sha256: string | null }> {
    if (from.kind === 'archive') return { url: from.download.url, sha256: from.sha256 ?? null }
    const snap = decodeSnapshot(from.snapshot)
    const row = await this.archiveOf(snap.archive)
    if (row === null || row.status === 'deleted') throw new Error('That snapshot is gone')
    if (row.node_id === nodeId && row.local_id !== null && row.local_deleted_at === null)
      return { snapshot: row.local_id }
    if (row.status !== 'ready' || row.object_key === null)
      throw new Error("That snapshot is on another node and isn't in the archive store yet")
    return this.fromStore(row.object_key, row.sha256)
  }

  /** A copy in the store, as a node restoring from it reads it. */
  async fromStore(objectKey: string, sha256: string | null): Promise<{ url: string; sha256: string | null }> {
    return { url: (await this.#store.getUrl(objectKey)).url, sha256 }
  }

  /**
   * Deleted archives whose copies couldn't all go when they were deleted: one in the store, which
   * keeps its key until the store lets it go, and one on a node, once its node can be asked.
   */
  async dropDeletedCopies(limit: number): Promise<void> {
    const deleted = await rows<ArchiveRow>(
      this.#db,
      sql`SELECT a.* FROM fleet_archives a LEFT JOIN fleet_nodes n ON n.id = a.node_id
          WHERE a.status = 'deleted'
            AND (a.object_key IS NOT NULL
              OR (a.local_id IS NOT NULL AND a.local_deleted_at IS NULL AND ${this.#askable()}))
          ORDER BY a.deleted_at LIMIT ${limit}`,
    )
    for (const row of deleted) await this.#dropCopies(row).catch(() => undefined)
  }

  /**
   * Node copies of snapshots the store holds too, but the newest per server: a restore of the
   * latest backup stays instant on its node, and older ones come from the store. Where the disk
   * can't share blocks, each node copy is a whole data, so this bounds what backups take there.
   */
  async trimLocalCopies(limit: number): Promise<number> {
    const older = await rows<ArchiveRow>(
      this.#db,
      sql`SELECT a.* FROM fleet_archives a JOIN fleet_nodes n ON n.id = a.node_id
          WHERE a.purpose = 'snapshot' AND a.status = 'ready' AND a.local_id IS NOT NULL
            AND a.local_deleted_at IS NULL AND n.lifecycle IN ('active', 'draining')
            AND EXISTS (SELECT 1 FROM fleet_archives b
                        WHERE b.workload = a.workload AND b.node_id = a.node_id AND b.purpose = 'snapshot'
                          AND b.status = 'ready' AND b.local_id IS NOT NULL AND b.local_deleted_at IS NULL
                          AND b.captured_at > a.captured_at)
          ORDER BY a.captured_at LIMIT ${limit}`,
    )
    let trimmed = 0
    for (const row of older) {
      try {
        if (!(await this.#dropCopies({ ...row, object_key: null }))) continue
        await event(this.#db, 'snapshot.local_trimmed', {
          node: row.node_id,
          workload: row.workload,
          data: { id: row.id },
        })
        trimmed++
      } catch (error) {
        this.#log('local copy left for the next pass', {
          id: row.id,
          error: (error as Error).message,
        })
      }
    }
    return trimmed
  }
}
