// SPDX-License-Identifier: FSL-1.1-ALv2
/** Handle contracts over the current agent protocol: eager commands, line framing, bounded
 * results, binary data, lifecycle invalidation and one-shot cleanup. */
import { expect, test } from 'bun:test'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../../control/src/durations.ts'
import { seconds } from '../src/duration.ts'
import { Pryx, PryxError } from '../src/index.ts'

const view = { id: 'sbx_one', state: 'running', endpoints: [] }
const result = {
  code: 0,
  timedOut: false,
  killed: false,
  durationMs: 2,
  stdoutTruncated: false,
  stderrTruncated: false,
}
const output = (stream: string, data: string) => ({
  type: 'output',
  stream,
  bytes: Buffer.from(data).toString('base64'),
})
function fixture(
  events: () => Response,
  other?: (path: string, init?: RequestInit) => Response | Promise<Response | undefined> | undefined,
) {
  const calls: { path: string; body: Record<string, unknown>; key: string | null }[] = []
  const client = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async (input, init) => {
      const path = new URL(String(input)).pathname
      const body =
        typeof init?.body === 'string' && new Headers(init.headers).get('content-type') === 'application/json'
          ? JSON.parse(init.body)
          : {}
      calls.push({ path, body, key: new Headers(init?.headers).get('idempotency-key') })
      const response = await other?.(path, init)
      if (response) return response
      if (path.endsWith('/token'))
        return Response.json({
          token: 'scoped',
          expiresAt: Date.now() / SECOND_MS + 600,
          url: 'http://agent',
          epoch: 1,
          publicKey: 'public',
        })
      if (path === '/_pryx/processes')
        return Response.json({ id: 'process', pid: 123, running: true, result: null })
      if (path.endsWith('/output')) return events()
      if (path.endsWith('/snapshots')) return Response.json({ id: 'snap_one' })
      if (path.endsWith('/sleep')) return Response.json({ sandbox: { ...view, state: 'sleeping' } })
      if (path.endsWith('/wake')) return Response.json({ sandbox: view })
      return Response.json(view)
    }) as typeof fetch,
  })
  return { client, calls }
}
const frames = (events: unknown[]) =>
  new Response(events.map((event) => `${JSON.stringify(event)}\n`).join(''))
test('handles normalize durations and assign one command identity', async () => {
  const { client, calls } = fixture(() =>
    frames([
      output('stdout', 'hi\n'),
      output('stderr', 'warn'),
      { type: 'exit', result: { ...result, stderrTruncated: true } },
    ]),
  )
  const box = await client.sandbox({ image: 'node:22', idle: '10m', ttl: '2h' })
  expect(calls[0]?.body).toMatchObject({ idle: 600, ttl: 7200 })
  expect(calls[0]?.key).toBeString()
  expect('api' in client).toBe(false)
  expect(JSON.stringify(client)).not.toContain('secret')
  const run = box.run('printf hi', { timeout: '30s', env: { CI: '1' } })
  expect(run).toBeInstanceOf(Promise)
  expect(await run).toMatchObject({ stdout: 'hi\n', stderr: 'warn', code: 0, stderrTruncated: true })
  expect(await run).toMatchObject({ code: 0 })
  const starts = calls.filter((c) => c.path === '/_pryx/processes')
  expect(starts).toHaveLength(1)
  expect(starts[0]?.body).toEqual({ command: 'printf hi', timeout: 30, env: { CI: '1' } })
  expect(starts[0]?.key).toBeString()
  await box.sleep()
  expect(box.state).toBe('sleeping')
  await box.wake()
  expect(box.state).toBe('running')
  const snap = await box.snapshot()
  expect(JSON.parse(JSON.stringify(snap))).toEqual(snap.view)
  expect((await snap.fork({ idle: '5m' })).id).toBe(view.id)
  expect(calls.at(-1)?.body.idle).toBe(300)
  expect(calls.at(-1)?.key).toBeString()
})
test('line iteration is live and keeps channel boundaries', async () => {
  let finish = () => {}
  const ended = new Promise<void>((resolve) => {
    finish = resolve
  })
  const { client } = fixture(
    () =>
      new Response(
        new ReadableStream({
          async start(controller) {
            const send = (event: unknown) =>
              controller.enqueue(new TextEncoder().encode(`${JSON.stringify(event)}\n`))
            send(output('stdout', 'first\npar'))
            await ended
            send(output('stderr', 'warning\n'))
            send(output('stdout', 'tial\r\n'))
            send({ type: 'exit', result })
            controller.close()
          },
        }),
      ),
  )
  const run = (await client.sandbox(view.id)).stream(['echo', 'first'])
  const lines = run.lines()
  expect(await lines.next()).toEqual({ value: 'first', done: false })
  finish()
  expect(await Array.fromAsync(lines)).toEqual(['warning', 'partial'])
  expect((await run.result()).stdout).toBe('first\npartial\r\n')
})
test('one-shot cleanup follows errors and missing exits remain unknown', async () => {
  let deletes = 0,
    execs = 0
  const { client } = fixture(
    () => {
      execs++
      return new Response('')
    },
    (_path, init) => {
      if (init?.method === 'DELETE') {
        deletes++
        return Response.json({ existed: true })
      }
    },
  )
  await expect(client.run('alpine', 'true')).rejects.toBeInstanceOf(PryxError)
  expect(execs).toBe(1)
  expect(deletes).toBe(1)
  for (const invalid of ['1ms', 'bad', '0s', '-1m']) expect(() => seconds(invalid as '1s')).toThrow()
  expect(seconds('1.5m')).toBe(90)
})
test('binary writes preserve UTF-8 and truncate to empty', async () => {
  const writes: Uint8Array[] = []
  const { client } = fixture(
    () => frames([]),
    async (path, init) => {
      if (path === '/_pryx/files/content') {
        writes.push(new Uint8Array(await new Response(init?.body).arrayBuffer()))
        return new Response(null, { status: StatusCodes.NO_CONTENT })
      }
    },
  )
  const box = await client.sandbox(view.id),
    data = 'é'.repeat(70_000)
  await box.write('src/app.ts', data)
  expect(Buffer.from(writes[0] ?? []).toString()).toBe(data)
  await box.write('src/app.ts', '')
  expect(writes.at(-1)?.length).toBe(0)
  const denied = new Pryx({
    url: 'http://api.test',
    key: 'secret',
    fetch: (async () =>
      Response.json(
        { error: { code: 'forbidden', message: 'Denied' } },
        { status: StatusCodes.FORBIDDEN },
      )) as unknown as typeof fetch,
  })
  await expect(denied.sandbox({ image: 'alpine' })).rejects.toBeInstanceOf(PryxError)
})
