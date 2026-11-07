// SPDX-License-Identifier: FSL-1.1-ALv2
/** Field surface contracts against a fake transport: no command or mutation is replayed, and
 * secrets remain in scoped Git input rather than the public create body or command argv. */
import { expect, test } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { Pryx, PryxError } from '../src/index.ts'

const result = {
  code: 0,
  timedOut: false,
  killed: false,
  durationMs: 1,
  stdoutTruncated: false,
  stderrTruncated: false,
}
function peer(version = 2) {
  const calls: {
    path: string
    method: string
    body: Record<string, unknown>
    raw: BodyInit | undefined | null
  }[] = []
  let command = ''
  let state = 'sleeping'
  let data = ''
  let failure: string | undefined
  const view = () => ({
    id: 'box',
    state,
    endpoints: [],
    name: 'named',
    labels: { role: 'agent' },
    workdir: '/data',
  })
  const client = new Pryx({
    url: 'http://api.test',
    key: 'project-secret',
    fetch: (async (input, init) => {
      const u = new URL(String(input)),
        path = u.pathname,
        method = init?.method ?? 'GET'
      const body =
        typeof init?.body === 'string' && new Headers(init.headers).get('content-type') === 'application/json'
          ? JSON.parse(init.body)
          : {}
      calls.push({ path, method, body, raw: init?.body })
      if (path.startsWith('/_pryx/'))
        expect(new Headers(init?.headers).get('authorization')).toBe('Bearer scoped')
      if (path.endsWith('/token'))
        return Response.json({
          url: 'http://agent.test',
          token: 'scoped',
          expiresAt: Date.now() + 999999,
          epoch: 1,
        })
      if (path === '/_pryx/health') return Response.json({ status: 'ok', apiVersion: version })
      if (path === '/v1/sandboxes' && method === 'GET') return Response.json({ sandboxes: [view()] })
      if (path.endsWith('/wake')) {
        state = 'running'
        return Response.json({ sandbox: view() })
      }
      if (path === '/_pryx/processes') {
        if (method === 'GET')
          return Response.json({ processes: [{ id: 'proc', pid: 1, running: true, result: null }] })
        command = JSON.stringify(body.command)
        return Response.json({ id: 'proc', pid: 1, running: true, result: null })
      }
      if (path.endsWith('/ready'))
        return failure
          ? Response.json(
              { error: { code: failure, message: failure } },
              { status: failure === 'wait_timeout' ? StatusCodes.REQUEST_TIMEOUT : StatusCodes.CONFLICT },
            )
          : Response.json(body.kind === 'exit' ? result : { ready: true, matched: true })
      if (path.endsWith('/kill-all')) return Response.json({ killed: ['proc'] })
      if (path === '/_pryx/shells') return Response.json({ id: 'shell' })
      if (path.endsWith('/shells/shell/run'))
        return Response.json({ ...result, stdout: 'state', stderr: 'err', code: 7 })
      if (path.endsWith('/output')) {
        const stdout = command.includes('status')
          ? ''
          : command.includes('rev-parse')
            ? 'abc\n'
            : command.includes('log')
              ? 'abc\0Name\0email\0date\0subject\0\n'
              : ''
        return new Response(
          [
            JSON.stringify({
              type: 'output',
              stream: 'stdout',
              bytes: Buffer.from(stdout).toString('base64'),
            }),
            JSON.stringify({ type: 'exit', result }),
          ].join('\n') + '\n',
        )
      }
      if (path.endsWith('/find')) return Response.json({ paths: ['known.txt'] })
      if (path.endsWith('/grep'))
        return Response.json({ matches: [{ path: 'known.txt', line: 2, text: 'known' }] })
      if (path.endsWith('/replace')) return Response.json({ replacements: 1 })
      if (path.endsWith('/stat'))
        return u.searchParams.get('path') === 'missing'
          ? Response.json({ error: { code: 'not_found' } }, { status: StatusCodes.NOT_FOUND })
          : Response.json({ kind: 'file', size: data.length })
      if (path.endsWith('/content')) {
        if (method === 'PUT') {
          data = await new Response(init?.body).text()
          return new Response(null, { status: StatusCodes.NO_CONTENT })
        }
        return new Response(data)
      }
      if (path.endsWith('/snapshots')) return Response.json({ id: '00000000-0000-4000-8000-000000000001' })
      if (path.includes('/templates/') && path.endsWith('/versions'))
        return Response.json({ id: 'template', name: 'named', version: 1, snapshot: body.snapshot })
      if (path === '/v1/templates') return Response.json({ templates: [] })
      if (path === '/v1/webhooks')
        return method === 'POST'
          ? Response.json({ id: 'hook', ...body, secret: 'shown-once' })
          : Response.json({ webhooks: [] })
      if (path.endsWith('/deadletters')) return Response.json({ deliveries: [] })
      if (path.endsWith('/events'))
        return new Response(JSON.stringify({ id: 1, kind: 'sandbox.created', at: 'now', data: {} }) + '\n')
      if (path.endsWith('/metrics')) return Response.json({ at: 'now', metrics: { memoryBytes: 12 } })
      if (method === 'DELETE') return Response.json({ existed: true })
      return Response.json(view())
    }) as typeof fetch,
  })
  return {
    client,
    calls,
    fail: (code: string | undefined) => {
      failure = code
    },
  }
}
test('process waits preserve errors, use one start and close one-shot stdin', async () => {
  const p = peer(),
    box = await p.client.sandbox('box'),
    process = await box.spawn('cat', { stdin: new Uint8Array([0, 255]) })
  await process.waitForPort(3000, { mode: 'http', path: '/ready', status: StatusCodes.OK, timeout: '2s' })
  await process.waitForLog('ready', '2s')
  expect((await process.waitForExit()).code).toBe(0)
  expect((await box.processes())[0]?.id).toBe('proc')
  expect(await box.killAll()).toEqual({ killed: ['proc'] })
  expect(p.calls.filter((c) => c.path === '/_pryx/processes' && c.method === 'POST')).toHaveLength(1)
  expect(p.calls.filter((c) => c.path.endsWith('/stdin')).map((c) => c.method)).toEqual(['POST', 'DELETE'])
  for (const code of ['process_exited', 'wait_timeout']) {
    p.fail(code)
    try {
      await process.waitForPort(1)
      throw new Error('accepted')
    } catch (error) {
      expect(error).toBeInstanceOf(PryxError)
      expect((error as PryxError).code).toBe(code)
    }
  }
  const old = peer(1)
  await expect((await old.client.sandbox('box')).shell()).rejects.toMatchObject({ code: 'unsupported' })
  expect(old.calls.some((c) => c.path === '/_pryx/shells')).toBe(false)
})
test('shell, file conveniences and disk progress share agent transport', async () => {
  const p = peer(),
    box = await p.client.sandbox('box'),
    sh = await box.shell({ env: { X: '1' } })
  expect(await sh.run('pwd')).toMatchObject({ stdout: 'state', stderr: 'err', code: 7 })
  await sh.close()
  expect(await box.files.exists('missing')).toBe(false)
  expect(await box.files.find('**/*.txt')).toEqual(['known.txt'])
  expect((await box.files.grep('known', { paths: ['known.txt'] }))[0]?.line).toBe(2)
  expect(await box.files.replace('known', 'new', { paths: ['known.txt'] })).toEqual({ replacements: 1 })
  await box.files.chmod('known.txt', 0o600)
  const root = mkdtempSync(join(tmpdir(), 'pryx-field-'))
  try {
    writeFileSync(join(root, 'source'), 'binary\0content')
    const progress: number[] = []
    await box.files.upload(join(root, 'source'), 'known.txt', { onProgress: (n) => progress.push(n) })
    await box.files.download('known.txt', join(root, 'dest'))
    expect(readFileSync(join(root, 'dest')).toString()).toBe('binary\0content')
    expect(progress.at(-1)).toBe(14)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
test('git parses results and keeps its token off create and argv', async () => {
  const p = peer(),
    box = await p.client.sandbox({
      repo: 'https://github.com/cubepals/pryx',
      isolation: 'wasm',
      token: 'git-secret',
    })
  expect((await box.git.status()).value.clean).toBe(true)
  await box.git.diff({ staged: true })
  await box.git.branch('topic')
  await box.git.checkout('topic', { create: true })
  await box.git.add('known.txt')
  expect((await box.git.commit('message', { author: { name: 'Name', email: 'email' } })).value.hash).toBe(
    'abc',
  )
  await box.git.pull()
  expect((await box.git.log({ limit: 1 })).value[0]?.subject).toBe('subject')
  const create = p.calls.find((c) => c.path === '/v1/sandboxes' && c.method === 'POST')
  expect(JSON.stringify(create?.body)).not.toContain('git-secret')
  expect(create?.body.packages).toContain('git')
  const clone = p.calls.find(
    (c) => c.path === '/_pryx/processes' && JSON.stringify(c.body.command).includes('clone'),
  )
  expect(JSON.stringify(clone?.body.command)).not.toContain('git-secret')
  expect((clone?.body.env as Record<string, string>).PRYX_GIT_TOKEN).toBe('git-secret')
  expect((clone?.body.env as Record<string, string>).PRYX_GIT_HOST).toBe('github.com')
  await expect(box.git.clone('https://secret@github.com/repo')).rejects.toThrow('credentials')
  await expect(box.git.checkout('--orphan')).rejects.toThrow('Invalid ref')
})
test('lookup, lifecycle, templates and webhooks use public ownership routes', async () => {
  const p = peer()
  const inspected = await p.client.sandbox({ name: 'named', wake: false })
  expect(inspected.state).toBe('sleeping')
  const box = await p.client.sandbox('box')
  expect(box.state).toBe('running')
  await p.client.sandboxes({ labels: { role: 'agent' }, name: 'named', state: 'running' })
  await box.keepAlive('2m')
  await box.setTtl('3m')
  await box.setIdle('1m')
  await box.waitUntil('running')
  expect((await box.metrics()).metrics?.memoryBytes).toBe(12)
  expect((await Array.fromAsync(box.events()))[0]?.kind).toBe('sandbox.created')
  const t = await p.client.template({
    name: 'named',
    isolation: 'wasm',
    run: ['true'],
    ready: 'true',
    start: 'sleep 60',
  })
  expect(t.version).toBe(1)
  expect((await t.rebuild()).name).toBe('named')
  await p.client.templates()
  const hook = await p.client.webhook({ url: 'https://example.com/hook', kinds: ['created', 'deleted'] })
  expect(hook.secret).toBe('shown-once')
  expect(await hook.deadletters()).toEqual([])
  await hook.delete()
  expect(await p.client.webhooks()).toEqual([])
})
