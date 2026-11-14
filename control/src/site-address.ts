// SPDX-License-Identifier: FSL-1.1-ALv2
/** Validate the optional documentation address without inventing a public hostname. */
export function siteAddress(value?: string) {
  if (!value) return { origin: undefined, prefix: '/', configured: undefined }
  const url = new URL(value)
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash)
    throw new Error('PRYX_SITE_URL must be an HTTP(S) address without credentials, query or fragment')
  const prefix = `${url.pathname.replace(/\/$/, '')}/`
  return { origin: url.origin, prefix, configured: `${url.origin}${prefix}` }
}
