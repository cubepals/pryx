// SPDX-License-Identifier: FSL-1.1-ALv2
/** Process-tier path regression with real shell commands and files behind a loopback agent fixture.
 * Full daemon lifecycle and rootless host policy are exercised by the CI deployment suite.
 */
import { expect, test } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { MINUTE_S, SECOND_MS } from '../../../control/src/durations.ts'
import { pryxProvider } from '../src/provider.ts'

test('process paths share the discovered persistent directory for files and execution', async () => {
  const root = mkdtempSync(join(tmpdir(), 'pryx-provider-process-'))
  const commands: { command: string | string[]; cwd?: string }[] = []
  const outputs = new Map<string, string>()
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request): Promise<Response> {
      const url = new URL(request.url)
      if (url.pathname === '/v1/sandboxes/box')
        return Response.json({
          id: 'box',
          state: 'running',
          isolation: 'process',
          endpoints: [],
          workdir: '/wrong-template-cwd',
        })
      if (url.pathname.endsWith('/token'))
        return Response.json({
          url: server.url.href,
          token: 'scoped',
          expiresAt: Date.now() / SECOND_MS + MINUTE_S,
        })
      if (url.pathname === '/_pryx/processes') {
        const body = (await request.json()) as { command: string | string[]; cwd?: string }
        commands.push(body)
        const child = Bun.spawn(
          typeof body.command === 'string' ? ['sh', '-c', body.command] : body.command,
          {
            cwd: body.cwd ?? root,
            stdout: 'pipe',
            stderr: 'pipe',
          },
        )
        const [stdout, stderr, code] = await Promise.all([
          new Response(child.stdout).text(),
          new Response(child.stderr).text(),
          child.exited,
        ])
        const id = String(commands.length)
        outputs.set(
          id,
          `${[
            { type: 'output', stream: 'stdout', bytes: btoa(stdout) },
            { type: 'output', stream: 'stderr', bytes: btoa(stderr) },
            {
              type: 'exit',
              result: {
                code,
                killed: false,
                timedOut: false,
                durationMs: 1,
                stdoutTruncated: false,
                stderrTruncated: false,
              },
            },
          ]
            .map((event) => JSON.stringify(event))
            .join('\n')}\n`,
        )
        return Response.json({ id, pid: child.pid, running: false, result: null })
      }
      if (url.pathname.endsWith('/output'))
        return new Response(outputs.get(url.pathname.split('/').at(-2) ?? ''))
      if (url.pathname === '/_pryx/files/content') {
        const file = join(root, url.searchParams.get('path') ?? '')
        if (request.method === 'PUT') {
          writeFileSync(file, new Uint8Array(await request.arrayBuffer()))
          return new Response(null, { status: StatusCodes.NO_CONTENT })
        }
        return new Response(readFileSync(file))
      }
      throw new Error(`Unexpected fixture route: ${url.pathname}`)
    },
  })
  try {
    const provider = pryxProvider({ url: server.url.href, key: 'secret' })
    expect(provider.cwd).toBeUndefined()
    const box = await provider.connect({ actorId: 'actor', client: () => undefined }, 'box')
    if (!box) throw new Error('Missing fixture sandbox')
    expect(box.cwd).toBe(root)
    expect(commands[0]?.cwd).toBeUndefined()
    await box.writeFile(`${box.cwd}/hello.txt`, 'persistent')
    expect(new TextDecoder().decode(await box.readFile(`${box.cwd}/hello.txt`))).toBe('persistent')
    expect(await box.exec('cat hello.txt; pwd', { cwd: box.cwd })).toMatchObject({
      exitCode: 0,
      stdout: `persistent${root}\n`,
    })
    expect(() => box.readFile('/data/hello.txt')).toThrow('outside persistent')
    expect(() => box.readFile(`${root}/../outside`)).toThrow('outside persistent')
  } finally {
    server.stop(true)
    rmSync(root, { recursive: true, force: true })
  }
})
