// SPDX-License-Identifier: FSL-1.1-ALv2
/** Live authenticated WebSocket forwarding is owned by the existing SSH lease route. */
import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import { StatusCodes } from 'http-status-codes'
import { agentContext } from './agent-context.ts'

test('HTTP SSH authenticates before wake and revocation closes its exact session', async () => {
  const c = agentContext(),
    session = randomUUID()
  const lease = {
    ...c.route,
    id: `${c.route.sandbox}:ssh-${session}`,
    port: `ssh-${session}`,
    front: 'port' as const,
    publicPort: c.route.publicPort,
    agent: undefined,
    ssh: { session, targetPort: c.route.agent?.manifest.spares[0]?.port as number },
  }
  const path = `/_pryx/ssh/${session}/connect`
  try {
    c.table.apply({ routes: [c.route, lease] })
    expect((await fetch(c.url + path)).status).toBe(StatusCodes.UNAUTHORIZED)
    expect(c.wakes).toBe(0)
    const socket = new WebSocket((c.url + path).replace('http:', 'ws:'), ['pryx', `pryx-token.${c.token()}`])
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true })
      socket.addEventListener('error', () => reject(new Error('SSH websocket did not open')), { once: true })
    })
    const closed = new Promise<void>((resolve) =>
      socket.addEventListener('close', () => resolve(), { once: true }),
    )
    c.table.apply({ routes: [c.route] })
    await closed
    expect(socket.readyState).toBe(WebSocket.CLOSED)
    expect((await fetch(c.url + path, { headers: { authorization: `Bearer ${c.token()}` } })).status).toBe(
      StatusCodes.GONE,
    )
    c.table.apply({ routes: [{ ...c.route, state: 'asleep' }] })
    expect((await fetch(c.url + path, { headers: { authorization: `Bearer ${c.token()}` } })).status).toBe(
      StatusCodes.GONE,
    )
    expect(c.wakes).toBe(0)
  } finally {
    c.close()
  }
})
