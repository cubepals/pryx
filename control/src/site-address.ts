// SPDX-License-Identifier: FSL-1.1-ALv2
/** Validate the optional documentation address without inventing a public hostname. */
import { PRYX_VERSION } from './generated/release.ts'

const invalidAddress = 'PRYX_SITE_URL must be an HTTP(S) address without credentials, query or fragment'
export function siteAddress(value?: string) {
  if (!value) return { origin: undefined, prefix: '/', configured: undefined }
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new Error(invalidAddress)
  }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash)
    throw new Error(invalidAddress)
  const prefix = `${url.pathname.replace(/\/$/, '')}/`
  return { origin: url.origin, prefix, configured: `${url.origin}${prefix}` }
}

const callers = [
  ['typescript', 'TypeScript'],
  ['python', 'Python'],
  ['cli', 'CLI'],
  ['action', 'GitHub Actions'],
  ['mcp', 'Agents over MCP'],
  ['webhooks', 'Webhooks'],
  ['http', 'Plain HTTP'],
] as const
export function welcomeConnectGuides(value?: string) {
  const { configured } = siteAddress(value)
  return configured
    ? callers.map(([id, title]) => ({ id, title, url: `${configured}v${PRYX_VERSION}/connect/${id}/` }))
    : []
}
