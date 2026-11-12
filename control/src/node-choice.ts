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

import agentContract from '../../agent/openapi.json'
import type { Queryable } from './database.ts'
import { type Guarantee, guarantees } from './guarantees.ts'
import type { Thresholds } from './health.ts'
import { type Isolation, nodeFeature, nodeIsolations } from './isolation.ts'
import {
  type Considered,
  type Decision,
  type NodeView,
  type PlacementOptions,
  type PlacementRequest,
  reservedCpu,
  selectNode,
  startRefusal,
} from './placement.ts'
import { holdsMemory, nodeSummaries, toNodeView } from './registry.ts'
import { RuntimeFull, type RuntimeSpec } from './runtime-model.ts'
import { workloadFeatures } from './workload-capabilities.ts'

/** What a node must support to hold a fleet workload. */
export const NEEDS = ['placement-epochs', 'data-transfer', 'local-snapshots', 'export-exclude']

export const describe = (considered: readonly Considered[]) =>
  considered.length === 0 ? 'no nodes' : considered.map((c) => `${c.name}: ${c.reason}`).join('; ')

export const missingGuarantees = (considered: readonly Considered[]): Guarantee[] => [
  ...new Set(considered.flatMap((node) => node.missingGuarantees ?? [])),
]

export const placementRefused = (
  provider: string,
  considered: readonly Considered[],
  isolation?: Isolation,
  requiredTools: readonly import('./tool-capabilities.ts').RequiredTool[] = [],
) => new RuntimeFull(provider, describe(considered), missingGuarantees(considered), isolation, requiredTools)

export class NodeChoice {
  readonly #o: {
    regionMap?: Readonly<Record<string, string>>
    cpuMillisPerGb: number
    thresholds: Thresholds
    placement: PlacementOptions
  }

  constructor(options: {
    regionMap?: Readonly<Record<string, string>>
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
      require: spec.require,
      isolation: spec.isolation,
      memoryMb: spec.resources.memoryMb,
      memoryLimit:
        (spec.isolation ?? 'container') === 'container' &&
        (spec.resources.allowUnenforcedMemory === false ||
          (spec.resources.allowUnenforcedMemory !== true && spec.resourceDefaults?.memory === false)),
      cpuMillis: reservedCpu(spec.resources, this.#o.cpuMillisPerGb),
      cpuLimits:
        spec.resources.cpuWeight != null || (spec.resources.cpuMillis != null && !spec.resourceDefaults?.cpu),
      diskGb: spec.storage.sizeGb,
      ports: spec.ports.length + (spec.agent ? 1 + (spec.sparePorts ?? 0) : 0),
      regionKey: region,
      needs: [
        ...NEEDS,
        ...(spec.fuseMounts ? [agentContract['x-pryx-tools'].fuseFeature] : []),
        ...workloadFeatures(spec),
        ...(spec.isolation === 'wasm' ? ['wasm-packages'] : []),
        ...(spec.agent || (spec.sparePorts ?? 0) > 0 ? ['agent'] : []),
      ],
      avoid,
      ...(only === null ? {} : { only }),
    }
  }

  /** Choose only for a request that left its tier open; capacity is still claimed once later. */
  async firstIsolation(
    q: Queryable,
    order: readonly Isolation[],
    required: readonly Guarantee[],
    region?: string,
    only?: string,
    needs: readonly string[] = [],
  ): Promise<Isolation | undefined> {
    const nodes = await this.#views(q)
    const mapped = region === undefined ? undefined : (this.#o.regionMap?.[region] ?? region)
    return order.find((tier) =>
      nodes.some(
        (node) =>
          node.health === 'healthy' &&
          node.lifecycle === 'active' &&
          !node.quarantined &&
          (mapped === undefined || node.regionKey === mapped) &&
          (only === undefined || node.id === only) &&
          nodeIsolations(node.features).includes(tier) &&
          required.every((name) => guarantees(node.features, tier)[name]) &&
          needs.every((feature) => nodeFeature(node.features, feature, tier)),
      ),
    )
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
