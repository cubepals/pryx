// SPDX-License-Identifier: FSL-1.1-ALv2
/** Each client receives an independent browser-target session on Puppeteer's maintained CDP
 * connection. Puppeteer owns framing, request IDs, errors and descendant session routing. */
export async function bridge(connection) {
  const { sessionId } = await connection.send('Target.attachToBrowserTarget')
  const root = connection.session(sessionId)
  const sessions = new Map()
  let closed = false
  const transport = {
    onmessage: undefined,
    onclose: undefined,
    send: text => {
      void send(text).catch(() => transport.close())
    },
    close: () => {
      if (closed) return
      closed = true
      for (const [session, listener] of sessions.values()) session.off('*', listener)
      sessions.clear()
      void root.detach().catch(() => {})
      transport.onclose?.()
    },
  }
  function subscribe(session, externalId) {
    const listener = (method, params) => {
      if (closed || typeof method !== 'string' || !method.includes('.')) return
      if (method === 'Target.attachedToTarget') {
        const child = connection.session(params.sessionId)
        if (child) subscribe(child, params.sessionId)
      }
      transport.onmessage?.(JSON.stringify({ method, params, ...(externalId ? { sessionId: externalId } : {}) }))
    }
    sessions.set(session.id(), [session, listener])
    session.on('*', listener)
  }
  subscribe(root)
  async function send(text) {
    const request = JSON.parse(text)
    if (!Number.isSafeInteger(request.id) || typeof request.method !== 'string') {
      transport.close()
      return
    }
    const target = request.sessionId ? sessions.get(request.sessionId)?.[0] : root
    if (!target) {
      transport.close()
      return
    }
    const identity = { id: request.id, ...(request.sessionId ? { sessionId: request.sessionId } : {}) }
    try {
      const result = await target.send(request.method, request.params)
      if (!closed) transport.onmessage?.(JSON.stringify({ ...identity, result }))
    } catch (error) {
      if (Number.isInteger(error.code) && typeof error.message === 'string') {
        if (!closed) transport.onmessage?.(JSON.stringify({ ...identity, error }))
      } else transport.close()
    }
  }
  return transport
}
