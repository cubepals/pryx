// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real Postgres query accounting surrounds production SDK and edge requests after the real
 * public API issues a token. The peer supplies protocol responses; Rust tests execute processes. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { agentContext } from '../../edge/test/agent-context.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle, encodeHandle } from '../src/handle.ts'
import { jsonb } from '../src/sql.ts'
import { publicContext } from './public-context.ts'

test('run, start, terminal, files and expose issue zero control queries after token warmup', async () => {
  let queries = 0,
    controlCalls = 0
  const c = await publicContext({
    logQuery: () => {
      queries++
    },
  })
  const made = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
    id: string
  }
  const peer = agentContext(made.id)
  try {
    const row = await c.boxes.record(made.id)
    if (!row.handle || !peer.route.agent) throw new Error('Missing fixture placement')
    const ref = decodeHandle(row.handle)
    const handle = encodeHandle({ ...ref, agent: peer.route.agent.manifest })
    await c.db.execute(
      sql`UPDATE sandboxes SET handle=${handle}, spec=spec||${jsonb({ sparePorts: 1 })} WHERE id=${made.id}`,
    )
    await c.db.execute(sql`UPDATE fleet_nodes SET features=features||${jsonb(['agent'])}`)
    await c.boxes.engine.refreshRegistry()
    c.nodes.once(/POST .*\/agent-token$/, async () => ({
      token: peer.token(),
      expiresAt: Math.floor(Date.now() / SECOND_MS) + 60,
      epoch: ref.epoch,
      agent: peer.route.agent?.manifest,
    }))
    const sdk = new Pryx({
      url: 'http://control.test',
      key: c.credential.key,
      fetch: (async (input, init) => {
        if (new URL(String(input)).hostname !== 'control.test') return fetch(input, init)
        controlCalls++
        const response = await c.app.request(String(input), init)
        // Only the ephemeral edge port differs from the deployed hostname/port 80.
        if (String(input).endsWith('/token') && response.ok)
          return Response.json({ ...(await response.json()), url: peer.url })
        return response
      }) as typeof fetch,
    })
    const box = await sdk.sandbox(made.id)
    await box.agent.token()
    expect(queries).toBeGreaterThan(0)
    queries = 0
    controlCalls = 0
    expect((await box.run('true')).code).toBe(0)
    const process = await box.start('cat', { stdin: true })
    await process.stdin.write('hello\n')
    await process.stdin.close()
    expect((await process.wait()).code).toBe(0)
    const terminal = await box.terminal()
    const output = terminal.stream()
    expect((await output.next()).value?.data).toBe('prompt\n')
    terminal.stdin.write('hello\n')
    expect((await output.next()).value?.data).toBe('hello\n')
    terminal.close()
    await output.return(undefined)
    await box.write('data', new Uint8Array([1, 2, 3]))
    expect(await box.readBytes('data')).toEqual(new Uint8Array([1, 2, 3]))
    expect(await box.files.list('.')).toHaveLength(1)
    await box.expose(3000)
    expect(queries).toBe(0)
    expect(controlCalls).toBe(0)
  } finally {
    peer.close()
    await c.close()
  }
})
