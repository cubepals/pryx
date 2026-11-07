// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The published sandbox-agent provider interface over pryx; agentOS uses its real process client, including stdin and background processes. */

import type { MakeRequest, Pryx } from '@cubepals/pryx'
import type { AgentOsSandboxClient, AgentOsSandboxProvider } from '@rivet-dev/agentos-core'
import { StatusCodes } from 'http-status-codes'
import { SandboxAgent, type SandboxProvider } from 'sandbox-agent'
import { SANDBOX_AGENT_PORT, SANDBOX_AGENT_TEMPLATE } from './template.ts'

export function pryxProvider(
  client: Pryx,
  options: {
    key: string
    image?: string
    template?: string
    agentPort?: number
    spec?: Omit<MakeRequest, 'image' | 'ports' | 'entrypoint' | 'start'>
  },
): SandboxProvider {
  async function remove(id: string) {
    try {
      await (await client.sandbox(id, { wake: false })).delete()
    } catch (error) {
      if (!(error instanceof Error && 'status' in error && error.status === StatusCodes.GONE)) throw error
    }
  }

  if (options.image && options.template) throw new Error('Choose image or template')
  const port = options.agentPort ?? SANDBOX_AGENT_PORT
  return {
    name: 'pryx',
    defaultCwd: '/data',
    async create() {
      const made = await client.sandbox({
        isolation: 'container',
        ...options.spec,
        ...(options.image
          ? { image: options.image }
          : { template: options.template ?? SANDBOX_AGENT_TEMPLATE }),
        idempotencyKey: options.key,
        start: true,
        entrypoint: ['sandbox-agent', 'server', '--no-token', '--host', '0.0.0.0', '--port', String(port)],
        ports: [{ name: 'agent', port, protocol: 'tcp', audience: 'private' }],
      })
      return made.id
    },
    async destroy(id) {
      await remove(id)
    },
    async kill(id) {
      await remove(id)
    },
    async pause(id) {
      await (await client.sandbox(id, { wake: false })).sleep()
    },
    async reconnect(id) {
      await (await client.sandbox(id, { wake: false })).wake()
    },
    async getUrl(id) {
      const endpoint = (await (await client.sandbox(id, { wake: false })).endpoints()).find(
        (p) => p.name === 'agent',
      )
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
