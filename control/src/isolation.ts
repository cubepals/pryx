// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public isolation choices and the capability announced by a node running each backend. */
import nodeSchema from '../../openapi/node-writes.openapi.json'

export const CPU_LIMIT_PROBLEM = nodeSchema['x-pryx-resources'].cpuLimitProblem
export const ISOLATIONS = ['container', 'process', 'wasm'] as const
export type Isolation = (typeof ISOLATIONS)[number]
export type IsolationDefault = Isolation | readonly Isolation[]
// The VM priority is inserted when that backend is available (K25).
export const DEFAULT_ISOLATION_ORDER: readonly Isolation[] = ['container', 'process', 'wasm']
export const isolationOrder = (preference: IsolationDefault) =>
  typeof preference === 'string' ? [preference] : preference
export function nodeIsolation(features: readonly string[]): Isolation {
  if (features.includes('wasm-tier')) return 'wasm'
  return features.includes('process-tier') ? 'process' : 'container'
}

/** Tier announcements do not change the legacy default markers. */
export function nodeIsolations(features: readonly string[]): readonly Isolation[] {
  const announced = ISOLATIONS.filter((tier) => features.includes(`tier-${tier}`))
  return features.includes('multi-tier') && announced.length ? announced : [nodeIsolation(features)]
}

export function isIsolationDefault(value: unknown): value is IsolationDefault {
  return typeof value === 'string'
    ? ISOLATIONS.includes(value as Isolation)
    : Array.isArray(value) && value.length > 0 && value.every((tier) => ISOLATIONS.includes(tier))
}
