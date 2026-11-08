// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real warmed interpreter templates traverse the edge, both SDKs, MCP and the CLI.
 * Measurements retain every sample; direct loopback timing bypasses only the edge proxy. */
import { expect, test } from 'bun:test'
import { readFileSync, writeFileSync } from 'node:fs'
import { builtinTemplate } from '../../cli/src/builtin-template.ts'
import type { Kernel, Sandbox } from '../../sdk/typescript/src/index.ts'
import { Pryx } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { AGENT_ROUTE_PORT } from '../src/edge-headers.ts'
import { decodeHandle } from '../src/handle.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'
import { until } from './waits.ts'

async function exercise(kernel: Kernel, python: boolean) {
  expect((await kernel.run(python ? 'x = 41' : 'var x = 41')).status).toBe('ok')
  expect((await kernel.run('x + 1')).text).toBe('42')
  const error = await kernel.run(python ? "raise ValueError('known')" : "throw new Error('known')")
  expect(error.error?.value).toContain('known')
  expect(error.error?.traceback.length).toBeGreaterThan(0)
  const stream = kernel.stream(
    python
      ? "print('interrupt-ready', flush=True)\nwhile True: pass"
      : "console.log('interrupt-ready'); while(true) {}",
  )
  let interrupted = false
  for await (const event of stream) {
    if (!interrupted && event.type === 'stream' && event.text.includes('interrupt-ready')) {
      interrupted = true
      await kernel.interrupt()
    }
  }
  expect(interrupted).toBe(true)
  expect((await kernel.run('x + 1')).text).toBe('42')
  expect((await kernel.run('1+1')).text).toBe('2')
  const old = kernel.pid
  await kernel.restart()
  expect(kernel.pid).not.toBe(old)
  expect((await kernel.run('x')).error).toBeDefined()
}

test(
  'warmed code interpreters keep state and return images in every client',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment('container')
    let box: Sandbox | undefined
    try {
      for (const [name, image] of [
        ['python-interpreter', 'alpine:pryx-python-interpreter'],
        ['node-interpreter', 'alpine:pryx-node-interpreter'],
      ]) {
        const plan = builtinTemplate(name as string, image)
        if (!plan) throw new Error('Missing built-in plan')
        await c.sdk.template(plan)
        box = await c.sdk.sandbox({ template: name, isolation: 'container', idle: '10m' })
        await c.route(box.id)
        const python = name === 'python-interpreter'
        const kernel = python ? await box.python() : await box.javascript()
        const kernels = await box.kernels()
        expect(kernels).toHaveLength(1)
        expect(kernels[0]?.state).toBe('idle')
        expect(kernel.id).toBe(kernels[0]?.id)
        expect((await box.processes()).some((process) => process.id === kernel.processId)).toBe(true)
        await exercise(kernel, python)
        if (!python) {
          await box.write('chart.cjs', readFileSync('sdk/typescript/test/fixtures/chart.cjs'))
          const chart = await kernel.run("$$.mime({'image/png':require('./chart.cjs')().toString('base64')})")
          expect(chart.png?.slice(0, 8)).toEqual(new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]))
        }
        const context = python
          ? await box.python({ context: 'analysis' })
          : await box.javascript({ context: 'analysis' })
        expect(context.id).not.toBe(kernel.id)
        expect((await context.run(python ? 'value = 9' : 'var value = 9')).status).toBe('ok')
        expect((await kernel.run('value')).error).toBeDefined()
        await context.close()
        if (python) {
          await box.write('sales.csv', 'a,b\n1,2\n3,4\n')
          const result = await kernel.run('import pandas as pd; df = pd.read_csv("sales.csv"); df.describe()')
          expect(result.html).toContain('<table')
          expect(Array.isArray(result.json)).toBe(true)
          const chart = await kernel.run('plt.plot([1, 2, 3]); plt.show()')
          expect(chart.png?.slice(0, 8)).toEqual(new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]))
          expect(chart.svg).toContain('<svg')
          const response = await fetch(await box.mcpUrl(), {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({
              jsonrpc: '2.0',
              id: 1,
              method: 'tools/call',
              params: { name: 'run_python', arguments: { code: 'plt.plot([3,2,1]); plt.show()' } },
            }),
          })
          const rpc = (await response.json()) as {
            result: { content: { type: string; mimeType?: string }[] }
          }
          expect(
            rpc.result.content.some((part) => part.type === 'image' && part.mimeType === 'image/png'),
          ).toBe(true)
          const child = Bun.spawn(
            [
              process.env.PRYX_MCP_CLI ?? process.execPath,
              ...(process.env.PRYX_MCP_CLI ? [] : ['cli/src/main.ts']),
              'sandbox',
              'python',
              '-c',
              'print(2+2)',
            ],
            { env: c.d.env, stdout: 'pipe', stderr: 'pipe' },
          )
          const [out, err, code] = await Promise.all([
            new Response(child.stdout).text(),
            new Response(child.stderr).text(),
            child.exited,
          ])
          expect({ out, err, code }).toEqual({ out: '4\n', err: '', code: 0 })
        }
        const child = Bun.spawn([process.env.PRYX_PYTHON ?? 'python3', 'sdk/python/e2e/interpreter.py'], {
          env: {
            ...c.d.env,
            PRYX_KERNEL_SANDBOX: box.id,
            PRYX_KERNEL_LANGUAGE: python ? 'python' : 'javascript',
          },
          stdout: 'inherit',
          stderr: 'inherit',
        })
        expect(await child.exited).toBe(0)
        const managed = (await box.processes()).find((process) => process.id === kernel.processId)
        if (!managed) throw new Error('Kernel was not supervised')
        await managed.kill()
        await managed.waitForExit()
        await expect(kernel.run('1+1')).rejects.toThrow()
        expect((await box.kernels())[0]?.state).toBe('dead')
        await kernel.restart()
        await kernel.close()
        await box.delete()
        box = undefined
      }
      const samples: Record<string, number[]> = {}
      for (let i = 0; i < 30; i++) {
        box = await c.sdk.sandbox({
          template: 'python-interpreter',
          isolation: 'container',
          idle: '10m',
          units: 4,
        })
        await c.route(box.id)
        const kernel = await box.python(),
          start = performance.now()
        expect((await kernel.run('1+1')).json).toBe(2)
        samples.first_run_warm_template ??= []
        samples.first_run_warm_template.push(performance.now() - start)
        await box.delete()
        box = undefined
      }
      box = await c.sdk.sandbox({
        template: 'python-interpreter',
        isolation: 'container',
        idle: '10m',
        units: 4,
      })
      await until('edge holds interpreter', async () =>
        c.edge.table.held.get(`${box?.id}:${AGENT_ROUTE_PORT}`),
      )
      const row = await c.d.control.sandboxes.record(box.id)
      const endpoint = row.handle && decodeHandle(row.handle).agent?.port.endpoints[0]
      if (!endpoint) throw new Error('Missing node loopback agent endpoint')
      const sdk = new Pryx({
        url: c.d.control.apiUrl,
        key: c.d.credential.key,
        fetch: (async (input, init) => {
          const response = await fetch(input, init)
          return String(input).endsWith('/token') && response.ok
            ? Response.json({ ...(await response.json()), url: `http://${endpoint}` })
            : response
        }) as typeof fetch,
      })
      const kernel = await (await sdk.sandbox(box.id)).python()
      for (const [name, code] of [
        ['trivial_loopback', '1+1'],
        ['chart_loopback', 'plt.plot([1,2,3]); plt.show()'],
        ['restart_loopback', ''],
      ]) {
        for (let i = 0; i < 30; i++) {
          const start = performance.now()
          if (code) {
            const result = await kernel.run(code)
            expect(result.status).toBe('ok')
            if (name === 'chart_loopback') expect(result.png?.length).toBeGreaterThan(100)
          } else await kernel.restart()
          samples[name as string] ??= []
          samples[name as string]?.push(performance.now() - start)
        }
      }
      const summary = Object.fromEntries(
        Object.entries(samples).map(([name, values]) => {
          const sorted = [...values].sort((a, b) => a - b)
          return [name, { n: values.length, p50Ms: sorted[14], p90Ms: sorted[26] }]
        }),
      )
      console.log('Interpreter measurements', JSON.stringify(summary))
      if (process.env.PRYX_INTERPRETER_MEASUREMENTS)
        writeFileSync(
          process.env.PRYX_INTERPRETER_MEASUREMENTS,
          JSON.stringify({ samples, summary }, null, 2),
        )
      expect(summary.trivial_loopback?.p90Ms).toBeLessThan(50)
      expect(summary.chart_loopback?.p90Ms).toBeLessThan(1000)
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      await box?.delete()
      await c.close()
    }
  },
  10 * MINUTE_MS,
)
