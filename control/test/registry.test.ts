// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Registry suites for enrollment, lifecycle, tokens and admission ordering. */

import 'reflect-metadata'
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { THRESHOLDS } from '../src/health.ts'
import { caSha256, joinToken } from '../src/join-token.ts'
import { answered, createToken, enroll, holdsMemory, nodeSummaries } from '../src/registry.ts'
import { rows } from '../src/sql.ts'
import type { WorkloadView } from '../src/wire.ts'
import { enrollNode, facts, freshDatabase, hasDatabase, nodeKey, registryOptions, testCa } from './nodes.ts'
import { registerRegistry1 } from './registry-checks-1.ts'
import { registerRegistry2 } from './registry-checks-2.ts'
import { registryContext as ctx } from './registry-context.ts'

describe.skipIf(!hasDatabase)('the node registry, against Postgres', () => {
  beforeAll(async () => {
    ;({ db: ctx.db, drop: ctx.drop } = await freshDatabase())
    ctx.ca = await testCa()
    // A node endpoint has been serving for a while, so silence counts against nodes.
    await ctx.db.execute(sql`INSERT INTO fleet_endpoints VALUES ('test', now() - interval '1 hour', now())`)
  })
  afterAll(() => ctx.drop?.())
  registerRegistry1(ctx)
  registerRegistry2(ctx)
})
describe.skipIf(!hasDatabase)('join tokens, against Postgres', () => {
  beforeAll(async () => {
    ;({ db: ctx.db, drop: ctx.drop } = await freshDatabase())
    ctx.ca = await testCa()
  })
  afterAll(() => ctx.drop?.())

  test('a token is for a fleet region the region map names, and nothing else', async () => {
    const regions = ['fsn1', 'ash']
    const refused = createToken(ctx.db, {
      regionKey: 'eu',
      ttlSeconds: 60,
      createdBy: 'test',
      regions,
    })
    await expect(refused).rejects.toMatchObject({
      code: 'unknown_region',
      status: StatusCodes.BAD_REQUEST,
    })
    await expect(refused).rejects.toThrow('PRYX_REGION_MAP maps to fsn1, ash')
    await createToken(ctx.db, {
      regionKey: 'ash',
      ttlSeconds: 60,
      createdBy: 'test',
      regions,
    })
    // One that re-enrolls a node takes the node's region, whatever it was given.
    const a = await enrollNode(ctx.db, ctx.ca, 'region-kept')
    await createToken(ctx.db, {
      regionKey: '',
      ttlSeconds: 60,
      createdBy: 'test',
      node: a.id,
      regions,
    })
  })

  test('a pasted token names the endpoint, the CA and the deployment, and enrolls by its secret', async () => {
    const { token } = await createToken(ctx.db, {
      regionKey: 'eu',
      ttlSeconds: 60,
      createdBy: 'test',
    })
    const join = {
      url: 'https://10.0.0.2:8443',
      caPem: ctx.ca.pem,
      deployment: 'test',
    }
    const pasted = joinToken(join, token)
    expect(pasted.startsWith('bk1.')).toBe(true)
    expect(JSON.parse(Buffer.from(pasted.slice(4), 'base64url').toString())).toEqual({
      u: 'https://10.0.0.2:8443',
      h: caSha256(ctx.ca.pem),
      d: 'test',
      s: token,
    })
    // pryxd sends the secret alone; one that sends the whole token (an older one) enrolls too.
    const key = await nodeKey()
    const enrolled = await enroll(ctx.db, ctx.ca, registryOptions(), {
      token: pasted,
      csrPem: key.csrPem,
      facts: facts('j1'),
    })
    const again = await enroll(ctx.db, ctx.ca, registryOptions(), {
      token,
      csrPem: key.csrPem,
      facts: facts('j1'),
    })
    expect(again.nodeId).toBe(enrolled.nodeId)
    await expect(
      enroll(ctx.db, ctx.ca, registryOptions(), {
        token: 'bk1.e30',
        csrPem: key.csrPem,
        facts: facts('j2'),
      }),
    ).rejects.toThrow('not valid')
  })
})
describe.skipIf(!hasDatabase)("the ledger and a node's own answers, against Postgres", () => {
  beforeAll(async () => {
    ;({ db: ctx.db, drop: ctx.drop } = await freshDatabase())
    ctx.ca = await testCa()
  })
  afterAll(() => ctx.drop?.())

  test("a suspended copy holds its memory; one the node's own answer says stopped frees it at once", async () => {
    const a = await enrollNode(ctx.db, ctx.ca, 'answers')
    const power = (workload: string, desired: 'running' | 'stopped') =>
      ctx.db.execute(sql`
        UPDATE fleet_placements SET desired_power = ${desired}, power_changed_at = now() WHERE workload = ${workload}`)
    const observe = (workload: string, state: string) =>
      ctx.db.execute(sql`
        INSERT INTO fleet_observations (node_id, workload, epoch, state, report, state_changed_at)
        VALUES (${a.id}, ${workload}, 1, ${state}, ${JSON.stringify({ id: workload, state })}::jsonb,
                now() - interval '10 minutes')`)
    /** The node's answer to a call about the copy, as far as the ledger reads it. */
    const view = (id: string, epoch: number, state: WorkloadView['state']) =>
      ({ id, epoch, state }) as WorkloadView
    // Asleep with its memory kept.
    await ctx.place('w-s1', a.id, 1)
    await power('w-s1', 'stopped')
    await observe('w-s1', 'suspended')
    // Running, as its node last reported it.
    await ctx.place('w-s2', a.id, 1)
    await power('w-s2', 'running')
    await observe('w-s2', 'running')
    expect(await holdsMemory(ctx.db, 'w-s1')).toBe(true)
    expect(await holdsMemory(ctx.db, 'w-s2')).toBe(true)
    // An answer about another epoch's copy says nothing of this one.
    expect(await answered(ctx.db, a.id, view('w-s2', 2, 'stopped'))).toBe(false)
    expect(await holdsMemory(ctx.db, 'w-s2')).toBe(true)
    // Stopped: the stop's answer counts before the node beats again.
    await power('w-s2', 'stopped')
    expect(await answered(ctx.db, a.id, view('w-s2', 1, 'stopped'))).toBe(true)
    expect(await holdsMemory(ctx.db, 'w-s2')).toBe(false)
    const [observed] = await rows<{ state: string; report: unknown }>(
      ctx.db,
      sql`SELECT state, report FROM fleet_observations WHERE workload = 'w-s2'`,
    )
    expect(observed).toEqual({
      state: 'stopped',
      report: { id: 'w-s2', state: 'stopped' },
    })
    // The same answer again changes nothing.
    expect(await answered(ctx.db, a.id, view('w-s2', 1, 'stopped'))).toBe(false)
    // A start whose answer says the copy didn't stay up gives its claim back too.
    await power('w-s2', 'running')
    expect(await holdsMemory(ctx.db, 'w-s2')).toBe(true)
    expect(await answered(ctx.db, a.id, view('w-s2', 1, 'crashed'))).toBe(true)
    expect(await holdsMemory(ctx.db, 'w-s2')).toBe(false)
    const summary = (await nodeSummaries(ctx.db, THRESHOLDS)).find((n) => n.id === a.id)
    expect(summary?.running).toEqual({
      memoryMb: 1024,
      cpuMillis: 250,
      workloads: 1,
    })
  })
})
