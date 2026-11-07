// SPDX-License-Identifier: FSL-1.1-ALv2
/** Concurrent admission outcomes and PSS before cleanup; only complete waves measure fifty-way latency. */
import { PryxError } from '../../../sdk/typescript/src/index.ts'
import type { Isolation } from '../../src/isolation.ts'
import { type Context, context } from './context.ts'
import { creation } from './operations.ts'
import { REPETITIONS, type Samples } from './report.ts'

export const CONCURRENT_REQUESTS = 50
type FailurePhase = 'creation' | 'cleanup' | 'cohort'
class ConcurrentFailure extends AggregateError {
  constructor(
    readonly phase: FailurePhase,
    errors: unknown[],
    message: string,
  ) {
    super(errors, message)
  }
}

/** Only public SDK fields enter artifacts; omit details, stacks and arbitrary internal error text. */
export function concurrentFailureDetails(error: unknown) {
  const details: Array<{
    phase?: FailurePhase
    name: string
    status?: number
    code?: string
    message?: string
  }> = []
  // At most fifty creation and fifty cleanup failures; bounded recursion also handles cycles.
  const visit = (cause: unknown, phase?: FailurePhase, depth = 0) => {
    if (details.length >= 2 * CONCURRENT_REQUESTS) return
    if (cause instanceof AggregateError && depth < 4) {
      for (const inner of cause.errors) {
        if (details.length >= 2 * CONCURRENT_REQUESTS) break
        visit(inner, cause instanceof ConcurrentFailure ? cause.phase : phase, depth + 1)
      }
    } else
      details.push(
        cause instanceof PryxError
          ? {
              phase,
              name: PryxError.name,
              status: cause.status,
              code: cause.code,
              message: cause.message.slice(0, 1024),
            }
          : { phase, name: Error.name },
      )
  }
  visit(error)
  return details
}

export interface ConcurrentOutcome {
  attempt: number
  requested: number
  running: number
  refused: number
  outcome: 'complete' | 'capacity_refused'
  elapsedMs: number
  baselinePssBytes: number
  contextPssBytes: number
  incrementalPssBytes: number
  refusals: Array<{ code: string; status: number; message: string; count: number }>
}

export async function concurrentAttempt(
  c: Pick<Context, 'sdk' | 'memory'>,
  tier: Isolation,
  attempt: number,
  measured: (outcome: ConcurrentOutcome) => void,
) {
  const baseline = await c.memory()
  const start = performance.now()
  const made = await Promise.allSettled(
    Array.from({ length: CONCURRENT_REQUESTS }, () => c.sdk.sandbox(creation(tier))),
  )
  const elapsedMs = performance.now() - start
  const boxes = made.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []))
  const failures: unknown[] = []
  try {
    const errors = made.flatMap((result) => (result.status === 'rejected' ? [result.reason] : []))
    // The public API owns this admission code. Do not classify errors by message text or retry creation.
    const fatal = errors.filter((error) => !(error instanceof PryxError && error.code === 'no_room'))
    if (fatal.length) throw new ConcurrentFailure('creation', fatal, 'Concurrent creation failed')
    if (boxes.some((box) => box.state !== 'running' || box.isolation !== tier || box.units !== 1))
      throw new Error('Concurrent sandbox did not run')
    const refusals: ConcurrentOutcome['refusals'] = []
    for (const error of errors as PryxError[]) {
      const same = refusals.find(
        (item) => item.code === error.code && item.status === error.status && item.message === error.message,
      )
      if (same) same.count++
      else refusals.push({ code: error.code, status: error.status, message: error.message, count: 1 })
    }
    const used = await c.memory()
    measured({
      attempt,
      requested: CONCURRENT_REQUESTS,
      running: boxes.length,
      refused: errors.length,
      outcome: errors.length ? 'capacity_refused' : 'complete',
      elapsedMs,
      baselinePssBytes: baseline,
      contextPssBytes: used,
      incrementalPssBytes: used - baseline,
      refusals,
    })
  } catch (error) {
    failures.push(error)
  }
  // Teardown is outside the measured cohort: do not introduce a second fifty-request mutation load.
  const cleanup: unknown[] = []
  for (const box of boxes) {
    try {
      await box.delete()
    } catch (error) {
      cleanup.push(error)
    }
  }
  if (cleanup.length) failures.push(new ConcurrentFailure('cleanup', cleanup, 'Concurrent cleanup failed'))
  if (failures.length === 1) throw failures[0]
  if (failures.length) throw new ConcurrentFailure('cohort', failures, 'Concurrent cohort failed')
}

export async function concurrent(
  mode: string,
  tier: Isolation,
  samples: Samples,
  existing?: Context,
  checkpoint = () => {},
  outcomes: ConcurrentOutcome[] = [],
) {
  samples.concurrent_50_increment_pss_bytes = []
  samples.concurrent_50_running_ms = []
  samples.concurrent_50_pss_bytes = []
  samples.concurrent_capacity_running_count = []
  samples.concurrent_capacity_outcome_ms = []
  samples.concurrent_capacity_pss_bytes = []
  samples.concurrent_capacity_increment_pss_bytes = []
  for (let i = 0; i < REPETITIONS; i++) {
    const c = existing ?? (await context(mode, tier))
    try {
      await concurrentAttempt(c, tier, i + 1, (outcome) => {
        outcomes.push(outcome)
        samples.concurrent_capacity_running_count?.push(outcome.running)
        samples.concurrent_capacity_outcome_ms?.push(outcome.elapsedMs)
        samples.concurrent_capacity_pss_bytes?.push(outcome.contextPssBytes)
        samples.concurrent_capacity_increment_pss_bytes?.push(outcome.incrementalPssBytes)
        if (outcome.outcome === 'complete') {
          samples.concurrent_50_running_ms?.push(outcome.elapsedMs)
          samples.concurrent_50_pss_bytes?.push(outcome.contextPssBytes)
          samples.concurrent_50_increment_pss_bytes?.push(outcome.incrementalPssBytes)
        }
        console.info('CONCURRENT_CAPACITY', tier, JSON.stringify(outcome))
        // Persist the admitted count, memory and refusal while successful sandboxes still exist.
        checkpoint()
      })
    } finally {
      if (!existing) await c.close()
    }
  }
  return outcomes
}
