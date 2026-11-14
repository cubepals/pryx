// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real deployment with the mandatory agent and an edge with ephemeral listeners. */
import type { AddressInfo } from 'node:net'
import { type MakeRequest, Pryx } from '../../sdk/typescript/src/index.ts'
import { AGENT_ROUTE_PORT } from '../src/edge-headers.ts'
import { AppRequests } from './app-requests.ts'
import { deployment } from './deployment.ts'
import { until } from './waits.ts'

export async function agentDeployment(
  tier = process.env.PRYX_TEST_TIER as import('../src/isolation.ts').Isolation | undefined,
  options: {
    webhookAllowHosts?: string[]
    egressPolicy?: boolean
    nativeTools?: boolean
    localPreviews?: boolean
    repoImage?: string
  } = {},
) {
  const image = process.env.PRYX_IMAGE_TEST ?? 'pryx-static-test:latest'
  const d = await deployment({
    ...options,
    tier,
    ...(tier === 'wasm' && process.env.PRYXD_STATIC_BIN ? { binary: process.env.PRYXD_STATIC_BIN } : {}),
    ...(process.env.PRYX_TIER_BENCH === '1'
      ? { allowedImages: [image.slice(0, image.lastIndexOf(':') + 1)] }
      : {}),
  })
  const edge = d.edge
  const port = (edge.listeners.get('http:80')?.address() as AddressInfo).port
  const origin = `http://127.0.0.1:${port}`
  const apps = new AppRequests(origin)
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
    app: (url: string) => apps.get(url),
    close: async () => {
      apps.close()
      await edge.stop()
      await d.close()
    },
  }
}
