// SPDX-License-Identifier: FSL-1.1-ALv2
/** A real worker retains the deployment's guarantees without relying on its login shell. */
import { expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import type { Guarantee } from '../../control/src/guarantees.ts'
import { localIsolation, localRequirements } from '../src/up-local-policy.ts'
import { localEnvironment } from '../src/up-local-process.ts'
import type { LocalInstallation } from '../src/up-local-record.ts'
import { userPaths } from '../src/up-paths.ts'

test('a worker receives persisted requirements and an empty override cannot erase them', async () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-policy-'))
  const require: Guarantee[] = ['private-network', 'memory-limit']
  const paths = userPaths({}, home)
  const record: LocalInstallation = {
    version: PRYX_VERSION,
    digest: 'a'.repeat(64),
    bundle: join(paths.data, 'releases/fixture'),
    preset: 'user',
    tier: 'wasm',
    apiUrl: 'http://127.0.0.1:7331',
    clientConfig: paths.client,
    paths,
    userUnit: true,
    enabledLinger: true,
    require,
  }
  try {
    expect(localRequirements(require, '')).toEqual(require)
    expect(localRequirements(require, 'cpu-limit,memory-limit')).toEqual([...require, 'cpu-limit'])
    const child = Bun.spawn([process.execPath, '-e', 'console.log(process.env.PRYX_REQUIRE)'], {
      env: localEnvironment(record),
      stdin: 'ignore',
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const output = await new Response(child.stdout).text()
    expect(await child.exited).toBe(0)
    expect(output.trim().split(',')).toEqual(require)
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})
test('an unknown deployment requirement refuses before service configuration', () => {
  expect(() => localRequirements([], 'private-netwrok')).toThrow('Unknown deployment requirement')
})

test('a new auto installation serves several tiers while old records stay explicit', () => {
  const paths = userPaths({}, '/home/fixture')
  const record: LocalInstallation = {
    version: PRYX_VERSION,
    digest: 'a'.repeat(64),
    bundle: '/bundle',
    preset: 'user',
    tier: 'process',
    apiUrl: 'http://127.0.0.1:7331',
    clientConfig: paths.client,
    paths,
    userUnit: false,
    enabledLinger: false,
  }
  expect(localEnvironment(record).PRYX_RUNTIME).toBe('process')
  expect(localEnvironment(record).PRYX_DEFAULT_TIER).toBe('process')
  const auto = localEnvironment({ ...record, runtime: 'auto' })
  expect(auto.PRYX_RUNTIME).toBe('auto')
  expect(auto.PRYX_DEFAULT_TIER).toBeUndefined()
})

test('the saved default isolation order reaches workers and survives an absent shell override', async () => {
  const paths = userPaths({}, '/home/fixture')
  const saved = ['wasm', 'process'] as const
  expect(localIsolation(saved, undefined)).toEqual(saved)
  expect(localIsolation(saved, 'process')).toBe('process')
  expect(() => localIsolation(saved, '')).toThrow('isolation')
  expect(() => localIsolation(saved, 'unknown')).toThrow('isolation')
  const record: LocalInstallation = {
    version: PRYX_VERSION,
    digest: 'a'.repeat(64),
    bundle: '/bundle',
    preset: 'user',
    tier: 'process',
    runtime: 'auto',
    defaultIsolation: saved,
    apiUrl: 'http://127.0.0.1:7331',
    clientConfig: paths.client,
    paths,
    userUnit: true,
    enabledLinger: false,
  }
  const child = Bun.spawn([process.execPath, '-e', 'console.log(process.env.PRYX_DEFAULT_ISOLATION)'], {
    env: localEnvironment(record),
    stdin: 'ignore',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  expect(await new Response(child.stdout).text()).toBe('wasm,process\n')
  expect(await child.exited).toBe(0)
})
