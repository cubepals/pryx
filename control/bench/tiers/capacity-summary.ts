// SPDX-License-Identifier: FSL-1.1-ALv2
/** Job-summary rendering of recorded admission outcomes; no capacity or latency is inferred. */
import type { ConcurrentOutcome } from './concurrent.ts'
import { REPETITIONS } from './report.ts'

export interface CapacityReport {
  context: string
  source: string
  status: string
  repetitions?: number
  concurrentCapacity?: ConcurrentOutcome[]
}
const cell = (value: string) =>
  value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('|', '&#124;')
    .replace(/[\r\n]+/g, ' ')
const mib = (bytes: number) => (bytes / 1024 ** 2).toFixed(2)

export function capacitySummary(reports: CapacityReport[]) {
  const lines = [
    '### Three-tier concurrent capacity',
    '',
    `Every wave requests all 50 sandboxes, for ${REPETITIONS} waves per tier in CI. Capacity refusal is a measured outcome. Only complete waves enter full-50 latency percentiles.`,
    'Memory is total context PSS (including fixed services) after all requests settle and before cleanup; delta subtracts the pre-wave baseline. It is not per-sandbox memory.',
    'A no_room response records observed API admission refusal, not a proven physical memory ceiling. Other creation, validation, memory and cleanup failures still fail the job.',
    '',
  ]
  for (const report of reports) {
    const outcomes = report.concurrentCapacity ?? []
    lines.push(
      `#### ${cell(report.context)} — ${cell(report.status)}`,
      '',
      `Source: \`${cell(report.source)}\`. Recorded waves: ${outcomes.length}/${report.repetitions ?? REPETITIONS}; full 50: ${outcomes.filter((wave) => wave.outcome === 'complete').length}.`,
      '',
      '| Wave | Requested | Running | Refused | Context PSS MiB | Delta MiB | Outcome ms | Outcome / refusal |',
      '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |',
    )
    for (const wave of outcomes) {
      const refusal = wave.refusals
        .map((item) => `${item.count}× ${item.code} (HTTP ${item.status}): ${item.message}`)
        .join('; ')
      lines.push(
        `| ${wave.attempt} | ${wave.requested} | ${wave.running} | ${wave.refused} | ${mib(wave.contextPssBytes)} | ${mib(wave.incrementalPssBytes)} | ${wave.elapsedMs.toFixed(2)} | ${cell(refusal || wave.outcome)} |`,
      )
    }
    if (!outcomes.length) lines.push('', 'No admission outcome was recorded; see the failure log.')
    lines.push('')
  }
  return `${lines.join('\n')}\n`
}
