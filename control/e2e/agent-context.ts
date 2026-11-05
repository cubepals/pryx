// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real agent-enabled deployment and an edge with ephemeral listeners. The legacy deployment
 * fixture separately exercises nodes without the agent feature. This fixture requires it. */
import type { AddressInfo } from 'node:net'
import { startEdge } from '../../edge/src/agent.ts'
import { edgeConfig } from '../../edge/src/config.ts'
import { type MakeRequest, Pryx } from '../../sdk/typescript/src/index.ts'
import { AGENT_ROUTE_PORT } from '../src/edge-headers.ts'
import { deployment } from './deployment.ts'
import { until } from './waits.ts'

export async function agentDeployment(
  tier = process.env.PRYX_TEST_TIER as import('../src/isolation.ts').Isolation | undefined,
) {
  const d = await deployment({
    agent: true,
    tier,
    ...(process.env.PRYX_TIER_BENCH === '1'
      ? { allowedImages: [process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test'] }
      : {}),
  })
  const edge = await startEdge({
    ...edgeConfig({
      PRYX_CONTROL_URL: d.control.apiUrl,
      PRYX_EDGE_TOKEN: d.control.config.edgeToken,
      PRYX_EDGE_HOST: '127.0.0.1',
    }),
    httpPort: 0,
    tlsPort: 0,
    metricsPort: 0,
  })
  const port = (edge.listeners.get('http:80')?.address() as AddressInfo).port
  const origin = `http://127.0.0.1:${port}`
  if (!d.control.edge.agents) throw new Error('Missing agent capability service')
  d.control.edge.agents.localOrigin = origin
  const sdk = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
  return {
    d,
    edge,
    origin,
    sdk,
    sandbox: (options: MakeRequest = {}) =>
      sdk.sandbox(tier && tier !== 'container' ? { ...options, image: undefined, isolation: tier } : options),
    route: (id: string) =>
      until('edge holds agent', async () => edge.table.held.get(`${id}:${AGENT_ROUTE_PORT}`)),
    app: (url: string) => fetch(origin, { headers: { host: new URL(url).hostname } }),
    close: async () => {
      await edge.stop()
      await d.close()
    },
  }
}
