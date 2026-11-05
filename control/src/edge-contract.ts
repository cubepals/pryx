// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** The edge OpenAPI document is generated from exactly the schemas used by its authenticated routes. */

import { StatusCodes } from 'http-status-codes'

import { z } from 'zod'
import { EDGE_FEATURES_HEADER } from './edge-headers.ts'
import {
  EdgeRoute,
  EdgeRoutes,
  IdleHint,
  PingWakeResult,
  SessionEvent,
  VisitReports,
  WakeRequest,
  WakeResult,
} from './edge-schemas.ts'
import { agentClaimsSchema } from './node-contract.ts'
export function edgeContract() {
  const schemas = Object.fromEntries(
    Object.entries({
      AgentClaims: agentClaimsSchema,
      EdgeRoute,
      EdgeRoutes,
      SessionEvent,
      VisitReports,
      WakeRequest,
      WakeResult,
      IdleHint,
      PingWakeResult,
    }).map(([k, s]) => {
      const { $schema: _, ...value } = z.toJSONSchema(s)
      return [k, value]
    }),
  )
  const ref = (name: string) => ({ $ref: `#/components/schemas/${name}` })
  const paths = Object.fromEntries(
    [
      ['routes', 'get', null, 'EdgeRoutes'],
      ['wake', 'post', 'WakeRequest', 'WakeResult'],
      ['ping-wake', 'post', 'WakeRequest', 'PingWakeResult'],
      ['idle', 'post', 'IdleHint', null],
      ['sessions', 'post', 'SessionEvent', null],
      ['visits', 'post', 'VisitReports', null],
    ].map(([path, method, body, response]) => [
      `/edge/v1/${path}`,
      {
        [method as string]: {
          security: [{ edgeToken: [] }],
          parameters: [
            { in: 'header', name: 'x-edge-id', required: true, schema: { type: 'string' } },
            ...(path === 'routes'
              ? [
                  { in: 'header', name: 'x-routes-held', schema: { type: 'string' } },
                  {
                    in: 'header',
                    name: EDGE_FEATURES_HEADER,
                    schema: { type: 'string' },
                    description:
                      'Comma-separated additive edge features; agent enables sandbox capabilities and agent manifests.',
                  },
                ]
              : []),
          ],
          ...(body
            ? { requestBody: { required: true, content: { 'application/json': { schema: ref(body) } } } }
            : {}),
          responses: {
            [response || path === 'ping-wake' ? StatusCodes.OK : StatusCodes.NO_CONTENT]: {
              description: 'Success',
              ...(response ? { content: { 'application/json': { schema: ref(response) } } } : {}),
            },
            [StatusCodes.UNAUTHORIZED]: { description: 'Edge token refused' },
          },
        },
      },
    ]),
  )
  return {
    openapi: '3.1.0',
    info: { title: 'pryx edge protocol', version: '1.0.0' },
    paths,
    components: { schemas, securitySchemes: { edgeToken: { type: 'http', scheme: 'bearer' } } },
  }
}
