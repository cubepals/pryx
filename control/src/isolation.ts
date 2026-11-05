// SPDX-License-Identifier: FSL-1.1-ALv2
/** Public isolation choices and the capability announced by a node running each backend. */
export const ISOLATIONS = ['container', 'process', 'wasm'] as const
export type Isolation = (typeof ISOLATIONS)[number]
export function nodeIsolation(features: readonly string[]): Isolation {
  if (features.includes('wasm-tier')) return 'wasm'
  return features.includes('process-tier') ? 'process' : 'container'
}
