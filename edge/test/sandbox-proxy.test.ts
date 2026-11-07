// SPDX-License-Identifier: FSL-1.1-ALv2
/** Agent authentication precedes wake and websocket relays preserve the first output frame. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { sandboxToken } from '../src/sandbox-token.ts'
import { agentContext } from './agent-context.ts'

test('invalid, expired, foreign and route-only capabilities cannot wake an agent', async () => {
  const c = agentContext()
  c.table.apply({ routes: [{ ...c.route, state: 'asleep' }] })
  try {
    const now = Math.floor(Date.now() / SECOND_MS)
    for (const token of [
      '',
      c.token({ sandbox: 'other' }),
      c.token({ epoch: 2 }),
      c.token({ expiresAt: now }),
      c.token({ scope: 'routes' }),
    ]) {
      const response = await c.proxy.fetch(
        new Request(`${c.url}/_pryx/health`, { headers: { authorization: `Bearer ${token}` } }),
      )
      expect(response?.status).toBe(StatusCodes.UNAUTHORIZED)
    }
    expect(c.wakes).toBe(0)
    expect(c.calls).toHaveLength(0)
    const response = await c.proxy.fetch(
      new Request(`${c.url}/_pryx/health`, { headers: { authorization: `Bearer ${c.token()}` } }),
    )
    expect(response?.status).toBe(StatusCodes.OK)
    await response?.text()
    expect(c.wakes).toBe(1)
    expect(sandboxToken(c.token(), { ...c.route, epoch: 2 })).toBeUndefined()
    expect(c.table.find(c.route.hostname, 'http', 80)).toBeUndefined()
  } finally {
    c.close()
  }
})

test('an exposed spare is public and the SDK terminal keeps initial output and stdin', async () => {
  const c = agentContext()
  const client = new Pryx({
    url: 'http://control',
    key: 'project-secret',
    fetch: (async (input, init) => {
      if (String(input).endsWith('/token'))
        return Response.json({
          token: c.token(),
          expiresAt: Date.now() / SECOND_MS + 60,
          url: c.url,
          publicKey: c.route.agent?.manifest.publicKey,
          epoch: 1,
        })
      if (String(input).startsWith('http://control/'))
        return Response.json({ id: 'one', state: 'running', endpoints: [] })
      return fetch(input, init)
    }) as typeof fetch,
  })
  try {
    const agent = await client.sandbox('one')
    expect(await agent.expose(3000)).toBe('https://one-3000.sandboxes.test')
    const response = await c.proxy.fetch(new Request('http://one-3000.sandboxes.test/'))
    expect(await response?.text()).toBe('exposed-app')
    const alias = await c.proxy.fetch(new Request(`${c.url}/ports/3000/`))
    expect(await alias?.text()).toBe('exposed-app')
    const unavailable = await c.proxy.fetch(new Request('http://one-3001.sandboxes.test/'))
    expect(unavailable?.status).toBe(StatusCodes.NOT_FOUND)
    const terminal = await agent.terminal({ cols: 120, rows: 40 })
    const output = terminal.stream()
    expect((await output.next()).value?.data).toBe('prompt\n')
    terminal.stdin.write('hello\n')
    expect((await output.next()).value?.data).toBe('hello\n')
    expect(c.frames).toContainEqual({ type: 'resize', cols: 120, rows: 40 })
    terminal.close()
    await output.return(undefined)
  } finally {
    c.close()
  }
})
