// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Concurrent benchmark admission through the real public API and Postgres reservation ledger. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { capacitySummary } from '../bench/tiers/capacity-summary.ts'
import { type ConcurrentOutcome, concurrent, concurrentAttempt } from '../bench/tiers/concurrent.ts'
import type { Context } from '../bench/tiers/context.ts'
import { REPETITIONS, type Samples } from '../bench/tiers/report.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { NodeRefused } from '../src/node-client.ts'
import { capacity, hasDatabase } from './nodes.ts'
import { publicContext } from './public-context.ts'

describe.skipIf(!hasDatabase)('tier comparison capacity', () => {
  let c: Awaited<ReturnType<typeof publicContext>>
  let sdk: Pryx
  let creates = 0
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
      fetch: ((input, init) => {
        if (init?.method === 'POST' && String(input).endsWith('/sandboxes')) creates++
        return c.app.request(new Request(input, init))
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
    await expect(
      concurrentAttempt({ sdk, memory: async () => 1000 }, 'container', 1, () => {
        recorded = true
      }),
    ).rejects.toThrow('Concurrent cleanup failed')
    expect(recorded).toBe(true)
  })
})
