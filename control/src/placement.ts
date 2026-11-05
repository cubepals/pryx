// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Choosing a node for a workload (docs/fleet.md, "Placement"): filter, then score. Pure: it
 * reads a snapshot of the nodes and of the reservation ledger and returns a decision with every
 * node's reason, so a refusal can say why and a decision can be replayed from its inputs.
 */

import type { Health } from './health.ts'
import { nodeIsolation } from './isolation.ts'

type Policy = 'balanced' | 'binpack' | 'spread'
export type Lifecycle = 'active' | 'draining' | 'lost' | 'retired'

export interface PlacementRequest {
  isolation?: import('./isolation.ts').Isolation
  memoryMb: number
  cpuMillis: number
  diskGb: number
  /** Host ports the workload needs. */
  ports: number
  regionKey: string
  /** Protocol features the node must have (`placement-epochs`, `data-transfer`). */
  needs: readonly string[]
  /** Nodes that must not be chosen: a move's source. */
  avoid?: readonly string[]
  /** The one node that may be chosen: an operator's move to a node they named. */
  only?: string
}

export interface NodeView {
  id: string
  name: string
  regionKey: string
  lifecycle: Lifecycle
  health: Health
  /** Two hosts have beaten with this node's identity: nothing new goes there. */
  quarantined: boolean
  features: readonly string[]
  /** As the node last reported it. */
  capacity: {
    allocatableMemoryMb: number
    cpus: number
    reservedCpuMillis: number
    diskTotalBytes: number | null
    diskAvailableBytes: number | null
    minFreeDiskMb: number
    portsTotal: number
    portsAllocated: number
  }
  /**
   * What the ledger has promised on the node, never the node's report. `allocated` is every
   * placement on it, running or not: its disk keeps their data sets. `running` is the placements that
   * run, or are being started or made: only they use memory and CPU (docs/fleet.md, "Admission").
   */
  allocated: {
    memoryMb: number
    cpuMillis: number
    diskGb: number
    workloads: number
  }
  running: { memoryMb: number; cpuMillis: number; workloads: number }
}

export interface PlacementOptions {
  policy: Policy
  /** Memory kept free on every node for a restore or a move to land. */
  headroomMb: number
  /**
   * How many times its memory the sandboxes placed on a node may add up to. Only the running ones
   * must fit in it, so a sleeping server holds none; 1 holds memory for every placed server.
   */
  memoryOvercommit: number
  /** Allocatable CPU is multiplied by this before requests are counted against it. */
  cpuOvercommit: number
  /** Share of a node's CPU above which `balanced` prefers other nodes. */
  cpuPressure: number
  /** How much more disk than a node has may be promised to the data sets on it. */
  diskOvercommit: number
}

export const DEFAULTS: PlacementOptions = {
  policy: 'balanced',
  headroomMb: 0,
  // A server is asleep most of the time, so few of a node's sandboxes run at once.
  memoryOvercommit: 4,
  cpuOvercommit: 1,
  cpuPressure: 0.8,
  diskOvercommit: 1,
}

export interface Considered {
  node: string
  name: string
  /** `ok`, or why the node was passed over. */
  reason: string
}

export type Decision = { node: NodeView; considered: Considered[] } | { node: null; considered: Considered[] }

const MIB = 1024 * 1024

/** The CPU a node can promise, in thousandths of a core, overcommit included. */
function cpuAllocatable(node: NodeView, overcommit: number): number {
  return Math.max(0, node.capacity.cpus * 1000 - node.capacity.reservedCpuMillis) * overcommit
}

/**
 * Why `node` can't take `request`, or null when it can. The first failing filter wins. A placement
 * starts where it lands, so it needs room to run now as well as room to stay.
 */
function refusal(node: NodeView, request: PlacementRequest, options: PlacementOptions): string | null {
  if (request.avoid?.includes(node.id)) return 'excluded'
  if (request.only !== undefined && request.only !== node.id) return 'not the node asked for'
  if (node.lifecycle !== 'active') return `lifecycle ${node.lifecycle}`
  if (node.quarantined) return 'quarantined: duplicate identity'
  if (node.health !== 'healthy') return `health ${node.health}`
  if (node.regionKey !== request.regionKey) return `region ${node.regionKey}`
  if ((request.isolation ?? 'container') !== nodeIsolation(node.features))
    return 'isolation tier does not match'
  const missing = request.needs.filter((f) => !node.features.includes(f))
  if (missing.length > 0) return `lacks ${missing.join(', ')}`
  const memoryFree = node.capacity.allocatableMemoryMb - node.running.memoryMb - options.headroomMb
  if (memoryFree < request.memoryMb)
    return `memory: ${memoryFree} MB free after headroom, needs ${request.memoryMb}`
  const placeable = placeableMemoryMb(node, options) - node.allocated.memoryMb
  if (placeable < request.memoryMb)
    return `memory: ${placeable} MB left to place sandboxes in, needs ${request.memoryMb}`
  const cpuFree = cpuAllocatable(node, options.cpuOvercommit) - node.running.cpuMillis
  if (cpuFree < request.cpuMillis) return `cpu: ${Math.floor(cpuFree)}m free, needs ${request.cpuMillis}m`
  if (node.capacity.diskAvailableBytes !== null) {
    const diskFreeMb = node.capacity.diskAvailableBytes / MIB - node.capacity.minFreeDiskMb
    if (diskFreeMb < request.diskGb * 1024)
      return `disk: ${Math.floor(diskFreeMb)} MB free above the floor, needs ${request.diskGb * 1024}`
  }
  // What data sets may grow into, not what they hold today: the sum of what was promised fits the disk.
  if (node.capacity.diskTotalBytes !== null) {
    const promisableGb =
      ((node.capacity.diskTotalBytes / MIB - node.capacity.minFreeDiskMb) / 1024) * options.diskOvercommit
    const unpromisedGb = promisableGb - node.allocated.diskGb
    if (unpromisedGb < request.diskGb)
      return `disk: ${Math.floor(unpromisedGb)} GB not yet promised, needs ${request.diskGb}`
  }
  const portsFree = node.capacity.portsTotal - node.capacity.portsAllocated
  if (portsFree < request.ports) return `ports: ${portsFree} free, needs ${request.ports}`
  return null
}

/** The memory the sandboxes placed on a node may add up to, running or not. */
function placeableMemoryMb(node: NodeView, options: Pick<PlacementOptions, 'memoryOvercommit'>): number {
  return node.capacity.allocatableMemoryMb * options.memoryOvercommit
}

/**
 * Why a server placed on `node` can't start there now, or null when it can: the sandboxes running
 * there leave too little memory or CPU for it. Nothing lands, so no headroom is kept for it, and
 * its data is there already. `held`: it holds its share already, as a retried start does.
 */
export function startRefusal(
  node: NodeView,
  request: Pick<PlacementRequest, 'memoryMb' | 'cpuMillis'>,
  options: Pick<PlacementOptions, 'cpuOvercommit'>,
  held = false,
): string | null {
  if (held) return null
  const memoryFree = node.capacity.allocatableMemoryMb - node.running.memoryMb
  if (memoryFree < request.memoryMb)
    return `memory: ${memoryFree} MB free beside the sandboxes running there, needs ${request.memoryMb}`
  const cpuFree = cpuAllocatable(node, options.cpuOvercommit) - node.running.cpuMillis
  if (cpuFree < request.cpuMillis) return `cpu: ${Math.floor(cpuFree)}m free, needs ${request.cpuMillis}m`
  return null
}

/** How a node that can take a placement would be left by it: what it scores on. */
interface Fit {
  node: NodeView
  /** Memory left to place sandboxes in after placing this one, before headroom. */
  leftoverMb: number
  /** Share of the CPU the sandboxes placed there may be counted for, after placing this one. */
  cpuShare: number
}

/**
 * Scored on what is placed, running or not, against what may be placed: a placement lasts as long
 * as its data stays, while what runs changes by the hour. With a memory overcommit of 1 that is
 * the node's own memory and CPU.
 */
function fit(node: NodeView, request: PlacementRequest, options: PlacementOptions): Fit {
  const cpu = cpuAllocatable(node, options.cpuOvercommit) * options.memoryOvercommit
  return {
    node,
    leftoverMb: placeableMemoryMb(node, options) - node.allocated.memoryMb - request.memoryMb,
    cpuShare: cpu === 0 ? 1 : (node.allocated.cpuMillis + request.cpuMillis) / cpu,
  }
}

/** Negative when `a` is the better home. Ties fall to the node id, so decisions are reproducible. */
function compare(a: Fit, b: Fit, options: PlacementOptions): number {
  const byId = a.node.id < b.node.id ? -1 : a.node.id > b.node.id ? 1 : 0
  switch (options.policy) {
    case 'binpack':
      return a.leftoverMb - b.leftoverMb || byId
    case 'spread':
      return b.leftoverMb - a.leftoverMb || byId
    case 'balanced': {
      // Best fit on memory among nodes under the CPU pressure line; past it, the least CPU-pressed.
      const aUnder = a.cpuShare <= options.cpuPressure
      const bUnder = b.cpuShare <= options.cpuPressure
      if (aUnder !== bUnder) return aUnder ? -1 : 1
      if (!aUnder) return a.cpuShare - b.cpuShare || a.leftoverMb - b.leftoverMb || byId
      return a.leftoverMb - b.leftoverMb || a.cpuShare - b.cpuShare || byId
    }
  }
}

export function selectNode(
  request: PlacementRequest,
  nodes: readonly NodeView[],
  options: PlacementOptions = DEFAULTS,
): Decision {
  const considered: Considered[] = []
  const fits: Fit[] = []
  for (const node of nodes) {
    const why = refusal(node, request, options)
    considered.push({ node: node.id, name: node.name, reason: why ?? 'ok' })
    if (why === null) fits.push(fit(node, request, options))
  }
  fits.sort((a, b) => compare(a, b, options))
  const best = fits[0]
  return best === undefined ? { node: null, considered } : { node: best.node, considered }
}

/** CPU a workload is counted for when its spec names none: one core per `perGbMillis` × GB. */
export function cpuRequest(memoryMb: number, perGbMillis: number): number {
  return Math.ceil((memoryMb / 1024) * perGbMillis)
}
