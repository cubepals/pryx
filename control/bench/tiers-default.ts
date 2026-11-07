// SPDX-License-Identifier: FSL-1.1-ALv2
/** Measure the image's actual implicit default through both the SDK and compiled CLI. */
import { NO_ENGINE_DEFAULT } from '../../deploy/image-tier.ts'
import { UNIT_DEFAULTS } from '../src/project-model.ts'
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
const extra: Record<string, unknown> = { tier, status: 'incomplete', resourceDefaults: UNIT_DEFAULTS }
const creationTimes: number[] = [],
  firstEchoTimes: number[] = [],
  firstBytes: number[] = [],
  cliFirstBytes: number[] = []
samples.sdk_create_running_ms = creationTimes
samples.sdk_first_echo_ms = firstEchoTimes
samples.sdk_create_to_first_byte_ms = firstBytes
samples.compiled_cli_first_byte_ms = cliFirstBytes
let c: Awaited<ReturnType<typeof context>> | undefined
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
  c = await context('hello', tier, Boolean(requested))
  const current = c
  await measure(samples, 'sdk_create_to_echo_ms', async () => {
    const began = performance.now()
    const box = await current.sdk.sandbox()
    const created = performance.now()
    creationTimes.push(created - began)
    try {
      if (box.isolation !== tier) throw new Error('Wrong image tier')
      const run = box.stream('echo hello'),
        stream = run[Symbol.asyncIterator]()
      if (!(await stream.next()).value?.data) throw new Error('Image emitted no output')
      const first = performance.now()
      firstBytes.push(first - began)
      firstEchoTimes.push(first - created)
      while (!(await stream.next()).done) {}
      const result = await run.result()
      if (result.code !== 0 || result.stdout !== 'hello\n' || result.stderr !== '')
        throw new Error('Image did not print hello')
    } catch (error) {
      await box.delete()
      throw error
    }
    return async () => {
      await box.delete()
    }
  })
  await measure(samples, 'compiled_pryx_run_echo_ms', async () => {
    const began = performance.now()
    const child = Bun.spawn([process.env.PRYX_MCP_CLI ?? 'dist/pryx', 'run', '--', 'echo', 'hello'], {
      env: current.env,
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const error = new Response(child.stderr).text()
    const reader = child.stdout.getReader()
    let value = ''
    let first = true
    for (;;) {
      const chunk = await reader.read()
      if (chunk.done) break
      if (first && chunk.value.length) {
        cliFirstBytes.push(performance.now() - began)
        first = false
      }
      value += new TextDecoder().decode(chunk.value)
    }
    if ((await child.exited) || (await error) !== '' || value !== 'hello\n')
      throw new Error('Compiled CLI default output differs')
  })
  extra.status = 'complete'
} catch (error) {
  extra.failure = error instanceof Error ? error.message : String(error)
  throw error
} finally {
  save(output, evidence, extra)
  await c?.close()
}
