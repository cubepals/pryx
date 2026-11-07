// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Current celld executes this ordinary Worker/DO; the proposed transport is explicit. */

export class PryxContainer {
  constructor(ctx, env) {
    this.ctx = ctx
    this.env = env
  }
  async fetch(request) {
    const headers = { authorization: `Bearer ${this.env.PRYX_BRIDGE_TOKEN}` }
    let id = await this.ctx.storage.get('sandbox')
    if (!id) {
      const made = await fetch(`${this.env.PRYX_BRIDGE_URL}/containers`, {
        method: 'POST',
        headers: { ...headers, 'content-type': 'application/json' },
        body: JSON.stringify({
          cell: `${this.ctx.id.toString()}:${(await this.ctx.storage.get('generation')) ?? 0}`,
        }),
      })
      if (!made.ok) return new Response('Container create failed', { status: made.status })
      id = (await made.json()).id
      await this.ctx.storage.put('sandbox', id)
    }
    const path = new URL(request.url).pathname
    if (path === '/sleep')
      return fetch(`${this.env.PRYX_BRIDGE_URL}/containers/${id}/sleep`, { method: 'POST', headers })
    if (path === '/delete') {
      const result = await fetch(`${this.env.PRYX_BRIDGE_URL}/containers/${id}`, {
        method: 'DELETE',
        headers,
      })
      if (result.ok) {
        await this.ctx.storage.delete('sandbox')
        await this.ctx.storage.put('generation', ((await this.ctx.storage.get('generation')) ?? 0) + 1)
      }
      return result
    }
    const target = new URL(request.url)
    return fetch(`${this.env.PRYX_BRIDGE_URL}/containers/${id}/fetch${target.pathname}${target.search}`, {
      method: request.method,
      headers: { ...Object.fromEntries(request.headers), ...headers },
      body: ['GET', 'HEAD'].includes(request.method) ? undefined : request.body,
    })
  }
}
export default {
  fetch(request, env) {
    return env.CONTAINERS.get(env.CONTAINERS.idFromName('prototype')).fetch(request)
  },
}
