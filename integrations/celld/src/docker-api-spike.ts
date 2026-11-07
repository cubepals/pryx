// SPDX-License-Identifier: FSL-1.1-ALv2
/** Unshipped Unix-socket experiment for an unmodified engine client. Only read-only discovery
 * is implemented: image import/build and privileged fence containers have no public pryx API.
 * Unsupported requests fail explicitly; this file is absent from the package exports and node. */
import { chmodSync } from 'node:fs'
import { createServer } from 'node:http'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../../control/src/durations.ts'
export async function dockerApiSpike(
  socket: string,
  version: { ApiVersion: string; MinAPIVersion: string; Os: string; Arch: string },
) {
  const requests: { method: string; path: string; status: number }[] = []
  const server = createServer({ maxHeaderSize: 16 * 1024 }, (request, response) => {
    const path = new URL(request.url ?? '/', 'http://engine').pathname.replace(/^\/v\d+\.\d+\//, '/')
    const status = path === '/_ping' || path === '/version' ? StatusCodes.OK : StatusCodes.NOT_IMPLEMENTED
    requests.push({ method: request.method ?? '', path, status })
    // Bounds the report itself even when an engine client repeatedly probes discovery.
    if (requests.length > 256) requests.shift()
    response.setHeader('API-Version', version.ApiVersion)
    response.setHeader('content-type', path === '/_ping' ? 'text/plain' : 'application/json')
    response.writeHead(status)
    response.end(
      path === '/_ping'
        ? 'OK'
        : JSON.stringify(
            path === '/version'
              ? { ...version, Version: 'pryx-unshipped-spike', Experimental: true }
              : { message: 'Unshipped pryx experiment: this engine operation is unsupported' },
          ),
    )
    request.resume()
  })
  server.requestTimeout = 10 * SECOND_MS
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(socket, () => {
      server.off('error', reject)
      resolve()
    })
  })
  chmodSync(socket, 0o600)
  return {
    requests,
    close: async () => {
      server.closeAllConnections()
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      )
    },
  }
}
