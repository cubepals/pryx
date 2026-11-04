// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Execute the proposal Worker on the pinned public celld binary, exercising a real pryx sandbox's HTTP and sleep/wake. */
import { expect, test } from 'bun:test'
import { cpSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { Pryx } from '@cubepals/pryx'
import { deployment, ready } from '../../../control/e2e/deployment.ts'
import { freePort, until } from '../../../control/e2e/waits.ts'
import { MINUTE_MS } from '../../../control/src/durations.ts'
import { containerBridge } from '../src/index.ts'

test.skipIf(!ready || !process.env.CELLD_BIN)(
  'celld Worker starts, uses, sleeps and wakes a pryx container',
  async () => {
    const d = await deployment(),
      dir = mkdtempSync(`${tmpdir()}/pryx-celld-`),
      port = await freePort()
    const client = new Pryx({ baseUrl: d.control.apiUrl, apiKey: d.credential.key }),
      token = crypto.randomUUID()
    const bridge = containerBridge(client, {
      token,
      image: 'alpine:3.22',
      port: 8080,
      idle: 60,
      spec: {
        resources: { memoryMb: 128 },
        entrypoint: ['sh', '-c', 'echo celld-on-pryx > /data/index.html; httpd -f -p 8080 -h /data'],
      },
    })
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: bridge.fetch })
    cpSync(new URL('../prototype/', import.meta.url), dir, { recursive: true })
    writeFileSync(
      `${dir}/.dev.vars`,
      `PRYX_BRIDGE_URL=http://127.0.0.1:${server.port}\nPRYX_BRIDGE_TOKEN=${token}\n`,
      { mode: 0o600 },
    )
    const child = Bun.spawn([process.env.CELLD_BIN ?? 'celld', 'dev', '--port', String(port), '--logs'], {
      cwd: dir,
      stdout: 'inherit',
      stderr: 'inherit',
    })
    const url = `http://127.0.0.1:${port}`
    try {
      await until(
        'celld Worker HTTP',
        async () => (await (await fetch(url)).text()).includes('celld-on-pryx'),
        2 * MINUTE_MS,
      )
      const box = (await client.list()).sandboxes[0]
      if (!box) throw new Error('Worker did not create a sandbox')
      expect((await fetch(`${url}/sleep`)).ok).toBe(true)
      expect((await client.get(box.id)).state).toBe('sleeping')
      expect(await (await fetch(url)).text()).toContain('celld-on-pryx')
      expect((await client.get(box.id)).state).toBe('running')
      expect((await fetch(`${url}/delete`)).ok).toBe(true)
    } finally {
      child.kill()
      await child.exited
      server.stop(true)
      await d.close()
      rmSync(dir, { recursive: true, force: true })
    }
  },
  4 * MINUTE_MS,
)
