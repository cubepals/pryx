// SPDX-License-Identifier: FSL-1.1-ALv2
/** K9 handles and real live output, files and lifecycle through the daemon. No fake output or timing-based proxy for a live stream. */
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { MINUTE_MS } from '../src/durations.ts'
import { deployment, ready } from './deployment.ts'

describe.skipIf(!ready)('SDK handles end to end', () => {
  let d: Awaited<ReturnType<typeof deployment>>, client: Pryx
  beforeAll(async () => {
    d = await deployment()
    client = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
  }, 2 * MINUTE_MS)
  afterAll(async () => {
    await d?.close()
  })
  test(
    'live output unblocks the command, large files survive a fork and every lifecycle verb works',
    async () => {
      const box = await client.sandbox({ image: 'alpine:3.22', idle: '5m', ttl: '2h' })
      try {
        const command = box.stream(
          'echo ready; while [ ! -f /data/continue ]; do sleep 0.1; done; echo done',
          {
            timeout: '10s',
          },
        )
        const lines = command.lines()
        expect(await lines.next()).toEqual({ value: 'ready', done: false })
        await box.write('continue', 'go')
        expect(await Array.fromAsync(lines)).toEqual(['done'])
        expect(await command.result()).toMatchObject({ code: 0, timedOut: false })
        const cli = Bun.spawn(
          [
            'bun',
            'cli/src/main.ts',
            'sandbox',
            'exec',
            box.id,
            '--',
            'sh',
            '-c',
            'echo cli-ready; while [ ! -f /data/cli-go ]; do sleep 0.1; done; echo cli-done',
          ],
          { env: d.env, stdout: 'pipe', stderr: 'pipe' },
        )
        const reader = cli.stdout.getReader(),
          decoder = new TextDecoder()
        expect(decoder.decode((await reader.read()).value)).toBe('cli-ready\n')
        await box.write('cli-go', 'go')
        let output = ''
        while (true) {
          const next = await reader.read()
          if (next.done) break
          output += decoder.decode(next.value, { stream: true })
        }
        expect(output).toBe('cli-done\n')
        expect(await new Response(cli.stderr).text()).toBe('')
        expect(await cli.exited).toBe(0)
        const data = crypto.getRandomValues(new Uint8Array(60_000))
        const large = new Uint8Array(3 * data.length)
        large.set(data)
        large.set(data, data.length)
        large.set(data, 2 * data.length)
        await box.write('nested/data.bin', large)
        expect(await box.readBytes('nested/data.bin')).toEqual(large)
        const snap = await box.snapshot(),
          twin = await snap.fork({ idle: '5m' })
        try {
          expect(await twin.readBytes('nested/data.bin')).toEqual(large)
        } finally {
          await twin.delete()
        }
        const heldUnits = (await client.project()).usage.units
        expect(heldUnits).toBeGreaterThan(0)
        await box.sleep()
        expect((await client.project()).usage.units).toBe(heldUnits)
        await box.wake()
        await box.stop()
        await box.start()
        await box.rest()
        await box.wake()
        expect((await box.refresh()).state).toBe('running')
        expect((await client.sandboxes()).some((b) => b.id === box.id)).toBe(true)
      } finally {
        await box.delete()
      }
    },
    3 * MINUTE_MS,
  )
})
