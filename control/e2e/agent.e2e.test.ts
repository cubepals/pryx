// SPDX-License-Identifier: FSL-1.1-ALv2
/** Both SDKs exercise real container processes, streams, PTYs, data, runtime ports and wake.
 * A missing agent is an assertion failure after the existing container prerequisites hold. */
import { expect, test } from 'bun:test'
import { execPath, env as hostEnvironment } from 'node:process'
import { StatusCodes } from 'http-status-codes'
import { MINUTE_MS } from '../src/durations.ts'
import { AGENT_ROUTE_PORT } from '../src/edge-headers.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'
import { until } from './waits.ts'

test.skipIf(!ready)(
  'TypeScript agent handles run every sandbox operation',
  async () => {
    const c = await agentDeployment()
    const box = await c.sdk.sandbox({ image: 'alpine:pryx-tls', resources: { memoryMb: 256 }, sparePorts: 1 })
    let stage = 'processes'
    try {
      const route = await c.route(box.id)
      expect(route.agent).toBeDefined()
      const process = await box.start('printf first; read value; printf "%s" "$value"', {
        stdin: true,
        env: { CI: '1' },
        cwd: '/data',
      })
      const output = process.stream()
      expect((await output.next()).value?.data).toBe('first')
      expect((await box.agent.process(process.id)).view.running).toBe(true)
      await process.stdin.write('last\n')
      expect((await output.next()).value?.data).toBe('last')
      await output.next()
      expect((await process.wait()).code).toBe(0)
      expect((await box.run('printf hello')).stdout).toBe('hello')
      const long = await box.start('sleep 60')
      await long.signal('TERM')
      expect((await long.wait()).code).not.toBe(0)
      const killed = await box.start('sleep 60')
      await killed.kill()
      expect((await killed.wait()).code).not.toBe(0)
      const terminal = await box.terminal({ cols: 120, rows: 40 })
      terminal.resize(80, 24)
      terminal.stdin.write('printf "terminal-ready\\n"\n')
      let terminalOutput = ''
      for await (const chunk of terminal.stream()) {
        terminalOutput += chunk.data
        if (terminalOutput.includes('terminal-ready\r\n')) break
      }
      terminal.close()
      expect(terminalOutput).toContain('terminal-ready')
      const cli = Bun.spawn(
        [
          hostEnvironment.PRYX_MCP_CLI ?? execPath,
          ...(hostEnvironment.PRYX_MCP_CLI ? [] : ['cli/src/main.ts']),
          'sandbox',
          'terminal',
          box.id,
        ],
        { env: c.d.env, stdin: 'pipe', stdout: 'pipe', stderr: 'pipe' },
      )
      cli.stdin.write("printf 'cli-%s\\n' terminal-ready; exit\n")
      await cli.stdin.end()
      const [cliOutput, cliError, cliCode] = await Promise.all([
        new Response(cli.stdout).text(),
        new Response(cli.stderr).text(),
        cli.exited,
      ])
      expect(cliCode).toBe(0)
      expect(cliError).toBe('')
      expect(cliOutput).toContain('cli-terminal-ready')
      stage = 'files'
      await box.files.mkdir('src')
      const changes = box.files.watch('src')
      expect((await changes.next()).value).toMatchObject({ type: 'ready' })
      const change = changes.next()
      await box.write('src/message', 'hello')
      expect((await change).done).toBe(false)
      await changes.return(undefined)
      await box.files.move('src/message', 'src/moved')
      expect((await box.files.stat('src/moved')).size).toBe(5)
      expect((await box.files.list('src')).some((f) => f.name === 'moved')).toBe(true)
      const grant = await box.files.url('src/moved')
      expect(await fetch(grant.url).then((r) => r.text())).toBe('hello')
      const bytes = new Uint8Array(3 * 1024 * 1024).fill(42)
      await box.write(
        'large',
        new ReadableStream({
          start(s) {
            s.enqueue(bytes)
            s.close()
          },
        }),
      )
      expect(await new Response(await box.readStream('large')).bytes()).toEqual(bytes)
      stage = 'ports'
      await box.write('index.html', 'exposed')
      const web = await box.start(['httpd', '-f', '-p', '3000', '-h', '/data'])
      const url = await box.expose(3000)
      expect(await c.app(url).then((r) => r.text())).toBe('exposed')
      await expect(box.expose(3001)).rejects.toMatchObject({ code: 'no_spare_port' })
      await box.unexpose(3000)
      expect((await c.app(url)).status).toBe(StatusCodes.NOT_FOUND)
      await web.kill()
      await web.wait()
      await box.files.delete('src/moved')
      stage = 'sleep'
      await box.sleep()
      await until(
        'edge observes sleeping agent',
        async () => c.edge.table.held.get(`${box.id}:${AGENT_ROUTE_PORT}`)?.state,
      )
      expect((await box.run('printf awake')).stdout).toBe('awake')
      await box.refresh()
      stage = 'stop'
      await box.stop()
      await box.write('stopped', bytes)
      expect(await box.readBytes('stopped')).toEqual(bytes)
      stage = 'start'
      await box.start()
      await box.write('stopped', 'updated by running agent')
      expect(await box.read('stopped')).toBe('updated by running agent')
      stage = 'network-none'
      await box.network('none')
      expect((await box.run('printf isolated')).stdout).toBe('isolated')
      stage = 'network-internet'
      await box.network('internet')
      expect((await box.run('printf online')).stdout).toBe('online')
    } catch (error) {
      console.error('Agent acceptance stage', stage)
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      await box.delete()
      await c.close()
    }
  },
  4 * MINUTE_MS,
)

test.skipIf(!ready)(
  'Python sync and async handles use the same real agent surface',
  async () => {
    const c = await agentDeployment()
    try {
      const child = Bun.spawn([process.env.PRYX_PYTHON ?? 'python3', 'sdk/python/e2e/agent.py'], {
        env: { ...c.d.env, PRYX_TEST_EDGE_URL: c.origin },
        stdout: 'inherit',
        stderr: 'inherit',
      })
      expect(await child.exited).toBe(0)
    } finally {
      await c.close()
    }
  },
  4 * MINUTE_MS,
)

test.skipIf(!ready)(
  'the static agent needs no shell or utilities in the image',
  async () => {
    const c = await agentDeployment()
    const box = await c.sdk.sandbox({ image: 'alpine:pryx-empty', resources: { memoryMb: 256 } })
    try {
      await c.route(box.id)
      expect((await box.run('printf shell-ready')).stdout).toBe('shell-ready')
      await box.files.mkdir('empty')
      await box.write('empty/message', 'file-ready')
      expect(await box.read('empty/message')).toBe('file-ready')
      const term = await box.terminal()
      term.stdin.write("printf 'terminal:%s\\n' ready\n")
      let text = ''
      for await (const chunk of term.stream()) {
        text += chunk.data
        if (text.includes('terminal:ready')) break
      }
      term.close()
      expect(text).toContain('terminal:ready')
    } finally {
      await box.delete()
      await c.close()
    }
  },
  2 * MINUTE_MS,
)
