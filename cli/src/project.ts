// SPDX-License-Identifier: FSL-1.1-ALv2
/** Operator project commands use the private HTTP API; policy stays in control. */

import { SECOND_MS } from '../../control/src/durations.ts'
import { LIMIT_COLUMNS } from '../../control/src/project-model.ts'
import { type Duration, seconds } from '../../sdk/typescript/src/duration.ts'
import { option } from './sandbox.ts'

export function projectOptions(args: string[]) {
  const body: Record<string, unknown> = {}
  const label = option(args, '--name')
  if (label) body.name = label
  for (const name of Object.keys(LIMIT_COLUMNS)) {
    if (name === 'expiresAt') continue
    const value = option(args, `--${name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`)
    if (value === undefined) continue
    body[name] =
      value === 'unlimited'
        ? null
        : name === 'allowedImages'
          ? value.split(',').filter(Boolean)
          : ['maxTtl', 'maxIdle', 'maxExecTimeout'].includes(name)
            ? seconds(/^\d+(?:\.\d+)?$/.test(value) ? Number(value) : (value as Duration))
            : Number(value)
  }
  const expiry = option(args, '--expires-in')
  if (expiry)
    body.expiresAt = new Date(
      Date.now() +
        seconds(/^\d+(?:\.\d+)?$/.test(expiry) ? Number(expiry) : (expiry as Duration)) * SECOND_MS,
    ).toISOString()
  if (args.includes('--key')) body.key = true
  return body
}
export async function project(args: string[]) {
  const [verb = 'ls', id] = args
  const token = process.env.PRYX_OPERATOR_TOKEN
  if (!token) throw new Error('Set PRYX_OPERATOR_TOKEN')
  if (
    !['create', 'ls', 'get', 'set', 'freeze', 'unfreeze', 'rm', 'usage'].includes(verb) ||
    (!['create', 'ls'].includes(verb) && !id)
  )
    throw new Error('project create|ls|get|set|freeze|unfreeze|rm|usage <id>')
  const method =
    verb === 'create' || verb === 'freeze' || verb === 'unfreeze'
      ? 'POST'
      : verb === 'set'
        ? 'PATCH'
        : verb === 'rm'
          ? 'DELETE'
          : 'GET'
  const suffix = ['create', 'ls'].includes(verb)
    ? ''
    : `/${encodeURIComponent(id as string)}${['freeze', 'unfreeze', 'usage'].includes(verb) ? `/${verb}` : ''}`
  const response = await fetch(
    `${process.env.PRYX_OPERATOR_API ?? 'http://127.0.0.1:8081'}/operator/v1/projects${suffix}`,
    {
      method,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      ...(['POST', 'PATCH'].includes(method) ? { body: JSON.stringify(projectOptions(args)) } : {}),
    },
  )
  const result = await response.json()
  if (!response.ok) throw new Error(`Project request refused: ${JSON.stringify(result)}`)
  return result
}
