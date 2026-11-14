// SPDX-License-Identifier: FSL-1.1-ALv2
/** OpenSSH ProxyCommand carries opaque frames with maintained ws stream backpressure.
 * It receives no key on argv, emits no JSON and never retries an established session. */

import { Readable, type Writable } from 'node:stream'
import { finished } from 'node:stream/promises'
import type { Pryx } from '@cubepals/pryx'
import WebSocket, { createWebSocketStream } from 'ws'
import { SDK_DEFAULTS } from '../../sdk/typescript/src/generated/defaults.ts'

export async function sshProxy(client: Pryx, sandbox: string, lease: string) {
  const box = await client.sandbox(sandbox, { wake: false })
  const connection = await box.ssh.connection(lease)
  return sshStream(connection, process.stdin, process.stdout)
}
export async function sshStream(
  connection: { url: string; protocols: string[] },
  input: Readable,
  output: Writable,
) {
  const socket = new WebSocket(connection.url, connection.protocols, {
    perMessageDeflate: false,
    maxPayload: SDK_DEFAULTS.captureBytes,
  })
  const stream = createWebSocketStream(socket)
  const framed = Readable.from(frames(input), { objectMode: false })
  framed.pipe(stream)
  stream.pipe(output, { end: false })
  try {
    await finished(stream, { readable: true, writable: false, cleanup: true })
  } finally {
    framed.destroy()
    input.pause()
    stream.unpipe(output)
    stream.destroy()
    socket.terminate()
  }
}
async function* frames(input: Readable) {
  for await (const chunk of input) {
    const bytes = Buffer.from(chunk)
    for (let at = 0; at < bytes.length; at += SDK_DEFAULTS.fileChunkBytes)
      yield bytes.subarray(at, at + SDK_DEFAULTS.fileChunkBytes)
  }
}
