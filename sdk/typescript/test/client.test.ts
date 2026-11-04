// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Exercise streaming UTF-8 boundaries and HTTP errors without a real node. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { ndjson, Pryx, PryxError } from '../src/index.ts'

test('NDJSON carries split UTF-8, final lines and cancels its source', async () => {
  const bytes = new TextEncoder().encode('{"data":"é"}\n{"last":true}')
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      for (const byte of bytes) c.enqueue(new Uint8Array([byte]))
      c.close()
    },
  })
  expect(await Array.fromAsync(ndjson(new Response(stream)))).toEqual([{ data: 'é' }, { last: true }])
})
test('a refused mutation carries its envelope and is sent once', async () => {
  let count = 0
  const client = new Pryx({
    baseUrl: 'http://example.test',
    apiKey: 'test',
    fetch: (async (request, options) => {
      count++
      expect(String(request)).toEndWith('/v1/sandboxes')
      expect(new Headers(options?.headers).get('authorization')).toBe('Bearer test')
      return Response.json(
        { error: { code: 'no_room', message: 'No room' } },
        { status: StatusCodes.SERVICE_UNAVAILABLE },
      )
    }) as typeof fetch,
  })
  await expect(client.make({ image: 'alpine' })).rejects.toBeInstanceOf(PryxError)
  expect(count).toBe(1)
})
