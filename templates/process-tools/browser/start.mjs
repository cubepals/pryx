// SPDX-License-Identifier: FSL-1.1-ALv2
/** The process browser uses Puppeteer's debugging pipe and an abstract Unix HTTP/WebSocket
 * listener. Chromium's sandbox is mandatory; host policy and private proc setup precede this.
 * Public discovery retains the agent's expiring capability; screenshots use a private token. */
import puppeteer from 'puppeteer-core'
import { launch } from '@puppeteer/browsers'
import { Connection } from 'puppeteer-core/internal/cdp/Connection.js'
import { PipeTransport } from 'puppeteer-core/internal/node/PipeTransport.js'
import { WebSocket, WebSocketServer } from 'ws'
import http from 'node:http'
import { constants } from 'node:http2'
import { timingSafeEqual } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { bridge } from './bridge.mjs'

const config = JSON.parse(readFileSync(0, 'utf8'))
function equal(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string') return false
  const left = Buffer.from(a), right = Buffer.from(b)
  return left.length === right.length && timingSafeEqual(left, right)
}
function access(request) {
  try {
    const url = new URL(request.url, 'http://localhost')
    const policy = JSON.parse(readFileSync(join(config.scratch, 'browser-access.json'), 'utf8'))
    return policy.expiresAt * 1000 > Date.now() && equal(url.searchParams.get('token'), policy.token) ? policy : undefined
  } catch { return undefined }
}
function endpoint(policy) {
  const url = new URL(policy.url)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  url.pathname = url.pathname.replace(/\/$/, '') + '/devtools/browser/pryx'
  url.search = new URLSearchParams({ token: policy.token }).toString()
  return url.toString()
}
const args = await puppeteer.defaultArgs({ headless: 'shell', userDataDir: join(config.scratch, 'chromium') })
args.push('--remote-debugging-pipe')
if (args.some(arg => arg === '--no-sandbox' || arg.startsWith('--remote-debugging-port'))) {
  throw new Error('Process Chromium requires its sandbox and debugging pipe')
}
const owner = launch({ executablePath: join(config.bundle, 'bin/chrome-headless-shell'), args, pipe: true, detached: false, env: process.env })
const pipe = new PipeTransport(owner.nodeProcess.stdio[3], owner.nodeProcess.stdio[4])
const connection = new Connection('', pipe, 0, config.timeoutMs, true)
const browser = await puppeteer.connect({ transport: await bridge(connection), defaultViewport: { width: 1280, height: 720 } })
const version = await connection.send('Browser.getVersion')
const server = http.createServer(async (request, response) => {
  response.setHeader('cache-control', 'no-store')
  if (request.url === '/_pryx/screenshot' && equal(request.headers['x-pryx-browser-private'], config.privateToken)) {
    let page
    try {
      let body = ''
      for await (const chunk of request) {
        body += chunk
        if (Buffer.byteLength(body) > config.maxRequestBytes) throw new Error('Request exceeds its bound')
      }
      const { url } = JSON.parse(body)
      const parsed = new URL(url)
      if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password) throw new Error('Invalid browser URL')
      page = await browser.newPage()
      await page.goto(url, { waitUntil: 'load', timeout: config.timeoutMs })
      response.setHeader('content-type', 'image/png')
      response.end(await page.screenshot({ type: 'png' }))
    } catch {
      response.writeHead(constants.HTTP_STATUS_BAD_GATEWAY).end()
    } finally { await page?.close() }
    return
  }
  const policy = access(request)
  if (!policy) { response.writeHead(constants.HTTP_STATUS_FORBIDDEN).end(); return }
  if (new URL(request.url, 'http://localhost').pathname.replace(/\/$/, '') === '/json/version') {
    response.setHeader('content-type', 'application/json')
    response.end(JSON.stringify({ Browser: version.product, 'Protocol-Version': version.protocolVersion,
      'User-Agent': version.userAgent, 'V8-Version': version.jsVersion, webSocketDebuggerUrl: endpoint(policy) }))
  } else response.writeHead(constants.HTTP_STATUS_NOT_FOUND).end()
})
const sockets = new WebSocketServer({ noServer: true, maxPayload: config.maxRequestBytes })
server.on('upgrade', (request, socket, head) => {
  const policy = access(request)
  if (!policy) { socket.destroy(); return }
  sockets.handleUpgrade(request, socket, head, async websocket => {
    // A client can send its first CDP request immediately after the upgrade. Pause the
    // maintained receiver until its independent browser session and handlers exist.
    websocket.pause()
    websocket.on('error', () => websocket.close())
    let transport
    try { transport = await bridge(connection) } catch { websocket.close(); return }
    if (websocket.readyState !== WebSocket.OPEN) { transport.close(); return }
    transport.onmessage = message => { if (websocket.readyState === WebSocket.OPEN) websocket.send(message) }
    transport.onclose = () => websocket.close()
    websocket.on('message', message => transport.send(message.toString()))
    websocket.on('close', () => transport.close())
    websocket.on('error', () => transport.close())
    const expiry = setTimeout(() => websocket.close(), Math.max(0, policy.expiresAt * 1000 - Date.now()))
    websocket.on('close', () => clearTimeout(expiry))
    websocket.resume()
  })
})
server.listen({ path: '\0' + config.address })
const stop = async () => { server.close(); sockets.close(); connection.dispose(); await owner.close(); process.exit(0) }
process.on('SIGTERM', stop)
process.on('SIGINT', stop)
browser.on('disconnected', () => { server.close(); process.exitCode = 1 })
