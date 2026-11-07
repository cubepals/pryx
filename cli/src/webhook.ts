// SPDX-License-Identifier: FSL-1.1-ALv2
/** Webhook registration prints the signing secret once; listings and dead letters omit it. */
import { option, publicTransport } from './sandbox.ts'
export async function webhook(args: string[]) {
  const [verb, id] = args
  const http = publicTransport()
  if (verb === 'create') {
    if (!id) throw new Error('webhook create URL --kinds created,deleted')
    return http.request('/v1/webhooks', 'POST', {
      url: id,
      kinds: (option(args, '--kinds') ?? 'created,deleted').split(','),
    })
  }
  if (verb === 'ls') return http.request('/v1/webhooks')
  if (verb === 'rm' && id) return http.request(`/v1/webhooks/${encodeURIComponent(id)}`, 'DELETE')
  if (verb === 'deadletters' && id) return http.request(`/v1/webhooks/${encodeURIComponent(id)}/deadletters`)
  throw new Error('webhook create|ls|rm|deadletters')
}
