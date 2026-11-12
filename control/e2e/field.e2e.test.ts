// SPDX-License-Identifier: FSL-1.1-ALv2
/** One owned sandbox traverses the field surface through TypeScript, both Python context modes
 * and the actual CLI. Host egress acceptance lives in egress.e2e.test.ts. */
import { expect, test } from 'bun:test'
import { createHmac } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'
import type { Sandbox, TemplateOptions } from '../../sdk/typescript/src/index.ts'
import { MINUTE_MS } from '../src/durations.ts'
import { Webhooks } from '../src/webhooks.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'
import { webCommand } from './tier-fixture.ts'
import { until } from './waits.ts'

async function cli(env: Record<string, string | undefined>, ...args: string[]) {
  const command = process.env.PRYX_MCP_CLI
    ? [process.env.PRYX_MCP_CLI]
    : [process.execPath, 'cli/src/main.ts']
  const child = Bun.spawn([...command, ...args], { env, stdout: 'pipe', stderr: 'pipe' })
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  if (code !== 0) throw new Error(`CLI ${args[0]} ${args[1]} failed: ${stderr}`)
  if (!stdout.trim()) return undefined
  try {
    return JSON.parse(stdout)
  } catch {
    return stdout
  }
}
test(
  'field surface through both SDKs and the CLI on one sandbox',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment(undefined, { webhookAllowHosts: ['127.0.0.1'] })
    const root = mkdtempSync(join(tmpdir(), 'pryx-field-e2e-'))
    const received: { body: string; headers: Headers }[] = []
    const receiver = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      fetch: async (request) => {
        received.push({ body: await request.text(), headers: request.headers })
        return new Response(null, { status: StatusCodes.NO_CONTENT })
      },
    })
    const hooks = new Webhooks(c.d.control.db, c.d.control.config.providerSealKey, ['127.0.0.1'])
    let box: Sandbox | undefined
    const hook = await c.sdk.webhook({ url: receiver.url.toString(), kinds: ['created', 'deleted'] })
    try {
      const tier = process.env.PRYX_TEST_TIER ?? 'container'
      const name = 'field-owned'
      box = await c.sandbox({
        image: 'alpine:pryx-tls',
        ...(tier === 'wasm' ? { packages: ['git'] } : {}),
        name,
        env: { FIELD_VALUE: 'kept' },
        labels: { suite: 'field' },
        metadata: { nested: { value: 1 } },
        ttl: 600,
        idle: 300,
        resources: { memoryMb: 512 },
        sparePorts: 2,
      })
      const id = box.id
      const web = await box.spawn(webCommand())
      await box.write('index.html', 'field-ready')
      await web.waitForPort(3000, { mode: 'http', timeout: '30s' })
      const url = await box.expose(3000)
      const response = await c.app(url)
      expect(response.status).toBe(StatusCodes.OK)
      expect(await response.text()).toBe('field-ready')
      const input = await box.run('cat', { stdin: new TextEncoder().encode('input\0bytes') })
      expect(input.stdout).toBe('input\0bytes')
      const sh = await box.shell()
      try {
        await sh.run('mkdir -p state; cd state; export KEPT=yes')
        const state = await sh.run('printf "$KEPT"; pwd; printf err >&2; false')
        expect(state.stdout).toEndWith('/state\n')
        expect(state.stdout).toStartWith('yes')
        expect(state.stderr).toBe('err')
        expect(state.code).toBe(1)
      } finally {
        await sh.close()
      }
      const log = await box.spawn('printf "field-log\\n"; sleep 600')
      await log.waitForLog('field-log')
      await log.kill()
      await log.waitForExit()
      expect((await box.processes()).some((process) => process.id === web.id)).toBe(true)
      await box.write('known.txt', 'first\nfield needle\nlast\n')
      expect(await box.files.exists('known.txt')).toBe(true)
      expect(await box.files.find('**/*.txt')).toContain('known.txt')
      expect((await box.files.grep('needle', { paths: ['known.txt'] }))[0]?.line).toBe(2)
      expect((await box.files.replace('needle', 'changed', { paths: ['known.txt'] })).replacements).toBe(1)
      await box.files.chmod('known.txt', 0o600)
      mkdirSync(join(root, 'source'))
      mkdirSync(join(root, 'source/empty'))
      writeFileSync(join(root, 'source/local.txt'), 'local content')
      await box.files.upload(join(root, 'source/local.txt'), 'uploaded.txt')
      await box.files.download('uploaded.txt', join(root, 'downloaded.txt'))
      expect(readFileSync(join(root, 'downloaded.txt'), 'utf8')).toBe('local content')
      await box.files.uploadDir(join(root, 'source'), 'tree')
      await box.files.downloadDir('tree', join(root, 'tree'))
      expect(readFileSync(join(root, 'tree/local.txt'), 'utf8')).toBe('local content')
      expect((await box.sync(join(root, 'source'), 'sync')).uploaded).toEqual(['local.txt'])
      expect((await box.sync(join(root, 'source'), 'sync')).unchanged).toEqual(['local.txt'])
      await box.git.clone('https://github.com/octocat/Hello-World.git', { depth: 1 })
      expect((await box.git.status()).value.clean).toBe(true)
      await box.git.branch('field')
      await box.git.checkout('field')
      await box.write('repo/field.txt', 'committed')
      await box.git.add('field.txt')
      expect((await box.git.diff({ staged: true })).value.patch).toContain('committed')
      await box.git.commit('field commit', { author: { name: 'Field Test', email: 'field@example.test' } })
      expect((await box.git.log({ limit: 1 })).value[0]?.subject).toBe('field commit')
      for (const args of [
        ['init', '--bare', tier === 'wasm' ? '/tmp/field-local.git' : 'local.git'],
        [
          '-C',
          tier === 'wasm' ? '/tmp/field-local.git' : 'local.git',
          'config',
          'receive.shallowUpdate',
          'true',
        ],
        [
          '-C',
          'repo',
          'remote',
          'set-url',
          'origin',
          tier === 'wasm' ? '/tmp/field-local.git' : '../local.git',
        ],
        ['-C', 'repo', 'config', 'push.default', 'current'],
        ['-C', 'repo', 'config', 'remote.origin.fetch', '+refs/heads/*:refs/remotes/origin/*'],
        ['-C', 'repo', 'config', 'branch.field.remote', 'origin'],
        ['-C', 'repo', 'config', 'branch.field.merge', 'refs/heads/field'],
      ])
        expect((await box.run(['git', ...args])).code).toBe(0)
      await box.git.push()
      await box.git.pull()
      expect((await box.git.status()).value.clean).toBe(true)
      expect(
        (await c.sdk.sandboxes({ name, labels: { suite: 'field' }, state: 'running' })).map((box) => box.id),
      ).toContain(id)
      expect((await c.sdk.sandbox({ name })).id).toBe(id)
      await box.keepAlive('10m')
      await box.setTtl('10m')
      await box.setIdle('5m')
      await box.waitUntil('running')
      await until('reported metrics', async () => (await box?.metrics())?.metrics)
      // History is stored in minute buckets; a current observation can precede the next bucket.
      const metricsSince = new Date(Date.now() - MINUTE_MS).toISOString()
      await until('retained minute metrics', async () => {
        const history = await box?.metrics({ since: metricsSince })
        return history?.samples?.length
      })
      expect((await box.metrics({ since: metricsSince })).samples?.length).toBeGreaterThan(0)
      await box.killAll()
      await box.sleep()
      expect((await c.sdk.sandbox(id, { wake: false })).state).toBe('sleeping')
      box = await c.sdk.sandbox(id)
      expect(box.state).toBe('running')
      await box.rest()
      box = await c.sdk.sandbox({ name })
      expect(box.state).toBe('running')
      expect((await box.git.status()).value.clean).toBe(true)
      const plan: TemplateOptions = {
        name: 'field-code',
        isolation: tier as TemplateOptions['isolation'],
        ...(tier === 'container' ? { from: 'alpine:pryx-tls' } : {}),
        ...(tier === 'process' ? {} : { workdir: '/data' }),
        copy: [[join(root, 'source'), 'context']],
        run: ['printf built > built.txt'],
        ready: 'test -s built.txt',
        start: 'printf started > start-marker; sleep 600',
      }
      const template = await c.sdk.template(plan)
      expect((await template.rebuild()).version).toBe(template.version + 1)
      const child = await c.sdk.sandbox({ template: template.name })
      try {
        await until('template start command', async () => {
          try {
            return (await child.read('start-marker')) === 'started'
          } catch (error) {
            if (error instanceof Error && 'status' in error && error.status === StatusCodes.NOT_FOUND)
              return false
            throw error
          }
        })
        expect(await child.read('start-marker')).toBe('started')
        expect(await child.read('context/local.txt')).toBe('local content')
      } finally {
        await child.delete()
      }
      const python = Bun.spawn([process.env.PRYX_PYTHON ?? 'python3', 'sdk/python/e2e/field.py'], {
        env: {
          ...c.d.env,
          PRYX_FIELD_SANDBOX: id,
          PRYX_FIELD_LOCAL: root,
          PRYX_FIELD_RECEIVER: receiver.url.toString(),
          PRYX_FIELD_GIT_REMOTE: tier === 'wasm' ? '/tmp/field-local.git' : '../local.git',
          PRYX_TEST_EDGE_URL: c.origin,
        },
        stdout: 'pipe',
        stderr: 'pipe',
      })
      const [pythonOut, pythonErr, pythonCode] = await Promise.all([
        new Response(python.stdout).text(),
        new Response(python.stderr).text(),
        python.exited,
      ])
      if (pythonCode !== 0) throw new Error(`Python field acceptance failed: ${pythonErr}\n${pythonOut}`)
      await cli(c.d.env, 'sandbox', 'ps', id)
      const spawned = (await cli(
        c.d.env,
        'sandbox',
        'spawn',
        id,
        '--',
        'sh',
        '-c',
        'printf cli-log; sleep 600',
      )) as { id: string }
      await cli(c.d.env, 'sandbox', 'wait-log', id, spawned.id, 'cli-log')
      await cli(c.d.env, 'sandbox', 'kill-all', id)
      await cli(c.d.env, 'sandbox', 'wait-exit', id, spawned.id)
      await cli(c.d.env, 'sandbox', 'shell', id, '--command', 'printf cli-shell')
      await cli(c.d.env, 'sandbox', 'exists', id, 'known.txt')
      await cli(c.d.env, 'sandbox', 'find', id, '**/*.txt')
      await cli(c.d.env, 'sandbox', 'grep', id, 'changed', '--paths', 'known.txt')
      await cli(c.d.env, 'sandbox', 'replace', id, 'changed', 'cli-changed', '--paths', 'known.txt')
      await cli(c.d.env, 'sandbox', 'chmod', id, '600', 'known.txt')
      await cli(c.d.env, 'sandbox', 'sync', id, join(root, 'source'), 'cli-sync')
      await cli(c.d.env, 'sandbox', 'cp', join(root, 'source'), `${id}:cli-tree`)
      await cli(c.d.env, 'sandbox', 'cp', `${id}:cli-tree`, join(root, 'cli-tree'))
      const cliWeb = await box.spawn(webCommand())
      await cli(c.d.env, 'sandbox', 'wait-port', id, cliWeb.id, '3000', '--http')
      await cliWeb.kill()
      await cli(
        c.d.env,
        'sandbox',
        'git',
        id,
        'clone',
        'https://github.com/octocat/Hello-World.git',
        '--path',
        'cli-repo',
        '--depth',
        '1',
      )
      await cli(c.d.env, 'sandbox', 'git', id, 'checkout', 'cli-field', '--create')
      await box.write('repo/cli-field.txt', 'cli commit')
      await cli(c.d.env, 'sandbox', 'git', id, 'add', 'cli-field.txt')
      await cli(
        c.d.env,
        'sandbox',
        'git',
        id,
        'commit',
        'CLI field commit',
        '--author',
        'Field Test <field@example.test>',
      )
      await box.git.checkout('field')
      for (const method of ['status', 'diff', 'branch', 'log', 'pull', 'push'])
        await cli(c.d.env, 'sandbox', 'git', id, method)
      for (const method of ['keep-alive', 'ttl', 'idle']) await cli(c.d.env, 'sandbox', method, id, '10m')
      await cli(c.d.env, 'sandbox', 'metrics', id)
      await cli(c.d.env, 'sandbox', 'wait', id, 'running')
      await cli(c.d.env, 'sandbox', 'ls', '--name', name)
      await cli(c.d.env, 'sandbox', 'events', id)
      const file = join(root, 'pryx.template.ts')
      writeFileSync(file, `export default ${JSON.stringify({ ...plan, name: 'cli-field-code' })}\n`)
      await cli(c.d.env, 'template', 'build', '-f', file)
      await cli(c.d.env, 'templates')
      const cliHook = (await cli(
        c.d.env,
        'webhook',
        'create',
        receiver.url.toString(),
        '--kinds',
        'created,deleted',
      )) as { id: string }
      await cli(c.d.env, 'webhook', 'ls')
      await cli(c.d.env, 'webhook', 'deadletters', cliHook.id)
      await cli(c.d.env, 'webhook', 'rm', cliHook.id)
      await box.delete()
      box = undefined
      await until('signed lifecycle deliveries', async () => {
        await hooks.tick()
        return received.filter((value) => JSON.parse(value.body).sandboxId === id).length >= 2
      })
      const own = received.filter((value) => JSON.parse(value.body).sandboxId === id)
      expect(own.map((value) => JSON.parse(value.body).event).sort()).toEqual(['created', 'deleted'])
      for (const value of own) {
        const timestamp = value.headers.get('pryx-timestamp')
        expect(value.headers.get('pryx-signature')).toBe(
          `v1=${createHmac('sha256', hook.secret).update(`${timestamp}.${value.body}`).digest('hex')}`,
        )
        expect(value.body).not.toContain('FIELD_VALUE')
      }
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      await box?.delete()
      await hook.delete()
      receiver.stop(true)
      await c.close()
      rmSync(root, { recursive: true, force: true })
    }
  },
  10 * MINUTE_MS,
)
