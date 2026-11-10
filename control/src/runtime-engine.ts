// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The fleet: Blockly's sandboxes on Linux machines it manages directly, each running pryxd, as
 * one more SandboxRuntime (docs/fleet.md). The application asks for a server's workload and gets
 * a handle; nothing above this file learns that nodes exist. A fleet of one node is a fleet.
 *
 * - Every new home for a data (a first placement, a move, a restore, a rebuild) is a new epoch,
 *   raised by compare-and-set in the transaction that chooses the node, and carried by every
 *   command, copy and handle. Nodes refuse older ones; heartbeats fence superseded copies.
 * - A placement is written before its node is called (`placing`), so a crash leaves an intent a
 *   retry finishes, never a copy nobody knows about.
 * - Nothing here starts, stops or moves a server on its own: the application's jobs call the port,
 *   and its server status stays the only power intent.
 *
 * Runtime concerns:
 * - `upkeep.ts`: observations and archive housekeeping.
 * - `observation.ts`: persisted reports mapped to runtime states.
 * - `admission.ts`: room to run on a node, claimed before a start and given back after a stop.
 * - `archives.ts`: a data's copies in `fleet_archives`, kept while held and deleted once not.
 * - `ledger.ts`: where each server's data lives, under which epoch, and the history of its homes.
 * - `node-cache.ts`: what the synchronous methods know of each node, reread every few seconds.
 * - `node-choice.ts`: what a server needs of a node, judged by placement against every node now.
 * - `power.ts`: a placed copy woken (resumed or booted), put to sleep or stopped, and waited on to run.
 * - `rehoming.ts`: a data's new home under a new epoch, its old copy fenced once the new one holds it.
 * - `transfers.ts`: a copy of a data sent off its node, to the store or a target, whole or in parts.
 * - `workload-api.ts`: the calls made to a node's workload API, each with its path, epoch and timeout.
 */

import { sql } from 'drizzle-orm'
import { Admission, NO_ROOM } from './admission.ts'
import type { FleetStore } from './archive-transport.ts'
import { Archives } from './archives.ts'
import type { Db } from './database.ts'
import { decodeHandle, decodeSnapshot, encodeHandle, encodeSnapshot, isHandle, isSnapshot } from './handle.ts'
import type { Thresholds } from './health.ts'
import { hostLost, Ledger, nextEpoch, place, placementOf, remove, StaleHandle } from './ledger.ts'
import { NodeCache } from './node-cache.ts'
import { describe, NEEDS, NodeChoice } from './node-choice.ts'
import { type NodeClient, NodeRefused } from './node-client.ts'
import { type Considered, cpuRequest, type PlacementOptions, type PlacementRequest } from './placement.ts'
import { Power } from './power.ts'
import { getNode, lockRegion } from './registry.ts'
import { Rehoming } from './rehoming.ts'
import {
  type ArchiveTarget,
  type Audience,
  type Endpoint,
  type InstallSeed,
  type Placement,
  type ProgressSink,
  type RestoreSource,
  type RuntimeCapacity,
  RuntimeFull,
  type RuntimeHandle,
  type RuntimeKey,
  type RuntimeLocation,
  type RuntimeObservation,
  type RuntimePrices,
  type RuntimeSpec,
  type SandboxRuntime,
  type SnapshotHandle,
} from './runtime-model.ts'
import { RuntimeViews } from './runtime-views.ts'
import { event, rows } from './sql.ts'
import { Transfers } from './transfers.ts'
import { FleetUpkeep } from './upkeep.ts'
import { WorkloadApi } from './workload-api.ts'
import { requireWorkloadFeatures } from './workload-capabilities.ts'

export interface FleetRuntimeOptions {
  db: Db
  nodes: NodeClient
  store: FleetStore
  deployment: string
  /** Product region → the fleet region nodes enroll into; a region not listed is its own. */
  regionMap: Readonly<Record<string, string>>
  placement: PlacementOptions
  /** CPU a server is counted for, per GB of memory, in thousandths of a core. */
  cpuMillisPerGb: number
  thresholds: Thresholds
  /** Whether a sleep suspends, where the node can; off, every sleep stops. */
  beforeStart?: (nodeId: string, workload: string, epoch: number) => Promise<void>
  suspend?: boolean
  log?: (message: string, fields?: object) => void
}

export { StaleHandle } from './ledger.ts'

export class FleetRuntime implements SandboxRuntime {
  readonly #views: RuntimeViews

  readonly provider = 'fleet'
  /** Capacity changes as nodes join; a number read once can't say that. */
  readonly serverCeiling = null
  /** Copies stay until the application deletes them. */
  readonly snapshotLifetimeDays = null
  /**
   * A placed server keeps its node's address and its host ports while stopped, and no other server
   * is given them; a move or a restore issues a new handle.
   */
  readonly stableEndpoints = true
  readonly #o: FleetRuntimeOptions
  readonly #log: (message: string, fields?: object) => void
  readonly #api: WorkloadApi
  readonly #known: NodeCache
  readonly #choice: NodeChoice
  readonly #ledger: Ledger
  readonly #admission: Admission
  readonly #archives: Archives
  readonly #transfers: Transfers
  readonly #rehoming: Rehoming
  readonly #maintenance: FleetUpkeep
  readonly #power: Power

  constructor(options: FleetRuntimeOptions) {
    this.#o = options
    this.#log = options.log ?? ((message, fields) => console.warn(`fleet: ${message}`, fields ?? ''))
    this.#api = new WorkloadApi(options.nodes, options.beforeStart)
    this.#known = new NodeCache(options.db, this.#log)
    this.#choice = new NodeChoice({
      cpuMillisPerGb: options.cpuMillisPerGb,
      thresholds: options.thresholds,
      placement: options.placement,
    })
    this.#ledger = new Ledger({
      db: options.db,
      deployment: options.deployment,
    })
    this.#admission = new Admission({
      db: options.db,
      api: this.#api,
      choice: this.#choice,
      ledger: this.#ledger,
      provider: this.provider,
    })
    this.#archives = new Archives({
      db: options.db,
      api: this.#api,
      store: options.store,
      suspectSeconds: options.thresholds.suspectSeconds,
      log: this.#log,
    })
    this.#transfers = new Transfers({
      db: options.db,
      api: this.#api,
      store: options.store,
      ledger: this.#ledger,
      archives: this.#archives,
      provider: this.provider,
      log: this.#log,
    })
    this.#rehoming = new Rehoming({
      db: options.db,
      api: this.#api,
      ledger: this.#ledger,
      choice: this.#choice,
      admission: this.#admission,
      archives: this.#archives,
      transfers: this.#transfers,
      provider: this.provider,
      log: this.#log,
    })
    this.#power = new Power({
      db: options.db,
      api: this.#api,
      admission: this.#admission,
      ledger: this.#ledger,
      suspend: options.suspend ?? false,
      provider: this.provider,
      log: this.#log,
    })
    this.#maintenance = new FleetUpkeep(
      options,
      this.#ledger,
      this.#archives,
      this.#rehoming,
      this.#transfers,
      this.#log,
    )

    this.#views = new RuntimeViews(options, this.#known, this.#ledger)
  }

  /** Reads the registry now and every few seconds after, for the methods that must stay pure. */
  watchRegistry(): Promise<() => void> {
    return this.#known.watch()
  }

  refreshRegistry(): Promise<void> {
    return this.#known.refresh()
  }
  home(handle: RuntimeHandle) {
    return this.#known.home(handle)
  }
  nodeAddress(handle: RuntimeHandle): { id: string; apiAddress: string; features?: string[] } | null {
    return this.#known.address(handle)
  }

  #region(regionKey: string): string {
    return this.#o.regionMap[regionKey] ?? regionKey
  }

  // ─── provisioning and power ────────────────────────────────────────────────────────────────

  owns(handle: RuntimeHandle): boolean {
    return isHandle(handle)
  }

  /**
   * Its nodes, which are paid for full or empty: their price from the `monthly_cost_cents` label,
   * what the ledger holds for the sandboxes running on each and has placed on each, and what each
   * reported in use at its last heartbeat. A lost node is still listed until it is retired, since
   * it is billed until it is cancelled.
   */
  capacity(): Promise<RuntimeCapacity> {
    return this.#views.capacity()
  }

  ownsSnapshot(snapshot: SnapshotHandle): boolean {
    return isSnapshot(snapshot)
  }

  /** A node in the region with room for the server and the headroom kept on it, as placement would choose now. */
  async hasRoom(placement: Placement, size: { memoryMb: number; storageGb: number }): Promise<boolean> {
    const request: PlacementRequest = {
      memoryMb: size.memoryMb,
      cpuMillis: cpuRequest(size.memoryMb, this.#o.cpuMillisPerGb),
      diskGb: size.storageGb,
      // A capacity hint without a spec; actual placement counts the spec ports.
      ports: 2,
      regionKey: this.#region(placement.regionKey),
      needs: NEEDS,
      avoid: [],
    }
    return (await this.#choice.choose(this.#o.db, request)).node !== null
  }

  /**
   * A placement with no node, as a released one: its data comes from the archive store when it is
   * restored, onto whichever node has room then. One already here is handed back as it is.
   */
  async adopt(key: RuntimeKey, placement: Placement, spec: RuntimeSpec): Promise<RuntimeHandle> {
    const region = this.#region(placement.regionKey)
    const request = this.#choice.request(spec, region)
    const ports = Object.fromEntries(spec.ports.map((port) => [port.name, port.port]))
    return this.#ledger.adopt(key, region, request, ports)
  }

  async ensureProvisioned(
    key: RuntimeKey,
    placement: Placement,
    spec: RuntimeSpec,
    progress: ProgressSink,
    // A speed-up a runtime may ignore, as the port allows; a node pulls what the image downloads.
    _install: InstallSeed | null = null,
    _storage?: unknown,
    started = true,
    onlyNode?: string,
  ): Promise<RuntimeHandle> {
    await progress.step('allocating')
    const region = this.#region(placement.regionKey)
    const request = this.#choice.request(spec, region, [], onlyNode ?? null)
    const chosen = await this.#o.db.transaction(async (t) => {
      await lockRegion(t, region)
      const row = await placementOf(t, key, true)
      const none = [] as Considered[]
      if (row !== null) {
        const target = row.node_id ? await getNode(t, row.node_id) : null
        if (target) requireWorkloadFeatures(spec, target.features)
        if (row.state === 'displaced') throw hostLost()
        // The application never provisions a resting server; nor does the fleet, since it would give
        // the data new, empty storage.
        if (row.state === 'released') throw new Error("This server's data rests in the archive store.")
        // A move a crash interrupted after its new home was chosen: finished, then started.
        if (row.state === 'placing' && (row.completing === 'move' || row.completing === 'recover'))
          return {
            row,
            created: false,
            considered: none,
            full: null,
            unfinished: true,
          }
        if (row.completing !== null)
          throw new Error(`A ${row.completing} of this server is under way; retrying it finishes it.`)
        // Asleep, it holds no memory: it starts where it is if its node has room to run it now.
        return {
          row,
          created: false,
          considered: none,
          full: started ? await this.#admission.claim(t, row, request) : null,
          unfinished: false,
        }
      }
      const decision = await this.#choice.choose(t, request)
      if (decision.node === null)
        return {
          row: null,
          created: false,
          considered: decision.considered,
          full: null,
          unfinished: false,
        }
      const epoch = await nextEpoch(t, key)
      // It starts as soon as it is made, so its room to run is claimed with it.
      const inserted = await place(t, {
        key,
        node: decision.node.id,
        epoch,
        region,
        request,
        policy: this.#o.placement.policy,
        considered: decision.considered,
      })
      return {
        row: inserted,
        created: true,
        considered: decision.considered,
        full: null,
        unfinished: false,
      }
    })
    if (chosen.row === null) {
      await event(this.#o.db, 'placement.refused', {
        workload: key,
        data: { considered: chosen.considered },
      })
      throw new RuntimeFull(this.provider, describe(chosen.considered), [
        ...new Set(chosen.considered.flatMap((node) => node.missingGuarantees ?? [])),
      ])
    }
    if (chosen.unfinished) {
      const moved = await this.#rehoming.finish(chosen.row, spec, progress)
      await progress.step('booting')
      await this.#admission.startOn(
        await this.#ledger.nodeOf(chosen.row.node_id),
        key,
        Number(chosen.row.epoch),
        chosen.row.region_key,
      )
      return moved
    }
    if (chosen.full !== null) return this.#rehoming.startElsewhere(chosen.row, spec, progress, chosen.full)
    const node = await this.#ledger.nodeOf(chosen.row.node_id)
    try {
      return await this.#power.wake(chosen.row, node, spec, progress, chosen.created, started)
    } catch (error) {
      if (!(error instanceof NodeRefused && NO_ROOM.has(error.code))) throw error
      // The node counts what the ledger doesn't, such as a copy it still runs: it moves after all.
      await this.#admission.unclaim(key, Number(chosen.row.epoch))
      const placed = await placementOf(this.#o.db, key)
      if (placed === null) throw error
      return this.#rehoming.startElsewhere(placed, spec, progress, `${node.name}: ${error.message}`)
    }
  }

  async apply(handle: RuntimeHandle, spec: RuntimeSpec): Promise<RuntimeHandle> {
    const { row, node } = await this.#ledger.current(handle)
    return this.#power.apply(row, node, spec)
  }

  /**
   * On the node it was placed on, under the same handle: its address stays while it is placed.
   * RuntimeFull when the sandboxes running there leave it no room now; a start that has the server's
   * spec, `ensureProvisioned`, moves it to a node with room instead.
   */
  async start(handle: RuntimeHandle): Promise<RuntimeHandle> {
    const { ref, row, node } = await this.#ledger.current(handle)
    await this.#admission.startOn(node, ref.key, ref.epoch, row.region_key)
    return handle
  }

  /** A stop, killed where it won't, then a start, on the same node and ports. */
  async restart(handle: RuntimeHandle): Promise<void> {
    await this.stop(handle).catch(() => this.forceStop(handle))
    await this.start(handle)
  }

  async stop(handle: RuntimeHandle): Promise<void> {
    const { ref, node } = await this.#ledger.current(handle)
    await this.#power.halt(ref, node, 'stop')
  }

  /** Suspended where the deployment's switch is on and the node can, so a start resumes it; else stopped. */
  async sleep(handle: RuntimeHandle): Promise<'suspended' | 'stopped'> {
    const { ref, node } = await this.#ledger.current(handle)
    return this.#power.sleep(ref, node)
  }

  async forceStop(handle: RuntimeHandle): Promise<void> {
    const { ref, node } = await this.#ledger.current(handle)
    await this.#power.halt(ref, node, 'kill')
  }

  async waitRunning(handle: RuntimeHandle, signal: AbortSignal): Promise<void> {
    const { ref, node } = await this.#ledger.current(handle)
    await this.#power.waitRunning(ref, node, signal)
  }

  // ─── snapshots ─────────────────────────────────────────────────────────────────────────────

  /**
   * A copy on the server's own node, made at once while the application holds its saving still,
   * then uploaded to the archive store in the background so it outlives the node. The handle is
   * issued before the upload ends: a restore on the same node uses the local copy, and a rebuild
   * elsewhere waits for, or falls back from, the uploaded one.
   */
  async snapshot(handle: RuntimeHandle) {
    const { ref, node } = await this.#ledger.current(handle)
    const { id, sizeBytes, at } = await this.#archives.take(ref.key, ref.epoch, node)
    // Off the node as soon as it can be; upkeep tries again whatever this doesn't finish.
    void this.upload(id).catch((error) =>
      this.#log('snapshot upload failed', {
        id,
        error: (error as Error).message,
      }),
    )
    return {
      snapshot: encodeSnapshot({
        deployment: this.#o.deployment,
        key: ref.key,
        archive: id,
        epoch: ref.epoch,
      }),
      sizeBytes,
      at,
    }
  }

  /**
   * Uploads one node copy to the archive store. One upload at a time per copy: the row is claimed
   * by compare-and-set, and a claim nobody finished is taken over once it is stale.
   */
  upload(archiveId: string, waitForStore = false) {
    return waitForStore
      ? this.#transfers.stored(archiveId).then(() => 'ready' as const)
      : this.#transfers.upload(archiveId)
  }

  async goneSnapshots(snapshots: readonly SnapshotHandle[]): Promise<ReadonlySet<SnapshotHandle>> {
    const gone = new Set<SnapshotHandle>()
    for (const snapshot of snapshots) {
      if (!isSnapshot(snapshot)) continue
      const ref = decodeSnapshot(snapshot)
      if (ref.deployment !== this.#o.deployment) continue
      const row = await this.#archives.archiveOf(ref.archive)
      if (row === null || !(await this.#archives.held(row))) gone.add(snapshot)
    }
    return gone
  }

  async deleteSnapshot(snapshot: SnapshotHandle): Promise<void> {
    const ref = decodeSnapshot(snapshot)
    await this.#archives.deleteSnapshot(ref.archive)
  }

  /**
   * A tar.gz of the snapshot where `target` says, in one PUT or in parts: sent by its node while
   * the node holds it, else copied from the store.
   */
  exportSnapshot(
    snapshot: SnapshotHandle,
    target: ArchiveTarget,
  ): Promise<{ sizeBytes: number; sha256: string }> {
    return this.#transfers.exportSnapshot(snapshot, target)
  }

  // ─── restores and moves ────────────────────────────────────────────────────────────────────

  /**
   * A new epoch, always: the data's lineage changes, so copies and backups of the old one are
   * superseded. In place for a placed server; on a newly chosen node for a released one (the data
   * rests in the store) or a displaced one (its host was confirmed lost).
   */
  restore(
    handle: RuntimeHandle,
    from: RestoreSource,
    spec: RuntimeSpec,
    progress: ProgressSink,
  ): Promise<RuntimeHandle> {
    return this.#rehoming.restore(handle, from, spec, progress)
  }

  /**
   * Without `from`, a planned move: stop, export what can't be made again, a new epoch on another
   * node, restore, and the old copy fenced and deleted. It is left stopped; the application starts
   * it where it wants it running. With `from`, the host was lost: only a server whose host an
   * operator confirmed lost is rebuilt, and nothing on that host is touched.
   */
  relocate(
    handle: RuntimeHandle,
    to: Placement,
    spec: RuntimeSpec,
    progress: ProgressSink,
    from?: SnapshotHandle,
  ): Promise<RuntimeHandle> {
    return this.#rehoming.relocate(handle, this.#region(to.regionKey), spec, progress, from)
  }

  // ─── letting go ────────────────────────────────────────────────────────────────────────────

  /** Compute goes; the data stays on the node (`retained`), and so does its disk. */
  async decommission(handle: RuntimeHandle): Promise<void> {
    const { ref, node } = await this.#ledger.current(handle)
    await this.#api.deleteKeepingData(node, ref.key, ref.epoch)
    await this.#admission.stopped(ref.key)
  }

  /** Compute and storage go (the node keeps the data in its trash for a day), and the reservation. */
  async release(handle: RuntimeHandle): Promise<RuntimeHandle> {
    const ref = decodeHandle(handle)
    const row = await placementOf(this.#o.db, ref.key)
    const released = (epoch: number) =>
      encodeHandle({
        ...ref,
        epoch,
        node: null,
        nodeName: null,
        edgeHost: null,
        controlHost: null,
      })
    if (row === null || row.state === 'released')
      return released(row === null ? ref.epoch : Number(row.epoch))
    if (Number(row.epoch) !== ref.epoch)
      throw new StaleHandle(`This handle is for epoch ${ref.epoch}; the server is at ${row.epoch}`)
    if (row.node_id !== null) {
      const node = await getNode(this.#o.db, row.node_id)
      if (node !== null && (node.lifecycle === 'active' || node.lifecycle === 'draining')) {
        await this.#api.deleteWithData(node, ref.key, ref.epoch)
        // The node's copies of its snapshots went with its data; the store's stay.
        await this.#archives.forgetNodeCopies(ref.key, node.id)
      }
    }
    const result = released(ref.epoch)
    await this.#ledger.release(ref.key, ref.epoch, row.node_id, result)
    return result
  }

  /** Everything, on every node that holds a copy, and every copy the fleet keeps in the store. */
  async destroy(target: RuntimeKey | RuntimeHandle): Promise<void> {
    const key = isHandle(target) ? decodeHandle(target).key : target
    const row = await placementOf(this.#o.db, key)
    const holders = new Set<string>(row?.node_id ? [row.node_id] : [])
    const observed = await rows<{ node_id: string; epoch: string | null }>(
      this.#o.db,
      sql`SELECT node_id, epoch FROM fleet_observations WHERE workload = ${key}`,
    )
    for (const o of observed) holders.add(o.node_id)
    const epoch = Math.max(Number(row?.epoch ?? 0), ...observed.map((o) => Number(o.epoch ?? 0)))
    for (const id of holders) {
      const node = await getNode(this.#o.db, id)
      // A lost node can't be asked; if it returns, its copy has no placement and is an orphan.
      if (node === null || node.lifecycle === 'lost' || node.lifecycle === 'retired') continue
      await this.#api.deleteWithData(node, key, epoch)
    }
    await this.#archives.deleteAllStored(key)
    await this.#o.db.transaction(async (t) => {
      await this.#archives.markAllDeleted(t, key)
      if (row !== null) await remove(t, key, Number(row.epoch))
      await event(t, 'placement.destroyed', {
        workload: key,
        data: { nodes: [...holders] },
      })
    })
  }

  // ─── observation ───────────────────────────────────────────────────────────────────────────

  /**
   * The current placement's copy only (the application's `sameCompute` rule, applied here too): a
   * copy left on another node is never reported as the server. `unknown` while its node is silent,
   * `hostLost` only once an operator confirmed the loss.
   */
  observe(handle: RuntimeHandle): Promise<RuntimeObservation> {
    return this.#views.observe(handle)
  }

  observeChanged(since: Date): AsyncIterable<{
    key: RuntimeKey
    handle: RuntimeHandle
    observation: RuntimeObservation
  }> {
    return this.#views.observeChanged(since)
  }

  /**
   * Pure: false while its node drains, which is how draining moves sandboxes (the application's
   * relocation sweep moves what isn't placed); true on a lost node, whose sandboxes the application's
   * host-lost path rebuilds instead. An unknown node counts as placed: nothing moves on missing
   * information.
   */
  isPlaced(handle: RuntimeHandle, placement: Placement): boolean {
    return this.#views.isPlaced(handle, placement)
  }

  sameCompute(a: RuntimeHandle, b: RuntimeHandle): boolean {
    return this.#views.sameCompute(a, b)
  }

  /** Placements, and copies nodes report that no placement accounts for (orphans). */
  inventory(): AsyncIterable<{
    key: RuntimeKey
    handle: RuntimeHandle
  }> {
    return this.#views.inventory()
  }

  /**
   * Its node's address for the audience, and the host port. Synchronous and free of I/O: the
   * address is the registry's as last read (every few seconds), so a node whose address changed
   * keeps its routes; the handle's own is the fallback.
   */
  endpoint(handle: RuntimeHandle, port: string, audience: Audience): Endpoint {
    return this.#views.endpoint(handle, port, audience)
  }

  /** Nodes hold no tags; operators read whose a server is on the admin page. */
  async tag(): Promise<number> {
    return 0
  }

  locate(handle: RuntimeHandle): RuntimeLocation {
    return this.#views.locate(handle)
  }

  /**
   * A dedicated node costs the same whether its sandboxes run or not, so a server's price is its
   * share of the node's month, counted as storage (paid while it holds its place) rather than per
   * running hour. Sleeping sandboxes hold no memory, so a full node holds its memory times the memory
   * overcommit in placed sandboxes: a server's share is its memory's part of that. Null where the
   * operator gave the node no cost.
   */
  prices(handle: RuntimeHandle, size: { memoryMb: number; storageGb: number }): RuntimePrices | null {
    return this.#views.prices(handle, size)
  }

  /**
   * An operator's request that a server leave its node, for `to` if given. It takes effect at the
   * server's next relocation, which the caller asks the application for: the application stops,
   * snapshots and restarts it around the move as it does for every move.
   */
  requestMove(sandboxId: string, to: string | null, by: string): Promise<void> {
    return this.#ledger.requestMove(sandboxId, to, by)
  }

  upkeep(): Promise<void> {
    return this.#maintenance.upkeep()
  }

  probeSilent(): Promise<void> {
    return this.#maintenance.probeSilent()
  }

  finishReported(): Promise<number> {
    return this.#maintenance.finishReported()
  }

  abandonUnmade(): Promise<number> {
    return this.#maintenance.abandonUnmade()
  }

  tidy(): Promise<number> {
    return this.#maintenance.tidy()
  }

  uploadPending(limit = 4): Promise<number> {
    return this.#maintenance.uploadPending(limit)
  }

  dropDeletedCopies(limit = 20): Promise<void> {
    return this.#maintenance.dropDeletedCopies(limit)
  }

  trimLocalCopies(limit = 20): Promise<number> {
    return this.#maintenance.trimLocalCopies(limit)
  }

  // ─── internals ─────────────────────────────────────────────────────────────────────────────
}
