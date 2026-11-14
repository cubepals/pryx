// SPDX-License-Identifier: FSL-1.1-ALv2
/** Fixture byte transport; OpenSSH owns its protocol and maintained streams own backpressure. */
import net from 'node:net'
import { pipeline } from 'node:stream/promises'

const upstream = net.createConnection({ path:'\0' + process.argv[2] })
const send = pipeline(process.stdin, upstream).catch(() => {})
try {
  await pipeline(upstream, process.stdout, { end:false })
} finally {
  process.stdin.destroy()
  upstream.destroy()
  await send
}
