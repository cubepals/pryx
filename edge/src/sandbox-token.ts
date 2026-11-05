// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Offline sandbox capability verification, using the node's committed schema and public key. */
import { createPublicKey, verify } from 'node:crypto'
import { z } from 'zod'
import { SECOND_MS } from '../../control/src/durations.ts'
import schema from '../../openapi/node-writes.openapi.json'
import type { components } from './generated/api.ts'
import type { EdgeRoute } from './wire.ts'

type Claims = components['schemas']['AgentClaims']
function resolve(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(resolve)
  if (value && typeof value === 'object') {
    const item = value as Record<string, unknown>
    if (typeof item.$ref === 'string')
      return resolve(
        schema.components.schemas[
          item.$ref.replace('#/components/schemas/', '') as keyof typeof schema.components.schemas
        ],
      )
    return Object.fromEntries(Object.entries(item).map(([key, item]) => [key, resolve(item)]))
  }
  return value
}
const claimsSchema = z.fromJSONSchema(
  resolve(schema.components.schemas.Claims) as Parameters<typeof z.fromJSONSchema>[0],
)
const policy = schema['x-pryx-agent']
export function sandboxToken(
  token: string | undefined,
  route: EdgeRoute,
  at = Math.floor(Date.now() / SECOND_MS),
): Claims | undefined {
  if (!token || token.length > policy.maxTokenBytes || !route.agent) return undefined
  const pieces = token.split('.')
  if (
    pieces.length !== 3 ||
    pieces[0] !== policy.tokenPrefix ||
    !pieces.slice(1).every((part) => /^[a-zA-Z0-9_-]+$/.test(part))
  )
    return undefined
  try {
    const key = createPublicKey({
      key: { kty: 'OKP', crv: 'Ed25519', x: route.agent.manifest.publicKey },
      format: 'jwk',
    })
    if (
      !verify(
        null,
        Buffer.from(`${pieces[0]}.${pieces[1]}`),
        key,
        Buffer.from(pieces[2] as string, 'base64url'),
      )
    )
      return undefined
    const claims = claimsSchema.parse(
      JSON.parse(Buffer.from(pieces[1] as string, 'base64url').toString('utf8')),
    ) as Claims
    if (
      claims.sandbox !== route.sandbox ||
      claims.epoch !== route.epoch ||
      claims.expiresAt <= at ||
      claims.expiresAt <= claims.issuedAt ||
      claims.issuedAt > at + policy.tokenClockSkewSeconds ||
      claims.expiresAt - claims.issuedAt > policy.tokenLifetimeSeconds
    )
      return undefined
    return claims
  } catch {
    return undefined
  }
}
export function requestToken(request: Request) {
  return (
    request.headers.get('authorization')?.replace(/^Bearer /, '') ??
    request.headers
      .get('sec-websocket-protocol')
      ?.split(',')
      .map((p) => p.trim())
      .find((p) => p.startsWith('pryx-token.'))
      ?.slice('pryx-token.'.length) ??
    new URL(request.url).searchParams.get('token') ??
    undefined
  )
}

export function fileToken(claims: Claims | undefined, request: Request, path: string) {
  return (
    claims?.scope === 'file' &&
    path === '/_pryx/files/content' &&
    claims.file?.path === new URL(request.url).searchParams.get('path') &&
    claims.file?.method === request.method &&
    claims.expiresAt - claims.issuedAt <= policy.fileTokenLifetimeSeconds
  )
}
