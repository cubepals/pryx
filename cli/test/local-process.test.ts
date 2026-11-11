// SPDX-License-Identifier: FSL-1.1-ALv2
/** A stale or substituted PID record cannot stop an unrelated real child. Distribution and
 * service-manager acceptance exercises the successfully authenticated supervisor separately. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { createPrivateFile } from '../../deploy/image-state.ts'
import { processStart } from '../../deploy/supervisor-process.ts'
import { stopLocal } from '../src/up-local-process.ts'
import { type LocalInstallation, readInstallation, writeInstallation } from '../src/up-local-record.ts'
import { userPaths } from '../src/up-paths.ts'

function record(home: string): LocalInstallation {
  const paths = userPaths({}, home)
  return {
    version: PRYX_VERSION,
    digest: 'a'.repeat(64),
    bundle: join(paths.data, 'releases/fixture'),
    preset: 'user',
    tier: 'process',
    apiUrl: 'http://127.0.0.1:8080',
    clientConfig: paths.client,
    paths,
    userUnit: false,
    enabledLinger: false,
  }
}
test('stale start time and foreign argv leave a real unrelated process alive', async () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-pid-'))
  const installation = record(home)
  mkdirSync(installation.paths.runtime, { recursive: true, mode: 0o700 })
  const child = Bun.spawn(['/bin/sleep', String(MINUTE_MS / SECOND_MS)], {
    stdin: 'ignore',
    stdout: 'ignore',
    stderr: 'ignore',
  })
  try {
    const start = processStart(child.pid)
    expect(start).toBeDefined()
    const identity = {
      pid: child.pid,
      start: `${start}-stale`,
      bundle: installation.bundle,
      state: installation.paths.state,
      ready: true,
    }
    const path = join(installation.paths.runtime, 'supervisor.json')
    createPrivateFile(path, JSON.stringify(identity))
    await stopLocal(installation)
    expect(child.exitCode).toBeNull()
    process.kill(child.pid, 0)
    createPrivateFile(path, JSON.stringify({ ...identity, start }), true)
    await expect(stopLocal(installation)).rejects.toThrow('identity differs')
    expect(child.exitCode).toBeNull()
    process.kill(child.pid, 0)
  } finally {
    child.kill('SIGKILL')
    await child.exited
    rmSync(home, { recursive: true, force: true })
  }
})
test('an installation record cannot redirect its client credential outside the managed paths', () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-record-'))
  const installation = record(home)
  mkdirSync(installation.paths.config, { recursive: true, mode: 0o700 })
  try {
    writeInstallation(installation)
    expect(readInstallation(installation.paths)).toEqual(installation)
    for (const engineSocket of [
      'relative.sock',
      'tcp://127.0.0.1:1234',
      '/tmp/line\nsock',
      '/tmp/zero\0sock',
    ]) {
      writeInstallation({ ...installation, engineSocket })
      expect(() => readInstallation(installation.paths)).toThrow('Invalid managed installation')
    }
    writeInstallation({ ...installation, clientConfig: join(tmpdir(), 'foreign-pryx-credential') })
    expect(() => readInstallation(installation.paths)).toThrow('Invalid managed installation')
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})

test('a clean service process restores the selected engine from its private installation record', async () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-engine-record-'))
  const socket = join(home, 'engine.sock')
  const installation = { ...record(home), engineSocket: socket }
  mkdirSync(installation.paths.config, { recursive: true, mode: 0o700 })
  const server = createServer((peer) => peer.end('selected-engine'))
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(socket, resolve)
  })
  try {
    writeInstallation(installation)
    const script = [
      'const {readInstallation}=await import(process.env.RECORD_MODULE);',
      'const {localEnvironment}=await import(process.env.PROCESS_MODULE);',
      'const record=readInstallation(JSON.parse(process.env.RECORD_PATHS));',
      'const environment=localEnvironment(record);',
      'const {connect}=await import("node:net");',
      'const socket=new URL(environment.DOCKER_HOST).pathname;',
      'if(environment.PRYX_LOCAL_ENGINE_SOCKET!==socket)throw new Error("Engine configuration differs");',
      'await new Promise((resolve,reject)=>{',
      'const peer=connect(socket); peer.on("data",data=>process.stdout.write(data));',
      'peer.on("end",resolve);peer.on("error",reject); });',
    ].join('\n')
    const child = Bun.spawn([process.execPath, '-e', script], {
      env: {
        RECORD_MODULE: new URL('../src/up-local-record.ts', import.meta.url).href,
        PROCESS_MODULE: new URL('../src/up-local-process.ts', import.meta.url).href,
        RECORD_PATHS: JSON.stringify(installation.paths),
      },
      stdin: 'ignore',
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const [output, error, code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    expect(code, error).toBe(0)
    expect(output).toBe('selected-engine')
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()))
    rmSync(home, { recursive: true, force: true })
  }
})
