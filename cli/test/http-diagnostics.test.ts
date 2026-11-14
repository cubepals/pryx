// SPDX-License-Identifier: FSL-1.1-ALv2
/** Phase tracing retains single-call outcomes while keeping credentials and request data private.
 * Actual request and response semantics remain the SDK's responsibility. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { diagnosticFetch } from '../src/http-diagnostics.ts'

test('HTTP diagnostics disclose only phases and never replay a failed request', async () => {
  const events: Record<string, string | number>[] = []
  let calls = 0
  const failure = Object.assign(new Error('private native error message'), { code: 'ECONNRESET' })
  const response = new Response('private response', { status: StatusCodes.OK })
  const original = Object.assign(
    async () => {
      if (++calls === 2) throw failure
      return response
    },
    { preconnect: fetch.preconnect },
  ) as typeof fetch
  const traced = diagnosticFetch((event) => events.push(event), original)
  expect(
    await traced('http://private-user:private-password@localhost/v1/sandboxes?secret=private-query', {
      method: 'POST',
      headers: { authorization: 'Bearer private-project-key' },
      body: 'private-request-body',
    }),
  ).toBe(response)
  let thrown: unknown
  try {
    await traced('http://localhost/sandboxes/private-id/_pryx/processes', {
      method: 'POST',
      headers: { authorization: 'Bearer private-agent-key' },
    })
  } catch (error) {
    thrown = error
  }
  expect(thrown).toBe(failure)
  expect(calls).toBe(2)
  expect(events).toEqual([
    { request: 1, scope: 'public', method: 'POST', phase: 'start' },
    { request: 1, scope: 'public', method: 'POST', phase: 'response', status: StatusCodes.OK },
    { request: 2, scope: 'agent', method: 'POST', phase: 'start' },
    { request: 2, scope: 'agent', method: 'POST', phase: 'failed', code: 'ECONNRESET' },
  ])
  expect(JSON.stringify(events)).not.toContain('private')
})

test('unknown method and error-code values cannot disclose arbitrary input', async () => {
  const events: Record<string, string | number>[] = []
  const failure = Object.assign(new Error('PRIVATE_MESSAGE'), { code: 'PRIVATE_CODE' })
  let calls = 0
  const original = Object.assign(
    async () => {
      calls++
      throw failure
    },
    { preconnect: fetch.preconnect },
  ) as typeof fetch
  const traced = diagnosticFetch((event) => events.push(event), original)
  await expect(traced('http://localhost/private', { method: 'PRIVATE_METHOD' })).rejects.toBe(failure)
  expect(calls).toBe(1)
  expect(events).toEqual([
    { request: 1, scope: 'agent', method: 'OTHER', phase: 'start' },
    { request: 1, scope: 'agent', method: 'OTHER', phase: 'failed', code: 'transport_error' },
  ])
  expect(JSON.stringify(events)).not.toContain('PRIVATE')
})
