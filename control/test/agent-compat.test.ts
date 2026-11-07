// SPDX-FileCopyrightText: 2026 The Cubepals Authors
/** The compatibility API persists exact outcomes on real Postgres and traverses the real edge
 * relay to an authenticated peer, preserving raw exec and log records for previous clients. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { agentContext } from '../../edge/test/agent-context.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle, encodeHandle } from '../src/handle.ts'
import { publicContext } from './public-context.ts'

test('public exec forwards through the agent and replays its durable cached outcome', async () => {
  const c = await publicContext()
  const made = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
    id: string
  }
  const peer = agentContext(made.id)
  try {
    const row = await c.boxes.record(made.id)
    if (!row.handle || !peer.route.agent) throw new Error('Missing fixture agent')
    const ref = decodeHandle(row.handle),
      handle = encodeHandle({ ...ref, agent: peer.route.agent.manifest })
    await c.db.execute(sql`UPDATE sandboxes SET handle=${handle} WHERE id=${made.id}`)
    await c.db.execute(sql`UPDATE fleet_nodes SET features='["agent"]'::jsonb`)
    await c.boxes.engine.refreshRegistry()
    c.agents.localOrigin = peer.url.slice(0, peer.url.indexOf('/sandboxes/'))
    const token = () =>
      c.nodes.once(/POST .*\/agent-token$/, async () => ({
        token: peer.token(),
        expiresAt: Date.now() / SECOND_MS + 60,
        epoch: ref.epoch,
        agent: peer.route.agent?.manifest,
      }))
    const sdk = new Pryx({
      url: 'http://127.0.0.1',
      key: c.credential.key,
      fetch: ((input, init) => c.app.request(String(input), init)) as typeof fetch,
    })
    peer.output.unshift({
      type: 'output',
      stream: 'stdout',
      bytes: Buffer.from('hello\n').toString('base64'),
    })
    token()
    const first = await Array.fromAsync(sdk.api.exec(made.id, { command: ['echo', 'hello'] }, 'same-key'))
    expect(first).toContainEqual({ type: 'stdout', data: 'hello\n', truncated: false })
    expect(first.at(-1)).toMatchObject({ type: 'exit', code: 0 })
    const starts = peer.calls.filter((c) => c === 'POST /_pryx/processes').length
    expect(await Array.fromAsync(sdk.api.exec(made.id, { command: ['echo', 'hello'] }, 'same-key'))).toEqual(
      first,
    )
    expect(peer.calls.filter((c) => c === 'POST /_pryx/processes')).toHaveLength(starts)
    expect(
      c.nodes.calls.some((call) => call.path.endsWith('/exec') || call.path.endsWith('/exec-stream')),
    ).toBe(false)
    const at = Date.now()
    peer.logFrames.push({
      type: 'replay',
      chunks: [{ at, stream: 'stdout', bytes: Buffer.from('first\nlast\n').toString('base64') }],
    })
    token()
    const logs = await Array.fromAsync(sdk.api.logs(made.id, { tail: 1 }))
    expect(logs).toEqual([{ ts: new Date(at).toISOString(), stream: 'stdout', line: 'last' }])
    token()
    expect(await Array.fromAsync(sdk.api.logs(made.id, { since: new Date(at + 1).toISOString() }))).toEqual(
      [],
    )
    expect(c.nodes.calls.some((call) => call.path.includes('/logs'))).toBe(false)
  } finally {
    peer.close()
    await c.close()
  }
})
