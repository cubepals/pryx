// SPDX-License-Identifier: FSL-1.1-ALv2
/** Actual upstream browser pipe, Unix proxy, screenshot and expiring CDP acceptance. This
 * bundle-level test complements the user-install/edge and hostile-neighbour process tests. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { mkdtempSync, writeFileSync, readFileSync, readdirSync } from 'node:fs'
import http from 'node:http'
import { constants } from 'node:http2'
import net from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { setTimeout as pause } from 'node:timers/promises'
import puppeteer from 'puppeteer-core'
import { WebSocket } from 'ws'

assert.notEqual(process.getuid(), 0)
const bundle = resolve(process.argv[2])
const scratch = mkdtempSync(join(tmpdir(), 'pryx-browser-pipe-'))
const address = 'pryx-browser-smoke-' + randomBytes(24).toString('hex')
const privateToken = randomBytes(32).toString('hex')
const pageServer = http.createServer((_request, response) => response.end('<html><body><h1>Private pipe browser</h1><p>漢字 😀</p></body></html>'))
await new Promise(resolve => pageServer.listen(0, '127.0.0.1', resolve))
const pageUrl = `http://127.0.0.1:${pageServer.address().port}/`
const child = spawn(join(bundle, 'bin/node'), [join(bundle, 'browser-client/start.mjs')], { stdio: ['pipe', 'pipe', 'pipe'] })
let diagnostic = ''
child.stderr.on('data', bytes => { diagnostic = (diagnostic + bytes).slice(-16384) })
child.stdin.end(JSON.stringify({ bundle, scratch, address, privateToken, timeoutMs: 30000, maxRequestBytes: 1024 * 1024 }))
function request(path, body, headers = {}) {
  return new Promise((resolve, reject) => {
    const call = http.request({ socketPath: '\0' + address, path, method: body ? 'POST' : 'GET', headers,
      timeout: 30000 }, response => {
      const chunks = []
      response.on('data', bytes => chunks.push(bytes))
      response.on('end', () => resolve({ status: response.statusCode, bytes: Buffer.concat(chunks) }))
    })
    call.on('error', reject)
    call.on('timeout', () => call.destroy(new Error('Browser request timed out')))
    call.end(body)
  })
}
let browser
try {
  const deadline = Date.now() + 30000
  while (true) {
    try { assert.equal((await request('/json/version')).status, constants.HTTP_STATUS_FORBIDDEN); break }
    catch (error) {
      if (child.exitCode !== null || Date.now() >= deadline) throw new Error(diagnostic || error.message)
      await pause(25)
    }
  }
  const shot = await request('/_pryx/screenshot', JSON.stringify({ url: pageUrl }), { 'x-pryx-browser-private': privateToken })
  assert.equal(shot.status, constants.HTTP_STATUS_OK, diagnostic)
  assert.equal(shot.bytes.subarray(1, 4).toString(), 'PNG')
  const policy = { token: randomBytes(32).toString('hex'), expiresAt: Math.floor(Date.now() / 1000) + 10,
    url: 'http://example.test/browser-prefix' }
  writeFileSync(join(scratch, 'browser-access.json'), JSON.stringify(policy), { mode: 0o600 })
  const version = await request('/json/version?token=' + policy.token)
  assert.equal(version.status, constants.HTTP_STATUS_OK)
  const external = new URL(JSON.parse(version.bytes).webSocketDebuggerUrl)
  assert.equal(external.hostname, 'example.test')
  assert.equal(external.pathname, '/browser-prefix/devtools/browser/pryx')
  assert.equal(external.searchParams.get('token'), policy.token)
  const websocket = new WebSocket('ws://localhost/devtools/browser/pryx?token=' + policy.token,
    { createConnection: () => net.createConnection({ path: '\0' + address }) })
  await new Promise((resolve, reject) => { websocket.once('open', resolve); websocket.once('error', reject) })
  const closed = new Promise(resolve => websocket.once('close', resolve))
  const transport = { send: message => websocket.send(message), close: () => websocket.close() }
  websocket.on('message', bytes => transport.onmessage?.(bytes.toString()))
  websocket.on('close', () => transport.onclose?.())
  browser = await puppeteer.connect({ transport })
  const page = await browser.newPage()
  await page.goto(pageUrl)
  assert.equal(await page.title(), '')
  assert.equal(await page.$eval('h1', element => element.textContent), 'Private pipe browser')
  assert.equal(Buffer.from(await page.screenshot()).subarray(1, 4).toString(), 'PNG')
  const parents = new Map()
  // PPid is present independently of CONFIG_CHECKPOINT_RESTORE's optional children files.
  for (const pid of readdirSync('/proc').filter(name => /^\d+$/.test(name))) {
    try {
      const parent = readFileSync(`/proc/${pid}/status`, 'utf8').match(/^PPid:\s+(\d+)/m)
      if (parent) parents.set(pid, parent[1])
    } catch (error) {
      if (!['ENOENT', 'ESRCH', 'EACCES'].includes(error.code)) throw error
    }
  }
  const descendants = []
  const owners = new Set([String(child.pid)])
  for (let changed = true; changed;) {
    changed = false
    for (const [pid, parent] of parents) {
      if (owners.has(parent) && !owners.has(pid)) { owners.add(pid); descendants.push(pid); changed = true }
    }
  }
  const renderer = descendants.find(pid => readFileSync(`/proc/${pid}/cmdline`, 'utf8').includes('--type=renderer'))
  assert.ok(renderer, 'A real renderer must exist')
  function field(pid, name) {
    return readFileSync(`/proc/${pid}/status`, 'utf8').split('\n').find(line => line.startsWith(name + ':')).split(':')[1].trim()
  }
  assert.notEqual(Number(field(renderer, 'Uid').split(/\s+/)[0]), 0)
  assert.equal(Number(field(renderer, 'NoNewPrivs')), 1)
  assert.ok(field(renderer, 'NSpid').split(/\s+/).length > field(child.pid, 'NSpid').split(/\s+/).length,
    'Chromium renderer must be in its own nested PID namespace')
  assert.ok(Number(field(renderer, 'Seccomp_filters')) > Number(field(child.pid, 'Seccomp_filters')),
    'Chromium renderer must install its own seccomp filter')
  await page.close()
  await Promise.race([closed, pause(12000).then(() => { throw new Error('Expired CDP session remained open') })])
  assert.equal((await request('/json/version?token=' + policy.token)).status, constants.HTTP_STATUS_FORBIDDEN)
  console.log(JSON.stringify({ uid: process.getuid(), debuggingPipe: true, abstractUnix: true,
    screenshot: true, cdp: true, expires: true, chromiumSandbox: true, rendererPidNamespace: true, rendererSeccomp: true }))
} catch (error) {
  throw new Error(`Browser pipe acceptance failed: ${error.message}\n${diagnostic}`, { cause: error })
} finally {
  browser?.disconnect()
  child.kill('SIGTERM')
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), pause(5000).then(() => child.kill('SIGKILL'))])
  pageServer.close()
}
