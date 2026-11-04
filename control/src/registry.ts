// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The node registry (docs/fleet.md, "Nodes"): enrollment tokens, enrollment, heartbeats,
 * certificate renewal, and the lifecycle an operator drives. Every write is one transaction, and
 * every time is the database's.
 *
 * Parts (`registry/`):
 * - `node.ts`: a node's row as read, its health, and what the other parts share.
 * - `presence.ts`: whether the node endpoint is up, so silence counts against a node only then.
 * - `enrollment.ts`: tokens, and how a machine enrolls or enrolls again as a node.
 * - `heartbeat.ts`: a heartbeat checked, applied, and answered with fences.
 * - `renewal.ts`: new certificates for an enrolled node.
 * - `summaries.ts`: nodes with their reservations, for placement and operators.
 * - `lifecycle.ts`: an operator's changes to a node, from draining to retiring.
 * - `upgrades.ts`: the pryxd rollout, one node per region at a time, answered in heartbeats.
 */

export { createToken, enroll } from './registry/enrollment.ts'
export { heartbeat, sessionOf } from './registry/heartbeat.ts'
export {
  clearQuarantine,
  confirmLost,
  drain,
  reinstate,
  retire,
  setLabels,
  undrain,
} from './registry/lifecycle.ts'
export type { NodeRow, RegistryOptions } from './registry/node.ts'
export { getNode, lockRegion, Refused } from './registry/node.ts'
export { endpointSeen, listeningSeconds } from './registry/presence.ts'
export { renew } from './registry/renewal.ts'

export {
  answered,
  holdsMemory,
  nodeSummaries,
  recordHealth,
  toNodeView,
} from './registry/summaries.ts'

export {
  isOlder,
  retryUpgrade,
  UPGRADE_TIMEOUT_SECONDS,
  upgradeSummaries,
} from './registry/upgrades.ts'
