// SPDX-License-Identifier: FSL-1.1-ALv2
/** K14 complete three-tier comparison in existing one-box and end-to-end CI jobs.
 * Parts (`tiers/`):
 * - `context.ts`: real host and ordinary-image contexts with PSS accounting.
 * - `operations.ts`: validated same-source latency, files, packages and 50-way cohorts.
 * - `report.ts`: raw evidence, host identity and nearest-rank percentiles.
 */
import { context } from './tiers/context.ts'
import { cold, concurrent, creation, hot, packages, snapshots } from './tiers/operations.ts'
import { command, report, type Samples, save } from './tiers/report.ts'

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
for (const tier of ['container', 'process', 'wasm'] as const) {
  const samples: Samples = {},
    unsupported: Record<string, string> = {}
  const evidence = report(`${mode}/${tier}`, samples, unsupported)
  let c: Awaited<ReturnType<typeof context>> | undefined
  const extra: Record<string, unknown> = { tier, status: 'incomplete' }
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
    const box = await c.sdk.sandbox({ ...creation(tier), resources: { memoryMb: 512, cpuMillis: 250 } })
    try {
      await hot(box, samples)
      await snapshots(box, samples)
      await packages(box, samples, unsupported)
    } finally {
      await box.delete()
    }
    await concurrent(c, tier, samples)
    if (tier === 'wasm')
      unsupported.vm_suspend =
        'agentOS 0.4.7 exports disposal, not per-VM suspend; stopping the shared worker would stop other sandboxes; n=0'
    extra.status = 'complete'
  } catch (error) {
    extra.failure = error instanceof Error ? error.message : String(error)
    throw error
  } finally {
    save(`${output}-${tier}.json`, evidence, extra)
    await c?.close()
  }
}
