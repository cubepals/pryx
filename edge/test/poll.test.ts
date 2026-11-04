// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** What a running edge says of its routes, as the control plane answers its polls or doesn't. */
import { expect, test } from 'bun:test'
import { ROUTES_VERSION_HEADER } from '../../packages/contracts/src/edge-poll.ts'
import { RoutesPoll } from './poll.ts'

const list = () =>
  new Response(JSON.stringify({ routes: [] }), {
    headers: { etag: '"a"', [ROUTES_VERSION_HEADER]: '1' },
  })

test('holds routes once a list is applied, and keeps doing so while the control plane is unreachable', async () => {
  let answer: () => Promise<Response> = async () => list()
  const poll = new RoutesPoll(
    () => answer(),
    async () => true,
  )
  expect(poll.heldOnce()).toBe(false)
  expect(poll.current()).toBe(false)

  await poll.sync()
  expect(poll.heldOnce()).toBe(true)
  expect(poll.current()).toBe(true)

  answer = async () => {
    throw new Error('connect ECONNREFUSED')
  }
  await expect(poll.sync()).rejects.toThrow('ECONNREFUSED')
  answer = async () => new Response(null, { status: 503 })
  await expect(poll.sync()).rejects.toThrow('503')
  expect(poll.heldOnce()).toBe(true)
})

test('is not held while the first list is not applied', async () => {
  const poll = new RoutesPoll(
    async () => list(),
    async () => false,
  )
  await poll.sync()
  expect(poll.heldOnce()).toBe(false)
})
