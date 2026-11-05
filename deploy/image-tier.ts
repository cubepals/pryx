// SPDX-License-Identifier: FSL-1.1-ALv2
/** Image backend selection is host configuration. The measured no-engine choice is documented
 * in docs/tiers.md; workloads cannot change availability or enable a host engine. */
import { ISOLATIONS, type Isolation } from '../control/src/isolation.ts'

export const NO_ENGINE_DEFAULT: 'process' | 'wasm' = 'process'

export function imageTier(docker: boolean, override?: string): Isolation {
  if (override && !ISOLATIONS.includes(override as Isolation))
    throw new Error('PRYX_DEFAULT_TIER must be container, process or wasm')
  const tier = (override as Isolation | undefined) ?? (docker ? 'container' : NO_ENGINE_DEFAULT)
  if (tier === 'container' && !docker)
    throw new Error('PRYX_DEFAULT_TIER=container requires a usable Docker socket')
  return tier
}
