// SPDX-License-Identifier: FSL-1.1-ALv2
/** Provider boundary regressions over a loopback HTTP fixture. Real runtime behavior is in CI's e2e suite. */
import { expect, test } from 'bun:test'
import type { SandboxActorContext, SandboxProvider } from '@rivet-dev/sandbox-adapter'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../../control/src/durations.ts'
import { PryxError } from '../src/index.ts'
import { pryxPortAddress, pryxProvider } from '../src/provider.ts'

const actor: SandboxActorContext = { actorId: 'actor', client: () => undefined }
function fixture() {
  const calls: { path: string; method: string; body: unknown }[] = []
  let state = 'sleeping'
  let status = StatusCodes.OK
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request): Promise<Response> {
      const url = new URL(request.url)
      const body = await request.text()
      calls.push({ path: url.pathname, method: request.method, body: body ? JSON.parse(body) : undefined })
      if (url.pathname === '/v1/project')
        return Response.json({ defaults: { isolation: 'container', repoImage: 'configured:image' } })
      if (status !== StatusCodes.OK)
        return Response.json(
          { error: { code: 'fixture_refusal', message: 'Fixture refuses this request' } },
          { status },
        )
      if (url.pathname.endsWith('/token'))
        return Response.json({
          url: server.url.href,
          token: 'scoped',
          expiresAt: Date.now() / SECOND_MS + 60,
        })
      if (url.pathname === '/_pryx/ports') return Response.json({ url: 'https://preview.pryx.test/app' })
      if (url.pathname.endsWith('/wake') || url.pathname.endsWith('/start')) state = 'running'
      if (url.pathname.endsWith('/sleep')) state = 'sleeping'
      if (request.method === 'DELETE') status = StatusCodes.NOT_FOUND
      if (url.pathname === '/_pryx/files/stat') return Response.json({ kind: 'directory', name: 'dir' })
      if (url.pathname === '/_pryx/files/list') return Response.json({ files: [{ name: 'file' }] })
      const view = { id: 'box', state, isolation: 'container', endpoints: [] }
      return Response.json(
        url.pathname.endsWith('/wake') || url.pathname.endsWith('/sleep') || url.pathname.endsWith('/start')
          ? { sandbox: view }
          : view,
      )
    },
  })
  return {
    calls,
    server,
    options: { url: server.url.href, key: 'secret' },
    stop: () => {
      state = 'stopped'
    },
    refuse: (value: number) => {
      status = value
    },
  }
}

test('provider maps lifecycle, paths, defaults and edge preview', async () => {
  const f = fixture()
  try {
    const provider: SandboxProvider = pryxProvider(f.options)
    expect(await provider.create(actor)).toBe('box')
    expect(f.calls.find((c) => c.method === 'POST')?.body).toMatchObject({ image: 'configured:image' })
    const box = await provider.connect(actor, 'box')
    if (!box) throw new Error('Missing fixture sandbox')
    expect(box.cwd).toBe('/data')
    await box.mkdir('/data/dir')
    expect(await box.stat('/data/dir')).toEqual({ isDirectory: true })
    expect(await box.readdir('/data/dir')).toEqual(['file'])
    expect(await box.exists('/data/dir')).toBe(true)
    expect(() => box.readFile('/etc/passwd')).toThrow('outside persistent')
    expect(() => box.writeFile('/data/../outside', 'text')).toThrow('outside persistent')
    expect(() => box.mkdir('relative')).toThrow('absolute')
    f.stop()
    await provider.connect(actor, 'box')
    expect(f.calls.filter((c) => c.path.endsWith('/start'))).toHaveLength(1)
    await provider.suspend?.(actor, 'box')
    expect(f.calls.filter((c) => c.path.endsWith('/wake'))).toHaveLength(1)
    await pryxPortAddress('box', 8080, f.options)
    await provider.destroy?.(actor, 'box')
    expect(await provider.connect(actor, 'box')).toBeUndefined()
    await provider.destroy?.(actor, 'box')
    await provider.suspend?.(actor, 'box')
    f.refuse(StatusCodes.GONE)
    expect(await provider.connect(actor, 'box')).toBeUndefined()
  } finally {
    f.server.stop(true)
  }
})

test('requirements and tier mismatches refuse before execution', async () => {
  const f = fixture()
  try {
    await expect(pryxProvider({ ...f.options, require: ['separate-users'] }).create(actor)).rejects.toThrow(
      'separate-users',
    )
    expect(f.calls).toHaveLength(0)
    await expect(pryxProvider({ ...f.options, tier: 'wasm' }).connect(actor, 'box')).rejects.toThrow(
      'Required tier',
    )
    expect(f.calls.filter((c) => c.path.endsWith('/wake'))).toHaveLength(0)
    f.refuse(StatusCodes.FORBIDDEN)
    await expect(pryxProvider(f.options).connect(actor, 'box')).rejects.toBeInstanceOf(PryxError)
  } finally {
    f.server.stop(true)
  }
})

test('exec streams before exit, kills once on deadlines and aborts, and cleans listeners', async () => {
  let exit = Promise.withResolvers<boolean>()
  const finish = (killed: boolean) => exit.resolve(killed)
  let abortOnStart: AbortController | undefined
  let payload = 'live'
  let rejectKill = false
  let kills = 0
  let starts = 0
  const result = (killed: boolean) => ({
    code: killed ? null : 0,
    killed,
    timedOut: false,
    durationMs: 1,
    stdoutTruncated: false,
    stderrTruncated: false,
  })
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request): Promise<Response> {
      const url = new URL(request.url)
      if (url.pathname.endsWith('/token'))
        return Response.json({
          url: server.url.href,
          token: 'scoped',
          expiresAt: Date.now() / SECOND_MS + 60,
        })
      if (url.pathname === '/v1/sandboxes/box')
        return Response.json({ id: 'box', state: 'running', isolation: 'container', endpoints: [] })
      if (url.pathname === '/_pryx/processes') {
        starts++
        exit = Promise.withResolvers<boolean>()
        abortOnStart?.abort()
        return Response.json({ id: 'process', pid: 123, running: true, result: null })
      }
      if (url.pathname.endsWith('/output'))
        return new Response(
          new ReadableStream({
            async start(controller) {
              controller.enqueue(
                new TextEncoder().encode(
                  `${JSON.stringify({ type: 'output', stream: 'stdout', bytes: btoa(payload) })}\n`,
                ),
              )
              const killed = await exit.promise
              controller.enqueue(
                new TextEncoder().encode(`${JSON.stringify({ type: 'exit', result: result(killed) })}\n`),
              )
              controller.close()
            },
          }),
        )
      if (url.pathname.endsWith('/signal')) {
        kills++
        if (rejectKill)
          return Response.json(
            { error: { code: 'signal_refused', message: 'Fixture rejects signal' } },
            { status: StatusCodes.CONFLICT },
          )
        finish(true)
        return new Response(null, { status: StatusCodes.NO_CONTENT })
      }
      throw new Error(`Unexpected fixture route: ${url.pathname}`)
    },
  })
  try {
    const box = await pryxProvider({ url: server.url.href, key: 'secret' }).connect(actor, 'box')
    if (!box) throw new Error('Missing fixture sandbox')
    const normal = new AbortController()
    expect(
      await box.exec('stream', {
        cwd: '/data',
        signal: normal.signal,
        onData: (chunk) => {
          expect(new TextDecoder().decode(chunk)).toBe('live')
          finish(false)
        },
      }),
    ).toEqual({ stdout: 'live', stderr: '', timedOut: false, exitCode: 0 })
    const longDeadline = await box.exec('long-deadline', {
      cwd: '/data',
      timeoutMs: Number.MAX_SAFE_INTEGER,
      onData: () => {
        setTimeout(() => finish(false), SECOND_MS / 100)
      },
    })
    expect(longDeadline.timedOut).toBe(false)
    payload = String.fromCharCode(255, 254)
    await box.exec('binary', {
      cwd: '/data',
      onData: (chunk) => {
        expect(chunk).toEqual(new Uint8Array([255, 254]))
        finish(false)
      },
    })
    payload = 'live'
    normal.abort()
    expect(kills).toBe(0)
    expect(await box.exec('timeout', { cwd: '/data', timeoutMs: SECOND_MS / 100 })).toMatchObject({
      timedOut: true,
      exitCode: null,
    })
    expect(kills).toBe(1)
    const abort = new AbortController()
    await expect(
      box.exec('abort', { cwd: '/data', signal: abort.signal, onData: () => abort.abort() }),
    ).rejects.toThrow('aborted')
    expect(kills).toBe(2)
    const callbackError = new Error('caller callback failed')
    await expect(
      box.exec('callback', {
        cwd: '/data',
        onData: () => {
          throw callbackError
        },
      }),
    ).rejects.toBe(callbackError)
    expect(kills).toBe(3)
    // An abort before spawn has completed must still kill the eventual process.
    abortOnStart = new AbortController()
    await expect(box.exec('spawn-race', { cwd: '/data', signal: abortOnStart.signal })).rejects.toThrow(
      'aborted',
    )
    expect(kills).toBe(4)
    await expect(box.exec('pre-abort', { cwd: '/data', signal: abortOnStart.signal })).rejects.toThrow(
      'aborted',
    )
    expect(starts).toBe(7)
    abortOnStart = undefined
    rejectKill = true
    await expect(box.exec('failed-kill', { cwd: '/data', timeoutMs: SECOND_MS / 100 })).rejects.toThrow(
      'Fixture rejects signal',
    )
    finish(false)
    expect(kills).toBe(5)
  } finally {
    server.stop(true)
  }
})
