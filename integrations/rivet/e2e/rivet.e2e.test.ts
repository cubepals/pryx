// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The coding actor runs on Rivet's self-hosted public engine binary, with real pryx/Docker below its SDK. CI only. */
import { expect, test } from 'bun:test'
import { Pryx } from '@cubepals/pryx'
import { createClient } from 'rivetkit/client'
import { deployment, ready } from '../../../control/e2e/deployment.ts'
import { deploymentDiagnostics, rivetDiagnostics } from '../../../control/e2e/diagnostics.ts'
import { freePort, until } from '../../../control/e2e/waits.ts'
import { MINUTE_MS } from '../../../control/src/durations.ts'
import type { registry } from '../example/main.ts'
import type { agentOsProvider } from '../src/agentos.ts'

test.skipIf(!ready)(
  'Rivet actor memory, shell, lifecycle and destruction',
  async () => {
    const d = await deployment(),
      enginePort = await freePort(),
      httpPort = await freePort()
    const child = Bun.spawn(['bun', 'integrations/rivet/example/main.ts'], {
      env: { ...d.env, RIVET_HTTP_PORT: String(httpPort), RIVET_ENGINE_PORT: String(enginePort) },
      stdout: 'inherit',
      stderr: 'inherit',
    })
    const sdk = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
    const client = createClient<typeof registry>({ endpoint: `http://127.0.0.1:${enginePort}` })
    try {
      await until('Rivet engine health', async () => {
        const response = await fetch(`http://127.0.0.1:${enginePort}/health`, {
          signal: AbortSignal.timeout(MINUTE_MS / 60),
        })
        return response.ok
      })
      const agent = client.codingAgent.getOrCreate([crypto.randomUUID()])
      await agent.remember('session memory')
      const box = await agent.make({
        image: 'alpine:3.22',
        resources: { memoryMb: 128 },
        entrypoint: ['sh', '-c', 'trap "exit 0" TERM; while :; do sleep 1; done'],
      })
      expect(await agent.exec({ command: ['echo', 'rivet-shell'] }, crypto.randomUUID())).toContainEqual(
        expect.objectContaining({ type: 'stdout', data: 'rivet-shell\n' }),
      )
      expect((await agent.memory()).notes).toContain('session memory')
      await agent.sleep()
      await until(
        'sandbox sleeps with actor',
        async () => (await sdk.sandbox(box.id, { wake: false })).state === 'sleeping',
      )
      expect((await agent.memory()).sandbox).toBe(box.id)
      expect((await sdk.sandbox(box.id, { wake: false })).state).toBe('running')
      const snapshot = await agent.snapshot()
      const fork = await agent.fork(snapshot.id, { start: false }, crypto.randomUUID())
      await (await sdk.sandbox(fork.id, { wake: false })).delete()
      await agent.destroyActor()
      await until(
        'actor destruction deletes sandbox',
        async () => !(await sdk.sandboxes()).some((b) => b.id === box.id),
      )
    } catch (error) {
      deploymentDiagnostics(d)
      rivetDiagnostics()
      throw error
    } finally {
      await client.dispose()
      child.kill()
      await child.exited
      await d.close()
    }
  },
  5 * MINUTE_MS,
)

test.skipIf(!ready)(
  'agentOS external sandbox backend runs a real process',
  async () => {
    const d = await deployment({ allowedImages: ['rivetdev/sandbox-agent:'] })
    const client = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
    try {
      if (!process.env.PRYX_PATCH_AGENTOS) throw new Error('The public agentOS patch must be built first')
      const patched = (await import(process.env.PRYX_PATCH_AGENTOS)) as {
        pryxSandboxProvider: typeof agentOsProvider
      }
      const remote = await patched
        .pryxSandboxProvider(client, {
          key: crypto.randomUUID(),
          image: 'rivetdev/sandbox-agent:0.5.5-full',
          agentPort: 3000,
        })
        .start()
      try {
        const result = await remote.runProcess({
          command: 'sh',
          args: ['-c', 'echo agentos-on-pryx'],
          cwd: '/data',
        })
        expect(result.exitCode).toBe(0)
        expect(result.stdout).toContain('agentos-on-pryx')
      } finally {
        await remote.dispose?.()
      }
    } catch (error) {
      deploymentDiagnostics(d)
      throw error
    } finally {
      await d.close()
    }
  },
  5 * MINUTE_MS,
)
