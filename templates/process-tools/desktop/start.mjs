// SPDX-License-Identifier: FSL-1.1-ALv2
/** Fixed desktop supervision and private HTTP forwarding. Maintained programs implement
 * X11, Chromium, RFB, noVNC and WebSocket; guest commands never enter this worker. */
import { spawn, execFile } from 'node:child_process'
import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { constants } from 'node:http2'
import net from 'node:net'
import { createRequire } from 'node:module'
import { join } from 'node:path'
import { setTimeout as pause } from 'node:timers/promises'
import protocol from './protocol.json' with { type: 'json' }

const config = JSON.parse(readFileSync(0, 'utf8'))
const component = join(config.bundle, 'desktop')
const require = createRequire(join(config.bundle, 'browser-client/package.json'))
const puppeteer = require('puppeteer-core')
const children = []
function phase(name) { writeFileSync(join(config.scratch, 'desktop-phase.json'), JSON.stringify({ phase:name }), { mode:0o600 }) }
const environment = { ...process.env, XAUTHORITY: join(config.scratch, 'Xauthority'),
  HOME: join(config.scratch, 'home'), TMPDIR: '/tmp',
  FONTCONFIG_FILE: join(config.bundle, 'etc/fonts/fonts.conf'),
  IMLIB2_LOADER_PATH: join(component, 'payload/usr/lib/x86_64-linux-gnu/imlib2/loaders') }
mkdirSync('/tmp/pryx-desktop', { mode: 0o700 })
const vnc = protocol.vncSocket, web = protocol.webSocket
function program(name) { return join(component, 'bin', name) }
function start(executable, args, extra = {}) {
  const child = spawn(executable, args, { env: environment, stdio: ['ignore', 'ignore', 'inherit'], ...extra })
  children.push(child)
  child.on('error', () => stop(1))
  child.on('exit', () => stop(1))
  return child
}
async function run(name, args, input, timeoutMs = config.timeoutMs) {
  const child = execFile(program(name), args, { env: environment, timeout: timeoutMs, maxBuffer: config.maxRequestBytes })
  if (input !== undefined) child.stdin.end(input)
  return new Promise((resolve, reject) => {
    let output = ''
    child.stdout.on('data', bytes => { output += bytes })
    child.once('error', reject)
    child.once('exit', code => code === 0 ? resolve(output) : reject(new Error('Desktop command failed')))
  })
}
function equal(left, right) {
  if (typeof left !== 'string' || typeof right !== 'string') return false
  const a = Buffer.from(left), b = Buffer.from(right)
  return a.length === b.length && timingSafeEqual(a, b)
}
// Every request but the private command presents the current, unexpired access token. A page
// request may instead carry the cookie a token request set, so noVNC's relative files load; the
// cookie is keyed to this worker and the token, so it never resolves in websockify's token file.
const pageCookie = 'pryx-desktop', pageKey = randomBytes(32)
function pageValue(token) { return createHmac('sha256', pageKey).update(token).digest('base64url') }
function pageCookies(request) {
  return (request.headers.cookie ?? '').split(';').map(part => part.trim())
    .filter(part => part.startsWith(`${pageCookie}=`)).map(part => part.slice(pageCookie.length + 1))
}
function access(request, page = false) {
  try {
    const policy = JSON.parse(readFileSync(join(config.scratch, 'desktop-access.json'), 'utf8'))
    if (!(policy.expiresAt * 1000 > Date.now())) return undefined
    if (equal(new URL(request.url, 'http://localhost').searchParams.get('token'), policy.token)) return { ...policy, presented: true }
    return page && pageCookies(request).some(value => equal(value, pageValue(policy.token))) ? policy : undefined
  } catch { return undefined }
}
function remember(response, policy) {
  const seconds = Math.max(0, Math.floor(policy.expiresAt - Date.now() / 1000))
  response.setHeader('set-cookie', `${pageCookie}=${pageValue(policy.token)}; Max-Age=${seconds}; HttpOnly; SameSite=Strict`)
}
let stopping = false, browser
async function stop(code = 0) {
  if (stopping) return
  stopping = true
  for (const child of children) child.kill('SIGTERM')
  await browser?.close().catch(() => {})
  process.exit(code)
}
process.on('SIGTERM', () => stop())
process.on('SIGINT', () => stop())
const cookie = randomBytes(16).toString('hex')
phase('create-display-cookie')
await run('xauth', ['add', ':0', '.', cookie])
phase('start-display')
const display = start(program('Xvfb'), ['-displayfd', '1', '-nolisten', 'tcp', '-noreset',
  '-screen', '0', '1280x720x24', '-auth', environment.XAUTHORITY,
  '-xkbdir', join(component, 'payload/usr/share/X11/xkb')], { stdio: ['ignore', 'pipe', 'inherit'] })
environment.DISPLAY = await new Promise((resolve, reject) => {
  let received = ''
  const deadline = setTimeout(() => reject(new Error('Display readiness deadline elapsed')), config.timeoutMs)
  display.stdout.on('data', bytes => {
    received += bytes
    if (/^\d+\n/.test(received)) { clearTimeout(deadline); resolve(':' + received.trim()) }
  })
  display.once('exit', () => reject(new Error('Display exited before readiness')))
})
await run('xauth', ['add', environment.DISPLAY, '.', cookie])
phase('own-display-client')
await run('xdotool', ['getmouselocation'])
phase('start-private-vnc-web')
start(join(config.bundle, 'bin/python3'), [join(component, 'adapter/vnc.py'), vnc, program('x11vnc')])
writeFileSync(join(config.scratch, 'tokens'), '', { mode: 0o600 })
start(join(config.bundle, 'bin/python3'), [join(component, 'adapter/web.py'),
  '--web', join(component, 'web/novnc'), '--unix-listen', web,
  '--token-plugin', 'policy.ExpiringTokenFile', '--token-source', join(config.scratch, 'tokens')],
  { env: { ...environment, PYTHONPATH: join(component, 'web/python') } })
const deadline = Date.now() + config.timeoutMs
while (!existsSync(web) || !existsSync(vnc)) {
  if (Date.now() >= deadline) throw new Error('Private VNC readiness deadline elapsed')
  await pause(20)
}
phase('start-sandboxed-headful-browser')
browser = await puppeteer.launch({ executablePath: program('chromium'), headless: false, pipe: true,
  dumpio:true, timeout:config.timeoutMs,
  userDataDir: join(config.scratch, 'chromium'), env: environment,
  args: ['--disable-gpu', '--use-gl=angle', '--use-angle=swiftshader', '--window-position=0,0', '--window-size=1280,720', '--no-first-run'] })
browser.on('disconnected', () => stop(1))
phase('open-application')
const pages = await browser.pages()
await pages[0].goto(config.applicationUrl || 'about:blank', { timeout: config.timeoutMs })
const server = http.createServer(async (request, response) => {
  response.setHeader('cache-control', 'no-store')
  if (request.url === protocol.commandPath) {
    if (!equal(request.headers[protocol.privateHeader], config.privateToken)) { response.writeHead(constants.HTTP_STATUS_FORBIDDEN).end(); return }
    try {
      let bytes = ''
      for await (const chunk of request) {
        bytes += chunk
        if (Buffer.byteLength(bytes) > config.maxRequestBytes) throw new Error('Command too large')
      }
      const { name, args, input, timeoutMs = config.timeoutMs } = JSON.parse(bytes)
      if (!['xdotool', 'scrot'].includes(name) || !Array.isArray(args) || args.some(value => typeof value !== 'string')) throw new Error('Invalid tool')
      if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > config.timeoutMs) throw new Error('Invalid command deadline')
      response.end(await run(name, args, input, timeoutMs))
    } catch { response.writeHead(constants.HTTP_STATUS_BAD_GATEWAY).end() }
    return
  }
  const policy = access(request, true)
  if (!policy) { response.writeHead(constants.HTTP_STATUS_FORBIDDEN).end(); return }
  if (policy.presented) remember(response, policy)
  // A forwarded request stays plain: only the upgrade handler below opens a WebSocket.
  const { upgrade: _plain, ...headers } = request.headers
  // Each request has its own connection: websockify may close a kept one as the next is written.
  const upstream = http.request({ socketPath: web, path: request.url, method: request.method, headers, agent: false }, result => {
    // A short body fails on the response stream after headers have already been forwarded.
    result.once('error', () => response.destroy())
    response.writeHead(result.statusCode, result.headers)
    result.pipe(response)
  })
  upstream.on('error', () => response.destroy())
  response.once('close', () => {
    if (!response.writableFinished) upstream.destroy()
  })
  request.pipe(upstream)
})
server.on('upgrade', (request, socket, head) => {
  const policy = access(request)
  if (!policy) { socket.destroy(); return }
  const upstream = net.createConnection({ path: web }, () => {
    // One write: websockify can answer and close as soon as the request is whole, and a later
    // write's EPIPE would discard that answer unread.
    let forwarded = `${request.method} ${request.url} HTTP/${request.httpVersion}\r\n`
    for (let i = 0; i < request.rawHeaders.length; i += 2) forwarded += `${request.rawHeaders[i]}: ${request.rawHeaders[i + 1]}\r\n`
    upstream.write(Buffer.concat([Buffer.from(forwarded + '\r\n'), head]))
    socket.pipe(upstream).pipe(socket)
  })
  const expiry = setTimeout(() => { socket.destroy(); upstream.destroy() }, Math.max(0, policy.expiresAt * 1000 - Date.now()))
  socket.on('error', () => upstream.destroy())
  upstream.on('error', () => socket.destroy())
  socket.on('close', () => { clearTimeout(expiry); upstream.destroy() })
  upstream.on('close', () => socket.destroy())
})
writeFileSync(join(config.scratch, 'desktop-ready.json'), JSON.stringify({ display: environment.DISPLAY, xauthority: environment.XAUTHORITY, uid: process.getuid() }), { mode: 0o600 })
server.listen({ path: '\0' + config.address })
phase('serve-desktop')
