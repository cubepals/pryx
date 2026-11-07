// SPDX-License-Identifier: FSL-1.1-ALv2
/** Observe an actual child-process file before awaiting its SDK result; the owned loopback
 * HTTP fixture exercises eagerness without substituting for real sandbox acceptance. */

import { expect, test } from 'bun:test'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../../control/src/durations.ts'
import * as surface from '../src/index.ts'
import { Pryx } from '../src/index.ts'

test.each(['run', 'stream'] as const)('%s starts before its result is awaited', async (method) => {
  const folder = mkdtempSync(join(tmpdir(), 'pryx-eager-'))
  const marker = join(folder, 'started')
  let starts = 0
  let child: Bun.Subprocess<'ignore', 'pipe', 'pipe'> | undefined
  const result = {
    code: 0,
    timedOut: false,
    killed: false,
    durationMs: 0,
    stdoutTruncated: false,
    stderrTruncated: false,
  }
  const server: Bun.Server<unknown> = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request): Promise<Response> {
      const path = new URL(request.url).pathname
      if (path === '/v1/sandboxes')
        return Response.json({
          id: 'eager',
          state: 'running',
          agent: { url: server.url.origin },
          files: true,
          endpoints: [],
        })
      if (path.endsWith('/token'))
        return Response.json({
          token: 'owned-fixture',
          expiresAt: Date.now() / SECOND_MS + 60,
          epoch: 1,
          publicKey: 'fixture',
          url: server.url.origin,
        })
      if (request.headers.get('authorization') !== 'Bearer owned-fixture')
        return new Response('Denied', { status: StatusCodes.UNAUTHORIZED })
      if (path === '/_pryx/processes') {
        starts++
        const body = (await request.json()) as { command: string | string[] }
        const command = typeof body.command === 'string' ? ['sh', '-c', body.command] : body.command
        child = Bun.spawn(command, { cwd: folder, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' })
        return Response.json({ id: 'process', pid: child.pid, running: true, result: null })
      }
      if (path.endsWith('/output')) {
        if (!child) throw new Error('No child')
        const [code, stdout, stderr] = await Promise.all([
          child.exited,
          new Response(child.stdout).text(),
          new Response(child.stderr).text(),
        ])
        const events: unknown[] = []
        for (const [stream, data] of [
          ['stdout', stdout],
          ['stderr', stderr],
        ])
          if (data) events.push({ type: 'output', stream, bytes: Buffer.from(data).toString('base64') })
        events.push({ type: 'exit', result: { ...result, code } })
        return new Response(events.map((e) => `${JSON.stringify(e)}\n`).join(''))
      }
      return Response.json(result)
    },
  })
  try {
    const sdk = new Pryx({ url: server.url.origin, key: 'project-fixture' })
    const box = await sdk.sandbox({ image: 'owned-fixture' })
    const operation = box[method]('printf started > started')
    const run =
      method === 'run'
        ? (operation as Promise<surface.RunResult>)
        : (operation as surface.CommandStream).result()
    const deadline = Date.now() + 5 * SECOND_MS
    while (!existsSync(marker) && Date.now() < deadline) await Bun.sleep(SECOND_MS / 100)
    const observedWithoutAwait = existsSync(marker)
    const startedWithoutAwait = starts
    const completed = await run
    expect(observedWithoutAwait).toBe(true)
    expect(startedWithoutAwait).toBe(1)
    expect(starts).toBe(1)
    expect(run).toBeInstanceOf(Promise)
    expect(completed.code).toBe(0)
  } finally {
    server.stop(true)
    rmSync(folder, { recursive: true, force: true })
  }
})
test('runtime exports have one surface', () => {
  expect(Object.keys(surface).sort()).toEqual(
    ['Pryx', 'Sandbox', 'Snapshot', 'Process', 'Terminal', 'Files', 'PryxError'].sort(),
  )
})
