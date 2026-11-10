// SPDX-License-Identifier: FSL-1.1-ALv2
/** Install the downloaded CLI bootstrap below the preset's owned paths. The full runtime
 * still requires a release signature; this entrypoint does not activate sandbox services. */
import { chmodSync, cpSync, mkdirSync, realpathSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { createPrivateFile } from '../../deploy/image-state.ts'
import { acquireLifecycle } from './up-local-lock.ts'
import { containerPaths, userPaths } from './up-paths.ts'

export function bootstrapInstall(args: string[]) {
  const [source, bootstrap, destination] = args
  const paths = process.getuid?.() === 0 ? containerPaths() : userPaths()
  if (
    !source ||
    bootstrap !== paths.bootstrap ||
    destination !== paths.cli ||
    realpathSync(source) !== realpathSync(process.env.PRYX_BUNDLE_DIR ?? '')
  )
    throw new Error('Bootstrap source or destination differs from its managed preset')
  if (process.getuid?.() === 0) {
    mkdirSync(paths.home, { recursive: true, mode: 0o755 })
    chmodSync(paths.home, 0o755)
  }
  const unlock = acquireLifecycle(paths.operation)
  const release = join(bootstrap, `bootstrap-${crypto.randomUUID()}`)
  try {
    mkdirSync(bootstrap, { recursive: true, mode: 0o700 })
    cpSync(source, release, { recursive: true, errorOnExist: true, force: false })
    const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`
    const contents = `#!/bin/sh\n# Managed pryx installation\nexec ${quote(join(release, 'bin/pryx'))} "$@"\n`
    for (const target of [join(bootstrap, 'bin/pryx'), destination]) {
      mkdirSync(dirname(target), { recursive: true, mode: 0o700 })
      createPrivateFile(target, contents, true, 0o755)
    }
  } catch (error) {
    rmSync(release, { recursive: true, force: true })
    throw error
  } finally {
    unlock()
  }
}
