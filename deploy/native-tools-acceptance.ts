// SPDX-License-Identifier: FSL-1.1-ALv2
/** Exercise a real ordinary-user installation through its public API and edge. Existing Python
 * SDK acceptance runs unchanged; this script retains raw process-tier timing samples only. */
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import { readClientConfig } from '../cli/src/client-config.ts'
import { sshProxy } from '../cli/src/ssh-proxy.ts'
import { readInstallation } from '../cli/src/up-local-record.ts'
import { userPaths } from '../cli/src/up-paths.ts'
import { SECOND_MS } from '../control/src/durations.ts'
import { Agent } from '../sdk/typescript/src/agent.ts'
import { Pryx, type Sandbox } from '../sdk/typescript/src/index.ts'
import { Transport } from '../sdk/typescript/src/transport.ts'
import { hostProbe } from './host-probe.ts'
import { interactiveAcceptance, nativeSshCommand } from './native-interactive-acceptance.ts'

const credential = readClientConfig()
assert(credential, 'Installed CLI credentials are required')
assert(process.getuid?.() !== 0, 'Native acceptance must run as an ordinary user')
const sdk = new Pryx(credential)
if (process.argv[2] === 'sandbox' && process.argv[3] === 'ssh-proxy') {
  assert(process.argv[4] && process.argv[5])
  await sshProxy(sdk, process.argv[4], process.argv[5])
  process.exit(0)
}
const installed = readInstallation(userPaths())
assert(installed)
assert.equal(installed.preset, 'user')
assert.equal(installed.tier, 'process')
const browserOn = process.env.PRYX_NATIVE_BROWSER === 'on'
const host = await hostProbe(join(installed.bundle, 'bin/pryxd'))
const doctor = JSON.parse(await command([installed.paths.cli, 'doctor', '--json'], process.env))
assert.equal(doctor.capabilities['process-desktop'].state, browserOn ? 'on' : 'off')
assert.equal(doctor.capabilities['process-ssh'].state, browserOn ? 'on' : 'off')
const systemGlibc = Bun.spawnSync(['getconf', 'GNU_LIBC_VERSION'], { stdout: 'pipe', stderr: 'pipe' })
assert.equal(systemGlibc.exitCode, 0)
assert.equal(host.cLibrary, systemGlibc.stdout.toString().trim())
const nodeConfig = Bun.TOML.parse(readFileSync(join(installed.paths.config, 'pryxd.toml'), 'utf8')) as Record<
  string,
  unknown
>
const nodeState = nodeConfig.state_dir
assert(typeof nodeState === 'string')
const descriptor = JSON.parse(readFileSync(join(installed.bundle, 'bin/process-tools.json'), 'utf8'))
const runtimeRoot = join(
  String(nodeState),
  'process-tools',
  String(process.getuid?.()),
  'cache',
  descriptor.sha256,
)
const inputs = JSON.parse(readFileSync('templates/process-tools/artifacts.json', 'utf8')) as {
  name: string
  version: string
}[]
function version(name: string) {
  const pinned = inputs.find((input) => input.name === name)
  assert(pinned)
  return pinned.version
}
assert(['on', 'off'].includes(process.env.PRYX_NATIVE_BROWSER ?? ''))
const samples: Record<string, number[]> = {}
const pngSignature = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])

function png(bytes: Uint8Array | undefined) {
  assert(bytes && bytes.length > 100)
  assert.deepEqual(bytes.subarray(0, pngSignature.length), pngSignature)
}
async function command(argv: string[], env: Record<string, string | undefined>) {
  const child = Bun.spawn(argv, { env, stdout: 'pipe', stderr: 'pipe' })
  const [code, out, error] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  assert.equal(code, 0, `Installed client acceptance failed: ${error.slice(-2000)}`)
  return out
}
async function python(box: Sandbox, kind: string, extra: Record<string, string> = {}) {
  const interpreter = process.env.PRYX_PYTHON ?? 'python3'
  const env = { ...process.env, ...extra, PRYX_API_URL: credential?.url, PRYX_API_KEY: credential?.key }
  if (['browser', 'native-desktop', 'ssh'].includes(kind))
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
        assert.equal(result.result.product.split('/').at(-1), version('headless-shell'))
        resolve()
      } catch (error) {
        reject(error)
      }
    }
  }).finally(() => websocket.close())
}
async function make(requireBrowser = false, requiredTools?: ('desktop' | 'ssh')[]) {
  const box = await sdk.sandbox({
    isolation: 'process',
    units: 4,
    idle: '10m',
    ...(requiredTools ? { requiredTools } : {}),
    ...(requireBrowser ? { require: ['browser-sandbox'] } : {}),
  })
  assert.equal(box.isolation, 'process')
  return box
}
async function browserPage(box: Sandbox) {
  await box.write('index.html', '<!doctype html><h1>Installed native browser 漢字 😀</h1>')
  const server = await box.spawn(['python3', '-m', 'http.server', '3000', '--bind', '127.0.0.1'])
  await server.waitForPort(3000)
  return 'http://127.0.0.1:3000'
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
    const runtime = JSON.parse(
      (
        await kernel.run(
          language === 'python'
            ? "print(__import__('json').dumps({'executable':__import__('sys').executable,'version':__import__('platform').python_version()}))"
            : 'console.log(JSON.stringify({executable:process.execPath,version:process.versions.node}))',
        )
      ).stdout.trim(),
    )
    assert.equal(runtime.version, version(language === 'python' ? 'python-standalone' : 'node'))
    assert(runtime.executable.startsWith(`${join(runtimeRoot, language === 'python' ? 'python' : 'node')}/`))
    const error = await kernel.run(
      language === 'python' ? "raise ValueError('known')" : "throw new Error('known')",
    )
    assert.match(error.error?.value ?? '', /known/)
    assert(error.error && error.error.traceback.length > 0)
    let interrupted = false
    for await (const event of kernel.stream(
      language === 'python'
        ? "print('interrupt-ready',flush=True)\nwhile True: pass"
        : "console.log('interrupt-ready');while(true){}",
    )) {
      if (!interrupted && event.type === 'stream' && event.text.includes('interrupt-ready')) {
        interrupted = true
        await kernel.interrupt()
      }
    }
    assert(interrupted)
    assert.equal((await kernel.run('value+2')).text, '42')
    const old = kernel.pid
    await kernel.restart()
    assert.notEqual(kernel.pid, old)
    assert((await kernel.run('value')).error)
    await kernel.close()
    await python(box, language)
    if (language === 'python') {
      const agent = new Agent(new Transport(credential), box.id)
      const rpc = await agent.request<{
        result: { content: { type: string; mimeType?: string }[]; isError?: boolean }
      }>('/_pryx/mcp', 'POST', {
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: { name: 'run_python', arguments: { code: chart.python } },
      })
      assert(!rpc.result.isError)
      assert(rpc.result.content.some((part) => part.type === 'image' && part.mimeType === 'image/png'))
      assert.equal(
        await command([installed.paths.cli, 'sandbox', 'python', box.id, '-c', 'print(2+2)'], process.env),
        '4\n',
      )
    }
    console.log(
      JSON.stringify({
        installed: true,
        language,
        stateRetained: true,
        png: true,
        bothPythonModes: true,
        bundleRuntime: true,
        version: runtime.version,
        interrupt: true,
        restart: true,
        ...(language === 'python' ? { mcpRunPython: true, installedCli: true } : {}),
      }),
    )
  } finally {
    await box.delete()
  }
}
const box = await make(browserOn)
try {
  if (browserOn) {
    png(await box.browser.screenshot(await browserPage(box)))
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
    await assert.rejects(box.desktop(), /user namespaces/)
    await assert.rejects(box.ssh(), /user namespaces/)
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
  await interactiveAcceptance({
    make: () => make(false, ['desktop', 'ssh']),
    png,
    python,
    command,
    cli: installed.paths.cli,
  })
  for (const kind of ['browser', 'python', 'javascript', 'desktop', 'ssh'] as const) {
    samples[kind] = []
    for (let sample = 0; sample < 30; sample++) {
      const started = performance.now()
      const box = await make(false, kind === 'desktop' ? ['desktop'] : kind === 'ssh' ? ['ssh'] : undefined)
      try {
        if (kind === 'desktop') png(await box.desktop.screenshot())
        else if (kind === 'ssh') await nativeSshCommand(box)
        else if (kind === 'browser') png(await box.browser.screenshot(await browserPage(box)))
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
  assert(process.env.PRYX_NATIVE_MEASUREMENTS)
  writeFileSync(
    process.env.PRYX_NATIVE_MEASUREMENTS,
    `${JSON.stringify({ uid: process.getuid?.(), preset: 'user', isolation: 'process', architecture: host.architecture, hostCLibrary: host.cLibrary, landlockAbi: host.landlockAbi, unit: 'ms', point: 'create-to-first-image', samplePoints: { ssh: 'create-to-command' }, samples }, null, 2)}\n`,
  )
}
