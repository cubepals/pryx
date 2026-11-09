// SPDX-License-Identifier: FSL-1.1-ALv2
/** Kernel streams retain rich displays and errors, and interrupted mutations are sent once. */
import { expect, test } from 'bun:test'
import { SECOND_MS } from '../../../control/src/durations.ts'
import { SDK_DEFAULTS } from '../src/generated/defaults.ts'
import { KERNEL_DEFAULTS } from '../src/generated/kernels.ts'
import { Pryx } from '../src/index.ts'

function fixture(events: unknown[]) {
  const calls: { path: string; body: unknown; auth: string | null }[] = []
  const view = { id: 'kernel', language: 'python', context: '', processId: 'process', pid: 12, state: 'idle' }
  const client = new Pryx({
    url: 'http://control.test',
    key: 'project',
    fetch: (async (input, init) => {
      const url = new URL(String(input))
      calls.push({
        path: url.pathname,
        body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
        auth: new Headers(init?.headers).get('authorization'),
      })
      if (url.pathname.endsWith('/token'))
        return Response.json({
          token: 'scoped',
          url: 'http://agent.test',
          expiresAt: Date.now() / SECOND_MS + SDK_DEFAULTS.tokenRenewBeforeSeconds + 60,
          epoch: 1,
        })
      if (url.pathname === '/_pryx/health') return Response.json({ kernelsVersion: KERNEL_DEFAULTS.version })
      if (url.pathname.endsWith('/execute'))
        return new Response(events.map((event) => JSON.stringify(event)).join('\n') + '\n')
      if (url.pathname === '/_pryx/kernels' || url.pathname.endsWith('/restart')) return Response.json(view)
      return Response.json({
        sandbox: {
          id: 'one',
          state: 'running',
          agent: { url: 'http://agent.test' },
          files: true,
          endpoints: [],
        },
      })
    }) as typeof fetch,
  })
  return { client, calls }
}
test('rich results preserve every MIME and image bytes separately from output', async () => {
  const { client, calls } = fixture([
    { type: 'stream', stream: 'stdout', text: 'hello' },
    {
      type: 'display',
      data: {
        'image/png': btoa('\x89PNG\r\n\x1a\n'),
        'image/svg+xml': '<svg/>',
        'text/html': '<table/>',
        'application/json': [{ x: 4 }],
      },
      metadata: {},
      displayId: null,
      update: false,
    },
    { type: 'result', data: { 'text/plain': '4', 'application/json': 4 }, metadata: {}, executionCount: 1 },
    { type: 'done', executionCount: 1, status: 'ok' },
  ])
  const py = await (await client.sandbox('one')).python({ context: 'analysis' })
  const result = await py.run('4')
  expect(result.text).toBe('4')
  expect(result.json).toBe(4)
  expect(result.html).toBe('<table/>')
  expect(result.svg).toBe('<svg/>')
  expect(result.png?.slice(0, 4)).toEqual(new Uint8Array([137, 80, 78, 71]))
  expect(result.stdout).toBe('hello')
  expect(result.results).toHaveLength(2)
  expect(calls.find((call) => call.path === '/_pryx/kernels')?.body).toEqual({
    language: 'python',
    context: 'analysis',
  })
  // The agent picks the timeout within the token's ceiling unless the caller names one.
  expect(calls.find((call) => call.path.endsWith('/execute'))?.body).toEqual({ code: '4' })
  await py.run('4', { timeout: '5s' })
  expect(calls.filter((call) => call.path.endsWith('/execute'))[1]?.body).toEqual({ code: '4', timeout: 5 })
  expect(
    calls.filter((call) => call.path.startsWith('/_pryx/')).every((call) => call.auth === 'Bearer scoped'),
  ).toBe(true)
})
test('structured errors survive while missing completion is an unknown outcome', async () => {
  const { client } = fixture([
    { type: 'error', name: 'ValueError', value: 'bad', traceback: ['frame'] },
    { type: 'done', executionCount: 2, status: 'error' },
  ])
  const result = await (await (await client.sandbox('one')).python()).run('raise ValueError()')
  expect(result.error).toEqual({ name: 'ValueError', value: 'bad', traceback: ['frame'] })
  for (const events of [
    [],
    [{ type: 'done', status: 'ok' }],
    [{ type: 'stream', stream: 'unknown', text: '' }],
  ]) {
    const { client, calls } = fixture(events)
    const py = await (await client.sandbox('one')).python()
    await expect(py.run('mutate()')).rejects.toThrow()
    expect(calls.filter((call) => call.path.endsWith('/execute'))).toHaveLength(1)
  }
})
