// SPDX-License-Identifier: FSL-1.1-ALv2
/** Guarantee names come from the node's Rust contract. Unannounced enforcement is always false. */
import contract from '../../openapi/node-writes.openapi.json'
import type { components } from './generated/node-writes.ts'

export type Guarantee = components['schemas']['Guarantee']
export const GUARANTEES = contract.components.schemas.Guarantee.enum as [Guarantee, ...Guarantee[]]
export function guarantees(features: readonly string[]): Record<Guarantee, boolean> {
  return Object.fromEntries(GUARANTEES.map((name) => [name, features.includes(name)])) as Record<
    Guarantee,
    boolean
  >
}
