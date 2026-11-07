// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Provider contracts rent and bootstrap hosts; placement, retries and scaling policy belong to control. */
import { z } from 'zod'
export const DEPLOYMENT_LABEL = 'pryx.deployment'
export const ACQUIRE_LABEL = 'pryx.acquire'
export const shapeSchema = z.object({
  name: z.string().min(1),
  cpus: z.number().positive(),
  memoryGb: z.number().positive(),
  diskGb: z.number().positive(),
  price: z.object({
    currency: z.string(),
    hour: z.string().nullable(),
    month: z.string().nullable(),
    source: z.string().optional(),
  }),
})
export type Shape = z.infer<typeof shapeSchema>
export interface Host {
  id: string
  region: string
  address: string | null
  requestKey: string
  shape: string
}
export interface Bootstrap {
  line: string
  allowedImages: string[]
}
export interface Provider {
  readonly name: string
  readonly managed: boolean
  acquire(region: string, shape: Shape, key: string, bootstrap: Bootstrap): Promise<Host>
  bootstrap(host: Host, bootstrap: Bootstrap, key: string): Promise<void>
  release(host: Host, key: string): Promise<void>
  list(key?: string): Promise<Host[]>
  shapes(region: string, key?: string): Promise<Shape[]>
}
export class ProviderError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status?: number,
  ) {
    super(message)
    this.name = 'ProviderError'
  }
}
export const shellQuote = (value: string) => `'${value.replaceAll("'", "'\"'\"'")}'`
export function bootstrapScript(bootstrap: Bootstrap, key: string) {
  return `set -eu
install -d -m 0700 /var/lib/pryx-provider
marker=/var/lib/pryx-provider/acquire-key
if [ -f /etc/pryxd/pryxd.toml ] && [ ! -f "$marker" ]; then exit 1; fi
if [ -f "$marker" ] && [ "$(cat "$marker")" != ${shellQuote(key)} ]; then exit 1; fi
printf %s ${shellQuote(key)} > "$marker"
${bootstrap.line} ${bootstrap.allowedImages.flatMap((p) => ['--allow-image', shellQuote(p)]).join(' ')}
`
}
