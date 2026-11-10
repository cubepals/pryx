// SPDX-License-Identifier: FSL-1.1-ALv2
/** Competing installer summaries must reveal a credential once across real processes.
 * The probe fixture supplies presentation data; the filesystem claim and barrier are real. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { SECOND_MS } from '../../control/src/durations.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { writeClientConfig } from '../src/client-config.ts'
import type { LocalInstallation } from '../src/up-local-record.ts'
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
    writeFileSync(join(bundle, 'bin/pryxd'), "#!/bin/sh\nprintf '%s\\n' '{\"capabilities\":{}}'\n", {
      mode: 0o700,
    })
    writeClientConfig({ url: 'http://127.0.0.1:7331', key: crypto.randomUUID() }, paths.client)
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
