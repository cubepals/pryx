// SPDX-License-Identifier: FSL-1.1-ALv2
/** Concurrent sandbox cohorts and their memory samples; runtime scheduling stays in the product. */
import type { Isolation } from '../../src/isolation.ts'
import { type Context, context } from './context.ts'
import { creation } from './operations.ts'
import { REPETITIONS, type Samples } from './report.ts'

export async function concurrent(
  mode: string,
  tier: Isolation,
  samples: Samples,
  existing?: Context,
  checkpoint = () => {},
) {
  samples.concurrent_50_increment_pss_bytes = []
  samples.concurrent_50_running_ms = []
  samples.concurrent_50_pss_bytes = []
  for (let i = 0; i < REPETITIONS; i++) {
    const c = existing ?? (await context(mode, tier))
    try {
      const baseline = await c.memory()
      const start = performance.now()
      const made = await Promise.allSettled(Array.from({ length: 50 }, () => c.sdk.sandbox(creation(tier))))
      const boxes = made.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []))
      const failure = made.find((result) => result.status === 'rejected')
      if (
        failure ||
        boxes.some((box) => box.state !== 'running' || box.isolation !== tier || box.units !== 1)
      ) {
        await Promise.all(boxes.map((box) => box.delete()))
        throw failure?.status === 'rejected' ? failure.reason : new Error('Concurrent sandbox did not run')
      }
      samples.concurrent_50_running_ms.push(performance.now() - start)
      try {
        const used = await c.memory()
        samples.concurrent_50_pss_bytes.push(used)
        samples.concurrent_50_increment_pss_bytes.push(used - baseline)
        console.info('CONCURRENT_50', tier, i + 1, samples.concurrent_50_running_ms.at(-1), used)
        checkpoint()
      } finally {
        await Promise.all(boxes.map((box) => box.delete()))
      }
    } finally {
      if (!existing) await c.close()
    }
  }
}
