// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Read TLS ClientHello SNI or HTTP/1 Host without consuming or changing replay bytes; raw ports need no greeting. */
// Named framing values have no library exports; their owner is the RFC sections below.
const HANDSHAKE_RECORD = 22
const TLS_MAJOR = 3
const CLIENT_HELLO = 1
const SERVER_NAME_EXTENSION = 0
const HOST_NAME = 0
const RECORD_HEADER_BYTES = 5
const HANDSHAKE_HEADER_BYTES = 4
// RFC 1035 section 2.3.4: a 255-octet encoded name has at most 253 presentation characters.
const HOSTNAME_CHARACTERS = 253
// TLS framing follows RFC 8446 sections 4.1.2 and 5.1; SNI follows RFC 6066 section 3.
export class BadGreeting extends Error {}
export type Greeter = (bytes: Buffer) => string | null
const hostname = (text: string) => {
  const name = text.toLowerCase().replace(/\.$/, '')
  if (!/^[a-z0-9][a-z0-9.-]*$/.test(name) || name.length > HOSTNAME_CHARACTERS)
    throw new BadGreeting('Invalid hostname')
  return name
}
export const httpHost: Greeter = (bytes) => {
  const end = bytes.indexOf('\r\n\r\n')
  if (end < 0) return null
  const lines = bytes.subarray(0, end).toString('latin1').split('\r\n')
  if (!/^[A-Z!#$%&'*+.^_`|~-]+ \S+ HTTP\/1\.[01]$/.test(lines.shift() ?? ''))
    throw new BadGreeting('Invalid HTTP request line')
  const hosts = lines.filter((l) => /^host:/i.test(l))
  if (hosts.length !== 1 || lines.some((l) => /^\s/.test(l)))
    throw new BadGreeting('One Host header is required')
  const host = hosts[0]?.slice('host:'.length).trim() ?? ''
  if (!/^[^:\s]+(?::[0-9]+)?$/.test(host)) throw new BadGreeting('Invalid Host header')
  return hostname(host.split(':')[0] ?? '')
}
export const tlsSni: Greeter = (bytes) => {
  const records: Buffer[] = []
  let at = 0,
    total = 0
  while (at + RECORD_HEADER_BYTES <= bytes.length) {
    if (bytes[at] !== HANDSHAKE_RECORD || bytes[at + 1] !== TLS_MAJOR)
      throw new BadGreeting('Expected TLS handshake')
    const length = bytes.readUInt16BE(at + 3)
    if (at + RECORD_HEADER_BYTES + length > bytes.length) return null
    records.push(bytes.subarray(at + RECORD_HEADER_BYTES, at + RECORD_HEADER_BYTES + length))
    total += length
    at += RECORD_HEADER_BYTES + length
    const hello = Buffer.concat(records, total)
    if (hello.length < HANDSHAKE_HEADER_BYTES) continue
    if (hello[0] !== CLIENT_HELLO) throw new BadGreeting('Expected ClientHello')
    const size = hello.readUIntBE(1, 3) + HANDSHAKE_HEADER_BYTES
    if (hello.length < size) continue
    let p = HANDSHAKE_HEADER_BYTES + 2 + 32
    const take = (n: number) => {
      if (p + n > size) throw new BadGreeting('Truncated ClientHello')
      const start = p
      p += n
      return start
    }
    const u8 = () => hello[take(1)] as number
    const u16 = () => hello.readUInt16BE(take(2))
    take(u8())
    take(u16())
    take(u8())
    if (p === size) throw new BadGreeting('SNI is required')
    const extensions = u16(),
      limit = p + extensions
    if (limit !== size) throw new BadGreeting('Invalid extensions length')
    while (p < limit) {
      const kind = u16(),
        len = u16(),
        next = p + len
      if (next > limit) throw new BadGreeting('Truncated extension')
      if (kind === SERVER_NAME_EXTENSION) {
        const list = u16(),
          listEnd = p + list
        if (listEnd !== next) throw new BadGreeting('Invalid SNI list')
        let found: string | undefined
        while (p < next) {
          const type = u8(),
            n = u16(),
            start = take(n)
          if (type === HOST_NAME) {
            if (found) throw new BadGreeting('Duplicate SNI')
            found = hostname(hello.subarray(start, start + n).toString('ascii'))
          }
        }
        if (found) return found
      }
      p = next
    }
    throw new BadGreeting('SNI is required')
  }
  return null
}
