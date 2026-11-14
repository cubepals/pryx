// SPDX-License-Identifier: FSL-1.1-ALv2
/** K30 acceptance uses a real daemon, control and edge in this job. No provider method is mocked. */
import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pryxPortAddress, pryxProvider } from '../../sdk/typescript/src/provider.ts'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { runProviderExamples } from '../tools/provider-examples.ts'
import { agentDeployment } from './agent-context.ts'
import { ready } from './deployment.ts'
import { deploymentDiagnostics } from './diagnostics.ts'

const actor = { actorId: 'provider-acceptance', client: () => undefined }
const httpFixture = readFileSync(new URL('./fixtures/provider-http.cjs', import.meta.url), 'utf8')
const webCommand = ['node', '/data/provider-http.cjs', '3000', '0.0.0.0']

test(
  'provider uses persistent process paths as an ordinary user',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment('process')
    const options = { url: c.d.control.apiUrl, key: c.d.credential.key }
    const provider = pryxProvider(options)
    let id: string | undefined
    try {
      id = await provider.create(actor)
      const box = await provider.connect(actor, id)
      if (!box) throw new Error('Created process sandbox is absent')
      expect((await c.sdk.sandbox(id)).isolation).toBe('process')
      expect(box.cwd).not.toBe('/data')
      await box.mkdir(`${box.cwd}/work`)
      await box.writeFile(`${box.cwd}/work/hello.txt`, 'process-persistent')
      expect(await box.exec('cat hello.txt; pwd', { cwd: `${box.cwd}/work` })).toMatchObject({
        exitCode: 0,
        stdout: `process-persistent${box.cwd}/work\n`,
      })
      expect(new TextDecoder().decode(await box.readFile(`${box.cwd}/work/hello.txt`))).toBe(
        'process-persistent',
      )
      expect(await box.stat(`${box.cwd}/work`)).toEqual({ isDirectory: true })
      expect(await box.readdir(`${box.cwd}/work`)).toContain('hello.txt')
      await provider.suspend?.(actor, id)
      const awake = await provider.connect(actor, id)
      if (!awake) throw new Error('Sleeping process sandbox disappeared')
      expect(await awake.exists(`${awake.cwd}/work/hello.txt`)).toBe(true)
      await provider.destroy?.(actor, id)
      expect(await provider.connect(actor, id)).toBeUndefined()
      id = undefined
      await runProviderExamples(options)
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      if (id) await provider.destroy?.(actor, id)
      await c.close()
    }
  },
  3 * MINUTE_MS,
)

test(
  'sandbox provider methods, cancellation, streams, persistence and preview',
  async () => {
    expect(ready).toBe(true)
    const c = await agentDeployment(undefined, { repoImage: 'alpine:pryx-browser', localPreviews: true })
    const options = { url: c.d.control.apiUrl, key: c.d.credential.key }
    const provider = pryxProvider(options)
    let id: string | undefined
    try {
      // The provider inherits its image and tier from the project, just as documented.
      id = await provider.create(actor)
      const box = await provider.connect(actor, id)
      if (!box) throw new Error('Created sandbox is absent')
      await box.mkdir('/data/work')
      await box.writeFile('/data/work/kept', 'kept\0bytes\n')
      expect(new TextDecoder().decode(await box.readFile('/data/work/kept'))).toBe('kept\0bytes\n')
      expect(await box.stat('/data/work')).toEqual({ isDirectory: true })
      expect(await box.stat('/data/work/kept')).toEqual({ isDirectory: false })
      expect(await box.readdir('/data/work')).toContain('kept')
      expect(await box.exists('/data/work/kept')).toBe(true)
      expect(await box.exists('/data/missing')).toBe(false)
      const result = await box.exec('printf "$VALUE"; pwd; printf separate >&2; exit 7', {
        cwd: '/data/work',
        env: { VALUE: 'environment\n' },
        timeoutMs: 10 * SECOND_MS,
      })
      expect(result).toEqual({
        exitCode: 7,
        timedOut: false,
        stdout: 'environment\n/data/work\n',
        stderr: 'separate',
      })
      // Only receipt of live output opens the gate: buffering until exit deadlocks this case.
      let gate: Promise<void> | undefined
      const streamed = await box.exec('echo ready; while [ ! -f go ]; do sleep 0.1; done; echo finished', {
        cwd: '/data/work',
        timeoutMs: 10 * SECOND_MS,
        onData(chunk) {
          if (new TextDecoder().decode(chunk).includes('ready')) gate = box.writeFile('/data/work/go', 'go')
        },
      })
      await gate
      expect(streamed.stdout).toBe('ready\nfinished\n')
      const timeout = await box.exec('echo before-timeout; sleep 60; touch timeout-leaked', {
        cwd: '/data/work',
        timeoutMs: SECOND_MS / 4,
      })
      expect(timeout).toMatchObject({ exitCode: null, timedOut: true, stdout: 'before-timeout\n' })
      const abort = new AbortController()
      await expect(
        box.exec('echo abort-ready; sleep 60; touch abort-leaked', {
          cwd: '/data/work',
          signal: abort.signal,
          onData: () => abort.abort(),
        }),
      ).rejects.toThrow('aborted')
      const alreadyAborted = new AbortController()
      alreadyAborted.abort()
      await expect(
        box.exec('touch pre-abort-leaked', { cwd: '/data/work', signal: alreadyAborted.signal }),
      ).rejects.toThrow('aborted')
      expect(await box.exists('/data/work/pre-abort-leaked')).toBe(false)
      const raw = await c.sdk.sandbox(id)
      expect((await raw.processes()).every((process) => !process.view.running)).toBe(true)
      await raw.stop()
      const restarted = await provider.connect(actor, id)
      if (!restarted) throw new Error('Stopped sandbox disappeared')
      expect(await restarted.exists('/data/work/kept')).toBe(true)
      await provider.suspend?.(actor, id)
      expect((await c.sdk.sandbox(id, { wake: false })).state).toBe('sleeping')
      const awake = await provider.connect(actor, id)
      if (!awake) throw new Error('Sleeping sandbox disappeared')
      expect(new TextDecoder().decode(await awake.readFile('/data/work/kept'))).toBe('kept\0bytes\n')
      const resumedRaw = await c.sdk.sandbox(id)
      await resumedRaw.write('index.html', 'provider-preview')
      await resumedRaw.write('provider-http.cjs', httpFixture)
      const web = await resumedRaw.spawn(webCommand)
      await web.waitForPort(3000, { mode: 'http', timeout: '30s' })
      const address = await pryxPortAddress(id, 3000, options)
      expect(await (await c.app(address)).text()).toBe('provider-preview')
      await web.kill()
      await provider.destroy?.(actor, id)
      expect(await provider.connect(actor, id)).toBeUndefined()
      id = undefined
      await runProviderExamples(options)
      const patched = (await import(resolve('target/k30-open-orange/built/providers.js'))) as {
        sandboxProviderFromEnvironment(env: Record<string, string>): {
          create(owner: string): Promise<string>
          connect(
            owner: string,
            id: string,
          ): Promise<
            | {
                cwd: string
                writeFile(path: string, bytes: Uint8Array): Promise<void>
                readFile(path: string): Promise<Uint8Array>
                list(path: string, recursive: boolean): Promise<{ name: string }[]>
                rename(from: string, to: string): Promise<void>
                deleteFile(path: string): Promise<void>
                previewFetch(port: number, request: Request): Promise<Response>
              }
            | undefined
          >
          destroy(owner: string, id: string): Promise<void>
        }
      }
      const orange = patched.sandboxProviderFromEnvironment({
        ORANGE_SANDBOX_PROVIDER: 'pryx',
        PRYX_API_URL: options.url,
        PRYX_API_KEY: options.key,
      })
      const orangeId = await orange.create(actor.actorId)
      try {
        const backend = await orange.connect(actor.actorId, orangeId)
        if (!backend) throw new Error('Patched binding lost the sandbox')
        expect(backend.cwd).toBe('/data')
        const binary = new Uint8Array([0, 255, 254, 128])
        await backend.writeFile('/data/binary', binary)
        expect(await backend.readFile('/data/binary')).toEqual(binary)
        expect((await backend.list('/data', true)).map((entry) => entry.name)).toContain('binary')
        await backend.rename('/data/binary', '/data/moved')
        expect(await backend.readFile('/data/moved')).toEqual(binary)
        await backend.deleteFile('/data/moved')
        const orangeRaw = await c.sdk.sandbox(orangeId)
        await orangeRaw.write('provider-http.cjs', httpFixture)
        const web = await orangeRaw.spawn(webCommand)
        await web.waitForPort(3000, { mode: 'tcp', timeout: '30s' })
        try {
          const previewHost = new URL(await pryxPortAddress(orangeId, 3000, options)).host
          const response = await backend.previewFetch(
            3000,
            new Request('http://binding/echo?value=hello', {
              method: 'POST',
              headers: { host: 'binding.invalid' },
              body: 'payload',
            }),
          )
          expect(response.ok).toBe(true)
          expect(await response.text()).toBe(`${previewHost}:POST:/echo?value=hello:payload`)
        } finally {
          await web.kill()
        }
      } finally {
        await orange.destroy(actor.actorId, orangeId)
      }
      expect(await orange.connect(actor.actorId, orangeId)).toBeUndefined()
      await expect(pryxProvider({ ...options, require: ['separate-users'] }).create(actor)).rejects.toThrow(
        'separate-users',
      )
      expect((await c.sdk.sandboxes()).length).toBe(0)
    } catch (error) {
      deploymentDiagnostics(c.d)
      throw error
    } finally {
      if (id) await provider.destroy?.(actor, id)
      await c.close()
    }
  },
  3 * MINUTE_MS,
)
