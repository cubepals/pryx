// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Fleet observations, endpoints and capacity; placement and power remain in the engine. */

import { sql } from 'drizzle-orm'
import type { FleetStore } from './archive-transport.ts'
import type { Db } from './database.ts'
import { decodeHandle, encodeHandle } from './handle.ts'
import type { Thresholds } from './health.ts'
import { type Ledger, OBSERVE_SQL, type ObservedRow } from './ledger.ts'
import type { NodeCache } from './node-cache.ts'
import type { NodeClient } from './node-client.ts'
import { observation as observeReport } from './observation.ts'
import type { PlacementOptions } from './placement.ts'
import { listeningSeconds, nodeSummaries } from './registry.ts'
import type {
  Audience,
  Endpoint,
  Placement,
  RuntimeCapacity,
  RuntimeHandle,
  RuntimeKey,
  RuntimeLocation,
  RuntimeMachine,
  RuntimeObservation,
  RuntimePrices,
} from './runtime-model.ts'
import { one, rows } from './sql.ts'
import type { WorkloadReport } from './wire.ts'

interface ViewOptions {
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
  suspend?: boolean
  log?: (message: string, fields?: object) => void
}

export class RuntimeViews {
  readonly #o: ViewOptions
  readonly #known: NodeCache
  readonly #ledger: Ledger
  constructor(options: ViewOptions, known: NodeCache, ledger: Ledger) {
    this.#o = options
    this.#known = known
    this.#ledger = ledger
  }
  #region(regionKey: string): string {
    return this.#o.regionMap[regionKey] ?? regionKey
  }

  /**
   * Its nodes, which are paid for full or empty: their price from the `monthly_cost_cents` label,
   * what the ledger holds for the sandboxes running on each and has placed on each, and what each
   * reported in use at its last heartbeat. A lost node is still listed until it is retired, since
   * it is billed until it is cancelled.
   */
  async capacity(): Promise<RuntimeCapacity> {
    const nodes = await nodeSummaries(this.#o.db, this.#o.thresholds)
    return {
      machines: nodes.map((node): RuntimeMachine => {
        const cents = Number(node.labels.monthly_cost_cents)
        const c = node.capacity
        return {
          name: node.name,
          region: node.region_key,
          state: node.lifecycle,
          monthlyCents: Number.isFinite(cents) && cents > 0 ? cents : null,
          allocatableMemoryMb: c.allocatableMemoryMb ?? 0,
          allocatedMemoryMb: node.running.memoryMb,
          placedMemoryMb: node.allocated.memoryMb,
          usedMemoryMb:
            c.usedMemoryBytes === null || c.usedMemoryBytes === undefined
              ? null
              : Math.round(c.usedMemoryBytes / 1_048_576),
          cpus: c.cpus ?? 0,
          allocatedCpuMillis: node.running.cpuMillis,
          usedCpuCores: c.usedCpuCores ?? null,
          sandboxes: node.allocated.workloads,
        }
      }),
    }
  }

  // ─── observation ───────────────────────────────────────────────────────────────────────────

  /**
   * The current placement's copy only (the application's `sameCompute` rule, applied here too): a
   * copy left on another node is never reported as the server. `unknown` while its node is silent,
   * `hostLost` only once an operator confirmed the loss.
   */
  async observe(handle: RuntimeHandle): Promise<RuntimeObservation> {
    const ref = decodeHandle(handle)
    const row = await one<ObservedRow>(this.#o.db, sql`${OBSERVE_SQL} WHERE p.workload = ${ref.key}`)
    return observeReport(row, ref, await listeningSeconds(this.#o.db), this.#o.thresholds)
  }

  async *observeChanged(since: Date): AsyncIterable<{
    key: RuntimeKey
    handle: RuntimeHandle
    observation: RuntimeObservation
  }> {
    // Reports land up to a heartbeat late, and health changes with time alone.
    const after = since.getTime() - 60_000
    const listening = await listeningSeconds(this.#o.db)
    const found = await rows<ObservedRow>(this.#o.db, sql`${OBSERVE_SQL} WHERE p.state <> 'released'`)
    for (const row of found) {
      const observation = observeReport(row, null, listening, this.#o.thresholds)
      if (observation.at.getTime() < after) continue
      yield {
        key: row.workload as RuntimeKey,
        handle: this.#ledger.handleOf(row),
        observation,
      }
    }
  }

  /**
   * Pure: false while its node drains, which is how draining moves sandboxes (the application's
   * relocation sweep moves what isn't placed); true on a lost node, whose sandboxes the application's
   * host-lost path rebuilds instead. An unknown node counts as placed: nothing moves on missing
   * information.
   */
  isPlaced(handle: RuntimeHandle, placement: Placement): boolean {
    const ref = decodeHandle(handle)
    if (ref.node === null) return true
    if (ref.region !== this.#region(placement.regionKey)) return false
    const node = this.#known.get(ref.node)
    return node === undefined || node.lifecycle === 'active' || node.lifecycle === 'lost'
  }

  sameCompute(a: RuntimeHandle, b: RuntimeHandle): boolean {
    const x = decodeHandle(a)
    const y = decodeHandle(b)
    return x.key === y.key && x.node === y.node && x.epoch === y.epoch
  }

  /** Placements, and copies nodes report that no placement accounts for (orphans). */
  async *inventory(): AsyncIterable<{
    key: RuntimeKey
    handle: RuntimeHandle
  }> {
    const placed = await rows<ObservedRow>(this.#o.db, sql`${OBSERVE_SQL} WHERE p.state <> 'released'`)
    for (const row of placed)
      yield {
        key: row.workload as RuntimeKey,
        handle: this.#ledger.handleOf(row),
      }
    const orphans = await rows<{
      workload: string
      epoch: string | null
      report: WorkloadReport
      node_id: string
      name: string
      region_key: string
      edge_host: string
      control_host: string
    }>(
      this.#o.db,
      sql`SELECT o.workload, o.epoch, o.report, n.id AS node_id, n.name, n.region_key, n.edge_host, n.control_host
          FROM fleet_observations o JOIN fleet_nodes n ON n.id = o.node_id
          LEFT JOIN fleet_placements p ON p.workload = o.workload WHERE p.workload IS NULL`,
    )
    for (const o of orphans)
      yield {
        key: o.workload as RuntimeKey,
        handle: encodeHandle({
          deployment: this.#o.deployment,
          key: o.workload,
          node: o.node_id,
          nodeName: o.name,
          epoch: Number(o.epoch ?? 0),
          region: o.region_key,
          edgeHost: o.edge_host,
          controlHost: o.control_host,
          ports: o.report.ports,
        }),
      }
  }

  /**
   * Its node's address for the audience, and the host port. Synchronous and free of I/O: the
   * address is the registry's as last read (every few seconds), so a node whose address changed
   * keeps its routes; the handle's own is the fallback.
   */
  endpoint(handle: RuntimeHandle, port: string, audience: Audience): Endpoint {
    const ref = decodeHandle(handle)
    const hostPort = ref.ports[port]
    if (hostPort === undefined) throw new Error(`This runtime exposes no port named ${port}`)
    const node = ref.node === null ? undefined : this.#known.get(ref.node)
    const host =
      audience === 'edge' ? (node?.edgeHost ?? ref.edgeHost) : (node?.controlHost ?? ref.controlHost)
    // A released server's route is asleep: its endpoint is never dialled, and can't be.
    return { host: host ?? 'unplaced.invalid', port: hostPort }
  }

  locate(handle: RuntimeHandle): RuntimeLocation {
    const ref = decodeHandle(handle)
    return {
      names: [
        { label: 'Node', value: ref.nodeName ?? '(resting)' },
        { label: 'Region', value: ref.region },
        { label: 'Epoch', value: String(ref.epoch) },
        { label: 'Container', value: `blockly-${ref.deployment}-${ref.key}` },
      ],
      link: null,
    }
  }

  /**
   * A dedicated node costs the same whether its sandboxes run or not, so a server's price is its
   * share of the node's month, counted as storage (paid while it holds its place) rather than per
   * running hour. Sleeping sandboxes hold no memory, so a full node holds its memory times the memory
   * overcommit in placed sandboxes: a server's share is its memory's part of that. Null where the
   * operator gave the node no cost.
   */
  prices(handle: RuntimeHandle, size: { memoryMb: number; storageGb: number }): RuntimePrices | null {
    const ref = decodeHandle(handle)
    const node = ref.node === null ? undefined : this.#known.get(ref.node)
    if (node === undefined || node.monthlyCents === null || node.allocatableMemoryMb <= 0) return null
    const placeable = node.allocatableMemoryMb * this.#o.placement.memoryOvercommit
    return {
      runningHourCents: 0,
      storageMonthCents: Math.round((node.monthlyCents * size.memoryMb) / placeable),
    }
  }
}
