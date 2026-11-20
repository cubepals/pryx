// SPDX-License-Identifier: FSL-1.1-ALv2
/** Route delivery and optional capability recovery across a stopped node response, with real Postgres and signed HTTP peers. */
import { expect, test } from 'bun:test'
import { setTimeout as pause } from 'node:timers/promises'
import { sql } from 'drizzle-orm'
import { StatusCodes } from 'http-status-codes'
import { Hold } from '../../edge/src/hold.ts'
import { RoutesPoll } from '../../edge/src/poll.ts'
import { RouteTable } from '../../edge/src/routes.ts'
import { SandboxProxy } from '../../edge/src/sandbox-proxy.ts'
import { SECOND_MS } from '../src/durations.ts'
import { edgeApi } from '../src/edge-api.ts'
import { AGENT_ROUTE_PORT, EDGE_FEATURES_HEADER, EDGE_ID_HEADER } from '../src/edge-headers.ts'
import { EdgeService } from '../src/edge-service.ts'
import { one } from '../src/sql.ts'
import { publicContext } from './public-context.ts'

test(
  'a pending container capability cannot hide a process route and appears once it recovers',
  async () => {
    const c = await publicContext()
    const held = Promise.withResolvers<void>(),
      started = Promise.withResolvers<void>()
    const table = new RouteTable()
    const proxy = new SandboxProxy(
      table,
      new Hold(async () => ({ outcome: 'denied', reason: 'unknown' })),
      SECOND_MS,
    )
    let first: Promise<boolean> | undefined
    try {
      await c.db.execute(
        sql`UPDATE fleet_nodes SET features=features || '["multi-tier","tier-container","tier-process","tier-process:agent"]'::jsonb`,
      )
      await c.boxes.engine.refreshRegistry()
      const container = await c.request('/sandboxes', 'POST', {
        isolation: 'container',
        image: 'alpine:3.22',
      })
      const process = await c.request('/sandboxes', 'POST', { isolation: 'process' })
      expect(container.status).toBe(StatusCodes.CREATED)
      expect(process.status).toBe(StatusCodes.CREATED)
      const { id: containerId } = (await container.json()) as { id: string }
      const { id: processId, isolation } = (await process.json()) as { id: string; isolation: string }
      expect(isolation).toBe('process')
      c.agents.capabilityVersion = () => 1
      const path = `/v1/workloads/${containerId}/agent-token`
      c.nodes.once(new RegExp(`^POST ${path}$`), async (answer) => {
        started.resolve()
        await held.promise
        return answer()
      })

      const service = new EdgeService(c.db, c.boxes, 'sandboxes.test', undefined, c.agents)
      const app = edgeApi(service, 'edge-outage-test')
      let responseStatus = 0
      const poll = new RoutesPoll(
        async (headers) => {
          const response = await app.request('http://control.test/routes', {
            headers: {
              ...headers,
              authorization: 'Bearer edge-outage-test',
              [EDGE_ID_HEADER]: 'outage',
              [EDGE_FEATURES_HEADER]: 'agent',
            },
          })
          responseStatus = response.status
          return response
        },
        async (list) => {
          table.apply(list)
          return true
        },
      )
      first = poll.sync()
      await started.promise
      // The stalled answer has no deadline of its own and cannot arrive until the test releases it.
      const timeout = new AbortController()
      try {
        expect(await Promise.race([first, pause(SECOND_MS, false, { signal: timeout.signal })])).toBe(true)
      } finally {
        timeout.abort()
      }
      const peer = c.peers.get(processId)
      if (!peer) throw new Error('The process sandbox has no peer')
      const health = await proxy.fetch(
        new Request(`http://localhost/sandboxes/${processId}/_pryx/health`, {
          headers: { authorization: `Bearer ${peer.token()}` },
        }),
      )
      expect(health?.status).toBe(StatusCodes.OK)
      await health?.body?.cancel()

      const processRoute = `${processId}:${AGENT_ROUTE_PORT}`,
        containerRoute = `${containerId}:${AGENT_ROUTE_PORT}`
      const deadline = performance.now() + SECOND_MS
      while (!table.held.get(processRoute)?.agent?.routeToken && performance.now() < deadline)
        await poll.sync()
      expect(Boolean(table.held.get(processRoute)?.agent?.routeToken)).toBe(true)
      expect(Boolean(table.held.get(containerRoute)?.agent?.routeToken)).toBe(false)
      expect(await poll.sync()).toBe(false)
      expect(responseStatus).toBe(StatusCodes.NOT_MODIFIED)
      expect(c.nodes.calls.filter((call) => call.path === path)).toHaveLength(1)

      const version = poll.version
      held.resolve()
      const recovery = performance.now() + SECOND_MS
      while (!table.held.get(containerRoute)?.agent?.routeToken && performance.now() < recovery)
        await poll.sync()
      expect(Boolean(table.held.get(containerRoute)?.agent?.routeToken)).toBe(true)
      expect(responseStatus).toBe(StatusCodes.OK)
      expect(poll.version).toBe(version)
      expect(
        await one(
          c.db,
          sql`SELECT 1 FROM edge_route_lists WHERE jsonb_path_exists(routes,'$[*].agent.routeToken')`,
        ),
      ).toBeNull()
    } finally {
      held.resolve()
      await first?.catch(() => {})
      proxy.close()
      await c.close()
    }
  },
  10 * SECOND_MS,
)
