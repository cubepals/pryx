// SPDX-License-Identifier: FSL-1.1-ALv2
/** Direct SDK requests retain command identity, cache scoped tokens and switch file transports after lifecycle changes. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { SDK_DEFAULTS } from '../src/generated/defaults.ts'
import { Pryx, PryxError } from '../src/index.ts'

test('agent run is live, caches its token and keeps the project key off sandbox requests', async () => {
  const calls: Array<{ path: string; auth: string | null; key: string | null; body: unknown }> = []
  const view = { id: 'one', state: 'running', agent: { url: 'http://agent' }, files: true, endpoints: [] }
  let finish: () => void = () => {}
  const gated = new Promise<void>((resolve) => {
    finish = resolve
  })
  const result = {
    code: 0,
    timedOut: false,
    killed: false,
    durationMs: 1,
    stdoutTruncated: false,
    stderrTruncated: false,
  }
  const client = new Pryx({
    url: 'http://control',
    key: 'project-secret',
    fetch: (async (input, init) => {
      const url = new URL(String(input)),
        headers = new Headers(init?.headers)
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined
      calls.push({
        path: url.pathname,
        auth: headers.get('authorization'),
        key: headers.get('idempotency-key'),
        body,
      })
      if (url.hostname === 'control') {
        if (url.pathname.endsWith('/token'))
          return Response.json({
            token: 'sandbox-token',
            expiresAt: Date.now() / SDK_DEFAULTS.secondMs + SDK_DEFAULTS.tokenRenewBeforeSeconds + 60,
            epoch: 1,
            url: 'http://agent',
            publicKey: 'public',
          })
        if (url.pathname === '/v1/sandboxes') return Response.json(view)
        return Response.json({ sandbox: view })
      }
      if (url.pathname === '/_pryx/processes')
        return Response.json({ id: 'process', pid: 123, running: true, result: null })
      if (url.pathname.endsWith('/output'))
        return new Response(
          new ReadableStream({
            async start(controller) {
              const send = (event: unknown) =>
                controller.enqueue(new TextEncoder().encode(`${JSON.stringify(event)}\n`))
              send({ type: 'output', stream: 'stdout', bytes: btoa('first\n') })
              await gated
              send({ type: 'output', stream: 'stderr', bytes: btoa('last\n') })
              send({ type: 'exit', result })
              controller.close()
            },
          }),
        )
      return Response.json(result)
    }) as typeof fetch,
  })
  const box = await client.sandbox({ image: 'scratch' })
  const run = box.stream('printf first', { idempotencyKey: 'run-key', env: { CI: '1' } })
  const iterator = run.lines()
  expect(await iterator.next()).toEqual({ done: false, value: 'first' })
  expect(calls.filter((c) => c.path === '/_pryx/processes')).toHaveLength(1)
  finish()
  expect(await iterator.next()).toEqual({ done: false, value: 'last' })
  await iterator.next()
  expect((await run.result()).code).toBe(0)
  await box.spawn(['echo', 'again'])
  expect(calls.filter((c) => c.path.endsWith('/token'))).toHaveLength(1)
  const starts = calls.filter((c) => c.path === '/_pryx/processes')
  expect(starts[0]?.key).toBe('run-key')
  expect(starts[0]?.body).toEqual({
    command: 'printf first',
    env: { CI: '1' },
    timeout: SDK_DEFAULTS.runTimeoutSeconds,
  })
  expect(
    calls.filter((c) => c.path.startsWith('/_pryx/')).every((c) => c.auth === 'Bearer sandbox-token'),
  ).toBe(true)
  await box.start()
  expect(calls.some((c) => c.path === '/v1/sandboxes/one/start')).toBe(true)
})

test('agent mutations are sent once and preserve structured refusal', async () => {
  let starts = 0
  const client = new Pryx({
    url: 'http://control',
    key: 'project-secret',
    fetch: (async (input) => {
      if (String(input).endsWith('/token'))
        return Response.json({
          token: 'sandbox',
          expiresAt: Date.now() / SDK_DEFAULTS.secondMs + 60,
          url: 'http://agent',
          epoch: 1,
          publicKey: 'key',
        })
      if (String(input).includes('/v1/sandboxes/'))
        return Response.json({ id: 'one', state: 'running', endpoints: [] })
      starts++
      return Response.json(
        { error: { code: 'no_spare_port', message: 'No spare port' } },
        { status: StatusCodes.CONFLICT },
      )
    }) as typeof fetch,
  })
  await expect((await client.sandbox('one')).expose(3000)).rejects.toBeInstanceOf(PryxError)
  expect(starts).toBe(1)
})

test('file streams use the agent while running and node data after stop', async () => {
  const writes: Array<{ host: string; path: string; data: Uint8Array<ArrayBuffer> }> = []
  const view = { id: 'one', state: 'running', agent: { url: 'http://agent' }, files: true, endpoints: [] }
  const client = new Pryx({
    url: 'http://control',
    key: 'project-secret',
    fetch: (async (input, init) => {
      const url = new URL(String(input))
      if (url.pathname.endsWith('/token'))
        return Response.json({
          token: 'sandbox',
          expiresAt: Date.now() / SDK_DEFAULTS.secondMs + 60,
          url: 'http://agent',
          epoch: 1,
          publicKey: 'key',
        })
      if (url.pathname === '/v1/sandboxes') return Response.json(view)
      if (url.pathname.endsWith('/stop')) return Response.json({ sandbox: { ...view, state: 'stopped' } })
      if (init?.method === 'PUT') {
        writes.push({
          host: url.host,
          path: url.pathname,
          data: new Uint8Array(await new Response(init.body).arrayBuffer()),
        })
        return new Response(null, { status: StatusCodes.NO_CONTENT })
      }
      return new Response(writes.at(-1)?.data)
    }) as typeof fetch,
  })
  const box = await client.sandbox({ image: 'scratch' })
  const bytes = new Uint8Array(SDK_DEFAULTS.fileChunkBytes * 4 + 7).fill(42)
  await box.write(
    'big',
    new ReadableStream({
      start(controller) {
        controller.enqueue(bytes)
        controller.close()
      },
    }),
  )
  expect(writes[0]?.host).toBe('agent')
  expect(await box.readBytes('big')).toEqual(bytes)
  await box.stop()
  await box.write('big', bytes)
  expect(writes[1]?.path).toBe('/v1/sandboxes/one/data/content')
  expect(writes[1]?.data).toEqual(bytes)
})
