// SPDX-License-Identifier: FSL-1.1-ALv2
/** HTTP relay responses retain representation headers and discard per-connection metadata. */
export function responseHeaders(input: Headers) {
  const headers = new Headers(input)
  // RFC 9110 section 7.6.1 requires removing Connection and every field it nominates.
  for (const name of (headers.get('connection') ?? '').split(','))
    if (name.trim()) headers.delete(name.trim())
  for (const name of [
    'connection',
    'keep-alive',
    'proxy-authenticate',
    'proxy-authorization',
    'te',
    'trailer',
    'transfer-encoding',
    'upgrade',
  ])
    headers.delete(name)
  return headers
}
