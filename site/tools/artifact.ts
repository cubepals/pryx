// SPDX-License-Identifier: FSL-1.1-ALv2
/** Static inventories, checksums and source Git provenance. HTML rendering and host configuration
 * belong elsewhere; file walks refuse links and Git ignores inherited repository overrides. */

import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

export function sourceGit(repository: string, ...args: string[]) {
  return execFileSync('git', args, {
    cwd: repository,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env,
      GIT_DIR: undefined,
      GIT_WORK_TREE: undefined,
      GIT_COMMON_DIR: undefined,
      GIT_INDEX_FILE: undefined,
      GIT_OBJECT_DIRECTORY: undefined,
      GIT_ALTERNATE_OBJECT_DIRECTORIES: undefined,
      GIT_CONFIG_COUNT: '0',
      GIT_CONFIG_PARAMETERS: undefined,
    },
  }).trim()
}

export function files(directory: string, prefix = ''): string[] {
  if (lstatSync(join(directory, prefix)).isSymbolicLink())
    throw new Error('Static artifact root is a symlink')
  return readdirSync(join(directory, prefix))
    .sort()
    .flatMap((name) => {
      const file = join(prefix, name)
      const stat = lstatSync(join(directory, file))
      if (stat.isSymbolicLink()) throw new Error(`Static artifact contains symlink: ${file}`)
      return stat.isDirectory() ? files(directory, file) : [file]
    })
}

export function fileDigests(directory: string) {
  return Object.fromEntries(
    files(directory).map((file) => [
      file,
      createHash('sha256')
        .update(readFileSync(join(directory, file)))
        .digest('hex'),
    ]),
  )
}

export function verifyArtifact(directory: string, expected: Record<string, string>) {
  const actual = fileDigests(directory)
  if (Object.keys(actual).sort().join('\n') !== Object.keys(expected).sort().join('\n'))
    throw new Error('Static artifact file inventory changed')
  for (const [file, digest] of Object.entries(expected))
    if (actual[file] !== digest) throw new Error(`Static artifact content changed: ${file}`)
}
