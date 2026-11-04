// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * A node's health, from its heartbeats (docs/fleet.md, "Health"). Pure: the ages it reads are
 * database time (`now() - last_heartbeat_at`), so no clock is compared across machines.
 *
 * Health is what the control plane has heard, not a verdict, and nothing moves on it. `lost` is not
 * a health: it is a lifecycle an operator confirms, and nothing here ever produces it.
 */
export type Health = 'healthy' | 'degraded' | 'suspect' | 'unavailable'

export interface Thresholds {
  /** Silence past this is suspect: no new servers go there; observations are kept. */
  suspectSeconds: number
  /** Silence past this is unavailable: its servers read `unknown`. */
  unavailableSeconds: number
}

export const THRESHOLDS: Thresholds = { suspectSeconds: 15, unavailableSeconds: 45 }

export interface HealthInput {
  /** Seconds since the last heartbeat, in database time; null if the node never beat. */
  ageSeconds: number | null
  runtimeUp: boolean
  reconciled: boolean
  /**
   * Seconds the node endpoint has been served without a gap. Heartbeats sent while nothing
   * listened are not the nodes' fault, so a returning endpoint gives every node one suspect
   * window first.
   */
  listeningSeconds: number
}

export function deriveHealth(input: HealthInput, thresholds: Thresholds = THRESHOLDS): Health {
  const { ageSeconds } = input
  if (ageSeconds === null) return 'unavailable'
  if (ageSeconds > thresholds.unavailableSeconds)
    return input.listeningSeconds < thresholds.unavailableSeconds ? 'suspect' : 'unavailable'
  if (ageSeconds > thresholds.suspectSeconds) return 'suspect'
  return input.runtimeUp && input.reconciled ? 'healthy' : 'degraded'
}
