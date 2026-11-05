// SPDX-License-Identifier: FSL-1.1-ALv2
/** A real control ledger and edge hold preserve the agent route while waking the placement.
 * The node and agent peers answer the protocol; kernel/container resume belongs to CI. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { Hold } from '../../edge/src/hold.ts'
import { SandboxProxy } from '../../edge/src/sandbox-proxy.ts'
import { agentContext } from '../../edge/test/agent-context.ts'
import { SECOND_MS } from '../src/durations.ts'
import { EdgeService } from '../src/edge-service.ts'
import { decodeHandle, encodeHandle } from '../src/handle.ts'
import { jsonb } from '../src/sql.ts'
import type { WorkloadView } from '../src/wire.ts'
import { publicContext } from './public-context.ts'

test('a sleeping agent request wakes through control and retains its capability', async () => {
  const c = await publicContext()
  const made = (await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()) as {
    id: string
  }
  const peer = agentContext(made.id)
  let proxy: SandboxProxy | undefined
  try {
    const row = await c.boxes.record(made.id)
    if (!row.handle || !peer.route.agent) throw new Error('Missing placement')
    const ref = decodeHandle(row.handle),
      handle = encodeHandle({ ...ref, agent: peer.route.agent.manifest })
    await c.db.execute(
      sql`UPDATE sandboxes SET handle=${handle}, spec=spec||${jsonb({ agent: { domain: 'sandboxes.test', keepAlive: true } })} WHERE id=${made.id}`,
    )
    await c.db.execute(sql`UPDATE fleet_placements SET handle=${handle} WHERE workload=${made.id}`)
    await c.db.execute(sql`UPDATE fleet_nodes SET features=features||${jsonb(['agent'])}`)
    await c.boxes.engine.refreshRegistry()
    const tokens = () =>
      c.nodes.once(/POST .*\/agent-token$/, async () => {
        tokens()
        const body = c.nodes.calls.at(-1)?.body as { scope?: string }
        return {
          token: peer.token({ scope: body.scope ?? 'sandbox' }),
          expiresAt: Date.now() / SECOND_MS + 60,
          epoch: ref.epoch,
          agent: peer.route.agent?.manifest,
        }
      })
    tokens()
    const puts = () =>
      c.nodes.once(/PUT .*\/workloads\//, async (answer) => {
        puts()
        const response = answer() as { workload: WorkloadView }
        response.workload.agent = peer.route.agent?.manifest
        return response
      })
    puts()
    const service = new EdgeService(c.db, c.boxes, 'sandboxes.test', undefined, c.agents)
    await c.boxes.sleep(made.id)
    const asleep = await service.routes(true)
    peer.table.apply(asleep)
    proxy = new SandboxProxy(peer.table, new Hold((id) => service.wake(id)), 5 * SECOND_MS)
    const response = await proxy.fetch(
      new Request(`http://127.0.0.1/sandboxes/${made.id}/_pryx/processes`, {
        method: 'POST',
        headers: { authorization: `Bearer ${peer.token()}`, 'content-type': 'application/json' },
        body: JSON.stringify({ command: 'true' }),
      }),
    )
    expect(response?.status).toBe(StatusCodes.CREATED)
    await response?.text()
    expect((await c.boxes.record(made.id)).state).toBe('running')
    expect(decodeHandle((await c.boxes.record(made.id)).handle as string).agent?.publicKey).toBe(
      peer.route.agent.manifest.publicKey,
    )
  } finally {
    proxy?.close()
    peer.close()
    await c.close()
  }
})
