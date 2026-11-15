// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real-daemon execution of every canonical connect block on container and process defaults.
 * This verifies public callers and signed deliveries, not deployment on an external platform. */
import { expect, test } from 'bun:test'
import { createHmac } from 'node:crypto'
import { StatusCodes } from 'http-status-codes'
import { MINUTE_MS } from '../src/durations.ts'
import { WEBHOOK_HEADERS, Webhooks } from '../src/webhooks.ts'
import { WELCOME_IMAGE } from '../src/welcome-examples.ts'
import { runSiteCallers } from '../tools/site-callers.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'
import { until } from './waits.ts'

for (const tier of ['container', 'process'] as const) {
  test(
    `all site connect blocks on the ${tier} tier`,
    async () => {
      expect(ready).toBe(true)
      const cli = process.env.PRYX_MCP_CLI
      const python = process.env.PRYX_PYTHON
      if (!cli || !python) throw new Error('Site acceptance requires the built CLI and Python SDK')
      const c = await agentDeployment(tier, {
        webhookAllowHosts: ['127.0.0.1'],
        repoImage: 'alpine:pryx-tls',
      })
      const received: Array<{ body: string; headers: Headers }> = []
      const receiver = Bun.serve({
        hostname: '127.0.0.1',
        port: 0,
        async fetch(request) {
          received.push({ body: await request.text(), headers: request.headers })
          return new Response(null, { status: StatusCodes.NO_CONTENT })
        },
      })
      const hooks = new Webhooks(c.d.control.db, c.d.control.config.providerSealKey, ['127.0.0.1'])
      try {
        await runSiteCallers({
          env: c.d.env,
          cli,
          python,
          receiver: receiver.url.toString(),
          async verifyWebhook({ secret }) {
            const box = await c.sandbox(tier === 'container' ? { image: WELCOME_IMAGE } : {})
            const id = box.id
            await box.delete()
            const deliveries = await until('site signed lifecycle deliveries', async () => {
              await hooks.tick()
              const own = received.filter((entry) => JSON.parse(entry.body).sandboxId === id)
              return new Set(own.map((entry) => JSON.parse(entry.body).event)).size === 2 ? own : undefined
            })
            expect([...new Set(deliveries.map((entry) => JSON.parse(entry.body).event))].sort()).toEqual([
              'created',
              'deleted',
            ])
            for (const entry of deliveries) {
              const timestamp = entry.headers.get(WEBHOOK_HEADERS.timestamp)
              expect(entry.headers.get(WEBHOOK_HEADERS.signature)).toBe(
                `v1=${createHmac('sha256', secret).update(`${timestamp}.${entry.body}`).digest('hex')}`,
              )
              expect(entry.body).not.toContain(secret)
              expect(entry.body).not.toContain(c.d.credential.key)
            }
          },
        })
      } catch (error) {
        deploymentDiagnostics(c.d)
        throw error
      } finally {
        receiver.stop(true)
        await c.close()
      }
    },
    4 * MINUTE_MS,
  )
}
