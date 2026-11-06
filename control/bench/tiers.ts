// SPDX-License-Identifier: FSL-1.1-ALv2
/** K14 complete three-tier comparison in existing one-box and end-to-end CI jobs.
 * Parts (`tiers/`):
 * - `context.ts`: real host and ordinary-image contexts with PSS accounting.
 * - `operations.ts`: validated same-source latency, files, packages and 50-way cohorts.
 * - `report.ts`: raw evidence, host identity and nearest-rank percentiles.
 */

import { StatusCodes } from 'http-status-codes'
import {
  TIER_BENCH_CPU_OVERCOMMIT,
  TIER_BENCH_LARGE_UNIT,
  TIER_BENCH_LARGE_UNITS,
  TIER_BENCH_UNIT,
} from '../e2e/runtime-config.ts'
import { NodeClient, NodeRefused } from '../src/node-client.ts'
import { context } from './tiers/context.ts'
import { cold, concurrent, creation, hot, packages, snapshots } from './tiers/operations.ts'
import { command, report, type Samples, save } from './tiers/report.ts'

// Benchmark-only refusal evidence excludes request bodies, headers and connection identities.
const nodeCall = NodeClient.prototype.call
let nativeRefusals = false
NodeClient.prototype.call = async function <T>(...args: Parameters<NodeClient['call']>): Promise<T> {
  try {
    return (await nodeCall.apply(this, args)) as T
  } catch (error) {
    if (nativeRefusals && error instanceof NodeRefused && error.status >= StatusCodes.INTERNAL_SERVER_ERROR)
      console.error('NODE_RUNTIME_FAILURE', error.status, error.message.slice(0, 1024))
    throw error
  }
}

const [mode, output] = process.argv.slice(2)
if (!output || (mode !== 'one-box' && mode !== 'image'))
  throw new Error('tiers.ts one-box|image output-prefix')
process.env.PRYX_TIER_BENCH = '1'
// Take the explicit tag from Docker rather than using an untagged policy prefix.
process.env.PRYX_IMAGE_TEST = (
  await command([
    'docker',
    'image',
    'inspect',
    '--format',
    '{{index .RepoTags 0}}',
    process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test',
  ])
).trim()
if (!process.env.PRYX_IMAGE_TEST.includes(':')) throw new Error('Comparison image must have a repository tag')
const failures: unknown[] = []
for (const tier of ['container', 'process', 'wasm'] as const) {
  nativeRefusals = tier === 'process'
  const samples: Samples = {},
    unsupported: Record<string, string> = {}
  const evidence = report(`${mode}/${tier}`, samples, unsupported)
  let c: Awaited<ReturnType<typeof context>> | undefined
  const extra: Record<string, unknown> = {
    tier,
    status: 'incomplete',
    coldAndConcurrentResources: TIER_BENCH_UNIT,
    hotAndInstallResources: TIER_BENCH_LARGE_UNIT,
    cpuAdmissionOvercommit: TIER_BENCH_CPU_OVERCOMMIT,
    concurrentContext: mode === 'image' ? 'fresh context per sample' : 'one node reused for all samples',
  }
  console.info('TIER', mode, tier)
  try {
    extra.imageId = (
      await command([
        'docker',
        'image',
        'inspect',
        '--format',
        '{{.Id}}',
        process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test',
      ])
    ).trim()
    extra.binarySha256 = (await command(['sha256sum', process.env.PRYXD_BIN ?? 'target/debug/pryxd'])).split(
      ' ',
    )[0]
    c = await context(mode, tier)
    extra.topology = c.topology
    await cold(c, tier, samples)
    const box = await c.sdk.sandbox({ ...creation(tier), resources: TIER_BENCH_LARGE_UNIT })
    try {
      if (box.units !== TIER_BENCH_LARGE_UNITS) throw new Error('Large cohort unit envelope differs')
      await hot(box, samples)
      await snapshots(box, samples)
      await packages(c, tier, box, samples)
    } finally {
      await box.delete()
    }
    if (mode === 'image') {
      await c.close()
      c = undefined
    }
    await concurrent(mode, tier, samples, c)
    if (tier === 'wasm')
      unsupported.vm_suspend =
        'agentOS 0.4.7 exports disposal, not per-VM suspend; stopping the shared worker would stop other sandboxes; n=0'
    extra.status = 'complete'
  } catch (error) {
    extra.failure = error instanceof Error ? error.message : String(error)
    failures.push(error)
    console.error('TIER_FAILURE', tier, extra.failure)
  } finally {
    try {
      await c?.close()
    } catch (error) {
      extra.status = 'incomplete'
      extra.cleanupFailure = error instanceof Error ? error.message : String(error)
      failures.push(error)
    }
    save(`${output}-${tier}.json`, evidence, extra)
  }
}
if (failures.length) throw new AggregateError(failures, 'Three-tier comparison failed')
