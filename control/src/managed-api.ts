// SPDX-License-Identifier: FSL-1.1-ALv2

/** Private endpoints between the server preset's edge and API TLS service. */
export const API_TLS_UNIT = 'pryx-api-tls'
export const MANAGED_API_TLS_SOCKET = `/run/${API_TLS_UNIT}/tls.sock`
export const MANAGED_API_RUNTIME = 'pryx-edge'
export const MANAGED_API_SOCKET = `/run/${MANAGED_API_RUNTIME}/api.sock`

export function fetchAgent(origin: string, path: string, init: RequestInit, unix?: string) {
  if (!unix) return fetch(`${origin}${path}`, init)
  const target = new URL(`${origin}${path}`)
  return fetch(`http://localhost${target.pathname}${target.search}`, { ...init, unix })
}
