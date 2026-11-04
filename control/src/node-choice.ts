// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The question the fleet runtime asks `../placement.ts`: what a server needs of a node, judged
 * against every node as the ledger has it now, and a refusal put in words. It reads node summaries
 * and writes nothing.
 *
 * Claiming the room it finds is `admission.ts`'s; the rules that judge a node are placement's own.
 */

import type { Queryable } from './database.ts'
import type { Thresholds } from './health.ts'
import {
  type Considered,
  cpuRequest,
  type Decision,
  type NodeView,
  type PlacementOptions,
  type PlacementRequest,
  selectNode,
  startRefusal,
} from './placement.ts'
import { holdsMemory, nodeSummaries, toNodeView } from './registry.ts'
import type { RuntimeSpec } from './runtime-model.ts'

/** What a node must support to hold a fleet workload. */
export const NEEDS = ['placement-epochs', 'data-transfer', 'local-snapshots', 'export-exclude']

export const describe = (considered: readonly Considered[]) =>
  considered.length === 0 ? 'no nodes' : considered.map((c) => `${c.name}: ${c.reason}`).join('; ')

export class NodeChoice {
  readonly #o: {
    cpuMillisPerGb: number
    thresholds: Thresholds
    placement: PlacementOptions
  }

  constructor(options: {
    cpuMillisPerGb: number
    thresholds: Thresholds
    placement: PlacementOptions
  }) {
    this.#o = options
  }

  request(
    spec: RuntimeSpec,
    region: string,
    avoid: string[] = [],
    only: string | null = null,
  ): PlacementRequest {
    return {
      memoryMb: spec.resources.memoryMb,
      cpuMillis: cpuRequest(spec.resources.memoryMb, this.#o.cpuMillisPerGb),
      diskGb: spec.storage.sizeGb,
      ports: spec.ports.length,
      regionKey: region,
      needs: NEEDS,
      avoid,
      ...(only === null ? {} : { only }),
    }
  }

  /** The node placement would choose now, read in `q` (the caller's transaction, where it has one). */
  async choose(q: Queryable, request: PlacementRequest): Promise<Decision> {
    return selectNode(request, await this.#views(q), this.#o.placement)
  }

  /** Why the server's node has no room to run it now; null when it has. */
  async refusalToStart(
    q: Queryable,
    row: { workload: string; node_id: string | null },
    request: Pick<PlacementRequest, 'memoryMb' | 'cpuMillis'>,
  ): Promise<string | null> {
    const node = (await this.#views(q)).find((n) => n.id === row.node_id)
    if (node === undefined) return 'its node has left the fleet'
    return startRefusal(node, request, this.#o.placement, await holdsMemory(q, row.workload))
  }

  async #views(q: Queryable): Promise<NodeView[]> {
    return (await nodeSummaries(q, this.#o.thresholds)).map(toNodeView)
  }
}
