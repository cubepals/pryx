// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * A world's new home under a new epoch, after its first: a restore, a planned move off its node,
 * a move to a node with room to start, and a rebuild once its host was confirmed lost. Each takes
 * its new epoch in the transaction that chooses the node, fills the new copy before it is placed,
 * and only then fences the old one, so no step leaves a world without a copy; a move that can't
 * go on puts the server back where it was.
 *
 * The epochs and rows are `ledger.ts`'s, the room claimed `admission.ts`'s, and the copies that
 * carry a world `archives.ts`'s and `transfers.ts`'s: this file holds the order they go in.
 */
import type { Db, Tx } from '@blockly/db'
import { sql } from 'drizzle-orm'
import {
  type ProgressSink,
  type RestoreSource,
  RuntimeFull,
  type RuntimeHandle,
  type RuntimeSpec,
  RuntimeUnsupported,
  type SnapshotHandle,
} from '../../../app/ports/runtime.ts'
import { decodeHandle, decodeSnapshot, type FleetRef } from '../handle.ts'
import { NodeRefused, NodeUnreachable } from '../node-client.ts'
import type { Considered, PlacementRequest } from '../placement.ts'
import { getNode, lockRegion, type NodeRow } from '../registry.ts'
import { event, one } from '../sql.ts'
import type { EnsureResponse } from '../wire.ts'
import { type Admission, NO_ROOM } from './admission.ts'
import type { ArchiveRow, Archives } from './archives.ts'
import { type Ledger, nextEpoch, type PlacementRow, placementOf, rehome, StaleHandle } from './ledger.ts'
import { describe, type NodeChoice } from './node-choice.ts'
import { TooBigToMove, type Transfers } from './transfers.ts'
import type { WorkloadApi } from './workload-api.ts'

export class Rehoming {
  readonly #db: Db
  readonly #api: Pick<WorkloadApi, 'view' | 'restore' | 'fence' | 'deleteWithData'>
  readonly #ledger: Pick<Ledger, 'current' | 'nodeOf' | 'settle' | 'dropMoveRequest'>
  readonly #choice: Pick<NodeChoice, 'request' | 'choose'>
  readonly #admission: Pick<
    Admission,
    'put' | 'power' | 'startOn' | 'claimMoved' | 'lookAgain' | 'claim' | 'unclaim'
  >
  readonly #archives: Pick<
    Archives,
    'archiveOf' | 'discardExport' | 'source' | 'fromStore' | 'forgetNodeCopies'
  >
  readonly #transfers: Pick<Transfers, 'exportForMove' | 'tooBigToMove' | 'uploadBeforeDelete'>
  readonly #provider: string
  readonly #log: (message: string, fields?: object) => void

  constructor(options: {
    db: Db
    api: Pick<WorkloadApi, 'view' | 'restore' | 'fence' | 'deleteWithData'>
    ledger: Pick<Ledger, 'current' | 'nodeOf' | 'settle' | 'dropMoveRequest'>
    choice: Pick<NodeChoice, 'request' | 'choose'>
    admission: Pick<Admission, 'put' | 'power' | 'startOn' | 'claimMoved' | 'lookAgain' | 'claim' | 'unclaim'>
    archives: Pick<Archives, 'archiveOf' | 'discardExport' | 'source' | 'fromStore' | 'forgetNodeCopies'>
    transfers: Pick<Transfers, 'exportForMove' | 'tooBigToMove' | 'uploadBeforeDelete'>
    provider: string
    log: (message: string, fields?: object) => void
  }) {
    this.#db = options.db
    this.#api = options.api
    this.#ledger = options.ledger
    this.#choice = options.choice
    this.#admission = options.admission
    this.#archives = options.archives
    this.#transfers = options.transfers
    this.#provider = options.provider
    this.#log = options.log
  }

  /**
   * A new epoch, always: the world's lineage changes, so copies and backups of the old one are
   * superseded. In place for a placed server; on a newly chosen node for a released one (the world
   * rests in the store) or a displaced one (its host was confirmed lost).
   */
  async restore(
    handle: RuntimeHandle,
    from: RestoreSource,
    spec: RuntimeSpec,
    progress: ProgressSink,
  ): Promise<RuntimeHandle> {
    const ref = decodeHandle(handle)
    await progress.step('storage')
    const plan = await this.#db.transaction(async (t) => {
      const row = await placementOf(t, ref.key, true)
      if (row === null) throw new Error(`Nothing is placed for ${ref.key}`)
      await lockRegion(t, row.region_key)
      const epoch = Number(row.epoch)
      // A restore a crash interrupted: its new home is chosen; it is filled again.
      if (
        row.state === 'placing' &&
        row.completing === 'restore' &&
        epoch > ref.epoch &&
        row.node_id !== null
      )
        return { row, inPlace: false }
      if (epoch !== ref.epoch)
        throw new StaleHandle(`This handle is for epoch ${ref.epoch}; the server is at ${epoch}`)
      const request = this.#choice.request(
        spec,
        row.region_key,
        row.state === 'displaced' && row.node_id ? [row.node_id] : [],
      )
      if (row.state === 'placed' && row.node_id !== null) {
        const to = { node: row.node_id, epoch: await nextEpoch(t, ref.key), region: row.region_key, request }
        return {
          row: await rehome(t, row, to, { ended: 'restored', started: 'restore', restoreFrom: null }),
          inPlace: true,
        }
      }
      if (row.state !== 'released' && row.state !== 'displaced')
        throw new Error(`The placement is ${row.state}`)
      const decision = await this.#choice.choose(t, request)
      if (decision.node === null) throw new RuntimeFull(this.#provider, describe(decision.considered))
      const to = {
        node: decision.node.id,
        epoch: await nextEpoch(t, ref.key),
        region: row.region_key,
        request,
      }
      const ended = row.state === 'displaced' ? 'lost' : 'released'
      return {
        row: await rehome(t, row, to, { ended, started: 'restore', restoreFrom: null }),
        inPlace: false,
      }
    })
    let epoch = Number(plan.row.epoch)
    let node = await this.#ledger.nodeOf(plan.row.node_id)
    let wasRunning = false
    if (plan.inPlace) {
      const view = await this.#api.view(node, ref.key)
      wasRunning = view.state === 'running'
      // A newer epoch may always tear the old copy down.
      if (wasRunning) await this.#admission.power(node, ref.key, epoch, 'stop')
    }
    await progress.step('compute')
    let ensured: EnsureResponse
    const refused: string[] = []
    for (;;) {
      try {
        ensured = await this.#admission.put(node, ref.key, spec, epoch, false)
        break
      } catch (error) {
        // A new home its node refuses for room is chosen again, elsewhere, so no wake waits on it.
        if (plan.inPlace || !(error instanceof RuntimeFull)) throw error
        refused.push(node.id)
        const again = await this.#chooseAgain(ref.key, epoch, node, spec, refused, error)
        epoch = Number(again.epoch)
        node = await this.#ledger.nodeOf(again.node_id)
      }
    }
    const restored = await this.#api.restore(node, ref.key, epoch, await this.#archives.source(from, node.id))
    const fresh = await this.#ledger.settle(ref.key, node, epoch, ensured.workload)
    await progress.handle(fresh)
    await event(this.#db, 'placement.restored', {
      node: node.id,
      workload: ref.key,
      epoch,
      data: { sizeBytes: restored.sizeBytes, entries: restored.entries, ms: restored.durationMs },
    })
    if (wasRunning) await this.#admission.startOn(node, ref.key, epoch, plan.row.region_key)
    return fresh
  }

  /**
   * A restore's new home whose node refused it for room, given up for another node with room under
   * a new epoch, avoiding every node that refused it: `placement.refused` says so. RuntimeFull when
   * none has room; the row stays placing where it was, and the next wake chooses again.
   */
  async #chooseAgain(
    key: string,
    epoch: number,
    node: NodeRow,
    spec: RuntimeSpec,
    refused: string[],
    refusal: RuntimeFull,
  ): Promise<PlacementRow> {
    const again = await this.#db.transaction(async (t) => {
      const row = await placementOf(t, key, true)
      if (row === null) throw new Error(`Nothing is placed for ${key}`)
      await lockRegion(t, row.region_key)
      if (Number(row.epoch) !== epoch || row.node_id !== node.id || row.state !== 'placing')
        throw new StaleHandle('The placement changed while it was being restored')
      const request = this.#choice.request(spec, row.region_key, refused)
      const decision = await this.#choice.choose(t, request)
      await event(t, 'placement.refused', {
        node: node.id,
        workload: key,
        epoch,
        data: { why: refusal.message, considered: decision.considered },
      })
      if (decision.node === null) return { row: null, considered: decision.considered }
      const to = { node: decision.node.id, epoch: await nextEpoch(t, key), region: row.region_key, request }
      const ended = `refused: ${node.name}`
      return {
        row: await rehome(t, row, to, { ended, started: 'restore', restoreFrom: null }),
        considered: decision.considered,
      }
    })
    if (again.row === null)
      throw new RuntimeFull(
        this.#provider,
        `${node.name} refused it for room; elsewhere: ${describe(again.considered)}`,
      )
    return again.row
  }

  /**
   * Without `from`, a planned move: stop, export what can't be made again, a new epoch on another
   * node, restore, and the old copy fenced and deleted. It is left stopped; the application starts
   * it where it wants it running. With `from`, the host was lost: only a server whose host an
   * operator confirmed lost is rebuilt, and nothing on that host is touched.
   */
  async relocate(
    handle: RuntimeHandle,
    region: string,
    spec: RuntimeSpec,
    progress: ProgressSink,
    from?: SnapshotHandle,
  ): Promise<RuntimeHandle> {
    const ref = decodeHandle(handle)
    const pending = await placementOf(this.#db, ref.key)
    // A move or rebuild a crash interrupted after its new home was chosen: finish it.
    if (
      pending?.state === 'placing' &&
      (pending.completing === 'move' || pending.completing === 'recover') &&
      Number(pending.epoch) > ref.epoch
    )
      return this.finish(pending, spec, progress)
    if (from !== undefined) return this.#rebuild(ref, region, spec, progress, from)

    const { node: source, row: current } = await this.#ledger.current(handle)
    const requested = current.move_requested_at !== null
    if (source.region_key === region && source.lifecycle === 'active' && !requested) return handle
    const request = this.#choice.request(spec, region, [source.id], requested ? current.move_to : null)
    // A world whose last copy was too big for one upload can't move: declined before anything stops.
    const tooBig = await this.#transfers.tooBigToMove(ref.key)
    if (tooBig !== null) throw await this.#declineMove(current, tooBig)
    // Room first, so a move with nowhere to go doesn't stop the server for nothing.
    const precheck = await this.#choice.choose(this.#db, request)
    if (precheck.node === null) throw new RuntimeFull(this.#provider, describe(precheck.considered))
    await progress.step('storage')
    const view = await this.#api.view(source, ref.key)
    const wasRunning = view.state === 'running'
    if (view.state !== 'stopped' && view.state !== 'created')
      await this.#admission.power(source, ref.key, ref.epoch, 'stop')
    const archive = await this.#transfers
      .exportForMove(source, ref.key, ref.epoch, spec.storage.reconstructible ?? [])
      .catch(async (error: unknown) => {
        if (!(error instanceof TooBigToMove)) throw error
        // Nothing moved: it runs again where it was.
        if (wasRunning) await this.#admission.startOn(source, ref.key, ref.epoch, source.region_key)
        throw await this.#declineMove(current, error.why)
      })
    const moved = await this.#newHome(archive, async (t) => {
      await lockRegion(t, region)
      const row = await placementOf(t, ref.key, true)
      if (row === null || Number(row.epoch) !== ref.epoch || row.node_id !== source.id)
        throw new StaleHandle('The placement changed during the move')
      const decision = await this.#choice.choose(t, request)
      if (decision.node === null) return { row: null, considered: decision.considered }
      const target = { node: decision.node.id, epoch: await nextEpoch(t, ref.key), region, request }
      const rehomed = await rehome(t, row, target, {
        ended: 'moved',
        started: 'move',
        restoreFrom: archive.id,
      })
      // It ran, so the application starts it again where it lands: the room it was chosen for is
      // claimed for it now, as for a move to start, so nothing takes it while its world moves.
      if (wasRunning) await this.#admission.claimMoved(t, ref.key, target.epoch)
      return { row: rehomed, considered: decision.considered }
    })
    if (moved.row === null) {
      await this.#archives.discardExport(archive)
      if (wasRunning) await this.#admission.startOn(source, ref.key, ref.epoch, source.region_key)
      throw new RuntimeFull(this.#provider, describe(moved.considered))
    }
    return this.finish(moved.row, spec, progress)
  }

  /**
   * A sleeping server whose node has no room to run it now moves first, to a node in its region
   * that has room: its world goes through the archive store as a planned move's does, and it starts
   * there. Nobody is asked, since the start would fail otherwise and the world is the same
   * wherever it runs; operators see `placement.moved_for_room`. RuntimeFull when no node has room.
   *
   * Its node is asked how its copies are first: one that stopped since its last report leaves room
   * after all, and the server starts where it is.
   */
  async startElsewhere(
    row: PlacementRow,
    spec: RuntimeSpec,
    progress: ProgressSink,
    refusal: string,
  ): Promise<RuntimeHandle> {
    const key = row.workload
    const epoch = Number(row.epoch)
    const region = row.region_key
    const source = await this.#ledger.nodeOf(row.node_id)
    const request = this.#choice.request(spec, region, [source.id])
    const here = await this.#startHere(source, row, spec, progress, request)
    if (typeof here !== 'string') return here
    const why = here === '' ? refusal : here
    const full = (considered: readonly Considered[]) =>
      new RuntimeFull(this.#provider, `${source.name}: ${why}; elsewhere: ${describe(considered)}`)
    // A world too big to move waits for room where it is, as a start with nowhere to go does.
    const stuck = (tooBig: string) =>
      new RuntimeFull(this.#provider, `${source.name}: ${why}; it can't move to another node: ${tooBig}`)
    const tooBig = await this.#transfers.tooBigToMove(key)
    if (tooBig !== null) throw stuck(tooBig)
    // Room first, so a move with nowhere to go doesn't copy the world for nothing.
    const precheck = await this.#choice.choose(this.#db, request)
    if (precheck.node === null) throw full(precheck.considered)
    await progress.step('storage')
    const archive = await this.#transfers
      .exportForMove(source, key, epoch, spec.storage.reconstructible ?? [])
      .catch((error: unknown) => {
        throw error instanceof TooBigToMove ? stuck(error.why) : error
      })
    const moved = await this.#newHome(archive, async (t) => {
      await lockRegion(t, region)
      const current = await placementOf(t, key, true)
      if (current === null || Number(current.epoch) !== epoch || current.node_id !== source.id)
        throw new StaleHandle('The placement changed while its world was copied to move')
      const decision = await this.#choice.choose(t, request)
      if (decision.node === null) return { row: null, considered: decision.considered }
      const target = { node: decision.node.id, epoch: await nextEpoch(t, key), region, request }
      const rehomed = await rehome(t, current, target, {
        ended: 'moved',
        started: 'move',
        restoreFrom: archive.id,
      })
      // It moves to start: the room it moves for is claimed for it now.
      await this.#admission.claimMoved(t, key, target.epoch)
      await event(t, 'placement.moved_for_room', {
        node: target.node,
        workload: key,
        epoch: target.epoch,
        data: { fromNode: source.id, fromEpoch: epoch, why },
      })
      return { row: rehomed, considered: decision.considered }
    })
    if (moved.row === null) {
      await this.#archives.discardExport(archive)
      throw full(moved.considered)
    }
    const handle = await this.finish(moved.row, spec, progress)
    await progress.step('booting')
    await this.#admission.startOn(
      await this.#ledger.nodeOf(moved.row.node_id),
      key,
      Number(moved.row.epoch),
      region,
    )
    return handle
  }

  /**
   * The server started on its own node after all, when a fresh look at the node leaves room for it:
   * its handle. Else why not: the node's refusal for room, or '' when the look changed nothing.
   */
  async #startHere(
    node: NodeRow,
    row: PlacementRow,
    spec: RuntimeSpec,
    progress: ProgressSink,
    request: PlacementRequest,
  ): Promise<RuntimeHandle | string> {
    if (!(await this.#admission.lookAgain(node))) return ''
    const key = row.workload
    const epoch = Number(row.epoch)
    const claimed = await this.#db.transaction(async (t) => {
      await lockRegion(t, row.region_key)
      const current = await placementOf(t, key, true)
      if (current === null || Number(current.epoch) !== epoch || current.node_id !== node.id)
        throw new StaleHandle('The placement changed before it could start')
      return (await this.#admission.claim(t, current, request)) === null
    })
    if (!claimed) return ''
    await progress.step('compute')
    const ensured = await this.#admission.put(node, key, spec, epoch, false)
    const handle = await this.#ledger.settle(key, node, epoch, ensured.workload)
    await progress.handle(handle)
    await progress.step('booting')
    try {
      await this.#admission.power(node, key, epoch, 'start')
      return handle
    } catch (error) {
      if (!(error instanceof NodeRefused && NO_ROOM.has(error.code))) throw error
      await this.#admission.unclaim(key, epoch)
      return `${node.name}: ${error.message}`
    }
  }

  /**
   * A move between nodes declined, for a world too big for one upload: asking again changes
   * nothing, so an operator's request to move it is dropped. Each is recorded, saying why, so a
   * drain that can't finish says what holds it.
   */
  async #declineMove(row: PlacementRow, why: string): Promise<RuntimeUnsupported> {
    const requested = row.move_requested_at !== null
    if (requested) await this.#ledger.dropMoveRequest(row)
    await event(this.#db, 'placement.move_declined', {
      node: row.node_id,
      workload: row.workload,
      epoch: Number(row.epoch),
      data: { why, requested },
    })
    return new RuntimeUnsupported(this.#provider, `moving this server to another node: ${why}`)
  }

  /**
   * The transaction that gives a moving server its new home, once its copy is in the store. When
   * the placement changed meanwhile, no move will restore from that copy, so it is discarded.
   */
  async #newHome<T>(archive: { id: string; objectKey: string }, work: (t: Tx) => Promise<T>): Promise<T> {
    try {
      return await this.#db.transaction(work)
    } catch (error) {
      if (error instanceof StaleHandle) await this.#archives.discardExport(archive)
      throw error
    }
  }

  /**
   * A host-lost rebuild: the placement must be displaced, which only an operator makes it. From the
   * snapshot the application names when it reached the store, else from the newest of its server's
   * snapshots that did and is no newer.
   */
  async #rebuild(
    ref: FleetRef,
    region: string,
    spec: RuntimeSpec,
    progress: ProgressSink,
    from: SnapshotHandle,
  ): Promise<RuntimeHandle> {
    const snap = decodeSnapshot(from)
    if (snap.key !== ref.key) throw new Error('That snapshot is of another server')
    const asked = await this.#archives.archiveOf(snap.archive)
    const source =
      asked !== null && asked.status === 'ready'
        ? asked
        : await one<ArchiveRow>(
            this.#db,
            sql`SELECT * FROM fleet_archives WHERE workload = ${ref.key} AND purpose = 'snapshot' AND status = 'ready'
                  AND captured_at <= ${asked?.captured_at ?? new Date()} ORDER BY captured_at DESC LIMIT 1`,
          )
    if (source === null)
      throw new Error('No copy of this server reached the archive store before its host was lost')
    if (source.id !== snap.archive)
      await event(this.#db, 'recover.older_copy', {
        workload: ref.key,
        data: { asked: snap.archive, used: source.id, capturedAt: source.captured_at },
      })
    await progress.step('storage')
    const rebuilt = await this.#db.transaction(async (t) => {
      await lockRegion(t, region)
      const row = await placementOf(t, ref.key, true)
      if (row === null) throw new Error(`Nothing is placed for ${ref.key}`)
      if (row.state !== 'displaced')
        throw new Error(
          `A server is rebuilt elsewhere only once its host is confirmed lost; this one is ${row.state}. Its copy could still be running.`,
        )
      if (Number(row.epoch) !== ref.epoch) throw new StaleHandle(`This handle is for epoch ${ref.epoch}`)
      const request = this.#choice.request(spec, region, row.node_id ? [row.node_id] : [])
      const decision = await this.#choice.choose(t, request)
      if (decision.node === null) return { row: null, considered: decision.considered }
      const target = { node: decision.node.id, epoch: await nextEpoch(t, ref.key), region, request }
      return {
        row: await rehome(t, row, target, { ended: 'lost', started: 'recover', restoreFrom: source.id }),
        considered: decision.considered,
      }
    })
    if (rebuilt.row === null) throw new RuntimeFull(this.#provider, describe(rebuilt.considered))
    return this.finish(rebuilt.row, spec, progress)
  }

  /**
   * A copy a server left on a node, deleted with its data, which takes the snapshots on that node
   * with it: those only there go up to the store first. False when one may still go up, so the copy
   * stays for upkeep (`tidy`) to delete once it has.
   */
  async deleteLeftCopy(node: NodeRow, key: string, epoch: number): Promise<boolean> {
    if (!(await this.#transfers.uploadBeforeDelete(key, node.id))) return false
    await this.#api.deleteWithData(node, key, epoch)
    await this.#archives.forgetNodeCopies(key, node.id)
    return true
  }

  /**
   * Fills a placing row from its copy in the store and places it; then fences the copy the move
   * left behind, and deletes it if it was a planned move. Idempotent, so a retry after a crash
   * resumes here.
   */
  async finish(row: PlacementRow, spec: RuntimeSpec, progress: ProgressSink): Promise<RuntimeHandle> {
    const key = row.workload
    const epoch = Number(row.epoch)
    const archive = row.restore_from === null ? null : await this.#archives.archiveOf(row.restore_from)
    if (archive === null || archive.object_key === null || archive.status !== 'ready')
      throw new Error('The copy this move restores from is gone')
    const node = await this.#ledger.nodeOf(row.node_id)
    await progress.step('compute')
    const ensured = await this.#admission.put(node, key, spec, epoch, false)
    const restored = await this.#api.restore(
      node,
      key,
      epoch,
      await this.#archives.fromStore(archive.object_key, archive.sha256),
    )
    const handle = await this.#ledger.settle(key, node, epoch, ensured.workload)
    await progress.handle(handle)
    await event(this.#db, 'placement.restored', {
      node: node.id,
      workload: key,
      epoch,
      data: {
        sizeBytes: restored.sizeBytes,
        entries: restored.entries,
        ms: restored.durationMs,
        from: archive.id,
      },
    })
    // A move's copy has done its work; a rebuild's is a snapshot the application still holds.
    if (archive.purpose === 'move')
      await this.#archives.discardExport({ id: archive.id, objectKey: archive.object_key })
    const before = await one<{ node_id: string | null; epoch: string; end_reason: string | null }>(
      this.#db,
      sql`SELECT node_id, epoch, end_reason FROM fleet_placement_history
          WHERE workload = ${key} AND epoch < ${epoch} ORDER BY epoch DESC LIMIT 1`,
    )
    if (before?.node_id && before.node_id !== node.id) {
      const source = await getNode(this.#db, before.node_id)
      // At once rather than at its next heartbeat; the heartbeat answer and tidy() back this up.
      if (source !== null && (source.lifecycle === 'active' || source.lifecycle === 'draining')) {
        try {
          const fenced = await this.#api.fence(source, key, epoch)
          await event(this.#db, 'copy.fenced', {
            node: source.id,
            workload: key,
            epoch: Number(before.epoch),
            data: fenced,
          })
          if (before.end_reason === 'moved') await this.deleteLeftCopy(source, key, epoch)
        } catch (error) {
          if (!(error instanceof NodeRefused || error instanceof NodeUnreachable)) throw error
          this.#log('move left its source copy for upkeep', {
            node: source.id,
            workload: key,
            error: error.message,
          })
        }
      }
    }
    return handle
  }
}
