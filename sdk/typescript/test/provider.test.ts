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
      if (url.pathname.endsWith('/wake')) state = 'running'
      if (url.pathname.endsWith('/sleep')) state = 'sleeping'
      if (request.method === 'DELETE') status = StatusCodes.NOT_FOUND
      if (url.pathname === '/_pryx/files/stat') return Response.json({ kind: 'directory', name: 'dir' })
      if (url.pathname === '/_pryx/files/list') return Response.json({ files: [{ name: 'file' }] })
      const view = { id: 'box', state, isolation: 'container', endpoints: [] }
      return Response.json(
        url.pathname.endsWith('/wake') || url.pathname.endsWith('/sleep') ? { sandbox: view } : view,
      )
    },
  })
  return {
    calls,
    server,
    options: { url: server.url.href, key: 'secret' },
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
    await provider.suspend?.(actor, 'box')
    expect(f.calls.filter((c) => c.path.endsWith('/wake'))).toHaveLength(1)
    await pryxPortAddress('box', 8080, f.options)
    await provider.destroy?.(actor, 'box')
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
