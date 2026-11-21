// SPDX-License-Identifier: FSL-1.1-ALv2
/** Installer summaries use the managed service's observations and claim the first output once. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MINUTE_MS, SECOND_MS } from '../../control/src/durations.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { guarantees } from '../../control/src/guarantees.ts'
import { createPrivateFile, IMAGE_NODE_CONFIG } from '../../deploy/image-state.ts'
import { processStart } from '../../deploy/supervisor-process.ts'
import { writeClientConfig } from '../src/client-config.ts'
import { type LocalInstallation, supervisor } from '../src/up-local-record.ts'
import { userPaths } from '../src/up-paths.ts'

test('concurrent summaries claim the first key before any process prints', async () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-summary-'))
  const paths = userPaths({}, home)
  const bundle = join(home, 'bundle')
  const barrier = join(home, 'barrier')
  const workers = 4
  try {
    mkdirSync(join(bundle, 'bin'), { recursive: true })
    mkdirSync(barrier)
    writeFileSync(
      join(bundle, 'bin/pryxd'),
      '#!/bin/sh\n[ "$1" = probe ] && [ "$2" = --json ] && [ "$3" = --config ] && [ "$4" = "$PRYX_TEST_NODE_CONFIG" ] || exit 1\nprintf \'%s\\n\' \'{"capabilities":{}}\'\n',
      { mode: 0o700 },
    )
    writeClientConfig({ url: 'http://127.0.0.1:7331', key: crypto.randomUUID() }, paths.client)
    const configured = join(paths.config, IMAGE_NODE_CONFIG)
    writeFileSync(configured, '# owned process node configuration\n', { mode: 0o600 })
    const record: LocalInstallation = {
      version: PRYX_VERSION,
      digest: 'a'.repeat(64),
      bundle,
      preset: 'user',
      tier: 'process',
      apiUrl: 'http://127.0.0.1:7331',
      clientConfig: paths.client,
      paths,
      userUnit: true,
      enabledLinger: false,
    }
    const file = join(home, 'record.json')
    writeFileSync(file, JSON.stringify(record))
    const script = [
      'import {readFileSync,readdirSync,writeFileSync} from "node:fs";',
      'const {localSummary}=await import(process.env.PRYX_TEST_MODULE);',
      'const record=JSON.parse(readFileSync(process.env.PRYX_TEST_RECORD,"utf8"));',
      'let first=true; const output=console.log;',
      'console.log=(...values)=>{ if(first){ first=false;',
      'writeFileSync(process.env.PRYX_TEST_BARRIER+"/"+process.env.PRYX_TEST_ID,"");',
      'const deadline=Date.now()+Number(process.env.PRYX_TEST_DEADLINE_MS);',
      'while(readdirSync(process.env.PRYX_TEST_BARRIER).length<Number(process.env.PRYX_TEST_WORKERS)){',
      'if(Date.now()>deadline)throw new Error("Summary barrier timed out");',
      'Bun.sleepSync(Number(process.env.PRYX_TEST_POLL_MS)); } } output(...values); };',
      'await localSummary(record,"fixture");',
    ].join('\n')
    const results = await Promise.all(
      Array.from({ length: workers }, async (_, id) => {
        const child = Bun.spawn([process.execPath, '-e', script], {
          env: {
            ...process.env,
            PRYX_TEST_MODULE: new URL('../src/up-local-activate.ts', import.meta.url).href,
            PRYX_TEST_RECORD: file,
            PRYX_TEST_BARRIER: barrier,
            PRYX_TEST_ID: String(id),
            PRYX_TEST_NODE_CONFIG: configured,
            PRYX_TEST_WORKERS: String(workers),
            PRYX_TEST_DEADLINE_MS: String(10 * SECOND_MS),
            PRYX_TEST_POLL_MS: String(SECOND_MS / 100),
          },
          stdin: 'ignore',
          stdout: 'pipe',
          stderr: 'pipe',
        })
        const [stdout, , code] = await Promise.all([
          new Response(child.stdout).text(),
          new Response(child.stderr).text(),
          child.exited,
        ])
        return { printedKey: stdout.includes('"apiKey"'), code }
      }),
    )
    expect(results.filter((result) => result.printedKey)).toHaveLength(1)
    expect(results.map((result) => result.code)).toEqual(Array(workers).fill(0))
    expect(statSync(join(paths.config, 'key-printed')).mode & 0o777).toBe(0o600)
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})

test('summary reports process limits announced by the running managed service', async () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-summary-limits-'))
  const paths = userPaths({}, home)
  const bundle = join(paths.data, 'releases/fixture')
  const record: LocalInstallation = {
    version: PRYX_VERSION,
    digest: 'a'.repeat(64),
    bundle,
    preset: 'user',
    tier: 'process',
    apiUrl: 'http://127.0.0.1:7331',
    clientConfig: paths.client,
    paths,
    userUnit: true,
    enabledLinger: false,
  }
  let service: Bun.Subprocess | undefined
  try {
    mkdirSync(join(bundle, 'bin'), { recursive: true })
    mkdirSync(paths.runtime, { recursive: true, mode: 0o700 })
    const probe = JSON.stringify({
      capabilities: {
        'process-limits': {
          state: 'off',
          evidence: 'caller cgroup is not delegated',
          action: 'use a user unit',
        },
      },
    })
    writeFileSync(
      join(bundle, 'bin/pryxd'),
      `#!/bin/sh\n[ "$1" = probe ] && [ "$2" = --json ] || exit 1\nprintf '%s\\n' '${probe}'\n`,
      { mode: 0o700 },
    )
    writeClientConfig({ url: record.apiUrl, key: 'unused-fixture' }, paths.client)
    createPrivateFile(join(paths.config, 'key-printed'), 'printed\n')
    service = Bun.spawn(
      [process.execPath, '-e', `await Bun.sleep(${MINUTE_MS})`, '--', join(bundle, 'bin/pryx-image.real')],
      { env: {}, stdin: 'ignore', stdout: 'ignore', stderr: 'ignore' },
    )
    const start = processStart(service.pid)
    expect(start).toBeDefined()
    const observed = guarantees(
      ['multi-tier', 'tier-process', 'tier-process:memory-limit', 'tier-process:cpu-limit'],
      'process',
    )
    const processLimits = { 'memory-limit': observed['memory-limit'], 'cpu-limit': observed['cpu-limit'] }
    const file = join(home, 'record.json')
    createPrivateFile(file, JSON.stringify(record))
    const script = [
      'const {localSummary}=await import(process.env.PRYX_TEST_MODULE);',
      'const record=await Bun.file(process.env.PRYX_TEST_RECORD).json();',
      'await localSummary(record,"systemctl --user status pryx");',
    ].join('\n')
    for (const fixture of [
      { ready: true, limits: processLimits, state: 'on' },
      { ready: true, limits: { ...processLimits, 'cpu-limit': false }, state: 'off' },
      { ready: true, state: 'unknown' },
      { ready: false, limits: processLimits, state: 'unknown' },
    ]) {
      createPrivateFile(
        join(paths.runtime, 'supervisor.json'),
        JSON.stringify({
          pid: service.pid,
          start,
          bundle,
          state: paths.state,
          ready: fixture.ready,
          processLimits: fixture.limits,
        }),
        true,
      )
      expect(supervisor(record)?.ready).toBe(fixture.ready)
      const child = Bun.spawn([process.execPath, '-e', script], {
        env: {
          PRYX_TEST_MODULE: new URL('../src/up-local-activate.ts', import.meta.url).href,
          PRYX_TEST_RECORD: file,
        },
        stdin: 'ignore',
        stdout: 'pipe',
        stderr: 'pipe',
      })
      const [stdout, , code] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ])
      expect(code).toBe(0)
      const summary = JSON.parse(stdout.slice(0, stdout.indexOf('\n}') + 2))
      expect(summary).not.toHaveProperty('apiKey')
      const reported = summary.capabilities['process-limits']
      expect(reported.state).toBe(fixture.state)
      if (fixture.state === 'off') {
        expect(reported.evidence).toContain('cpu-limit')
        expect(reported.evidence).not.toContain('memory-limit')
      }
      expect(stdout).not.toContain('have no enforced resource limits')
    }
  } finally {
    if (service?.exitCode === null) service.kill('SIGKILL')
    await service?.exited
    rmSync(home, { recursive: true, force: true })
  }
})
