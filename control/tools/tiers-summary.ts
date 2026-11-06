// SPDX-License-Identifier: FSL-1.1-ALv2
/** Publish raw three-tier capacity evidence to the existing CI job summary. */
import { type CapacityReport, capacitySummary } from '../bench/tiers/capacity-summary.ts'

const reports: CapacityReport[] = []
for (const path of process.argv.slice(2)) {
  const file = Bun.file(path)
  if (!(await file.exists())) continue
  const report = await file.json()
  if (Array.isArray(report.concurrentCapacity)) reports.push(report)
}
console.log(capacitySummary(reports))
