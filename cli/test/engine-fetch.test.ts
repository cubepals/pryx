// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise the CI engine fetch function against actual HTTP failures and corrupted bytes.
 * The fixture never runs engine setup, installs packages or executes the fetched content. */
import { expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'

test('engine acquisition uses authenticated mirror bytes and refuses corruption from every source', async () => {
  const script = readFileSync(new URL('../../scripts/setup-test-engine.sh', import.meta.url), 'utf8')
  const fetch = script.match(/^fetch\(\) \{\n[\s\S]*?^\}/m)?.[0]
  if (!fetch) throw new Error('The engine acquisition function is missing')
  const directory = mkdtempSync(join(tmpdir(), 'pryx-engine-fetch-'))
  const content = new Uint8Array([0, 1, 2, 3, 128, 255])
  const digest = createHash('sha256').update(content).digest('hex')
  const requests: string[] = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch(request) {
      const path = new URL(request.url).pathname
      requests.push(path)
      if (path === '/missing') return new Response(null, { status: StatusCodes.NOT_FOUND })
      return new Response(path === '/valid' ? content : 'corrupted package')
    },
  })
  const acquire = async (primary: string, fallback: string, name: string) => {
    const output = join(directory, name)
    const child = Bun.spawn(
      [
        'bash',
        '-c',
        `${fetch}\nfetch "$@"`,
        'engine-fetch',
        `${server.url.origin}${primary}`,
        digest,
        output,
        `${server.url.origin}${fallback}`,
      ],
      {
        env: { ...process.env, NO_PROXY: '127.0.0.1', no_proxy: '127.0.0.1' },
        stdout: 'pipe',
        stderr: 'pipe',
      },
    )
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])
    return { code, stdout, stderr, output }
  }
  try {
    for (const primary of ['/missing', '/corrupt']) {
      const result = await acquire(primary, '/valid', primary.slice(1))
      expect(result.code, result.stderr).toBe(0)
      expect(new Uint8Array(await Bun.file(result.output).arrayBuffer())).toEqual(content)
    }
    const refused = await acquire('/corrupt', '/corrupt', 'refused')
    expect(refused.code).not.toBe(0)
    expect(requests).toEqual(['/missing', '/valid', '/corrupt', '/valid', '/corrupt', '/corrupt'])
  } finally {
    server.stop(true)
    rmSync(directory, { recursive: true, force: true })
  }
})
