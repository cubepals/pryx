// SPDX-License-Identifier: FSL-1.1-ALv2
/** Repeatable public SDK measurements for a single verified tier and installation. */
import assert from 'node:assert/strict'
import { type MakeRequest, type Pryx, PryxError, type Sandbox } from '../../sdk/typescript/src/index.ts'
import type { Isolation } from '../src/isolation.ts'
import { type ConcurrentOutcome, concurrentAttempt } from './tiers/concurrent.ts'
import { measure, percentile, REPETITIONS, type Samples } from './tiers/report.ts'

export const BASELINE_FILE_BYTES = 4 * 1024 * 1024
const WAVE_SIZES = [1, 10, 50] as const
const WAVES_PER_SIZE = 5

export interface BaselineContext {
  sdk: Pick<Pryx, 'sandbox'>
  concurrentSdk?: Pick<Pryx, 'sandbox'>
  // PSS of the same deployment before and after creating a single idle sandbox.
  memory: () => Promise<number>
}

export interface TierBaseline {
  tier: Isolation
  samples: Samples
  concurrentCreates: Array<Omit<ConcurrentOutcome, 'outcome'> & { outcome: 'complete' | 'api_refused' }>
  complete: boolean
  failure?: { phase: string; code: string }
}

export function baselineFailureCode(error: unknown) {
  if (error instanceof PryxError && /^[a-z][a-z0-9_]{0,63}$/.test(error.code)) return error.code
  if (error instanceof assert.AssertionError) return 'assertion_failed'
  return 'unexpected_error'
}

export async function collectBaseline(
  c: BaselineContext,
  tier: Isolation,
  request: MakeRequest,
  checkpoint: (result: TierBaseline) => void,
) {
  const fixed: number[] = []
  const idle: number[] = []
  const delta: number[] = []
  const creation: number[] = []
  const result: TierBaseline = {
    tier,
    samples: {
      fixed_context_pss_bytes: fixed,
      idle_context_pss_bytes: idle,
      idle_sandbox_pss_bytes: delta,
      create_ms: creation,
    },
    concurrentCreates: [],
    complete: false,
  }
  const { samples, concurrentCreates } = result
  const create = async () => {
    const box = await c.sdk.sandbox({ ...request, isolation: tier, units: 1 })
    try {
      assert.equal(box.state, 'running')
      assert.equal(box.isolation, tier)
      assert.equal(box.units, 1)
      return box
    } catch (error) {
      await box.delete()
      throw error
    }
  }

  let phase = 'create'
  try {
    for (let index = 0; index < REPETITIONS; index++) {
      const before = await c.memory()
      const started = performance.now()
      const box = await create()
      creation.push(performance.now() - started)
      try {
        const after = await c.memory()
        fixed.push(before)
        idle.push(after)
        delta.push(after - before)
      } finally {
        await box.delete()
      }
      checkpoint(result)
    }

    phase = 'exec_files_wake'
    const box = await create()
    try {
      await hotSamples(box, samples)
    } finally {
      await box.delete()
    }
    checkpoint(result)

    phase = 'concurrent_create'
    const concurrency = { sdk: c.concurrentSdk ?? c.sdk, memory: c.memory }
    for (const count of WAVE_SIZES) {
      const values: number[] = []
      samples[`concurrent_${count}_create_ms`] = values
      for (let attempt = 1; attempt <= WAVES_PER_SIZE; attempt++) {
        await concurrentAttempt(
          concurrency,
          tier,
          attempt,
          (outcome) => {
            concurrentCreates.push({ ...outcome, outcome: outcome.refused ? 'api_refused' : 'complete' })
            if (outcome.outcome === 'complete') values.push(outcome.elapsedMs)
          },
          count,
          { ...request, isolation: tier, units: 1 },
        )
        checkpoint(result)
      }
    }
    result.complete = true
    return result
  } catch (error) {
    result.failure = { phase, code: baselineFailureCode(error) }
    throw error
  } finally {
    checkpoint(result)
  }
}

async function hotSamples(box: Sandbox, samples: Samples) {
  const data = new Uint8Array(BASELINE_FILE_BYTES)
  for (let offset = 0; offset < data.length; offset += 65_536)
    crypto.getRandomValues(data.subarray(offset, offset + 65_536))
  const path = 'baseline-payload'
  const digest = Bun.hash(data)
  const verify = (received: Uint8Array) => {
    assert.equal(received.length, BASELINE_FILE_BYTES)
    assert.equal(Bun.hash(received), digest)
  }
  await measure(samples, 'exec_ms', async () => {
    const output = await box.run('true')
    assert.equal(output.code, 0)
    assert.equal(output.stdout, '')
    assert.equal(output.stderr, '')
  })
  await measure(samples, 'file_write_ms', async () => {
    await box.write(path, data)
    return async () => assert.equal((await box.files.stat(path)).size, BASELINE_FILE_BYTES)
  })
  await measure(samples, 'file_read_ms', async () => {
    const received = await box.files.readBytes(path)
    return async () => verify(received)
  })
  await measure(
    samples,
    'wake_ms',
    async () => {
      await box.wake()
      assert.equal(box.state, 'running')
      return async () => verify(await box.files.readBytes(path))
    },
    async () => {
      await box.sleep()
    },
  )
  await box.files.delete(path)

  for (const mode of ['read', 'write'] as const) {
    const durations = samples[`file_${mode}_ms`] ?? []
    samples[`file_${mode}_bytes_per_second`] = durations.map((ms) => (BASELINE_FILE_BYTES * 1_000) / ms)
  }
}

export function baselineSummary(
  samples: Samples,
): Record<string, { n: number; p50: number | null; p95: number | null }> {
  return Object.fromEntries(
    Object.entries(samples).map(([metric, values]) => [
      metric,
      { n: values.length, p50: percentile(values, 0.5) ?? null, p95: percentile(values, 0.95) ?? null },
    ]),
  )
}
