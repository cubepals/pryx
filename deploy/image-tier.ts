// SPDX-License-Identifier: FSL-1.1-ALv2
/** Image backend selection is host configuration. The measured no-engine choice is documented
 * in docs/tiers.md; workloads cannot change availability or enable a host engine. */
import { ISOLATIONS, type Isolation } from '../control/src/isolation.ts'

export const NO_ENGINE_DEFAULT: 'process' | 'wasm' = 'process'
type InstalledIsolation = Exclude<Isolation, 'vm'>
const VM_UNAVAILABLE = 'VM tier is not registered in this checkpoint; inspect the K25 acceptance record'

export function imageTier(docker: boolean, override?: string): InstalledIsolation {
  if (override === 'vm') throw new Error(VM_UNAVAILABLE)
  if (override && !ISOLATIONS.includes(override as Isolation))
    throw new Error('PRYX_DEFAULT_TIER must be container, process or wasm')
  const tier = (override as InstalledIsolation | undefined) ?? (docker ? 'container' : NO_ENGINE_DEFAULT)
  if (tier === 'container' && !docker)
    throw new Error('PRYX_DEFAULT_TIER=container requires a usable Docker socket')
  return tier
}

/** An image serves every available backend unless its host configuration selects one. */
export function imageRuntime(override?: string): 'auto' | InstalledIsolation {
  if (override === 'vm') throw new Error(VM_UNAVAILABLE)
  if (override && override !== 'auto' && !ISOLATIONS.includes(override as Isolation))
    throw new Error('PRYX_RUNTIME must be auto, container, process or wasm')
  return (override as 'auto' | InstalledIsolation | undefined) ?? 'auto'
}
