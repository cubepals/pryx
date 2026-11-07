// SPDX-License-Identifier: FSL-1.1-ALv2
/** Keep real application origins in separate client pools when fixtures send them to one edge. */
import { Agent, request } from 'node:http'
import { Readable } from 'node:stream'

export class AppRequests {
  private readonly pools = new Map<string, Agent>()
  constructor(readonly origin: string) {}
  get(target: string): Promise<Response> {
    const url = new URL(target)
    let agent = this.pools.get(url.origin)
    if (!agent) {
      agent = new Agent({ keepAlive: true })
      this.pools.set(url.origin, agent)
    }
    return new Promise((resolve, reject) => {
      const pending = request(
        new URL(`${url.pathname}${url.search}`, this.origin),
        { agent, headers: { host: url.host } },
        (response) => {
          const headers = new Headers()
          for (let i = 0; i < response.rawHeaders.length; i += 2)
            headers.append(response.rawHeaders[i] as string, response.rawHeaders[i + 1] as string)
          resolve(
            new Response(Readable.toWeb(response) as unknown as ReadableStream<Uint8Array>, {
              status: response.statusCode,
              headers,
            }),
          )
        },
      )
      pending.on('error', reject)
      pending.end()
    })
  }
  close() {
    for (const pool of this.pools.values()) pool.destroy()
    this.pools.clear()
  }
}
