// SPDX-License-Identifier: FSL-1.1-ALv2
/** The native tool and interpreter templates on the gvisor tier, through the edge and the SDK.
 * Without runsc registered (the main e2e run) the tier is refused. With it (`PRYX_TEST_GVISOR=1`,
 * after deploy/gvisor.sh), each template does its own work as on containers, every result is
 * written to `PRYX_GVISOR_TOOLS`, and each must succeed: a template gVisor couldn't run would
 * be refused on this tier by capability, with its reason, and asserted here instead. */
import { expect, test } from 'bun:test'
import { writeFileSync } from 'node:fs'
import { sql } from 'drizzle-orm'
import { builtinTemplate } from '../../cli/src/builtin-template.ts'
import { connectSsh } from '../../cli/src/sandbox-ssh.ts'
import type { Sandbox } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import type { RuntimeHandle } from '../src/runtime-model.ts'
import { one } from '../src/sql.ts'
import type { ExecResponse, WorkloadView } from '../src/wire.ts'
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
    const seconds: Record<string, number> = {}
    const boxes: Sandbox[] = []
    // Each tool runs alone, as its container case does, and its sandbox goes before the next.
    const attempt = async (name: string, work: () => Promise<void>) => {
      const began = performance.now()
      try {
        await work()
        results[name] = 'ok'
      } catch (error) {
        results[name] = String(error instanceof Error ? error.message : error).slice(0, 600)
      } finally {
        seconds[name] = Math.round((performance.now() - began) / 100) / 10
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
      // The published node exec, past its deadline: on gvisor the injected agent keeps the
      // deadline inside the sandbox, so the command's whole group is gone and the sandbox stays.
      await attempt('node-exec-deadline', async () => {
        const box = await c.sdk.sandbox({ image: 'alpine:3.22', isolation: 'gvisor' })
        boxes.push(box)
        const row = await one<{ handle: RuntimeHandle }>(
          c.d.control.db,
          sql`SELECT handle FROM sandboxes WHERE id = ${box.id}`,
        )
        if (!row?.handle) throw new Error('The sandbox has no node handle')
        const node = await c.d.control.runtime.home(row.handle)
        if (!node) throw new Error('The sandbox has no node')
        const ref = decodeHandle(row.handle)
        const began = performance.now()
        const result = await c.d.control.nodes.call<ExecResponse>(
          node,
          'POST',
          `/v1/workloads/${ref.key}/exec`,
          {
            body: { command: ['sh', '-c', 'setsid sh -c "sleep 60 & wait" & sleep 60'], timeoutSeconds: 2 },
            epoch: ref.epoch,
            timeoutMs: 30 * SECOND_MS,
          },
        )
        expect([result.timedOut, result.killed]).toEqual([true, true])
        expect(performance.now() - began).toBeLessThan(10 * SECOND_MS)
        const left = await box.run(['sh', '-c', "ps -o args | grep -c '^sleep 60' || true"])
        expect(left.stdout.trim()).toBe('0')
        expect((await box.refresh()).state).toBe('running')
      })
      await attempt('node-exec-owner-killed', async () => {
        const box = await c.sdk.sandbox({ image: 'alpine:pryx-python-interpreter', isolation: 'gvisor' })
        boxes.push(box)
        const row = await one<{ handle: RuntimeHandle }>(
          c.d.control.db,
          sql`SELECT handle FROM sandboxes WHERE id = ${box.id}`,
        )
        if (!row?.handle) throw new Error('The sandbox has no node handle')
        const node = await c.d.control.runtime.home(row.handle)
        if (!node) throw new Error('The sandbox has no node')
        const ref = decodeHandle(row.handle)
        const view = await c.d.control.nodes.call<WorkloadView>(node, 'GET', `/v1/workloads/${ref.key}`)
        const pending = c.d.control.nodes
          .call<ExecResponse>(node, 'POST', `/v1/workloads/${ref.key}/exec`, {
            body: {
              command: [
                'sh',
                '-c',
                'echo "$PPID" > /data/deadline-owner; setsid sh -c "echo \\$\\$ > /data/deadline-escaped; exec sleep 60" & sleep 60',
              ],
              timeoutSeconds: 15,
            },
            epoch: ref.epoch,
            timeoutMs: 30 * SECOND_MS,
          })
          .then(
            () => true,
            () => false,
          )
        const attack = await box
          .run([
            'python3',
            '-c',
            [
              'import ctypes,os,signal,time',
              'until=time.monotonic()+10',
              'while not (os.path.exists("/data/deadline-owner") and os.path.exists("/data/deadline-escaped")) and time.monotonic()<until: time.sleep(0.01)',
              'p=int(open("/data/deadline-owner").read())',
              'for suffix in ["mem","environ","fd/2"]:',
              ' try:',
              '  f=os.open("/proc/"+str(p)+"/"+suffix,os.O_RDONLY); os.close(f); raise RuntimeError("owner file unexpectedly readable")',
              ' except PermissionError: pass',
              'open("/data/deadline-attack-confirmed","w").write("protected")',
              'assert ctypes.CDLL(None).tgkill(p,p,signal.SIGKILL)==0',
              'print("owner protected and signalled")',
            ].join('\n'),
          ])
          .catch(() => undefined)
        // Engine retirement can end the attacking sibling before its success line flushes.
        expect(attack?.stderr ?? '').not.toContain('owner file unexpectedly readable')
        expect(await pending).toBe(false)
        const inspect = Bun.spawn(
          ['docker', 'inspect', '--format', '{{.State.Running}}', view.locate.containerName],
          { stdout: 'pipe', stderr: 'pipe' },
        )
        const running = await new Response(inspect.stdout).text()
        expect(await inspect.exited).toBe(0)
        expect(running.trim()).toBe('false')
        const proof = await c.d.control.nodes.transfer(
          node,
          'GET',
          `/v1/workloads/${ref.key}/files/content?${new URLSearchParams({ path: 'deadline-attack-confirmed' })}`,
          { epoch: ref.epoch, signal: AbortSignal.timeout(30 * SECOND_MS) },
        )
        expect(proof.ok).toBe(true)
        expect(await proof.text()).toBe('protected')
      })
      // A caller that goes away loses only the answer: the command runs on inside the sandbox
      // to its own end, and the sandbox keeps running.
      await attempt('node-exec-caller-gone', async () => {
        const box = await c.sdk.sandbox({ image: 'alpine:3.22', isolation: 'gvisor' })
        boxes.push(box)
        const row = await one<{ handle: RuntimeHandle }>(
          c.d.control.db,
          sql`SELECT handle FROM sandboxes WHERE id = ${box.id}`,
        )
        if (!row?.handle) throw new Error('The sandbox has no node handle')
        const node = await c.d.control.runtime.home(row.handle)
        if (!node) throw new Error('The sandbox has no node')
        const ref = decodeHandle(row.handle)
        const view = await c.d.control.nodes.call<WorkloadView>(node, 'GET', `/v1/workloads/${ref.key}`)
        const answered = await c.d.control.nodes
          .call<ExecResponse>(node, 'POST', `/v1/workloads/${ref.key}/exec`, {
            body: { command: ['sh', '-c', 'sleep 6; echo finished > /data/caller-gone'], timeoutSeconds: 60 },
            epoch: ref.epoch,
            timeoutMs: 2 * SECOND_MS,
          })
          .then(
            () => true,
            () => false,
          )
        expect(answered).toBe(false)
        await Bun.sleep(8 * SECOND_MS)
        const inspect = Bun.spawn(
          ['docker', 'inspect', '--format', '{{.State.Running}}', view.locate.containerName],
          { stdout: 'pipe', stderr: 'pipe' },
        )
        const running = await new Response(inspect.stdout).text()
        expect(await inspect.exited).toBe(0)
        expect(running.trim()).toBe('true')
        expect((await box.run(['cat', '/data/caller-gone'])).stdout.trim()).toBe('finished')
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
          if (!python) return
          // A writable home: matplotlib keeps its configuration and cache there, warning nothing.
          const home = await box.run([
            'python3',
            '-c',
            'import os, matplotlib; print(os.environ["HOME"]); print(matplotlib.get_cachedir())',
          ])
          expect(home.stderr).toBe('')
          const [dir, cache] = home.stdout.trim().split('\n')
          expect(cache?.startsWith(`${dir}/`)).toBe(true)
          // Evidence only: how long the template's ready step takes on this tier, beside the
          // default deadline its build gives each step.
          const ready = builtinTemplate(name, image)?.ready
          if (!ready) throw new Error('The python template has no ready step')
          const samples: number[] = []
          for (let sample = 0; sample < 3; sample++)
            samples.push((await box.run(['sh', '-c', ready], { timeout: '3m' })).durationMs)
          console.log('GVISOR_PYTHON_READY_MS', JSON.stringify(samples))
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
      console.log('GVISOR_TOOLS_SECONDS', JSON.stringify(seconds))
      if (process.env.PRYX_GVISOR_TOOLS)
        writeFileSync(process.env.PRYX_GVISOR_TOOLS, JSON.stringify(results, null, 2))
      // Every template ran on gvisor in CI run 43774620164; none needs refusing on this tier.
      expect(results).toEqual({
        image: 'ok',
        'node-exec-deadline': 'ok',
        'node-exec-owner-killed': 'ok',
        'node-exec-caller-gone': 'ok',
        'python-interpreter': 'ok',
        'node-interpreter': 'ok',
        browser: 'ok',
        desktop: 'ok',
        ssh: 'ok',
      })
    } finally {
      for (const box of boxes) await box.delete().catch(() => {})
      await c.close()
    }
  },
  15 * MINUTE_MS,
)
