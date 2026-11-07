// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Injected transport, decode and application failures must never acknowledge routes the edge does not hold. */

import { expect, spyOn, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import { HTTP_FRONT_PORT, ROUTES_HELD_HEADER, ROUTES_VERSION_HEADER } from '../../control/src/edge-headers.ts'
import type { EdgeRoutes } from '../../control/src/edge-schemas.ts'
import { RoutesPoll } from '../src/poll.ts'

async function fixture() {
  const { EdgeRoutes } = await import('../../control/src/edge-schemas.ts')
  const routes = EdgeRoutes.parse({
    routes: [
      {
        id: 'sandbox:http',
        sandbox: 'sandbox',
        hostname: 'maple.play.test',
        port: 'http',
        front: 'http',
        publicPort: HTTP_FRONT_PORT,
        destination: '127.0.0.1:8080',
        epoch: 1,
      },
    ],
  })
  const response = (version: string) =>
    Response.json(routes, { headers: { etag: `"${version}"`, [ROUTES_VERSION_HEADER]: version } })
  return { routes, response }
}

test('failed updates keep acknowledging only the last fully applied version, then recover', async () => {
  const { routes, response } = await fixture()
  const asked: Record<string, string>[] = []
  const applied: EdgeRoutes[] = []
  let answer = async () => response('1')
  let apply = async (next: EdgeRoutes) => {
    applied.push(next)
    return true
  }
  const poll = new RoutesPoll(
    async (headers) => {
      asked.push(headers)
      return answer()
    },
    (next) => apply(next),
  )
  expect(await poll.sync()).toBe(true)
  await poll.firstHold
  expect(applied).toEqual([routes])

  const disconnected = new Error('connection closed before the route response')
  answer = async () => {
    throw disconnected
  }
  await expect(poll.sync()).rejects.toBe(disconnected)
  answer = async () => new Response('{')
  await expect(poll.sync()).rejects.toBeInstanceOf(SyntaxError)
  expect(applied).toEqual([routes])

  answer = async () => response('2')
  apply = async () => false
  expect(await poll.sync()).toBe(false)
  const refused = new Error('router refused to install the new destination')
  apply = async () => {
    throw refused
  }
  await expect(poll.sync()).rejects.toBe(refused)
  for (const headers of asked.slice(1)) {
    expect(headers[ROUTES_HELD_HEADER]).toBe('1')
    expect(headers['if-none-match']).toBe('"1"')
  }
  expect(poll.heldOnce()).toBe(true)

  poll.forget()
  apply = async (next) => {
    applied.push(next)
    return true
  }
  expect(await poll.sync()).toBe(true)
  expect(asked.at(-1)?.[ROUTES_HELD_HEADER]).toBe('1')
  expect(asked.at(-1)?.['if-none-match']).toBeUndefined()
  expect(applied).toEqual([routes, routes])
  await poll.sync()
  expect(asked.at(-1)?.[ROUTES_HELD_HEADER]).toBe('2')
  expect(asked.at(-1)?.['if-none-match']).toBe('"2"')
})

test('a rejected first application neither resolves readiness nor claims a held version', async () => {
  const { response } = await fixture()
  const asked: Record<string, string>[] = []
  let accepted = false
  let ready = false
  const poll = new RoutesPoll(
    async (headers) => {
      asked.push(headers)
      return response('1')
    },
    async () => accepted,
  )
  void poll.firstHold.then(() => {
    ready = true
  })
  expect(await poll.sync()).toBe(false)
  await Promise.resolve()
  expect(ready).toBe(false)
  expect(poll.heldOnce()).toBe(false)
  expect(poll.current()).toBe(false)
  accepted = true
  expect(await poll.sync()).toBe(true)
  await poll.firstHold
  expect(ready).toBe(true)
  expect(poll.heldOnce()).toBe(true)
  for (const headers of asked) {
    expect(headers[ROUTES_HELD_HEADER]).toBeUndefined()
    expect(headers['if-none-match']).toBeUndefined()
  }
})

test('an unchanged response refreshes only an already held version and freshness expires after its boundary', async () => {
  const { response } = await fixture()
  let now = Date.UTC(2024, 1, 29)
  const clock = spyOn(Date, 'now').mockImplementation(() => now)
  try {
    let unchanged = true
    let applications = 0
    const poll = new RoutesPoll(
      async () => (unchanged ? new Response(null, { status: StatusCodes.NOT_MODIFIED }) : response('1')),
      async () => {
        applications++
        return true
      },
    )
    expect(await poll.sync()).toBe(false)
    expect(poll.heldOnce()).toBe(false)
    expect(poll.current()).toBe(false)
    expect(applications).toBe(0)
    unchanged = false
    expect(await poll.sync()).toBe(true)
    expect(applications).toBe(1)
    now += SECOND_MS
    expect(poll.current(SECOND_MS)).toBe(true)
    now++
    expect(poll.current(SECOND_MS)).toBe(false)
    unchanged = true
    expect(await poll.sync()).toBe(false)
    expect(poll.current(0)).toBe(true)
    expect(poll.heldOnce()).toBe(true)
    expect(applications).toBe(1)
  } finally {
    clock.mockRestore()
  }
})
