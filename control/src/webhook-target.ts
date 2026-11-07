// SPDX-License-Identifier: FSL-1.1-ALv2
/** Webhook delivery pins a public DNS snapshot for one request. Redirects, proxy overrides
 * and private destinations cannot turn a project webhook into a control-network request. */
import { lookup } from 'node:dns/promises'
import { request as http } from 'node:http'
import { request as https } from 'node:https'
import { BlockList, isIP } from 'node:net'
import { SECOND_MS } from './durations.ts'

const blocked = new BlockList()
for (const [ip, bits] of [
  ['0.0.0.0', 8],
  ['10.0.0.0', 8],
  ['100.64.0.0', 10],
  ['127.0.0.0', 8],
  ['169.254.0.0', 16],
  ['172.16.0.0', 12],
  ['192.168.0.0', 16],
  ['192.0.0.0', 24],
  ['192.0.2.0', 24],
  ['198.18.0.0', 15],
  ['198.51.100.0', 24],
  ['203.0.113.0', 24],
  ['224.0.0.0', 4],
  ['240.0.0.0', 4],
] as const)
  blocked.addSubnet(ip, bits, 'ipv4')
for (const [ip, bits] of [
  ['::', 96],
  ['2001::', 23],
  ['2001:db8::', 32],
  ['2002::', 16],
  ['3fff::', 20],
  ['fc00::', 7],
  ['fe80::', 10],
  ['ff00::', 8],
] as const)
  blocked.addSubnet(ip, bits, 'ipv6')
export function publicAddress(ip: string) {
  const family = isIP(ip)
  return family === 4
    ? !blocked.check(ip, 'ipv4')
    : family === 6 && /^2|^3/i.test(ip) && !blocked.check(ip, 'ipv6')
}
export async function deliver(
  url: string,
  payload: string,
  headers: Record<string, string>,
  operatorHosts: readonly string[] = [],
): Promise<number> {
  const target = new URL(url)
  const host = target.hostname.replace(/^\[|\]$/g, '')
  if (!['http:', 'https:'].includes(target.protocol) || target.username || target.password || target.hash)
    throw new Error('Invalid webhook destination')
  const signal = AbortSignal.timeout(10 * SECOND_MS)
  const addresses = await Promise.race([
    lookup(host, { all: true, verbatim: true }),
    new Promise<never>((_, reject) =>
      signal.addEventListener('abort', () => reject(signal.reason), { once: true }),
    ),
  ])
  signal.throwIfAborted()
  if (
    !addresses.length ||
    (!operatorHosts.includes(host) && addresses.some((a) => !publicAddress(a.address)))
  )
    throw new Error('Webhook requires public addresses')
  const pinned = addresses.find((address) => address.family === 4) ?? addresses[0]
  if (!pinned) throw new Error('No destination')
  return new Promise((resolve, reject) => {
    const request = (target.protocol === 'https:' ? https : http)(
      {
        protocol: target.protocol,
        hostname: pinned.address,
        port: target.port || undefined,
        servername: host,
        path: target.pathname + target.search,
        method: 'POST',
        signal,
        agent: false,
        headers: {
          ...headers,
          host: target.host,
          'content-type': 'application/json',
          'content-length': String(Buffer.byteLength(payload)),
        },
      },
      (response) => {
        resolve(response.statusCode ?? 0)
        response.destroy()
      },
    )
    request.once('error', reject)
    request.end(payload)
  })
}
