// SPDX-License-Identifier: FSL-1.1-ALv2
/** Execute the real composite shell to check empty image selects the install's default tier. */
import { expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

test('the Action omits an empty image and preserves an explicit image', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-action-image-'))
  try {
    const cli = join(directory, 'pryx')
    const source = '#!/bin/sh\nprintf "%s\\n" "$@" > "$PRYX_ACTION_ARGS"\nprintf "hello\\n"\n'
    writeFileSync(cli, source, { mode: 0o755 })
    writeFileSync(
      join(directory, 'SHA256SUMS'),
      `${createHash('sha256').update(source).digest('hex')}  pryx\n`,
    )
    const action = Bun.YAML.parse(readFileSync('action/action.yml', 'utf8')) as {
      runs: { steps: Array<{ run: string }> }
    }
    for (const image of ['', 'alpine:3.22']) {
      const args = join(directory, 'args')
      const child = Bun.spawn(['bash', '-c', action.runs.steps[0]?.run ?? 'exit 1'], {
        env: {
          ...process.env,
          PRYX_API_URL: 'http://127.0.0.1:8080',
          PRYX_API_KEY: 'test-key',
          PRYX_IMAGE: image,
          PRYX_COMMAND: 'echo hello',
          PRYX_SNAPSHOT: 'false',
          PRYX_FROM_SNAPSHOT: '',
          PRYX_ACTION_REF: '',
          PRYX_ACTION_CLI: cli,
          PRYX_ACTION_ARGS: args,
          RUNNER_TEMP: directory,
        },
        stdout: 'ignore',
        stderr: 'pipe',
      })
      const errors = await new Response(child.stderr).text()
      expect(await child.exited, errors).toBe(0)
      expect(readFileSync(args, 'utf8').trim().split('\n')).toEqual([
        'run',
        ...(image ? ['--image', image] : []),
        '--',
        'sh',
        '-c',
        'echo hello',
      ])
    }
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
