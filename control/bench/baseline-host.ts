// SPDX-License-Identifier: FSL-1.1-ALv2
/** Run equivalent SDK cohorts through a disposable one-box deployment per host tier. */
import assert from 'node:assert/strict'
import { engineTier } from '../src/isolation.ts'
import { writeBaseline } from './baseline-report.ts'
import { baselineFailureCode, collectBaseline, type TierBaseline } from './pryx-baseline.ts'
import { context } from './tiers/context.ts'

const [prefix] = process.argv.slice(2)
const { GITHUB_SHA: source, GITHUB_SERVER_URL, GITHUB_REPOSITORY, GITHUB_RUN_ID } = process.env
assert(prefix && source && GITHUB_SERVER_URL && GITHUB_REPOSITORY && GITHUB_RUN_ID)
const run = `${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}`
const image = process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test:latest'
process.env.PRYX_TIER_BENCH = '1'

const results = new Map<string, TierBaseline>()
const failures: string[] = []
const record = (result: TierBaseline) => {
  results.set(result.tier, result)
  writeBaseline(
    prefix,
    source,
    run,
    'Disposable one-box deployments',
    'Node children and matching container roots',
    [...results.values()],
  )
}
for (const tier of ['process', 'container', 'gvisor', 'wasm'] as const) {
  let phase = 'setup'
  try {
    const installation = await context('one-box', tier)
    try {
      phase = 'measurements'
      await collectBaseline(
        installation,
        tier,
        { sparePorts: 0, ...(engineTier(tier) ? { image } : {}) },
        record,
      )
    } finally {
      await installation.close()
    }
  } catch (error) {
    failures.push(tier)
    const result = results.get(tier) ?? { tier, samples: {}, concurrentCreates: [], complete: false }
    const failure = result.failure ?? {
      phase: result.complete ? 'cleanup' : phase,
      code: baselineFailureCode(error),
    }
    record({ ...result, complete: false, failure })
    console.error(`Baseline did not complete for ${tier} during ${failure.phase}: ${failure.code}`)
  }
}
if (failures.length) throw new Error(`Incomplete baseline tiers: ${failures.join(', ')}`)
