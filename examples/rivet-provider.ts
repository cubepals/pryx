// SPDX-License-Identifier: FSL-1.1-ALv2
/** Runnable provider lifecycle without a model credential. An Actor persists the returned sandbox id. */
import { pryxProvider } from '@cubepals/pryx/provider'

export const sandboxProvider = pryxProvider()

if (import.meta.main) {
  const actor = { actorId: crypto.randomUUID(), client: () => undefined }
  const id = await sandboxProvider.create(actor)
  try {
    const sandbox = await sandboxProvider.connect(actor, id)
    if (!sandbox) throw new Error('Sandbox disappeared')
    await sandbox.writeFile('/data/hello.txt', 'hello from pryx')
    const result = await sandbox.exec('cat hello.txt', {
      cwd: sandbox.cwd,
      onData: (chunk) => process.stdout.write(chunk),
    })
    if (result.exitCode !== 0 || result.stdout !== 'hello from pryx')
      throw new Error('Provider command failed')
    await sandboxProvider.suspend?.(actor, id)
    const awake = await sandboxProvider.connect(actor, id)
    if (!awake || !(await awake.exists('/data/hello.txt'))) throw new Error('File did not survive sleep')
  } finally {
    await sandboxProvider.destroy?.(actor, id)
  }
}
