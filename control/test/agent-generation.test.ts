// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real Postgres acknowledgments and signed HTTP peers model a placement changing while
 * capability issuance waits for the edge. Kernel recovery remains an end-to-end CI check. */
import { expect, test } from 'bun:test'
import { sql } from 'drizzle-orm'
import { agentContext } from '../../edge/test/agent-context.ts'
import nodeContract from '../../openapi/node-writes.openapi.json'
import { SECOND_MS } from '../src/durations.ts'
import { sawEdge } from '../src/edge-routed.ts'
import { EdgeService } from '../src/edge-service.ts'
import { decodeHandle, encodeHandle } from '../src/handle.ts'
import { heartbeat } from '../src/registry.ts'
import { beat, enrollNode, registryOptions, testCa } from './nodes.ts'
import { publicContext } from './public-context.ts'

for (const timing of ['during the wait', 'before the barrier'] as const)
  test(
    `a capability follows a placement changed ${timing}`,
    async () => {
      let waiting = false
      const pending = Promise.withResolvers<void>()
      const observed = Promise.withResolvers<void>()
      const release = Promise.withResolvers<void>()
      const c = await publicContext({
        logQuery(query) {
          if (waiting && query.includes('AND NOT b.internal')) pending.resolve()
        },
      })
      let peer: ReturnType<typeof agentContext> | undefined
      try {
        const made = await (await c.request('/sandboxes', 'POST', { image: 'alpine:3.22' })).json()
        const old = c.peers.get(made.id)
        const row = await c.boxes.record(made.id)
        if (!old || !row.handle) throw new Error('Missing old placement peer')
        const ref = decodeHandle(row.handle)
        const target = await enrollNode(c.db, await testCa(), 'recovery-target', {
          region: 'default',
          apiAddress: '10.0.0.2:7443',
        })
        await heartbeat(c.db, registryOptions(), target.id, target.sha, beat(target, 'target', 1))
        await c.boxes.engine.refreshRegistry()
        peer = agentContext(made.id)
        peer.route.epoch = ref.epoch + 1
        const current = peer
        const manifest = current.route.agent?.manifest
        if (!manifest) throw new Error('Missing target placement agent')
        c.agents.localOrigin = new URL(current.url).origin
        c.nodes.agentToken = () => {
          const epoch = c.nodes.calls.at(-1)?.epoch ?? ref.epoch
          const issuer = epoch === ref.epoch ? old : current
          return {
            token: issuer.token({ epoch }),
            expiresAt: Math.floor(Date.now() / SECOND_MS) + nodeContract['x-pryx-agent'].tokenLifetimeSeconds,
            epoch,
            agent: issuer.route.agent?.manifest,
          }
        }
        const service = new EdgeService(c.db, c.boxes, 'sandboxes.test', undefined, c.agents)
        const before = await service.routes(true)
        await sawEdge(c.db, 'generation-ack', before.version)
        // This committed lifecycle change requires a new edge acknowledgment.
        await c.db.execute(
          sql`UPDATE sandboxes SET route_listed_version=NULL,updated_at=clock_timestamp() WHERE id=${made.id}`,
        )
        if (timing === 'before the barrier') {
          const record = c.boxes.record.bind(c.boxes)
          let paused = false
          c.boxes.record = async (...args) => {
            const row = await record(...args)
            if (waiting && !paused) {
              paused = true
              observed.resolve()
              await release.promise
            }
            return row
          }
        }
        let issued = false
        waiting = true
        const issuance = c.agents.token(made.id, 'default', 'http://127.0.0.1/v1').then((answer) => {
          issued = true
          return answer
        })
        if (timing === 'before the barrier') await observed.promise
        else {
          await pending.promise
          await Bun.sleep(SECOND_MS / 20)
        }
        expect(issued).toBe(false)
        const handle = encodeHandle({
          ...ref,
          node: target.id,
          epoch: current.route.epoch,
          agent: manifest,
        })
        await acknowledgePlacement(c, service, made.id, handle, current)
        release.resolve()
        const answer = await issuance
        expect(answer.epoch).toBe(current.route.epoch)
        expect(answer.publicKey).toBe(manifest.publicKey)
        expect(current.calls).toContain('GET /_pryx/health')
      } finally {
        release.resolve()
        peer?.close()
        await c.close()
      }
    },
    10 * SECOND_MS,
  )

async function acknowledgePlacement(
  c: Awaited<ReturnType<typeof publicContext>>,
  service: EdgeService,
  id: string,
  handle: ReturnType<typeof encodeHandle>,
  peer: ReturnType<typeof agentContext>,
) {
  const ref = decodeHandle(handle)
  await c.db.transaction(async (t) => {
    await t.execute(
      sql`UPDATE fleet_placements SET node_id=${ref.node},epoch=${ref.epoch},handle=${handle} WHERE workload=${id}`,
    )
    await t.execute(
      sql`UPDATE sandboxes SET handle=${handle},route_listed_version=NULL,updated_at=clock_timestamp() WHERE id=${id}`,
    )
  })
  const after = await service.routes(true)
  peer.table.apply(after)
  await sawEdge(c.db, 'generation-ack', after.version)
}
