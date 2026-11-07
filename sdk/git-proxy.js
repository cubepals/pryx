// SPDX-License-Identifier: FSL-1.1-ALv2
/** A single-call Git HTTP bridge inside a virtual sandbox. Git's socket client reaches only
 * this loopback listener; guest fetch enforces the VM network policy and verifies upstream TLS.
 * The random path capability and fixed origin prevent use as an open proxy. No credential or
 * repository config is written by this process. The SDK kills it after the Git call completes. */
const http = require('node:http')
const origin = new URL(process.env.PRYX_PROXY_ORIGIN)
const prefix = `/${process.env.PRYX_PROXY_NONCE}/`
const limit = Number(process.env.PRYX_PROXY_MAX_BODY)
if (!['http:', 'https:'].includes(origin.protocol) || !Number.isSafeInteger(limit) || limit <= 0) process.exit(1)
const server = http.createServer(async (request, response) => {
  try {
    if (!request.url.startsWith(prefix)) { response.writeHead(403); response.end(); return }
    const path = new URL(request.url.slice(prefix.length - 1), 'http://loopback')
    const target = new URL(origin.origin)
    target.pathname = path.pathname
    target.search = path.search
    const headers = { ...request.headers }
    for (const name of ['host', 'connection', 'proxy-authorization', 'proxy-connection', 'transfer-encoding', 'content-length']) delete headers[name]
    headers['accept-encoding'] = 'identity'
    let body
    if (!['GET', 'HEAD'].includes(request.method)) {
      const chunks = []
      let length = 0
      for await (const chunk of request) {
        length += chunk.length
        if (length > limit) throw new Error('Git HTTP body exceeds the guest frame budget')
        chunks.push(chunk)
      }
      body = Buffer.concat(chunks)
    }
    const upstream = await fetch(target, { method: request.method, headers, body, redirect: 'manual' })
    const output = {}
    for (const [name, value] of upstream.headers) {
      if (!['connection', 'transfer-encoding', 'content-length', 'content-encoding'].includes(name)) output[name] = value
    }
    response.writeHead(upstream.status, output)
    if (upstream.body) {
      const reader = upstream.body.getReader()
      try {
        for (;;) {
          const part = await reader.read()
          if (part.done) break
          if (!response.write(Buffer.from(part.value))) await new Promise(resolve => response.once('drain', resolve))
        }
      } finally { await reader.cancel() }
    }
    response.end()
  } catch {
    if (!response.headersSent) response.writeHead(502)
    response.end()
  }
})
server.listen(0, '127.0.0.1', () => console.log(`PRYX_GIT_PROXY_READY ${server.address().port}`))
