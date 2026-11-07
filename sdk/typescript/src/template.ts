// SPDX-License-Identifier: FSL-1.1-ALv2
/** Local code-template builds use ordinary sandbox files, commands and snapshots. Source
 * paths stay on the caller; durable versions store only a prepared snapshot and plan text. */

import type { Sandbox, SandboxOptions } from './handles.ts'
import type { Transport } from './transport.ts'
import { PryxError } from './transport.ts'
export type TemplateOptions = {
  name: string
  from?: string
  isolation?: SandboxOptions['isolation']
  copy?: [string, string][]
  run?: string[]
  env?: Record<string, string>
  workdir?: string
  ready?: string
  start?: string
}
export type TemplateView = { id: string; name: string; version: number; snapshot: string; start?: string }
export class Template {
  constructor(
    readonly view: TemplateView,
    private build: () => Promise<Template>,
  ) {}
  get id() {
    return this.view.id
  }
  get name() {
    return this.view.name
  }
  get version() {
    return this.view.version
  }
  rebuild() {
    return this.build()
  }
}
export async function buildTemplate(
  http: Transport,
  create: (options: SandboxOptions) => Promise<Sandbox>,
  options: TemplateOptions,
): Promise<Template> {
  const box = await create({
    image: options.from,
    isolation: options.isolation,
    env: options.env,
    workdir: options.workdir,
  })
  let registered = false
  try {
    for (const [local, remote] of options.copy ?? []) {
      const { lstat } = await import('node:fs/promises')
      if ((await lstat(local)).isDirectory()) await box.files.uploadDir(local, remote)
      else await box.files.upload(local, remote)
    }
    for (const command of [...(options.run ?? []), ...(options.ready ? [options.ready] : [])]) {
      const result = await box.run(command)
      if (result.code !== 0)
        throw new PryxError(0, 'template_failed', `Template command failed: ${result.stderr}`)
    }
    const snapshot = await box.snapshot()
    await box.stop()
    const { copy: _, ...plan } = options
    const view = await http.request<TemplateView>(
      `/v1/templates/${encodeURIComponent(options.name)}/versions`,
      'POST',
      { snapshot: snapshot.id, start: options.start, plan },
    )
    registered = true
    return new Template(view, () => buildTemplate(http, create, options))
  } finally {
    if (!registered) await box.delete()
  }
}
