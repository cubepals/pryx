// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Edge listener and admission configuration. Defaults for rates come from Cubepals agent.ts. */
import { hostname } from 'node:os'
import { z } from 'zod'
import { SECOND_MS } from '../../control/src/durations.ts'

const positive = z.number().positive()
export function edgeConfig(env: Record<string, string | undefined> = process.env) {
  const n = (key: string, fallback: number) =>
    env[`PRYX_EDGE_${key}`] === undefined ? fallback : Number(env[`PRYX_EDGE_${key}`])
  return z
    .object({
      control: z.url(),
      token: z.string().min(32),
      id: z.string().min(1).max(128),
      host: z.string(),
      httpOnly: z.boolean().default(false),
      httpPort: z.number().int().nonnegative().max(65535),
      agentTlsCert: z.string().optional(),
      agentTlsKey: z.string().optional(),
      tlsPort: z.number().int().nonnegative().max(65535),
      metricsHost: z.string(),
      metricsPort: z.number().int().nonnegative().max(65535),
      holdMs: positive,
      greetingMs: positive,
      bufferBytes: positive.int(),
      requestMs: positive,
      limits: z.object({
        address: z.object({
          connections: z.object({ perSecond: positive, burst: positive }),
          greeting: positive.int(),
          open: positive.int(),
        }),
        server: z.object({
          connections: z.object({ perSecond: positive, burst: positive }),
          open: positive.int(),
        }),
      }),
    })
    .refine((value) => Boolean(value.agentTlsCert) === Boolean(value.agentTlsKey), {
      message: 'Agent TLS requires both a certificate and private key',
    })
    .parse({
      agentTlsCert: env.PRYX_EDGE_AGENT_TLS_CERT,
      agentTlsKey: env.PRYX_EDGE_AGENT_TLS_KEY,
      control: env.PRYX_CONTROL_URL ?? 'http://127.0.0.1:8082',
      token: env.PRYX_EDGE_TOKEN,
      id: env.PRYX_EDGE_ID ?? hostname(),
      host: env.PRYX_EDGE_HOST ?? '0.0.0.0',
      httpOnly: env.PRYX_EDGE_HTTP_ONLY === '1',
      httpPort: n('HTTP_PORT', 80),
      tlsPort: n('TLS_PORT', 443),
      metricsHost: env.PRYX_EDGE_METRICS_HOST ?? '127.0.0.1',
      metricsPort: n('METRICS_PORT', 9090),
      holdMs: n('HOLD_SECONDS', 5) * SECOND_MS,
      greetingMs: n('GREETING_SECONDS', 5) * SECOND_MS,
      bufferBytes: n('BUFFER_BYTES', 64 * 1024),
      requestMs: n('REQUEST_SECONDS', 3) * SECOND_MS,
      limits: {
        address: {
          connections: {
            perSecond: n('ADDRESS_CONNECTIONS_PER_SECOND', 5),
            burst: n('ADDRESS_CONNECTIONS_BURST', 30),
          },
          greeting: n('ADDRESS_GREETING', 10),
          open: n('ADDRESS_OPEN_CONNECTIONS', 64),
        },
        server: {
          connections: {
            perSecond: n('SERVER_CONNECTIONS_PER_SECOND', 20),
            burst: n('SERVER_CONNECTIONS_BURST', 100),
          },
          open: n('SERVER_OPEN_CONNECTIONS', 300),
        },
      },
    })
}
export type EdgeConfig = ReturnType<typeof edgeConfig>
