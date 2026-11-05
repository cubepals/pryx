// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * A copy of a data sent off its node: a snapshot's upload to the archive store, tried a few times
 * and taken over once stale; an export of one to a target the application names; and a move's
 * export from its stopped source. Each goes in one PUT, or in parts when it is larger than one PUT
 * carries, and a copy too big for one upload is marked so (`Too big to upload: `), which declines
 * its server's moves until a newer copy uploads.
 *
 * When a copy may be deleted, and deleting it, are `archives.ts`'s; what a move does with its
 * export is `rehoming.ts`'s.
 */

import { randomUUID } from 'node:crypto'
import { sql } from 'drizzle-orm'
import type { FleetStore } from './archive-transport.ts'
import type { ArchiveRow, Archives } from './archives.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { decodeSnapshot } from './handle.ts'
import { type Ledger, placementOf, StaleHandle } from './ledger.ts'
import { NodeRefused } from './node-client.ts'
import { getNode, type NodeRow } from './registry.ts'
import {
  type ArchiveTarget,
  type PartsUpload,
  RuntimeUnsupported,
  type SnapshotHandle,
} from './runtime-model.ts'
import { event, one, rows } from './sql.ts'
import type { ExportResponse } from './wire.ts'
import { TRANSFER_MS, type WorkloadApi } from './workload-api.ts'

/** Uploads of one copy before it is left on its node for good. */
const UPLOAD_ATTEMPTS = 5
/** An upload not heard from in this long is taken for dead, and tried again. */
const UPLOAD_STALE_MINUTES = 3 * 60
/**
 * A node packs a snapshot for one upload at a time and refuses another meanwhile (`snapshot_busy`):
 * one that must go too asks again this often until the first ends.
 */
const BUSY_PAUSE_MS = 2 * SECOND_MS

const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`

/**
 * A node's refusal of an archive bigger than one upload to the store can carry, which it says
 * before sending anything (`archive_too_large`). Final: the same copy is refused the same way.
 */
function tooLarge(error: unknown): { sizeBytes: number; limitBytes: number; reason: string } | null {
  if (!(error instanceof NodeRefused) || error.code !== 'archive_too_large') return null
  const details = (error.details ?? {}) as {
    sizeBytes?: unknown
    limitBytes?: unknown
  }
  const sizeBytes = Number(details.sizeBytes)
  const limitBytes = Number(details.limitBytes)
  const known = Number.isFinite(sizeBytes) && Number.isFinite(limitBytes)
  return {
    sizeBytes,
    limitBytes,
    reason: known
      ? `its archive is ${gb(sizeBytes)}, and one upload to the archive store carries at most ${gb(limitBytes)}`
      : 'its archive is bigger than one upload to the archive store carries',
  }
}

/** The node is packing that snapshot for another upload, which it does one at a time. */
const busy = (error: unknown) => error instanceof NodeRefused && error.code === 'snapshot_busy'

/** How an archive refused as too big for one upload is marked: the next move of its data reads it. */
const TOO_BIG = 'Too big to upload: '

/** A move's copy refused as too big for one upload to the store: no node can take it this way. */
export class TooBigToMove extends Error {
  readonly why: string
  constructor(why: string) {
    super(`This server's data is too big to move to another node: ${why}.`)
    this.name = 'TooBigToMove'
    this.why = why
  }
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

export class Transfers {
  readonly #db: Db
  readonly #api: Pick<WorkloadApi, 'send'>
  readonly #store: Pick<FleetStore, 'objectKey' | 'target' | 'head' | 'copyTo'>
  readonly #ledger: Pick<Ledger, 'nodeOf'>
  readonly #archives: Pick<Archives, 'archiveOf' | 'deleteStored'>
  readonly #provider: string
  readonly #log: (message: string, fields?: object) => void

  constructor(options: {
    db: Db
    api: Pick<WorkloadApi, 'send'>
    store: Pick<FleetStore, 'objectKey' | 'target' | 'head' | 'copyTo'>
    ledger: Pick<Ledger, 'nodeOf'>
    archives: Pick<Archives, 'archiveOf' | 'deleteStored'>
    provider: string
    log: (message: string, fields?: object) => void
  }) {
    this.#db = options.db
    this.#api = options.api
    this.#store = options.store
    this.#ledger = options.ledger
    this.#archives = options.archives
    this.#provider = options.provider
    this.#log = options.log
  }

  /**
   * Uploads one node copy to the archive store. One upload at a time per copy: the row is claimed
   * by compare-and-set, and a claim nobody finished is taken over once it is stale.
   */
  upload(archiveId: string) {
    return this.#upload(archiveId)
  }

  async #upload(archiveId: string): Promise<'ready' | 'skipped' | 'failed'> {
    const claimed = await one<ArchiveRow>(
      this.#db,
      sql`UPDATE fleet_archives SET status = 'uploading', attempts = attempts + 1, upload_started_at = now()
          WHERE id = ${archiveId} AND local_id IS NOT NULL AND local_deleted_at IS NULL
            AND (status = 'local'
              OR (status = 'uploading' AND upload_started_at < now() - make_interval(mins => ${UPLOAD_STALE_MINUTES})))
          RETURNING *`,
    )
    if (claimed === null) return 'skipped'
    const objectKey = claimed.object_key ?? this.#store.objectKey(claimed.workload, claimed.id)
    try {
      const node = await this.#ledger.nodeOf(claimed.node_id)
      const target = await this.#store.target(objectKey)
      await this.#db.execute(sql`UPDATE fleet_archives SET object_key = ${objectKey} WHERE id = ${archiveId}`)
      const sent = await this.#sendArchive(
        node,
        `/v1/workloads/${claimed.workload}/snapshots/${claimed.local_id}/upload`,
        {},
        target,
        claimed.size_bytes === null ? null : Number(claimed.size_bytes),
      )
      const held = await this.#store.head(objectKey)
      if (held === null || held.sizeBytes !== sent.sizeBytes)
        throw new Error(
          `The store holds ${held?.sizeBytes ?? 'nothing'}; the node sent ${sent.sizeBytes} bytes`,
        )
      // One transaction, so a copy seen as ready has its event already.
      await this.#db.transaction(async (t) => {
        await t.execute(sql`
          UPDATE fleet_archives SET status = 'ready', sha256 = ${sent.sha256}, size_bytes = ${sent.sizeBytes},
            uploaded_at = now(), error = NULL WHERE id = ${archiveId}`)
        await event(t, 'snapshot.uploaded', {
          node: claimed.node_id,
          workload: claimed.workload,
          epoch: Number(claimed.epoch),
          data: {
            id: archiveId,
            sizeBytes: sent.sizeBytes,
            ms: sent.durationMs,
          },
        })
      })
      return 'ready'
    } catch (error) {
      // The node is sending it elsewhere (an export for the application): not a failed try, and
      // the next round of uploads sends it.
      if (busy(error)) {
        await this.#db.execute(
          sql`UPDATE fleet_archives SET status = 'local', attempts = attempts - 1 WHERE id = ${archiveId}`,
        )
        return 'skipped'
      }
      const large = tooLarge(error)
      const message = large === null ? (error as Error).message : `${TOO_BIG}${large.reason}`
      // A copy too big for one upload stays too big: it is left on its node, and not sent again.
      const final = large !== null || claimed.attempts >= UPLOAD_ATTEMPTS
      await this.#db.transaction(async (t) => {
        await t.execute(sql`
          UPDATE fleet_archives SET status = ${final ? 'failed' : 'local'}, error = ${message} WHERE id = ${archiveId}`)
        if (final)
          await event(t, 'snapshot.upload_failed', {
            workload: claimed.workload,
            data: {
              id: archiveId,
              error: message,
              ...(large === null
                ? {}
                : {
                    code: 'archive_too_large',
                    sizeBytes: large.sizeBytes,
                    limitBytes: large.limitBytes,
                  }),
            },
          })
      })
      return 'failed'
    }
  }

  /**
   * A tar.gz of the snapshot where `target` says, in one PUT or in parts: sent by its node while
   * the node holds it, else copied from the store.
   */
  async exportSnapshot(
    snapshot: SnapshotHandle,
    target: ArchiveTarget,
  ): Promise<{ sizeBytes: number; sha256: string }> {
    const ref = decodeSnapshot(snapshot)
    const row = await this.#archives.archiveOf(ref.archive)
    if (row === null || row.status === 'deleted') throw new Error('That snapshot is gone')
    const local =
      row.local_id !== null && row.local_deleted_at === null && row.node_id !== null
        ? await getNode(this.#db, row.node_id)
        : null
    let stored = row
    if (local !== null && (local.lifecycle === 'active' || local.lifecycle === 'draining')) {
      // The snapshot's own upload off its node, begun when it was taken, may be under way: the node
      // packs it for one upload at a time, so this one waits its turn.
      const until = Date.now() + TRANSFER_MS
      for (;;) {
        if (Date.now() < until && (await this.#uploadUnderWay(row.id))) {
          await sleep(BUSY_PAUSE_MS)
          continue
        }
        try {
          const sent = await this.#sendArchive(
            local,
            `/v1/workloads/${row.workload}/snapshots/${row.local_id}/upload`,
            {},
            target,
            row.size_bytes === null ? null : Number(row.size_bytes),
          )
          return { sizeBytes: sent.sizeBytes, sha256: sent.sha256 }
        } catch (error) {
          if (busy(error) && Date.now() < until) {
            await sleep(BUSY_PAUSE_MS)
            continue
          }
          // Refused before anything was sent, and every try would be: no runtime here can pack it.
          const large = tooLarge(error)
          if (large !== null)
            throw new RuntimeUnsupported(this.#provider, `packing this data: ${large.reason}`)
          // Its upload may have finished while this waited.
          stored = (await this.#archives.archiveOf(row.id)) ?? row
          if (stored.status !== 'ready' || stored.object_key === null) throw error
          this.#log('export falls back to the stored copy', {
            id: row.id,
            error: (error as Error).message,
          })
          break
        }
      }
    }
    if (stored.status !== 'ready' || stored.object_key === null)
      throw new Error("That snapshot's node can't be reached, and it was never uploaded")
    const copied = await this.#store.copyTo(stored.object_key, target)
    if (stored.sha256 !== null && copied.sha256 !== stored.sha256)
      throw new Error(`The stored copy's bytes changed: ${copied.sha256}, not ${stored.sha256}`)
    return copied
  }

  /** The snapshot's upload off its node has begun and isn't yet given up as stalled. */
  async #uploadUnderWay(archiveId: string): Promise<boolean> {
    const under = await one<{ id: string }>(
      this.#db,
      sql`SELECT id FROM fleet_archives WHERE id = ${archiveId} AND status = 'uploading'
            AND upload_started_at >= now() - make_interval(mins => ${UPLOAD_STALE_MINUTES})`,
    )
    return under !== null
  }

  /** Snapshots still only on their nodes, oldest first: a few each pass, so one slow node holds up nothing. */
  async uploadPending(limit: number): Promise<number> {
    const pending = await rows<{ id: string }>(
      this.#db,
      sql`SELECT a.id FROM fleet_archives a JOIN fleet_nodes n ON n.id = a.node_id
          WHERE a.local_id IS NOT NULL AND a.local_deleted_at IS NULL
            AND n.lifecycle IN ('active', 'draining')
            AND (a.status = 'local'
              OR (a.status = 'uploading' AND a.upload_started_at < now() - make_interval(mins => ${UPLOAD_STALE_MINUTES})))
          ORDER BY a.captured_at LIMIT ${limit}`,
    )
    let ready = 0
    for (const { id } of pending) if ((await this.upload(id)) === 'ready') ready++
    return ready
  }

  /**
   * Moves' copies no move will restore from, marked deleted so that `dropDeletedCopies` removes them
   * from the store: an export a crash left `uploading`, one that reached the store while its server
   * never left the copy it was made from (which still holds the data), and one that failed before
   * failed exports were deleted. Only once stale, so a move under way keeps its copy; never one a
   * placement still restores from.
   */
  async dropUnusedMoveExports(limit: number): Promise<number> {
    const dropped = await rows<{
      id: string
      workload: string
      status: string
    }>(
      this.#db,
      sql`WITH unused AS (
            SELECT a.id, a.status FROM fleet_archives a
            WHERE a.purpose = 'move' AND a.object_key IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM fleet_placements p WHERE p.restore_from = a.id)
              AND ((a.status = 'failed' AND NOT starts_with(COALESCE(a.error, ''), ${TOO_BIG}))
                OR ((a.status = 'uploading' OR (a.status = 'ready' AND EXISTS (
                      SELECT 1 FROM fleet_placements p WHERE p.workload = a.workload AND p.epoch = a.epoch)))
                  AND a.upload_started_at < now() - make_interval(mins => ${UPLOAD_STALE_MINUTES})))
            ORDER BY a.upload_started_at LIMIT ${limit} FOR UPDATE SKIP LOCKED)
          UPDATE fleet_archives a SET status = 'deleted', deleted_at = now()
          FROM unused WHERE a.id = unused.id RETURNING a.id, a.workload, unused.status`,
    )
    for (const row of dropped)
      await event(this.#db, 'move.export_dropped', {
        workload: row.workload,
        data: { id: row.id, was: row.status },
      })
    return dropped.length
  }

  /**
   * Snapshots of a server held only on `node`, uploaded before the node deletes its copy with its
   * data, which takes them with it. True once none is left that may still upload: those that never
   * can (failed for good, or never made) go with the copy, and `forgetNodeCopies` says so.
   */
  async uploadBeforeDelete(key: string, nodeId: string): Promise<boolean> {
    const only = sql`workload = ${key} AND node_id = ${nodeId} AND purpose = 'snapshot'
      AND local_id IS NOT NULL AND local_deleted_at IS NULL`
    const pending = await rows<{ id: string }>(
      this.#db,
      sql`SELECT id FROM fleet_archives WHERE ${only} AND status IN ('local', 'uploading') ORDER BY captured_at`,
    )
    for (const { id } of pending) await this.upload(id)
    const left = await rows<{ id: string }>(
      this.#db,
      sql`SELECT id FROM fleet_archives WHERE ${only} AND status IN ('local', 'uploading')`,
    )
    return left.length === 0
  }

  /**
   * Why a server's data can't move between nodes, when the newest of its copies to finish was
   * refused as too big for one upload to the store; null otherwise. A newer copy that uploads (a
   * data that shrank) lets it move again.
   */
  async tooBigToMove(key: string): Promise<string | null> {
    const newest = await one<{ status: string; error: string | null }>(
      this.#db,
      sql`SELECT status, error FROM fleet_archives WHERE workload = ${key} AND status IN ('ready', 'failed')
          ORDER BY created_at DESC LIMIT 1`,
    )
    return newest?.status === 'failed' && newest.error?.startsWith(TOO_BIG) === true
      ? newest.error.slice(TOO_BIG.length)
      : null
  }

  /**
   * A node's upload or export of an archive to `target`: in one PUT, or in parts when it is larger
   * than one PUT carries. Parts are offered at once when the archive is expected to need them
   * (`expectBytes`, its data's size); otherwise the node's refusal says how large it is, and it is
   * asked again with parts for that. Parts it didn't use are dropped, as are those of a call that
   * failed. A node without `multipart-upload` is asked for one PUT only, and its refusal stands.
   */
  async #sendArchive(
    node: NodeRow,
    path: string,
    body: Record<string, unknown>,
    target: ArchiveTarget,
    expectBytes: number | null,
    epoch?: number,
  ): Promise<ExportResponse> {
    const inParts = node.features.includes('multipart-upload')
    let parts: PartsUpload | null =
      inParts && expectBytes !== null && expectBytes > target.maxPutBytes
        ? await target.inParts(expectBytes)
        : null
    for (let asked = 1; ; asked++) {
      try {
        const sent = await this.#api.send(
          node,
          path,
          {
            ...body,
            url: target.put.url,
            headers: target.put.headers,
            ...(parts === null
              ? {}
              : {
                  parts: {
                    partSize: parts.partSize,
                    urls: parts.urls,
                    headers: parts.headers,
                  },
                }),
          },
          epoch,
        )
        if (parts !== null && sent.parts != null) await parts.complete(sent.parts)
        else if (parts !== null)
          await parts.abort().catch((error: unknown) =>
            this.#log('parts the node did not use were left', {
              path,
              error: (error as Error).message,
            }),
          )
        return sent
      } catch (error) {
        await parts?.abort().catch(() => undefined)
        parts = null
        // Packed again, a data that changed meanwhile can come out larger: asked a third time at most.
        const large = tooLarge(error)
        if (large === null || !inParts || asked >= 3 || !Number.isFinite(large.sizeBytes)) throw error
        parts = await target.inParts(large.sizeBytes)
      }
    }
  }

  /** The newest size known of a server's data, packed or not: what an export of it expects. */
  async #sizeOf(key: string): Promise<number | null> {
    const newest = await one<{ size_bytes: string }>(
      this.#db,
      sql`SELECT size_bytes FROM fleet_archives WHERE workload = ${key} AND size_bytes IS NOT NULL
          ORDER BY captured_at DESC LIMIT 1`,
    )
    return newest === null ? null : Number(newest.size_bytes)
  }

  /**
   * What a move carries, uploaded from the stopped source to the store: the data and everything
   * else the server can't make again. `reconstructible` names what it can (the server jar and the
   * libraries it unpacks), which the target makes on its first start rather than receives.
   */
  async exportForMove(node: NodeRow, key: string, epoch: number, reconstructible: readonly string[]) {
    const id = randomUUID()
    const objectKey = this.#store.objectKey(key, id)
    await this.#db.execute(sql`
      INSERT INTO fleet_archives (id, workload, purpose, epoch, node_id, object_key, status, consistency, captured_at, attempts,
        upload_started_at)
      VALUES (${id}, ${key}, 'move', ${epoch}, ${node.id}, ${objectKey}, 'uploading', 'stopped', now(), 1, now())`)
    const target = await this.#store.target(objectKey)
    let exported: ExportResponse
    try {
      exported = await this.#sendArchive(
        node,
        `/v1/workloads/${key}/export`,
        { quiesced: false, exclude: [...reconstructible] },
        target,
        await this.#sizeOf(key),
        epoch,
      )
      const held = await this.#store.head(objectKey)
      if (held === null || held.sizeBytes !== exported.sizeBytes)
        throw new Error(
          `The store holds ${held?.sizeBytes ?? 'nothing'}; the node sent ${exported.sizeBytes} bytes`,
        )
    } catch (error) {
      const large = tooLarge(error)
      if (large === null) {
        // Some of it may have reached the store: no move restores from it, so it goes, now or by upkeep.
        await this.#db.execute(sql`
          UPDATE fleet_archives SET status = 'deleted', deleted_at = now(), error = ${(error as Error).message}
          WHERE id = ${id}`)
        await this.#archives.deleteStored(id, objectKey).catch(() => undefined)
        throw error
      }
      // Refused before anything was sent; kept as failed, since the next move of its data reads why.
      const message = `${TOO_BIG}${large.reason}`
      await this.#db.execute(
        sql`UPDATE fleet_archives SET status = 'failed', error = ${message} WHERE id = ${id}`,
      )
      await event(this.#db, 'snapshot.upload_failed', {
        node: node.id,
        workload: key,
        epoch,
        data: {
          id,
          purpose: 'move',
          error: message,
          code: 'archive_too_large',
          sizeBytes: large.sizeBytes,
          limitBytes: large.limitBytes,
        },
      })
      // Another try would be refused the same way: a data this big can't move between nodes.
      throw new TooBigToMove(large.reason)
    }
    const current = await this.#db.transaction(async (t) => {
      const row = await placementOf(t, key, true)
      const still = row !== null && Number(row.epoch) === epoch
      await t.execute(sql`
        UPDATE fleet_archives SET status = ${still ? 'ready' : 'deleted'}, sha256 = ${exported.sha256},
          size_bytes = ${exported.sizeBytes}, uploaded_at = now() WHERE id = ${id}`)
      await event(t, still ? 'move.exported' : 'move.export_superseded', {
        node: node.id,
        workload: key,
        epoch,
        data: {
          id,
          sizeBytes: exported.sizeBytes,
          entries: exported.entries,
          ms: exported.durationMs,
          left: reconstructible,
        },
      })
      return still
    })
    if (!current) {
      await this.#archives.deleteStored(id, objectKey).catch(() => undefined)
      throw new StaleHandle('The server moved while it was copied; the copy was discarded')
    }
    return {
      id,
      objectKey,
      sha256: exported.sha256,
      sizeBytes: exported.sizeBytes,
    }
  }
}
