// SPDX-License-Identifier: FSL-1.1-ALv2
/** The native tool and interpreter templates on the gvisor tier, through the edge and the SDK.
 * Without runsc registered (the main e2e run) the tier is refused by name. With it
 * (`PRYX_TEST_GVISOR=1`, after deploy/gvisor.sh), each template does its own work as on
 * containers, and every result is written to `PRYX_GVISOR_TOOLS`: what gVisor can't run is
 * found here, then refused on this tier by capability with its reason. */
import { expect, test } from 'bun:test'
import { writeFileSync } from 'node:fs'
import { builtinTemplate } from '../../cli/src/builtin-template.ts'
import { connectSsh } from '../../cli/src/sandbox-ssh.ts'
import type { Sandbox } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'

const PNG = [137, 80, 78, 71, 13, 10, 26, 10]
const isPng = (bytes: Uint8Array) => bytes.length > 100 && PNG.every((byte, i) => bytes[i] === byte)

test(
  'native tool and interpreter templates on the gvisor tier',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment('gvisor', { nativeTools: true })
    const results: Record<string, string> = {}
    const boxes: Sandbox[] = []
    // Each tool runs alone, as its container case does, and its sandbox goes before the next.
    const attempt = async (name: string, work: () => Promise<void>) => {
      try {
        await work()
        results[name] = 'ok'
      } catch (error) {
        results[name] = String(error instanceof Error ? error.message : error).slice(0, 600)
      } finally {
        for (const box of boxes.splice(0)) await box.delete().catch(() => {})
      }
    }
    const from = async (name: string, image: string, units?: number) => {
      const plan = builtinTemplate(name, image)
      if (!plan) throw new Error(`Missing ${name} plan`)
      await c.sdk.template({ ...plan, isolation: 'gvisor' })
      const box = await c.sdk.sandbox({ template: name, isolation: 'gvisor', units, idle: '10m' })
      boxes.push(box)
      await c.route(box.id)
      return box
    }
    try {
      if (process.env.PRYX_TEST_GVISOR !== '1') {
        // No node announces the tier without runsc, so a request naming it is never placed.
        await expect(c.sdk.sandbox({ image: 'alpine:3.22', isolation: 'gvisor' })).rejects.toThrow()
        return
      }
      await attempt('image', async () => {
        const box = await c.sdk.sandbox({ image: 'alpine:3.22', isolation: 'gvisor' })
        boxes.push(box)
        expect((await box.run(['uname', '-r'])).stdout).toContain('gvisor')
      })
      for (const [name, image, python] of [
        ['python-interpreter', 'alpine:pryx-python-interpreter', true],
        ['node-interpreter', 'alpine:pryx-node-interpreter', false],
      ] as const)
        await attempt(name, async () => {
          const box = await from(name, image)
          const kernel = python ? await box.python() : await box.javascript()
          expect((await kernel.run(python ? 'x = 41' : 'var x = 41')).status).toBe('ok')
          expect((await kernel.run('x + 1')).text).toBe('42')
        })
      await attempt('browser', async () => {
        const box = await from('browser', 'alpine:pryx-browser', 4)
        await box.write('index.html', '<!doctype html><title>gvisor browser</title><h1>Rendered</h1>')
        const server = await box.spawn([
          'python3',
          '-m',
          'http.server',
          '3000',
          '--bind',
          '127.0.0.1',
          '--directory',
          '/data',
        ])
        await server.waitForPort(3000)
        expect(isPng(await box.browser.screenshot('http://127.0.0.1:3000'))).toBe(true)
      })
      await attempt('desktop', async () => {
        const box = await from('desktop', 'alpine:pryx-desktop', 2)
        expect(isPng(await box.desktop.screenshot())).toBe(true)
      })
      await attempt('ssh', async () => {
        const box = await from('tools', 'alpine:pryx-native-tools')
        const lease = await box.ssh()
        expect(await connectSsh(lease, ['printf ssh-ok'])).toBe(0)
      })
      console.log('GVISOR_TOOLS', JSON.stringify(results))
      if (process.env.PRYX_GVISOR_TOOLS)
        writeFileSync(process.env.PRYX_GVISOR_TOOLS, JSON.stringify(results, null, 2))
      expect(Object.keys(results).sort()).toEqual(
        ['browser', 'desktop', 'image', 'node-interpreter', 'python-interpreter', 'ssh'].sort(),
      )
      // Proved on gvisor in CI run 43766414887; the others' results decide their refusals.
      expect([results.image, results['python-interpreter']]).toEqual(['ok', 'ok'])
    } finally {
      for (const box of boxes) await box.delete().catch(() => {})
      await c.close()
    }
  },
  15 * MINUTE_MS,
)
