// SPDX-License-Identifier: FSL-1.1-ALv2
/** Handle behavior against an observable transport: duration normalization, exactly-once runs, live iteration, file chunks and cleanup. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { Pryx, PryxError, seconds } from '../src/index.ts'

const view = { id: 'sbx_one', state: 'running', endpoints: [] }
test('handles normalize durations, retain raw methods and assign mutation keys once', async () => {
  const calls: { path: string; body: Record<string, unknown>; key: string | null }[] = []
  const client = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async (url, options) => {
      const path = new URL(String(url)).pathname
      const body = options?.body ? JSON.parse(String(options.body)) : {}
      calls.push({ path, body, key: new Headers(options?.headers).get('idempotency-key') })
      if (path.endsWith('/exec'))
        return new Response(
          '{"type":"stdout","data":"hi\\n","truncated":false}\n{"type":"stderr","data":"warn","truncated":true}\n{"type":"exit","code":0,"timedOut":false,"killed":false,"durationMs":2}\n',
        )
      if (path.endsWith('/fork')) throw new Error('Wrong fork endpoint')
      if (path.endsWith('/snapshots')) return Response.json({ id: 'snap_one' })
      if (path.endsWith('/sleep')) return Response.json({ sandbox: { ...view, state: 'sleeping' } })
      if (path.endsWith('/wake')) return Response.json({ sandbox: view })
      return Response.json(view)
    }) as typeof fetch,
  })
  const box = await client.sandbox({ image: 'node:22', idle: '10m', ttl: '2h' })
  expect(calls[0]?.body).toMatchObject({ idle: 600, ttl: 7200 })
  expect(calls[0]?.key).toBeString()
  expect(client.api).toBe(client)
  expect(JSON.stringify(client)).not.toContain('secret')
  const run = box.run('printf hi', { timeout: '30s', env: { CI: '1' } })
  expect(calls).toHaveLength(1)
  expect(await run).toMatchObject({ stdout: 'hi\n', stderr: 'warn', code: 0, stderrTruncated: true })
  expect(await run).toMatchObject({ code: 0 })
  expect(calls.filter((call) => call.path.endsWith('/exec'))).toHaveLength(1)
  expect(calls[1]?.body).toEqual({ command: ['sh', '-c', 'printf hi'], timeout: 30, env: { CI: '1' } })
  expect(calls[1]?.key).toBeString()
  await box.sleep()
  expect(box.state).toBe('sleeping')
  await box.wake()
  expect(box.state).toBe('running')
  const snap = await box.snapshot()
  expect((await snap.fork({ idle: '5m' })).id).toBe(view.id)
  expect(calls.at(-1)?.body.idle).toBe(300)
  expect(calls.at(-1)?.key).toBeString()
})
test('line iteration yields before exit and keeps stdout and stderr line boundaries', async () => {
  let finish: () => void = () => {}
  const ended = new Promise<void>((resolve) => {
    finish = resolve
  })
  const client = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async (url) => {
      if (!String(url).endsWith('/exec')) return Response.json(view)
      return new Response(
        new ReadableStream({
          async start(controller) {
            const send = (value: unknown) =>
              controller.enqueue(new TextEncoder().encode(JSON.stringify(value) + '\n'))
            send({ type: 'stdout', data: 'first\npar', truncated: false })
            await ended
            send({ type: 'stderr', data: 'warning\n', truncated: false })
            send({ type: 'stdout', data: 'tial\r\n', truncated: false })
            send({ type: 'exit', code: 0, timedOut: false, killed: false, durationMs: 1 })
            controller.close()
          },
        }),
      )
    }) as typeof fetch,
  })
  const box = await client.sandbox('sbx_one')
  const run = box.run(['echo', 'first'])
  const lines = run.lines()
  expect(await lines.next()).toEqual({ value: 'first', done: false })
  finish()
  expect(await Array.fromAsync(lines)).toEqual(['warning', 'partial'])
  expect((await run).stdout).toBe('first\npartial\r\n')
})
test('one-shot cleanup runs on command errors and a missing exit stays unknown', async () => {
  let deletes = 0,
    execs = 0
  const client = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async (_url, options) => {
      if (options?.method === 'DELETE') {
        deletes++
        return Response.json({ existed: true })
      }
      if (String(_url).endsWith('/exec')) {
        execs++
        return new Response('')
      }
      return Response.json(view)
    }) as typeof fetch,
  })
  await expect(client.run('alpine', 'true')).rejects.toBeInstanceOf(PryxError)
  expect(execs).toBe(1)
  expect(deletes).toBe(1)
  for (const invalid of ['1ms', 'bad', '0s', '-1m']) expect(() => seconds(invalid as '1s')).toThrow()
  expect(seconds('1.5m')).toBe(90)
})
test('writes exceed the old file bound in chunks, including UTF-8 and empty truncation', async () => {
  const writes: { offset: number; dataBase64: string }[] = []
  const client = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async (url, options) => {
      if (String(url).includes('/file-chunks/')) {
        writes.push(JSON.parse(String(options?.body)))
        return Response.json({ written: true })
      }
      return Response.json(view)
    }) as typeof fetch,
  })
  const box = await client.sandbox('sbx_one')
  const data = 'é'.repeat(70_000)
  await box.write('src/app.ts', data)
  expect(writes.length).toBeGreaterThan(1)
  expect(Buffer.concat(writes.map((write) => Buffer.from(write.dataBase64, 'base64'))).toString()).toBe(data)
  await box.write('src/app.ts', '')
  expect(writes.at(-1)).toEqual({ offset: 0, dataBase64: '' })
  const denied = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async (_url, _options) =>
      Response.json(
        { error: { code: 'forbidden', message: 'Denied' } },
        { status: StatusCodes.FORBIDDEN },
      )) as typeof fetch,
  })
  await expect(denied.sandbox({ image: 'alpine' })).rejects.toBeInstanceOf(PryxError)
})
