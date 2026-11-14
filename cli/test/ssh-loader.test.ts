// SPDX-License-Identifier: FSL-1.1-ALv2
/** A relocated compiled CLI must reenter its wrapper when Bun reports the ELF loader as
 * execPath. The actual proxy entry reaches the SDK and emits no SSH-transport stdout. */
import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { StatusCodes } from 'http-status-codes'

test.skipIf(process.platform !== 'linux' || process.arch !== 'x64')(
  'SSH proxy reexecutes the installed wrapper through a real ELF loader',
  async () => {
    const root = mkdtempSync(join(tmpdir(), 'pryx ssh loader '))
    const bin = join(root, 'bin')
    mkdirSync(bin)
    const loader = '/lib64/ld-linux-x86-64.so.2'
    const requests: string[] = []
    const server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      fetch(request) {
        requests.push(new URL(request.url).pathname)
        return Response.json(
          { error: { code: 'not_found', message: 'loader proxy reached the SDK' } },
          { status: StatusCodes.NOT_FOUND },
        )
      },
    })
    async function invoke(argv: string[], env = process.env) {
      const child = Bun.spawn(argv, { env, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' })
      const [code, out, error] = await Promise.all([
        child.exited,
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
      ])
      return { code, out, error }
    }
    try {
      const module = new URL('../src/sandbox-ssh.ts', import.meta.url).pathname
      const probe = join(root, 'probe.ts')
      writeFileSync(
        probe,
        `import {sshProxyCommand} from ${JSON.stringify(module)}; console.log(JSON.stringify({executable:process.execPath,command:sshProxyCommand('loader-box','loader-lease')}))`,
      )
      for (const [source, name] of [
        [probe, 'probe'],
        [new URL('../src/main.ts', import.meta.url).pathname, 'pryx'],
      ]) {
        const built = await invoke([
          process.execPath,
          'build',
          '--compile',
          source,
          '--outfile',
          join(bin, `${name}.real`),
        ])
        expect(built.code, built.error).toBe(0)
        writeFileSync(
          join(bin, name),
          `#!/bin/sh\nset -eu\nbundle=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)\nexport PRYX_BUNDLE_DIR="$bundle"\nexec ${loader} --argv0 "$0" "$0.real" "$@"\n`,
          { mode: 0o700 },
        )
      }
      const inspected = await invoke([join(bin, 'probe')])
      expect(inspected.code, inspected.error).toBe(0)
      const result = JSON.parse(inspected.out)
      expect(realpathSync(result.executable)).toBe(realpathSync(loader))
      const connected = await invoke(['/bin/sh', '-c', result.command], {
        ...process.env,
        PRYX_API_URL: server.url.toString(),
        PRYX_API_KEY: 'nonsecret-loader-fixture',
      })
      expect(connected.code).toBe(1)
      expect(requests).toEqual(['/v1/sandboxes/loader-box'])
      expect(connected.out).toBe('')
      expect(connected.error).toContain('loader proxy reached the SDK')
    } finally {
      await server.stop(true)
      rmSync(root, { recursive: true, force: true })
    }
  },
)
