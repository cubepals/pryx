// SPDX-License-Identifier: FSL-1.1-ALv2
/** Installed process-tier computer use and OpenSSH through the HTTP edge. The application
 * observes real input; maintained CLI/OpenSSH adapters own SSH framing and private keys. */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { setTimeout as pause } from 'node:timers/promises'
import { StatusCodes } from 'http-status-codes'
import { connectSsh } from '../cli/src/sandbox-ssh.ts'
import { SECOND_MS } from '../control/src/durations.ts'
import type { Sandbox } from '../sdk/typescript/src/index.ts'

const SSH_MARKER = 'native-http-ssh-ready\n'

type Observation = { kind: string; value?: string; input?: Point; button?: Point }
type Point = { x: number; y: number }
type Context = {
  make: () => Promise<Sandbox>
  png: (bytes: Uint8Array | undefined) => void
  python: (box: Sandbox, kind: string, env?: Record<string, string>) => Promise<void>
  command: (argv: string[], env: Record<string, string | undefined>) => Promise<string>
  cli: string
}
async function until(check: () => boolean) {
  const deadline = performance.now() + 30 * SECOND_MS
  while (!check()) {
    assert(performance.now() < deadline, 'Installed desktop application observation deadline')
    await pause(SECOND_MS / 50)
  }
}
async function novnc(box: Sandbox) {
  const access = await box.desktop()
  const page = new URL(access.url)
  const path = new URLSearchParams(page.hash.slice(1)).get('path')
  assert(path)
  page.hash = ''
  assert.equal((await fetch(page)).status, StatusCodes.OK)
  const socket = new URL(path, page.origin)
  socket.protocol = page.protocol === 'https:' ? 'wss:' : 'ws:'
  await new Promise<void>((resolve, reject) => {
    const stream = new WebSocket(socket)
    const timer = setTimeout(() => {
      stream.close()
      reject(new Error('Installed edge VNC greeting deadline'))
    }, 5 * SECOND_MS)
    stream.onerror = () => {
      clearTimeout(timer)
      reject(new Error('Installed edge refused VNC'))
    }
    stream.onmessage = async (event) => {
      clearTimeout(timer)
      stream.close()
      try {
        const data = event.data instanceof Blob ? await event.data.arrayBuffer() : event.data
        const banner = typeof data === 'string' ? data : new TextDecoder().decode(data)
        assert(banner.startsWith('RFB '))
        resolve()
      } catch (error) {
        reject(error)
      }
    }
  })
}
export async function nativeSshCommand(box: Sandbox, sleep = false) {
  await box.write('ssh-marker', SSH_MARKER)
  const access = await box.ssh()
  try {
    assert.equal(access.transport, 'websocket', 'The user preset must use its HTTP-only edge')
    assert(!JSON.stringify(access).includes('PRIVATE KEY'))
    if (sleep) {
      await box.sleep()
      assert.equal(box.state, 'sleeping')
    }
    assert.equal(await connectSsh(access, ['-T', 'cat ssh-marker'], box.id), 0)
    await box.waitUntil('running')
    assert.equal(box.state, 'running')
  } finally {
    await box.ssh.revoke()
  }
}
export async function interactiveAcceptance(context: Context) {
  const observations: Observation[] = []
  const html = readFileSync('agent/tests/fixtures/desktop-application.html', 'utf8')
  const application = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      if (new URL(request.url).pathname === '/observe' && request.method === 'POST') {
        observations.push((await request.json()) as Observation)
        return new Response(null, { status: StatusCodes.NO_CONTENT })
      }
      return new Response(html, { headers: { 'content-type': 'text/html; charset=utf-8' } })
    },
  })
  const box = await context.make()
  try {
    context.png(await box.desktop.screenshot())
    await box.desktop.key('ctrl+l')
    await box.desktop.type(application.url.toString())
    await box.desktop.key('Return')
    await until(() => observations.some((event) => event.kind === 'ready'))
    const geometry = observations.find((event) => event.kind === 'ready')
    assert(geometry?.input && geometry.button)
    await box.desktop.click(geometry.input.x, geometry.input.y)
    await box.desktop.type('TypeScript native desktop')
    await box.desktop.click(geometry.button.x, geometry.button.y)
    await until(
      () =>
        observations.some((event) => event.kind === 'type' && event.value === 'TypeScript native desktop') &&
        observations.some((event) => event.kind === 'click'),
    )
    await novnc(box)
    await context.python(box, 'native-desktop', { PRYX_DESKTOP_POINTS: JSON.stringify(geometry) })
    for (const mode of ['sync', 'async']) {
      await until(() =>
        observations.some(
          (event) => event.kind === 'type' && event.value === `Python ${mode} native desktop`,
        ),
      )
    }
    assert(observations.filter((event) => event.kind === 'click').length >= 3)
    console.log(
      JSON.stringify({
        installed: true,
        desktop: true,
        applicationClick: true,
        applicationText: true,
        screenshot: true,
        edgeNovnc: true,
        bothPythonModes: true,
      }),
    )
  } finally {
    await box.delete()
    await application.stop(true)
  }
  const ssh = await context.make()
  try {
    await nativeSshCommand(ssh)
    await nativeSshCommand(ssh, true)
    const output = await context.command(
      [context.cli, 'sandbox', 'ssh', ssh.id, '-T', 'cat ssh-marker'],
      process.env,
    )
    assert.equal(output, SSH_MARKER)
    await context.python(ssh, 'ssh', { PRYX_TOOLS_HTTP_SSH: '1' })
    console.log(
      JSON.stringify({
        installed: true,
        ssh: true,
        command: true,
        httpEdgeWake: true,
        installedCli: true,
        bothPythonModes: true,
      }),
    )
  } finally {
    await ssh.delete()
  }
}
