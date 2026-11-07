// SPDX-License-Identifier: FSL-1.1-ALv2
/** Virtual Git uses a transient guest loopback HTTP bridge; configuration rewrites live only
 * in this call's argv. The original HTTPS origin remains the repository's stored remote. */
import type { Process, StartOptions } from './agent.ts'
import { SDK_DEFAULTS } from './generated/defaults.ts'
import { GIT_PROXY_SOURCE } from './generated/git-proxy.ts'
import { PryxError } from './transport.ts'
export type GitSpawn = (command: string[], options?: StartOptions) => Promise<Process>
export async function gitProxy(spawn: GitSpawn, url: string) {
  const origin = new URL(url).origin
  const nonce = crypto.randomUUID().replaceAll('-', '')
  const process = await spawn(['node', '-e', GIT_PROXY_SOURCE], {
    timeout: SDK_DEFAULTS.runTimeoutSeconds,
    env: {
      PRYX_PROXY_ORIGIN: origin,
      PRYX_PROXY_NONCE: nonce,
      PRYX_PROXY_MAX_BODY: String(SDK_DEFAULTS.guestFrameBytes),
    },
  })
  let text = ''
  let port: number | undefined
  const stream = process.stream()
  try {
    for await (const chunk of stream) {
      if (chunk.stream === 'stderr')
        throw new PryxError(0, 'git_proxy_failed', 'Git HTTP bridge failed to start')
      text += chunk.data
      if (text.length > SDK_DEFAULTS.captureBytes)
        throw new PryxError(0, 'git_proxy_failed', 'Git HTTP bridge output exceeded its budget')
      const match = /PRYX_GIT_PROXY_READY (\d+)\n/.exec(text)
      if (match) {
        port = Number(match[1])
        break
      }
    }
    if (!port) throw new PryxError(0, 'git_proxy_failed', 'Git HTTP bridge exited before listening')
  } catch (error) {
    if (process.view.running) await process.kill()
    throw error
  }
  const host = `127.0.0.1:${port}`,
    base = `http://${host}/${nonce}/`
  return {
    host,
    protocol: 'http',
    path: nonce,
    args: ['-c', `url.${base}.insteadOf=${origin}/`, '-c', 'credential.useHttpPath=true'],
    close: async () => {
      try {
        await process.kill()
      } catch (error) {
        if ((await process.wait()).code === null) throw error
      }
      await process.wait()
    },
  }
}
