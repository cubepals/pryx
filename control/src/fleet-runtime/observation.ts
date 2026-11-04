// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Turn a persisted node report into a runtime observation; no I/O or power decisions. */
import type { RuntimeObservation } from '../../../app/ports/runtime.ts'
import type { FleetRef } from '../handle.ts'
import { deriveHealth, type Thresholds } from '../health.ts'
import { toDate } from '../sql.ts'
import type { ObservedRow } from './ledger.ts'

export function observation(row: ObservedRow | null, ref: FleetRef | null, listening: number, thresholds: Thresholds): RuntimeObservation {
    const now = new Date()
    if (row === null) return { state: 'absent', at: now }
    // The compute this handle names is not the server's any more.
    if (ref !== null && (Number(row.epoch) !== ref.epoch || row.node_id !== ref.node))
      return { state: 'absent', at: toDate(row.updated_at) }
    if (row.state === 'released') return { state: 'absent', at: toDate(row.updated_at) }
    if (row.state === 'displaced' || row.lifecycle === 'lost')
      return { state: 'unknown', at: toDate(row.lost_at ?? row.updated_at), hostLost: true }
    const health = deriveHealth(
      {
        ageSeconds: row.age,
        runtimeUp: row.runtime_up ?? false,
        reconciled: row.reconciled ?? false,
        listeningSeconds: listening,
      },
      thresholds,
    )
    // Suspect keeps the last report, so a missed beat or two doesn't flap the application.
    if (health === 'unavailable' || health === 'degraded')
      return { state: 'unknown', at: toDate(row.health_since) ?? now }
    const report = row.report
    if (report === null || Number(row.observed_epoch) !== Number(row.epoch))
      return { state: 'absent', at: toDate(row.updated_at) }
    const at = toDate(row.state_changed_at) ?? now
    const failedAt = report.lastFailureAt ? { failedAt: new Date(report.lastFailureAt) } : {}
    const exit = report.exit ? { exit: { code: report.exit.code, oom: report.exit.oomKilled } } : {}
    switch (report.state) {
      case 'running':
        return { state: 'running', at, ...failedAt }
      case 'restarting':
        return { state: 'starting', at, ...failedAt }
      case 'stopping':
        return { state: 'stopping', at }
      case 'crashed':
        return { state: 'crashed', at, ...exit }
      case 'creating':
      case 'created':
      case 'stopped':
      case 'suspended':
      case 'fenced':
        return { state: 'stopped', at, ...exit }
      case 'missing':
      case 'retained':
        return { state: 'absent', at }
      default:
        return { state: 'unknown', at }
    }
  }
