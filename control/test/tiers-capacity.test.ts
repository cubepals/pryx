// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Concurrent benchmark admission through the real public API and Postgres reservation ledger. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { Pryx, PryxError } from '../../sdk/typescript/src/index.ts'
import { capacitySummary } from '../bench/tiers/capacity-summary.ts'
import {
  type ConcurrentOutcome,
  concurrent,
  concurrentAttempt,
  concurrentFailureDetails,
} from '../bench/tiers/concurrent.ts'
import type { Context } from '../bench/tiers/context.ts'
import { REPETITIONS, type Samples } from '../bench/tiers/report.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { NodeRefused } from '../src/node-client.ts'
import { capacity, hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('tier comparison capacity', () => {
  let c: Awaited<ReturnType<typeof publicContext>>
  let sdk: Pryx
  let creates = 0
  let creating = 0
  let peakCreates = 0
  const room = async (count: number) =>
    c.db.execute(sql`UPDATE fleet_nodes SET
    last_heartbeat_at=now(), capacity=${JSON.stringify(
      capacity({
        allocatableMemoryMb: count * 512,
        cpus: 32,
      }),
    )}::jsonb`)
  beforeAll(async () => {
    c = await publicContext()
    sdk = new Pryx({
      url: 'http://control.test',
      key: c.credential.key,
      fetch: (async (input, init) => {
        const creation = init?.method === 'POST' && String(input).endsWith('/sandboxes')
        if (creation) {
          creates++
          peakCreates = Math.max(peakCreates, ++creating)
        }
        try {
          return await c.app.request(new Request(input, init))
        } finally {
          if (creation) creating--
        }
      }) as typeof fetch,
    })
    await room(3)
  })
  afterAll(async () => {
    await c?.close()
  })

  test(
    'all thirty waves keep fifty where they fit and record later capacity refusal',
    async () => {
      const samples: Samples = {}
      const outcomes: ConcurrentOutcome[] = []
      const measuredRunning: number[] = []
      const context = {
        sdk,
        memory: async () => {
          // The node double has no background heartbeat; keep its registry presence current.
          if (measuredRunning.length % 2 === 0) await room(measuredRunning.length === 0 ? 50 : 3)
          const running = [...c.nodes.copies.values()].filter((copy) => copy.state === 'running').length
          measuredRunning.push(running)
          return 1000 + running * 100
        },
      } as Context
      await concurrent(
        'one-box',
        'container',
        samples,
        context,
        () => {
          // Checkpoint runs before cleanup, while all admitted workloads still exist.
          expect([...c.nodes.copies.values()].filter((copy) => copy.state === 'running')).toHaveLength(
            outcomes.at(-1)?.running ?? -1,
          )
        },
        outcomes,
      )
      expect(creates).toBe(50 * REPETITIONS)
      expect(peakCreates).toBe(50)
      expect(samples.concurrent_capacity_running_count).toEqual([50, ...Array(REPETITIONS - 1).fill(3)])
      expect(samples.concurrent_capacity_pss_bytes).toEqual([6000, ...Array(REPETITIONS - 1).fill(1300)])
      expect(samples.concurrent_50_running_ms).toHaveLength(1)
      expect(samples.concurrent_50_pss_bytes).toEqual([6000])
      expect(samples.concurrent_50_increment_pss_bytes).toEqual([5000])
      expect(outcomes).toHaveLength(REPETITIONS)
      expect(outcomes[0]).toMatchObject({
        requested: 50,
        running: 50,
        refused: 0,
        outcome: 'complete',
        refusals: [],
      })
      for (const wave of outcomes.slice(1)) {
        expect(wave).toMatchObject({
          requested: 50,
          running: 3,
          refused: 47,
          outcome: 'capacity_refused',
          contextPssBytes: 1300,
          incrementalPssBytes: 300,
          refusals: [{ code: 'no_room', status: StatusCodes.SERVICE_UNAVAILABLE, count: 47 }],
        })
      }
      expect(measuredRunning).toEqual([
        0,
        50,
        ...Array.from({ length: REPETITIONS - 1 }, () => [0, 3]).flat(),
      ])
      expect([...c.nodes.copies.values()].filter((copy) => copy.state === 'running')).toEqual([])
    },
    2 * MINUTE_MS,
  )

  test('zero admitted still records refusal, memory and summary without inventing latency', async () => {
    await room(0)
    let result: ConcurrentOutcome | undefined
    await concurrentAttempt({ sdk, memory: async () => 1024 ** 2 }, 'container', 1, (wave) => {
      result = wave
    })
    expect(result).toMatchObject({ running: 0, refused: 50, contextPssBytes: 1024 ** 2 })
    if (!result) throw new Error('Missing admission outcome')
    const markdown = capacitySummary([
      {
        context: 'one-box/container',
        source: 'source',
        status: 'capacity_limited',
        concurrentCapacity: [result],
      },
    ])
    expect(markdown).toContain('full 50: 0')
    expect(markdown).toContain('| 1 | 50 | 0 | 50 | 1.00 | 0.00 |')
    expect(markdown).toContain('50× no_room (HTTP 503): No node can complete this request')
  })

  test('teardown deletes each admitted sandbox once without a second concurrent load', async () => {
    await room(3)
    let active = 0
    let peak = 0
    let attempts = 0
    for (let i = 0; i < 3; i++)
      c.nodes.once(/^DELETE /, async (answer) => {
        attempts++
        peak = Math.max(peak, ++active)
        try {
          await new Promise((resolve) => setTimeout(resolve, SECOND_MS / 10))
          return answer()
        } finally {
          active--
        }
      })
    await concurrentAttempt({ sdk, memory: async () => 1000 }, 'container', 1, (wave) => {
      expect(wave.running).toBe(3)
      expect(attempts).toBe(0)
    })
    expect(attempts).toBe(3)
    expect(peak).toBe(1)
    expect([...c.nodes.copies.values()].filter((copy) => copy.state === 'running')).toEqual([])
  })

  test('a non-capacity creation error still fails and cleans every successful creation', async () => {
    await room(3)
    c.nodes.once(/^PUT /, async () => {
      throw new NodeRefused(StatusCodes.UNPROCESSABLE_ENTITY, 'invalid_request', 'invalid spec', null)
    })
    let recorded = false
    await expect(
      concurrentAttempt({ sdk, memory: async () => 1000 }, 'container', 1, () => {
        recorded = true
      }),
    ).rejects.toThrow('Concurrent creation failed')
    expect(recorded).toBe(false)
    expect([...c.nodes.copies.values()].filter((copy) => copy.state === 'running')).toEqual([])
  })

  test('a failed memory read still fails and cleans every admitted sandbox', async () => {
    await room(3)
    let reads = 0
    await expect(
      concurrentAttempt(
        {
          sdk,
          memory: async () => {
            if (++reads === 2) throw new Error('PSS unavailable')
            return 1000
          },
        },
        'container',
        1,
        () => {
          throw new Error('Must not record missing memory')
        },
      ),
    ).rejects.toThrow('PSS unavailable')
    expect([...c.nodes.copies.values()].filter((copy) => copy.state === 'running')).toEqual([])
  })

  test('cleanup failure stays fatal after the admission outcome was recorded', async () => {
    await room(3)
    c.nodes.once(/^DELETE /, async () => {
      throw new NodeRefused(StatusCodes.SERVICE_UNAVAILABLE, 'no_room', 'delete failed', null)
    })
    let recorded = false
    let admitted = 0
    const before = c.nodes.calls.filter((call) => call.method === 'DELETE').length
    let failure: unknown
    try {
      await concurrentAttempt({ sdk, memory: async () => 1000 }, 'container', 1, (wave) => {
        recorded = true
        admitted = wave.running
      })
    } catch (error) {
      failure = error
    }
    expect(failure).toBeInstanceOf(AggregateError)
    expect((failure as Error).message).toBe('Concurrent cleanup failed')
    expect(recorded).toBe(true)
    expect(admitted).toBeGreaterThan(0)
    expect(c.nodes.calls.filter((call) => call.method === 'DELETE').length - before).toBe(admitted)
    expect(concurrentFailureDetails(failure)).toEqual([
      {
        phase: 'cleanup',
        name: PryxError.name,
        status: StatusCodes.SERVICE_UNAVAILABLE,
        code: 'no_room',
        message: 'No node can complete this request',
      },
    ])
  })
})

test('cohort failure diagnostics are bounded and omit private error fields', () => {
  const secret = 'private-request-details'
  const sdk = new PryxError(StatusCodes.SERVICE_UNAVAILABLE, 'no_room', 'Public refusal', { secret })
  const error = new AggregateError([sdk, new Error(secret)])
  const details = concurrentFailureDetails(error)
  expect(details[0]).toMatchObject({ status: sdk.status, code: sdk.code, message: sdk.message })
  expect(JSON.stringify(details)).not.toContain(secret)
  expect(concurrentFailureDetails(new AggregateError(Array(200).fill(sdk)))).toHaveLength(100)
  const cycle = new AggregateError([])
  cycle.errors.push(cycle)
  expect(concurrentFailureDetails(cycle)).toHaveLength(1)
})
