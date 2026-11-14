// SPDX-License-Identifier: FSL-1.1-ALv2
/** The installed CLI discovers its protected bundle's probe without requiring a PATH edit. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

test('doctor finds the bundled daemon and creates no installation state', async () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-doctor-'))
  const bundle = join(home, 'bundle')
  mkdirSync(join(bundle, 'bin'), { recursive: true })
  writeFileSync(join(bundle, 'bin/pryxd'), '#!/bin/sh\nprintf "bundle-probe %s\\n" "$*"\n', { mode: 0o700 })
  try {
    const source = new URL('../src/doctor.ts', import.meta.url).pathname
    const child = Bun.spawn(
      [process.execPath, '-e', `import {doctor} from ${JSON.stringify(source)}; await doctor(['--json'])`],
      {
        env: {
          ...process.env,
          HOME: home,
          BUN_RUNTIME_TRANSPILER_CACHE_PATH: '0',
          XDG_CONFIG_HOME: join(home, 'config'),
          PRYX_BUNDLE_DIR: bundle,
          PRYX_PRYXD_BIN: undefined,
          PRYX_LOCAL_CONFIG_DIR: undefined,
          PRYX_STATE_DIR: undefined,
          PATH: '/usr/bin:/bin',
        },
        stdout: 'pipe',
        stderr: 'pipe',
      },
    )
    const [code, out, error] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])
    expect(code).toBe(0)
    expect(error).toBe('')
    expect(out).toBe('bundle-probe probe --json\n')
    expect(readdirSync(home)).toEqual(['bundle'])
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})
