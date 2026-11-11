// SPDX-License-Identifier: FSL-1.1-ALv2
/** Guarantee names come from the node's Rust contract. Unannounced enforcement is always false. */
import contract from '../../openapi/node-writes.openapi.json'
import type { components } from './generated/node-writes.ts'
import { type Isolation, nodeIsolation } from './isolation.ts'

export type Guarantee = components['schemas']['Guarantee']
export const GUARANTEES = contract.components.schemas.Guarantee.enum as [Guarantee, ...Guarantee[]]
export function guarantees(
  features: readonly string[],
  tier: Isolation = nodeIsolation(features),
): Record<Guarantee, boolean> {
  const scoped = features.includes('multi-tier') && features.some((feature) => feature.startsWith('tier-'))
  return Object.fromEntries(
    GUARANTEES.map((name) => [name, features.includes(scoped ? `tier-${tier}:${name}` : name)]),
  ) as Record<Guarantee, boolean>
}

/** Placement capabilities are not evidence that an older copy was configured with limits. */
export function workloadGuarantees(
  features: readonly string[],
  tier: Isolation,
  observed?: Partial<Record<Guarantee, boolean>> | null,
): Record<Guarantee, boolean> {
  const result = guarantees(features, tier)
  if (tier === 'container' && features.includes(contract['x-pryx-resources'].observedGuaranteesFeature)) {
    for (const name of ['memory-limit', 'cpu-limit'] as const) result[name] &&= observed?.[name] === true
  }
  if (tier === 'container' && features.includes(contract['x-pryx-resources'].nativeBrowserPolicyFeature)) {
    result['browser-sandbox'] &&= observed?.['browser-sandbox'] === true
  }
  return result
}
