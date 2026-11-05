// SPDX-License-Identifier: FSL-1.1-ALv2
/** Measure the image's actual implicit default through both the SDK and compiled CLI. */
import { NO_ENGINE_DEFAULT } from '../../deploy/image-tier.ts'
import { context } from './tiers/context.ts'
import { command, measure, report, type Samples, save } from './tiers/report.ts'

const output = process.argv[2]
const requested = process.argv[3]
if (requested !== undefined && requested !== 'wasm' && requested !== 'process')
  throw new Error('Candidate must be wasm or process')
const tier = requested ?? NO_ENGINE_DEFAULT
if (!output) throw new Error('tiers-default.ts output.json')
const samples: Samples = {},
  unsupported: Record<string, string> = {}
const evidence = report(
  `ordinary image / ${requested ? `forced ${tier}` : 'implicit default'}`,
  samples,
  unsupported,
)
const extra: Record<string, unknown> = { tier, status: 'incomplete' }
let c: Awaited<ReturnType<typeof context>> | undefined
try {
  c = await context('hello', tier, Boolean(requested))
  const current = c
  await measure(samples, 'sdk_create_to_echo_ms', async () => {
    const box = await current.sdk.sandbox()
    try {
      if (box.isolation !== tier || (await box.run('echo hello')).stdout !== 'hello\n')
        throw new Error('Implicit default did not print hello')
    } catch (error) {
      await box.delete()
      throw error
    }
    return async () => {
      await box.delete()
    }
  })
  await measure(samples, 'compiled_pryx_run_echo_ms', async () => {
    const value = await command(
      [process.env.PRYX_MCP_CLI ?? 'dist/pryx', 'run', '--', 'echo', 'hello'],
      current.env,
    )
    if (value !== 'hello\n') throw new Error('Compiled CLI default output differs')
  })
  extra.status = 'complete'
} catch (error) {
  extra.failure = error instanceof Error ? error.message : String(error)
  throw error
} finally {
  save(output, evidence, extra)
  await c?.close()
}
