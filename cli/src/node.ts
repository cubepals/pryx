// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Operator commands over the control API; no database, provider credentials or host access. */

export async function node(args: string[]): Promise<unknown> {
  const [verb = 'ls', id, ...rest] = args
  const base = process.env.PRYX_OPERATOR_API ?? 'http://127.0.0.1:8081'
  const token = process.env.PRYX_OPERATOR_TOKEN
  if (!token) throw new Error('Set PRYX_OPERATOR_TOKEN')
  let path: string
  let body: Record<string, unknown> | undefined
  switch (verb) {
    case 'ls':
      path = '/nodes'
      break
    case 'get':
      if (!id) throw new Error('node get <id>')
      path = `/nodes/${encodeURIComponent(id)}`
      break
    case 'token':
      path = '/tokens'
      body = { region: id ?? 'default' }
      break
    case 'drain':
    case 'undrain':
    case 'retire':
    case 'reinstate':
    case 'clear-quarantine':
      if (!id) throw new Error(`node ${verb} <id>`)
      path = `/nodes/${encodeURIComponent(id)}/${verb}`
      body = {}
      break
    case 'lost': {
      if (!id) throw new Error('node lost <id> --fenced-by <proof> --reason <reason>')
      const option = (name: string) => rest[rest.indexOf(name) + 1]
      if (!rest.includes('--fenced-by') || !rest.includes('--reason'))
        throw new Error('lost requires --fenced-by and --reason')
      path = `/nodes/${encodeURIComponent(id)}/lost`
      body = {
        fencedBy: option('--fenced-by'),
        reason: option('--reason'),
        force: rest.includes('--force'),
      }
      break
    }
    case 'events':
      path = '/events'
      break
    case 'summary':
      path = '/summary'
      break
    default:
      throw new Error(`Unknown node command: ${verb}`)
  }
  const response = await fetch(`${base}/fleet/v1${path}`, {
    method: body ? 'POST' : 'GET',
    headers: {
      authorization: `Bearer ${token}`,
      'content-type': 'application/json',
      'x-operator': process.env.PRYX_OPERATOR ?? 'cli',
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  })
  const result = await response.json()
  if (!response.ok) throw new Error(`Operator API refused: ${response.status}`)
  return result
}
