// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Power on a placed copy (docs/waking.md, "On the fleet"): its wake, resumed where it was
 * suspended or else booted, with its spec sent only when it changed since the last one sent to the
 * placement; its sleep, suspended where the deployment and the node allow it, else stopped; its
 * stop; and the wait for it to run.
 *
 * Claiming room to run is `admission.ts`'s, and a start that finds none moves the server, which is
 * the runtime's to ask `rehoming.ts` for.
 */

import { createHash } from 'node:crypto'
import { setTimeout as pause } from 'node:timers/promises'
import { sql } from 'drizzle-orm'
import nodeContract from '../../openapi/node-writes.openapi.json'
import { type Admission, NO_ROOM } from './admission.ts'
import type { Db } from './database.ts'
import { SECOND_MS } from './durations.ts'
import { decodeHandle, type FleetRef } from './handle.ts'
import { type Ledger, type PlacementRow, StaleHandle } from './ledger.ts'
import { NodeRefused, NodeUnreachable } from './node-client.ts'
import type { NodeRow } from './registry.ts'
import { type ProgressSink, RuntimeFull, type RuntimeHandle, type RuntimeSpec } from './runtime-model.ts'
import { one } from './sql.ts'
import type { EnsureResponse, WorkloadView } from './wire.ts'
import type { WorkloadApi } from './workload-api.ts'
import { requireWorkloadFeatures } from './workload-capabilities.ts'

/** Node states in which a workload waited on to run never will without being asked again. */
const NOT_STARTING = new Set(['stopped', 'crashed', 'missing', 'retained', 'fenced', 'suspended'])

/** A start's refusals that a PUT answers: the node lost the copy's container, or holds no compute for it. */
const PUT_AGAIN = new Set(['not_found', 'not_created', 'container_missing', 'no_compute'])

/** The node's feature for `POST …/suspend`, `resumesOn` and a start's `resumed` (pryxd's docs/protocol.md). */
const SUSPEND_FEATURE = 'suspend'

const POLL_PAUSE_MS = 0.5 * SECOND_MS

/** Blockly's digest of a spec as sent, secrets included, which the node's own leaves out. */
const sentDigest = (spec: RuntimeSpec) => createHash('sha256').update(JSON.stringify(spec)).digest('hex')

const notInTime = () => new Error('The workload did not start in time')

/** `work`, or the signal's end, whichever comes first: nothing waited on outlasts what is left of it. */
function within<T>(signal: AbortSignal, work: Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(notInTime())
    if (signal.aborted) return abort()
    signal.addEventListener('abort', abort, { once: true })
    work.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort))
  })
}

/** Whether the copy runs: it throws where the copy is another placement's, or won't start unasked. */
function runs(view: WorkloadView, epoch: number): boolean {
  if (view.epoch !== epoch) throw new StaleHandle('The copy on the node belongs to another placement')
  if (view.state === 'running') return true
  if (!NOT_STARTING.has(view.state)) return false
  const exit = view.exit ? ` (exit ${view.exit.code}${view.exit.oomKilled ? ', out of memory' : ''})` : ''
  throw new Error(`The workload is ${view.state}${exit}`)
}

export class Power {
  readonly #db: Db
  readonly #api: Pick<WorkloadApi, 'put' | 'power' | 'view'>
  readonly #admission: Pick<Admission, 'put' | 'power'>
  readonly #ledger: Pick<Ledger, 'settle' | 'booted' | 'handleOf'>
  readonly #suspend: boolean
  readonly #provider: string
  readonly #log: (message: string, fields?: object) => void
  /** Copies whose own start answered `running`, by key, with the epoch it was for. */
  readonly #running = new Map<string, number>()

  constructor(options: {
    db: Db
    api: Pick<WorkloadApi, 'put' | 'power' | 'view'>
    admission: Pick<Admission, 'put' | 'power'>
    ledger: Pick<Ledger, 'settle' | 'booted' | 'handleOf'>
    /** The deployment's switch: whether a sleep suspends where the node can. */
    suspend: boolean
    provider: string
    log: (message: string, fields?: object) => void
  }) {
    this.#db = options.db
    this.#api = options.api
    this.#admission = options.admission
    this.#ledger = options.ledger
    this.#suspend = options.suspend
    this.#provider = options.provider
    this.#log = options.log
  }

  /**
   * Brings up a placed copy, its room to run claimed, and starts it. One made just now (`created`)
   * gets its spec and boots. One that exists is woken: its spec is sent only where it changed since
   * the last one sent to this placement, and a suspended copy resumes (`resuming`), going on as a
   * boot (`booting`) where the node's resume didn't come back. A start refused for room throws the
   * node's refusal, for the caller to move the server.
   */
  async wake(
    row: PlacementRow,
    node: NodeRow,
    spec: RuntimeSpec,
    progress: ProgressSink,
    created: boolean,
    shouldStart = true,
  ): Promise<RuntimeHandle> {
    requireWorkloadFeatures(spec, node.features)
    const key = row.workload
    const epoch = Number(row.epoch)
    const digest = sentDigest(spec)
    const unchanged = !created && row.state === 'placed' && row.sent_digest === digest
    const sent = unchanged ? null : await this.#send(row, node, spec, digest, created, progress)
    const handle =
      sent?.handle ??
      this.#ledger.handleOf({
        ...row,
        node_name: node.name,
        edge_host: node.edge_host,
        control_host: node.control_host,
      })
    const asleep = sent === null ? await this.#suspended(row) : sent.asleep
    await progress.handle(handle)
    if (!shouldStart) {
      await this.halt(decodeHandle(handle), node, 'stop')
      return handle
    }
    await progress.step(asleep ? 'resuming' : 'booting')
    const started = await this.#api.power(node, key, epoch, 'start', !created).catch((error) => {
      if (sent === null && error instanceof NodeRefused && PUT_AGAIN.has(error.code)) return null
      throw error
    })
    // The node lost what the ledger says it holds: the spec again, then the start.
    if (started === null) return this.wake({ ...row, sent_digest: null }, node, spec, progress, false)
    if (!asleep && started.resumed === true) await progress.step('resuming')
    else if (asleep && started.resumed === false) await progress.step('booting')
    const known = sent === null ? row.spec_digest : sent.nodeDigest
    if (started.resumed !== true && started.workload.specDigest !== known)
      await this.#ledger.booted(key, epoch, started.workload.specDigest)
    if (started.workload.state === 'running') this.#running.set(key, epoch)
    return handle
  }

  /** A changed spec to a placed copy, replacing only the one the ledger last knew it to run. */
  async apply(row: PlacementRow, node: NodeRow, spec: RuntimeSpec): Promise<RuntimeHandle> {
    const epoch = Number(row.epoch)
    const ensured = await this.#put(node, row.workload, spec, epoch, {
      ifMatch: row.spec_digest,
    })
    // A running workload whose spec changed is restarted by the node as it replaces it.
    return this.#ledger.settle(row.workload, node, epoch, ensured.workload, {
      digest: sentDigest(spec),
      deferred: ensured.outcome === 'deferred',
    })
  }

  /**
   * Suspended, where the deployment's switch is on and the node lists the feature: its memory kept,
   * and its room to run still claimed, since the node counts a suspended copy as a running one.
   * Otherwise, or where the node won't suspend it (one not running), stopped. Says which.
   */
  async sleep(ref: FleetRef, node: NodeRow): Promise<'suspended' | 'stopped'> {
    this.#running.delete(ref.key)
    if (this.#suspend && node.features.includes(SUSPEND_FEATURE) && (await this.#keepsMemory(ref, node))) {
      try {
        await this.#api.power(node, ref.key, ref.epoch, 'suspend')
        return 'suspended'
      } catch (error) {
        if (!(error instanceof NodeRefused)) throw error
        if (error.code !== 'not_running')
          this.#log('suspend refused, stopping instead', {
            workload: ref.key,
            code: error.code,
          })
      }
    }
    await this.halt(ref, node, 'stop')
    return 'stopped'
  }

  /** New nodes report the copy's own tier guarantee; legacy nodes retain their existing contract. */
  async #keepsMemory(ref: FleetRef, node: NodeRow): Promise<boolean> {
    if (!node.features.includes(nodeContract['x-pryx-resources'].unenforcedMemoryFeature)) return true
    const view = await this.#api.view(node, ref.key, true)
    if (view.epoch !== ref.epoch) throw new StaleHandle('The copy on the node belongs to another placement')
    return view.guarantees?.['memory-sleep'] === true
  }

  /** A stop, or a kill, which gives the copy's claim on its node back. */
  async halt(ref: FleetRef, node: NodeRow, verb: 'stop' | 'kill'): Promise<void> {
    this.#running.delete(ref.key)
    await this.#admission.power(node, ref.key, ref.epoch, verb)
  }

  /**
   * Over at once where the copy's own start answered `running`. Otherwise its node is asked until
   * the copy runs, or is in a state it won't leave unasked, or the signal ends the wait: each read
   * is bounded by what is left of it, and one the node doesn't answer is asked again.
   */
  async waitRunning(ref: FleetRef, node: NodeRow, signal: AbortSignal): Promise<void> {
    const answered = this.#running.get(ref.key)
    this.#running.delete(ref.key)
    if (answered === ref.epoch) return
    for (;;) {
      const view = await this.#read(node, ref.key, signal)
      if (view !== null && runs(view, ref.epoch)) return
      await within(signal, pause(POLL_PAUSE_MS))
    }
  }

  /** The copy as its node sees it, or null where the node didn't answer and there is time left. */
  #read(node: NodeRow, key: string, signal: AbortSignal): Promise<WorkloadView | null> {
    return within(signal, this.#api.view(node, key, true)).catch((error) => {
      if (error instanceof NodeUnreachable && !signal.aborted) return null
      throw error
    })
  }

  /** The spec to the node, and the placement settled on the node's answer. */
  async #send(
    row: PlacementRow,
    node: NodeRow,
    spec: RuntimeSpec,
    digest: string,
    created: boolean,
    progress: ProgressSink,
  ): Promise<{
    handle: RuntimeHandle
    asleep: boolean
    nodeDigest: string | null
  }> {
    const epoch = Number(row.epoch)
    await progress.step('compute')
    // Only a placed copy's PUT is a wake's: one a crash left `placing` may never have pulled its image.
    const ensured = created
      ? await this.#admission.put(node, row.workload, spec, epoch, true)
      : await this.#put(node, row.workload, spec, epoch, {
          wake: row.state === 'placed',
        })
    const deferred = ensured.outcome === 'deferred'
    const handle = await this.#ledger.settle(row.workload, node, epoch, ensured.workload, {
      digest,
      deferred,
    })
    return {
      handle,
      asleep: ensured.workload.state === 'suspended',
      nodeDigest: deferred ? null : ensured.workload.specDigest,
    }
  }

  /** The spec to a copy that exists: a refusal for room is RuntimeFull, as `Admission.put` makes it. */
  async #put(
    node: NodeRow,
    key: string,
    spec: RuntimeSpec,
    epoch: number,
    options: { wake?: boolean; ifMatch?: string | null },
  ): Promise<EnsureResponse> {
    try {
      return await this.#api.put(node, key, spec, epoch, options)
    } catch (error) {
      if (error instanceof NodeRefused && NO_ROOM.has(error.code))
        throw new RuntimeFull(this.#provider, `${node.name}: ${error.message}`)
      throw error
    }
  }

  /** Whether the node last reported the placement's copy suspended, which it does as soon as it is. */
  async #suspended(row: PlacementRow): Promise<boolean> {
    const observed = await one<{ state: string }>(
      this.#db,
      sql`SELECT state FROM fleet_observations
          WHERE node_id = ${row.node_id} AND workload = ${row.workload} AND epoch = ${row.epoch}`,
    )
    return observed?.state === 'suspended'
  }
}
