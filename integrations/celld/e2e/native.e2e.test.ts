// SPDX-License-Identifier: FSL-1.1-ALv2
/** The public release's own unmodified container Worker and Python image run through both
 * implementations of the prepared engine seam. Raw start/first-request samples are retained. */
import { expect, test } from 'bun:test'
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { cpus, release, tmpdir } from 'node:os'
import { join } from 'node:path'
import { Pryx } from '@cubepals/pryx'
import { deployment, ready } from '../../../control/e2e/deployment.ts'
import { deploymentDiagnostics } from '../../../control/e2e/diagnostics.ts'
import { freePort, until } from '../../../control/e2e/waits.ts'
import { MINUTE_MS, SECOND_MS } from '../../../control/src/durations.ts'

async function command(args: string[], cwd?: string) {
  const p = Bun.spawn(args, { cwd, stdout: 'pipe', stderr: 'inherit' })
  const text = await new Response(p.stdout).text()
  if (await p.exited) throw new Error(`${args[0]} failed`)
  return text.trim()
}
test(
  'celld own container example uses the native pryx engine',
  async () => {
    if (!ready || !process.env.CELLD_NATIVE_BIN || !process.env.CELLD_NATIVE_SOURCE)
      throw new Error('Native celld acceptance requires the patched public source and CI deployment')
    const binary = process.env.CELLD_NATIVE_BIN,
      source = process.env.CELLD_NATIVE_SOURCE
    const d = await deployment({ agent: true, allowedImages: ['celld-image:'] })
    const sdk = new Pryx({ url: d.control.apiUrl, key: d.credential.key })
    const result: Record<string, unknown> = {
      commit: '8702f6d186c77698dbfb30cf29e3b1922a88f008',
      profile: 'same patched binary, dev with debug=0 and incremental=0',
      host: { cpu: cpus()[0]?.model, cores: cpus().length, kernel: release() },
      completed: false,
      cohorts: {},
    }
    const cohorts = result.cohorts as Record<
      string,
      { startMs: number[]; startFirstRequestMs: number[]; firstRequestMs: number[] }
    >
    try {
      for (const backend of ['docker', 'pryx']) {
        const dir = mkdtempSync(join(tmpdir(), `pryx-celld-${backend}-`))
        cpSync(join(source, 'examples/container'), dir, { recursive: true })
        // The Worker, server, Dockerfile and class configuration are the upstream example as released.
        expect(readFileSync(join(dir, 'index.js'), 'utf8')).toBe(
          readFileSync(join(source, 'examples/container/index.js'), 'utf8'),
        )
        await command(['bun', 'install'], dir)
        const port = await freePort(),
          logPath = join(dir, 'celld.log')
        const log = Bun.file(logPath)
        const child = Bun.spawn([binary, 'dev', '--port', String(port), '--logs'], {
          cwd: dir,
          env: {
            ...process.env,
            CELLD_CONTAINER_BACKEND: backend,
            PRYX_API_URL: d.control.apiUrl,
            PRYX_API_KEY: d.credential.key,
            CELLD_PRYX_PORTS: '8080',
          },
          stdout: log,
          stderr: log,
        })
        const base = `http://127.0.0.1:${port}`
        const raw = {
          startMs: [] as number[],
          startFirstRequestMs: [] as number[],
          firstRequestMs: [] as number[],
        }
        cohorts[backend] = raw
        try {
          await until(
            'celld own Worker ready',
            async () => (await (await fetch(base)).text()).includes('Try /container/'),
            3 * MINUTE_MS,
          )
          for (let i = 0; i < 30; i++) {
            const path = `/container/acceptance-${backend}-${i}`
            const before =
              backend === 'docker'
                ? new Set(
                    (await command(['docker', 'ps', '-aq', '--filter', 'label=celld.cell']))
                      .split('\n')
                      .filter(Boolean),
                  )
                : new Set<string>()
            const start = performance.now()
            const response = await fetch(base + path, { signal: AbortSignal.timeout(MINUTE_MS) })
            const text = await response.text()
            expect(response.ok).toBe(true)
            expect(text).toContain('passed in by the Container class')
            raw.startFirstRequestMs.push(performance.now() - start)
            const first = performance.now(),
              repeated = await fetch(base + path)
            expect(repeated.ok).toBe(true)
            expect(await repeated.text()).toBe(text)
            raw.firstRequestMs.push(performance.now() - first)
            if (backend === 'pryx') {
              const boxes = (await sdk.list()).sandboxes
              expect(boxes.length).toBe(1)
              expect(boxes[0]?.isolation).toBe('container')
              for (const box of boxes) await sdk.delete(box.id)
            } else {
              const ids = (await command(['docker', 'ps', '-aq', '--filter', 'label=celld.cell']))
                .split('\n')
                .filter((id) => id && !before.has(id))
              expect(ids.length).toBe(1)
              await command(['docker', 'rm', '-f', ...ids])
            }
          }
          const logs = await Bun.file(logPath).text()
          raw.startMs = logs
            .split('\n')
            .filter((line) => line.includes('container_backend_start_timing'))
            .map((line) => /elapsed_ms=(\d+)/.exec(line)?.[1])
            .filter((value): value is string => value !== undefined)
            .map((value) => Number(value))
          expect(raw.startMs.length).toBe(30)
        } catch (error) {
          console.error(await log.text())
          throw error
        } finally {
          child.kill('SIGTERM')
          await Promise.race([child.exited, Bun.sleep(10 * SECOND_MS)])
          if (child.exitCode === null) child.kill('SIGKILL')
          await child.exited
          rmSync(dir, { recursive: true, force: true })
        }
      }
      result.completed = true
    } catch (error) {
      result.failure = error instanceof Error ? error.message : 'unknown failure'
      deploymentDiagnostics(d)
      throw error
    } finally {
      const path = process.env.PRYX_CELLD_MEASUREMENTS ?? join(tmpdir(), 'celld-native-measurements.json')
      writeFileSync(path, JSON.stringify(result, null, 2) + '\n')
      await d.close()
    }
  },
  20 * MINUTE_MS,
)
