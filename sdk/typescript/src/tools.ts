// SPDX-License-Identifier: FSL-1.1-ALv2
/** Callable native tool handles retain transport authority, not lifecycle state. Each mutation
 * is sent once; private SSH keys are deliberately absent from ordinary serialization. */
import type { Agent } from './agent.ts'
import type { components } from './generated/agent.ts'
import { TOOL_DEFAULTS } from './generated/tools.ts'
import type { Transport } from './transport.ts'

type Schemas = components['schemas']
export type ToolAccess = Schemas['ToolAccess']
export type Mount = Schemas['MountView']
export type SshAddress = {
  id: string
  host: string
  port: number
  user: string
  hostKey: string
  expiresAt: number
}
export class SshAccess {
  #key: string
  constructor(
    readonly address: SshAddress,
    key: string,
  ) {
    this.#key = key
  }
  get id() {
    return this.address.id
  }
  get host() {
    return this.address.host
  }
  get port() {
    return this.address.port
  }
  get user() {
    return this.address.user
  }
  get hostKey() {
    return this.address.hostKey
  }
  get expiresAt() {
    return this.address.expiresAt
  }
  get key() {
    return this.#key
  }
  toJSON() {
    return this.address
  }
}
export function nativeTools(agent: Agent, api: Transport, id: string) {
  async function request<T>(path: string, method = 'POST', body?: unknown) {
    await agent.requireTools(TOOL_DEFAULTS.version)
    return agent.request<T>(`/_pryx/${path}`, method, body)
  }
  async function png(path: string, body?: unknown) {
    await agent.requireTools(TOOL_DEFAULTS.version)
    const response = await agent.raw(
      `/_pryx/${path}`,
      'POST',
      body === undefined ? undefined : JSON.stringify(body),
      body === undefined ? {} : { 'content-type': 'application/json' },
    )
    return new Uint8Array(await response.arrayBuffer())
  }
  const desktop = Object.assign(() => request<ToolAccess>('desktop'), {
    screenshot: () => png('desktop/screenshot'),
    click: (x: number, y: number) => request<void>('desktop/click', 'POST', { x, y }),
    move: (x: number, y: number) => request<void>('desktop/move', 'POST', { x, y }),
    type: (text: string) => request<void>('desktop/type', 'POST', { text }),
    key: (combo: string) => request<void>('desktop/key', 'POST', { combo }),
    scroll: (dx: number, dy: number) => request<void>('desktop/scroll', 'POST', { dx, dy }),
  })
  const browser = Object.assign(() => request<ToolAccess>('browser'), {
    screenshot: (url: string) => png('browser/screenshot', { url }),
  })
  const path = `/v1/sandboxes/${encodeURIComponent(id)}/ssh`
  const ssh = Object.assign(
    async () => {
      const created = await request<Schemas['SshCreated']>('ssh')
      try {
        return new SshAccess(await api.request<SshAddress>(path, 'POST', { id: created.id }), created.key)
      } catch (error) {
        await agent.request(`/_pryx/ssh/${encodeURIComponent(created.id)}`, 'DELETE').catch(() => {})
        throw error
      }
    },
    { revoke: () => api.request<{ revoked: boolean }>(path, 'DELETE') },
  )
  return {
    desktop,
    browser,
    ssh,
    mounts: async () => (await request<{ mounts: Mount[] }>('mounts', 'GET')).mounts,
  }
}
