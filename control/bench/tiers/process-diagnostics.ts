// SPDX-License-Identifier: FSL-1.1-ALv2
/** Failure-only native supervisor evidence. Emit OS codes/kinds and a panic flag, never private
 * stderr text, paths, arguments, environments or signing material. */
import { closeSync, constants, fstatSync, openSync, readdirSync, readSync } from 'node:fs'
import { dirname, join } from 'node:path'
import type { deployment } from '../../e2e/deployment.ts'

export function processDiagnostics(d: Awaited<ReturnType<typeof deployment>>) {
  for (const { path } of d.children) {
    const root = join(dirname(path), 'state', 'process-runtime')
    let names: string[]
    try {
      names = readdirSync(root)
    } catch {
      continue
    }
    let emitted = 0
    for (const name of names.filter((name) => name.endsWith('.stderr'))) {
      let fd: number | undefined
      try {
        fd = openSync(join(root, name), constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
        const info = fstatSync(fd)
        if (!info.isFile() || info.size === 0) continue
        const buffer = Buffer.alloc(Math.min(info.size, 4096))
        const bytes = readSync(fd, buffer, 0, buffer.length, 0)
        const value = buffer.subarray(0, bytes).toString()
        const osErrors = [...value.matchAll(/code:\s*(-?\d+),\s*kind:\s*([A-Za-z]+)/g)]
          .slice(0, 5)
          .map((match) => ({ code: Number(match[1]), kind: match[2] }))
        console.error('PROCESS_PRIVATE_FAILURE', {
          stderrBytes: info.size,
          osErrors,
          panic: value.includes('panicked at '),
        })
        if (++emitted === 5) break
      } catch {
        console.error('PROCESS_PRIVATE_FAILURE', { unavailable: true })
      } finally {
        if (fd !== undefined) closeSync(fd)
      }
    }
  }
}
