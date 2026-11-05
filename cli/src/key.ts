// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Operator-authenticated key administration; only create prints a raw key. */
import { option } from './sandbox.ts'
export async function key(args: string[]) {
  const [verb = 'ls', id] = args
  const token = process.env.PRYX_OPERATOR_TOKEN
  if (!token) throw new Error('Set PRYX_OPERATOR_TOKEN')
  const body =
    verb === 'create'
      ? { name: option(args, '--name') ?? 'Key', project: option(args, '--project') }
      : undefined
  if (!['create', 'ls', 'revoke'].includes(verb) || (verb === 'revoke' && !id))
    throw new Error('key create|ls|revoke <id>')
  const response = await fetch(
    `${process.env.PRYX_OPERATOR_API ?? 'http://127.0.0.1:8081'}/fleet/v1/keys${verb === 'revoke' ? `/${encodeURIComponent(id ?? '')}` : ''}${option(args, '--project') ? `?${new URLSearchParams({ project: option(args, '--project') as string })}` : ''}`,
    {
      method: verb === 'create' ? 'POST' : verb === 'revoke' ? 'DELETE' : 'GET',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      ...(body ? { body: JSON.stringify(body) } : {}),
    },
  )
  if (!response.ok) throw new Error(`Key request refused: ${response.status}`)
  return response.json()
}
