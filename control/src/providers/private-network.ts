// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** IP classes used by the node's private-network policy; no DNS or public address is silently treated as private. */
import { isIP } from 'node:net'
// RFC 1918, RFC 6598 shared-address space and RFC 4193 unique-local IPv6, matching src/infer.rs.
export function privateIp(ip: string) {
  if (isIP(ip) === 6) return /^f[cd]/i.test(ip)
  if (isIP(ip) !== 4) return false
  const [a, b] = ip.split('.').map(Number)
  return (
    a === 10 ||
    (a === 172 && (b ?? 0) >= 16 && (b ?? 0) < 32) ||
    (a === 192 && b === 168) ||
    (a === 100 && (b ?? 0) >= 64 && (b ?? 0) < 128)
  )
}
