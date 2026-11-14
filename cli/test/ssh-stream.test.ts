// SPDX-License-Identifier: FSL-1.1-ALv2
/** The maintained WebSocket stream retains first bytes and a large binary body with backpressure. */
import { expect, test } from 'bun:test'
import { randomBytes } from 'node:crypto'
import { PassThrough, Writable } from 'node:stream'
import { sshStream } from '../src/ssh-proxy.ts'

test('SSH framing streams every opaque byte before close', async () => {
  const bytes = randomBytes(3 * 1024 * 1024),
    first = randomBytes(128)
  let received = 0
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch(request, server) {
      if (server.upgrade(request)) return
      return new Response('Upgrade required')
    },
    websocket: {
      open(socket) {
        socket.send(first)
      },
      message(socket, message) {
        received += Buffer.byteLength(message)
        socket.send(message)
        if (received === bytes.length) socket.close()
      },
    },
  })
  const input = new PassThrough(),
    parts: Buffer[] = []
  const output = new Writable({
    highWaterMark: 1024,
    write(chunk, _encoding, done) {
      parts.push(Buffer.from(chunk))
      setImmediate(done)
    },
  })
  try {
    const forwarded = sshStream({ url: `ws://127.0.0.1:${server.port}`, protocols: [] }, input, output)
    input.write(bytes)
    await forwarded
    expect(received).toBe(bytes.length)
    expect(Buffer.concat(parts)).toEqual(Buffer.concat([first, bytes]))
  } finally {
    input.destroy()
    output.destroy()
    server.stop(true)
  }
})
