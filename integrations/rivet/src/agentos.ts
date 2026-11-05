// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The published sandbox-agent provider interface over pryx; agentOS uses its real process client, including stdin and background processes. */

import type { MakeRequest, Pryx } from '@cubepals/pryx'
import type { AgentOsSandboxClient, AgentOsSandboxProvider } from '@rivet-dev/agentos-core'
import { SandboxAgent, type SandboxProvider } from 'sandbox-agent'

export function pryxProvider(
  client: Pryx,
  options: {
    key: string
    image: string
    agentPort: number
    spec?: Omit<MakeRequest, 'image' | 'ports' | 'entrypoint' | 'start'>
  },
): SandboxProvider {
  return {
    name: 'pryx',
    defaultCwd: '/data',
    async create() {
      const made = await client.make(
        {
          ...options.spec,
          image: options.image,
          start: true,
          entrypoint: [
            'sandbox-agent',
            'server',
            '--no-token',
            '--host',
            '0.0.0.0',
            '--port',
            String(options.agentPort),
          ],
          ports: [{ name: 'agent', port: options.agentPort, protocol: 'tcp', audience: 'private' }],
        },
        options.key,
      )
      return made.id
    },
    async destroy(id) {
      await client.delete(id)
    },
    async kill(id) {
      await client.delete(id)
    },
    async pause(id) {
      await client.action(id, 'sleep')
    },
    async reconnect(id) {
      await client.action(id, 'wake')
    },
    async getUrl(id) {
      const endpoint = (await client.endpoints(id)).endpoints.find((p) => p.name === 'agent')
      if (!endpoint) throw new Error('Sandbox has no private agent endpoint')
      const host = endpoint.host.includes(':') ? `[${endpoint.host}]` : endpoint.host
      return `http://${host}:${endpoint.port}`
    },
  }
}
export function agentOsProvider(
  client: Pryx,
  options: Parameters<typeof pryxProvider>[1],
): AgentOsSandboxProvider {
  return {
    async start() {
      const server = await SandboxAgent.start({ sandbox: pryxProvider(client, options) })
      return new Proxy(server, {
        get(target, property) {
          if (property === 'dispose') return () => target.destroySandbox()
          const value = Reflect.get(target, property, target)
          return typeof value === 'function' ? value.bind(target) : value
        },
      }) as unknown as AgentOsSandboxClient
    },
  }
}
