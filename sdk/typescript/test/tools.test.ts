// SPDX-License-Identifier: FSL-1.1-ALv2
/** Native handles keep agent authority scoped and never send or serialize SSH private keys. */
import { expect, test } from 'bun:test'
import { SDK_DEFAULTS } from '../src/generated/defaults.ts'
import { TOOL_DEFAULTS } from '../src/generated/tools.ts'
import { Pryx } from '../src/index.ts'

const png = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])
test('desktop/browser binary calls and SSH registration use their storage owners once', async () => {
  const calls: { path: string; auth: string | null; body: unknown }[] = []
  const client = new Pryx({
    url: 'http://control.test',
    key: 'project',
    fetch: (async (input, init) => {
      const path = new URL(String(input)).pathname,
        body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined
      calls.push({ path, body, auth: new Headers(init?.headers).get('authorization') })
      if (path.endsWith('/token'))
        return Response.json({
          token: 'scoped',
          url: 'http://agent.test',
          expiresAt: Date.now() / SDK_DEFAULTS.secondMs + 300,
          epoch: 1,
        })
      if (path === '/_pryx/health') return Response.json({ toolsVersion: TOOL_DEFAULTS.version })
      if (path.endsWith('/screenshot')) return new Response(png, { headers: { 'content-type': 'image/png' } })
      if (path === '/_pryx/ssh')
        return Response.json({
          id: 'a',
          key: 'PRIVATE-ONCE',
          hostKey: 'ssh-ed25519 public',
          port: 1234,
          slot: 2,
          user: 'pryx',
          expiresAt: 123,
          running: true,
        })
      if (path === '/v1/sandboxes/one/ssh')
        return Response.json(
          init?.method === 'DELETE'
            ? { revoked: true }
            : {
                id: 'a',
                host: 'one.test',
                port: 22000,
                user: 'pryx',
                hostKey: 'ssh-ed25519 public',
                expiresAt: 123,
              },
        )
      if (path === '/_pryx/mounts')
        return Response.json({
          mounts: [
            { bucket: 's3://test', path: '/mnt/data', state: 'mounted', backend: 'rclone', readOnly: false },
          ],
        })
      if (path.startsWith('/_pryx/')) return Response.json({ url: 'http://preview.test', expiresAt: 123 })
      return Response.json({
        id: 'one',
        state: 'running',
        agent: { url: 'http://agent.test' },
        endpoints: [],
      })
    }) as typeof fetch,
  })
  const box = await client.sandbox('one')
  expect(await box.desktop.screenshot()).toEqual(png)
  await box.desktop.click(20, 30)
  await box.desktop.type('text')
  await box.desktop.key('Return')
  await box.desktop.move(4, 5)
  await box.desktop.scroll(0, 1)
  expect(await box.browser.screenshot('https://page.test')).toEqual(png)
  const access = await box.ssh()
  expect(access.key).toBe('PRIVATE-ONCE')
  expect(JSON.stringify(access)).not.toContain('PRIVATE-ONCE')
  expect(calls.find((c) => c.path === '/v1/sandboxes/one/ssh')?.body).toEqual({ id: 'a' })
  expect(calls.filter((c) => c.path === '/_pryx/ssh')).toHaveLength(1)
  expect((await box.mounts())[0]?.bucket).toBe('s3://test')
  await box.ssh.revoke()
  expect(calls.filter((c) => c.path.startsWith('/_pryx/')).every((c) => c.auth === 'Bearer scoped')).toBe(
    true,
  )
  expect(JSON.stringify(calls)).not.toContain('PRIVATE-ONCE')
})
