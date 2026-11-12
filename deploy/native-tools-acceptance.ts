// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise a real ordinary-user installation through its public API and edge. Existing Python
 * SDK acceptance runs unchanged; this script retains raw process-tier timing samples only. */
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { readClientConfig } from '../cli/src/client-config.ts'
import { SECOND_MS } from '../control/src/durations.ts'
import { Pryx, type Sandbox } from '../sdk/typescript/src/index.ts'

const credential = readClientConfig()
assert(credential, 'Installed CLI credentials are required')
assert(process.getuid?.() !== 0, 'Native acceptance must run as an ordinary user')
const installed = JSON.parse(readFileSync(join(homedir(), '.config/pryx/installation.json'), 'utf8'))
assert.equal(installed.preset, 'user')
assert.equal(installed.tier, 'process')
const sdk = new Pryx(credential)
const browserOn = process.env.PRYX_NATIVE_BROWSER === 'on'
assert(['on', 'off'].includes(process.env.PRYX_NATIVE_BROWSER ?? ''))
const samples: Record<string, number[]> = {}
const pngSignature = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])

function png(bytes: Uint8Array | undefined) {
  assert(bytes && bytes.length > 100)
  assert.deepEqual(bytes.subarray(0, pngSignature.length), pngSignature)
}
async function command(argv: string[], env: Record<string, string | undefined>) {
  const child = Bun.spawn(argv, { env, stdout: 'ignore', stderr: 'pipe' })
  const [code, error] = await Promise.all([child.exited, new Response(child.stderr).text()])
  assert.equal(code, 0, `Existing Python SDK acceptance failed: ${error.slice(-2000)}`)
}
async function python(box: Sandbox, kind: string) {
  const interpreter = process.env.PRYX_PYTHON ?? 'python3'
  const env = { ...process.env, PRYX_API_URL: credential?.url, PRYX_API_KEY: credential?.key }
  if (kind === 'browser')
    await command([interpreter, 'sdk/python/e2e/native_tools.py'], {
      ...env,
      PRYX_TOOLS_SANDBOX: box.id,
      PRYX_TOOLS_KIND: kind,
    })
  else
    await command([interpreter, 'sdk/python/e2e/interpreter.py'], {
      ...env,
      PRYX_KERNEL_SANDBOX: box.id,
      PRYX_KERNEL_LANGUAGE: kind,
    })
}
async function cdp(url: string) {
  const without = await fetch(url.replace(/\?.*$/, ''))
  assert.equal(without.status, StatusCodes.FORBIDDEN)
  const websocket = new WebSocket(url.replace(/^http/, 'ws'))
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Installed edge CDP deadline')), 10 * SECOND_MS)
    websocket.onerror = () => {
      clearTimeout(timer)
      reject(new Error('Installed edge refused CDP'))
    }
    websocket.onopen = () => websocket.send(JSON.stringify({ id: 1, method: 'Browser.getVersion' }))
    websocket.onmessage = (event) => {
      const result = JSON.parse(String(event.data))
      if (result.id !== 1) return
      clearTimeout(timer)
      try {
        assert.match(result.result.product, /Chrome/)
        resolve()
      } catch (error) {
        reject(error)
      }
    }
  }).finally(() => websocket.close())
}
async function make(requireBrowser = false) {
  const box = await sdk.sandbox({
    isolation: 'process',
    units: 4,
    idle: '10m',
    ...(requireBrowser ? { require: ['browser-sandbox'] } : {}),
  })
  assert.equal(box.isolation, 'process')
  return box
}
const chart = {
  python: 'plt.plot([1,2,3]); plt.show()',
  javascript: "$$.mime({'image/png':require('./chart.cjs')().toString('base64')})",
}
for (const language of ['python', 'javascript'] as const) {
  const box = await make()
  try {
    await box.write('chart.cjs', readFileSync('sdk/typescript/test/fixtures/chart.cjs'))
    const kernel = language === 'python' ? await box.python() : await box.javascript()
    assert.equal((await kernel.run(language === 'python' ? 'value=40' : 'var value=40')).status, 'ok')
    assert.equal((await kernel.run('value+2')).text, '42')
    png((await kernel.run(chart[language])).png)
    await kernel.close()
    await python(box, language)
    console.log(
      JSON.stringify({ installed: true, language, stateRetained: true, png: true, bothPythonModes: true }),
    )
  } finally {
    await box.delete()
  }
}
const box = await make(browserOn)
try {
  if (browserOn) {
    await box.write('index.html', '<!doctype html><h1>Installed native browser 漢字 😀</h1>')
    const server = await box.spawn(['python3', '-m', 'http.server', '3000', '--bind', '127.0.0.1'])
    await server.waitForPort(3000)
    png(await box.browser.screenshot('http://127.0.0.1:3000'))
    const access = await box.browser()
    await cdp(access.url)
    await python(box, 'browser')
    await box.refresh()
    assert.equal(box.guarantees?.['browser-sandbox'], true)
    console.log(
      JSON.stringify({
        installed: true,
        browser: true,
        screenshot: true,
        edgeCdp: true,
        bothPythonModes: true,
      }),
    )
  } else {
    await assert.rejects(make(true), /browser-sandbox/)
    await assert.rejects(box.browser(), /user namespaces/)
    await box.refresh()
    assert.equal(box.guarantees?.['browser-sandbox'], false)
    console.log(
      JSON.stringify({ installed: true, browserRefused: true, reason: 'user namespaces', kernels: true }),
    )
  }
} finally {
  await box.delete()
}
if (browserOn) {
  const page = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () =>
      new Response('<h1>Native timed image 漢字 😀</h1>', { headers: { 'content-type': 'text/html' } }),
  })
  try {
    for (const kind of ['browser', 'python', 'javascript'] as const) {
      samples[kind] = []
      for (let sample = 0; sample < 30; sample++) {
        const started = performance.now()
        const box = await make()
        try {
          if (kind === 'browser') png(await box.browser.screenshot(`http://127.0.0.1:${page.port}`))
          else {
            await box.write('chart.cjs', readFileSync('sdk/typescript/test/fixtures/chart.cjs'))
            const kernel = kind === 'python' ? await box.python() : await box.javascript()
            png((await kernel.run(chart[kind])).png)
          }
          samples[kind].push(performance.now() - started)
        } finally {
          await box.delete()
        }
      }
      assert.equal(samples[kind].length, 30)
    }
  } finally {
    await page.stop(true)
  }
  assert(process.env.PRYX_NATIVE_MEASUREMENTS)
  writeFileSync(
    process.env.PRYX_NATIVE_MEASUREMENTS,
    `${JSON.stringify({ uid: process.getuid?.(), preset: 'user', isolation: 'process', unit: 'ms', point: 'create-to-first-image', samples }, null, 2)}\n`,
  )
}
