// SPDX-License-Identifier: FSL-1.1-ALv2
/** Render the same raw measurement cohorts as JSON and a job-summary table. */
import { writeFileSync } from 'node:fs'
import { cpus, release, totalmem } from 'node:os'
import { BASELINE_FILE_BYTES, baselineSummary, type TierBaseline } from './pryx-baseline.ts'

export function writeBaseline(
  prefix: string,
  source: string,
  run: string,
  deployment: string,
  memoryScope: string,
  results: readonly TierBaseline[],
) {
  const tiers = results.map((result) => ({ ...result, summary: baselineSummary(result.samples) }))
  const report = {
    source,
    run,
    deployment,
    host: { cpu: cpus()[0]?.model, cores: cpus().length, kernel: release(), memoryBytes: totalmem() },
    fileBytes: BASELINE_FILE_BYTES,
    idleMemory: 'PSS after one idle sandbox minus PSS immediately before creation',
    memoryScope,
    concurrency: 'Latency includes only waves where every requested sandbox reached running',
    tiers,
  }
  writeFileSync(`${prefix}.json`, `${JSON.stringify(report, null, 2)}\n`)
  const table = [
    '# pryx baseline',
    '',
    `Source ${source}. ${deployment}.`,
    `PSS scope is ${memoryScope}.`,
    '',
    '| Tier | Metric | Samples | p50 | p95 |',
    '| --- | --- | ---: | ---: | ---: |',
  ]
  for (const { tier, complete, failure, summary, concurrentCreates } of tiers) {
    const status = complete ? 'Complete' : failure ? `Partial (${failure.phase}, ${failure.code})` : 'Partial'
    table.push(`| ${tier} | Measurement status | ${status} | — | — |`)
    for (const [metric, { n, p50, p95 }] of Object.entries(summary)) {
      table.push(`| ${tier} | ${metric} | ${n} | ${p50?.toFixed(2) ?? '—'} | ${p95?.toFixed(2) ?? '—'} |`)
    }
    for (const { requested, running, refused, attempt } of concurrentCreates) {
      if (refused)
        table.push(
          `| ${tier} | Wave ${attempt} of ${requested} creates | ${running} running | ${refused} refused | — |`,
        )
    }
  }
  writeFileSync(`${prefix}.md`, `${table.join('\n')}\n`)
}
