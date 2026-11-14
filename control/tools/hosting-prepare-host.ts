// SPDX-License-Identifier: FSL-1.1-ALv2
/** Prepare a booted CI guest through the production installer before loading an unpublished
 * local image. No replacement Docker installer or provider behavior lives in this fixture. */
import { configureHostRuntime } from '../../cli/src/up-host.ts'

await configureHostRuntime(async (args) => {
  const child = Bun.spawn(args, { stdout: 'inherit', stderr: 'inherit' })
  if (await child.exited) throw new Error('Production host runtime setup failed')
})
