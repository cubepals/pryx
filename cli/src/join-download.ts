// SPDX-License-Identifier: FSL-1.1-ALv2
/** Bounded bootstrap reads: pin CA bytes, then require verified TLS with no redirects or socket reuse. */
import { createHash } from 'node:crypto'
import { get, type RequestOptions } from 'node:https'
import { StatusCodes } from 'http-status-codes'
import { SECOND_MS } from '../../control/src/durations.ts'

function read(url: string, tls: RequestOptions, maximum: number): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const request = get(
      url,
      { ...tls, agent: false, signal: AbortSignal.timeout(30 * SECOND_MS) },
      (response) => {
        if (response.statusCode !== StatusCodes.OK) {
          reject(new Error(`Bootstrap download refused (HTTP ${response.statusCode})`))
          response.destroy()
          return
        }
        const chunks: Buffer[] = []
        let size = 0
        response.on('data', (chunk: Buffer) => {
          size += chunk.length
          if (size > maximum) {
            reject(new Error('Bootstrap response exceeds its byte limit'))
            response.destroy()
            return
          }
          chunks.push(chunk)
        })
        response.on('end', () => resolve(Buffer.concat(chunks)))
        response.on('error', () => reject(new Error('Bootstrap response was interrupted')))
      },
    )
    request.on('error', () => reject(new Error('Bootstrap transport verification failed')))
  })
}
export async function pinnedJoinDownloads(base: string, hash: string) {
  // This one untrusted certificate read carries no secret and is accepted only by its token hash.
  const ca = await read(`${base}/fleet/v1/ca.pem`, { rejectUnauthorized: false }, 64 * 1024)
  if (createHash('sha256').update(ca).digest('hex') !== hash)
    throw new Error('Join CA pin verification FAILED')
  return {
    ca,
    get: (path: string, maximum: number) =>
      read(`${base}/fleet/v1/${path}`, { ca, rejectUnauthorized: true }, maximum),
  }
}
