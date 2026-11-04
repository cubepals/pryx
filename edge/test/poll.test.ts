// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { expect, test } from 'bun:test'
/** What a running edge says of its routes, as the control plane answers its polls or doesn't. */
import { StatusCodes } from 'http-status-codes'
import { ROUTES_VERSION_HEADER } from '../../control/src/edge-headers.ts'
import { RoutesPoll } from '../src/poll.ts'

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
  answer = async () => new Response(null, { status: StatusCodes.SERVICE_UNAVAILABLE })
  await expect(poll.sync()).rejects.toThrow(String(StatusCodes.SERVICE_UNAVAILABLE))
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
