// SPDX-License-Identifier: FSL-1.1-ALv2
/** Opt-in HTTP phase evidence. It never logs URLs, headers, bodies or native error text,
 * and preserves each response/error without retrying a request. */
import { METHODS } from 'node:http'
import { constants } from 'node:os'

export function diagnosticFetch(
  report: (event: Record<string, string | number>) => void,
  original: typeof fetch = globalThis.fetch,
): typeof fetch {
  let sequence = 0
  const traced: typeof fetch = Object.assign(
    async (input: Parameters<typeof fetch>[0], init?: Parameters<typeof fetch>[1]) => {
      const request = ++sequence
      let scope = 'unknown'
      try {
        const path = new URL(input instanceof Request ? input.url : input.toString()).pathname
        scope = path.startsWith('/v1/') ? 'public' : 'agent'
      } catch {}
      const supplied = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase()
      const method = METHODS.includes(supplied) ? supplied : 'OTHER'
      const context = { request, scope, method }
      report({ ...context, phase: 'start' })
      try {
        const response = await original(input, init)
        report({ ...context, phase: 'response', status: response.status })
        return response
      } catch (error) {
        const code = error && typeof error === 'object' && 'code' in error ? String(error.code) : ''
        report({
          ...context,
          phase: 'failed',
          code: Object.hasOwn(constants.errno, code) ? code : 'transport_error',
        })
        throw error
      }
    },
    { preconnect: original.preconnect },
  )
  return traced
}
