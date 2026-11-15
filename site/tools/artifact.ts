// SPDX-License-Identifier: FSL-1.1-ALv2
/** Enumerate static output without following links; HTML interpretation belongs to html.ts. */
import { lstatSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

export function files(directory: string, prefix = ''): string[] {
  return readdirSync(join(directory, prefix))
    .sort()
    .flatMap((name) => {
      const file = join(prefix, name)
      const stat = lstatSync(join(directory, file))
      if (stat.isSymbolicLink()) throw new Error(`Static artifact contains symlink: ${file}`)
      return stat.isDirectory() ? files(directory, file) : [file]
    })
}
