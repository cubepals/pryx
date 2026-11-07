// SPDX-License-Identifier: FSL-1.1-ALv2
/** Attempt the unmodified public release against the unshipped socket experiment and retain
 * its actual unsupported requests. A failure is evidence against shipping a node feature. */
import { expect, test } from 'bun:test'
import { chmodSync, cpSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Subprocess } from 'bun'
import { StatusCodes } from 'http-status-codes'
import { freePort } from '../../../control/e2e/waits.ts'
import { MINUTE_MS, SECOND_MS } from '../../../control/src/durations.ts'
import { dockerApiSpike } from '../src/docker-api-spike.ts'

test(
  'unmodified celld reports unsupported Docker image operations in the spike',
  async () => {
    const source = process.env.CELLD_NATIVE_SOURCE,
      binary = process.env.CELLD_BIN
    if (!source || !binary) throw new Error('The pinned public celld sources and release binary are required')
    const dir = mkdtempSync(join(tmpdir(), 'pryx-docker-spike-'))
    chmodSync(dir, 0o700)
    const example = join(dir, 'example')
    cpSync(join(source, 'examples/container'), example, { recursive: true })
    const version = Bun.spawnSync(['docker', 'version', '--format', '{{json .Server}}'])
    if (version.exitCode) throw new Error('The baseline engine version could not be read')
    const socket = join(dir, 'engine.sock'),
      spike = await dockerApiSpike(socket, JSON.parse(version.stdout.toString()))
    let child: Subprocess<'ignore', 'pipe', 'pipe'> | undefined
    const result: Record<string, unknown> = { release: '0.6.3', shipped: false, examplePassed: false }
    try {
      const install = Bun.spawn(['bun', 'install'], { cwd: example, stdout: 'ignore', stderr: 'inherit' })
      expect(await install.exited).toBe(0)
      for (const path of ['/_ping', '/version']) {
        const response = await fetch(`http://engine${path}`, { unix: socket })
        expect(response.status).toBe(StatusCodes.OK)
        await response.text()
      }
      child = Bun.spawn([binary, 'dev', '--port', String(await freePort()), '--logs'], {
        cwd: example,
        env: { ...process.env, DOCKER_HOST: `unix://${socket}` },
        stdout: 'pipe',
        stderr: 'pipe',
        stdin: 'ignore',
      })
      const stdout = new Response(child.stdout).text(),
        stderr = new Response(child.stderr).text()
      const exited = await Promise.race([child.exited, Bun.sleep(MINUTE_MS).then(() => 'timeout' as const)])
      if (exited === 'timeout') child.kill('SIGTERM')
      await Promise.race([child.exited, Bun.sleep(5 * SECOND_MS)])
      if (child.exitCode === null) child.kill('SIGKILL')
      await child.exited
      result.exitCode = child.exitCode
      result.stdout = await stdout
      result.stderr = await stderr
      result.requests = spike.requests
      expect(spike.requests.some((r) => r.status === StatusCodes.NOT_IMPLEMENTED)).toBe(true)
      expect(child.exitCode).not.toBe(0)
    } finally {
      if (child?.exitCode === null) {
        child.kill('SIGKILL')
        await child.exited
      }
      await spike.close()
      writeFileSync(
        process.env.PRYX_DOCKER_SPIKE_REPORT ?? join(tmpdir(), 'pryx-docker-spike.json'),
        `${JSON.stringify(result, null, 2)}\n`,
      )
      rmSync(dir, { recursive: true, force: true })
    }
  },
  3 * MINUTE_MS,
)
