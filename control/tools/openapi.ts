// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

// The typed contracts of the HTTP APIs Blockly calls. Each spec is vendored and pinned, the types
// are generated from it, and CI fails if the committed types no longer match what the pinned
// spec produces. blocklyd's specs are its own, written by its tests from its Rust wire types.
//   bun tools/openapi/openapi.ts generate         regenerate every API's types from its vendored spec
//   bun tools/openapi/openapi.ts check            fail if any spec or its types have drifted
//   bun tools/openapi/openapi.ts update <api>     fetch that API's current spec, then regenerate
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import openapiTS, { astToString } from 'openapi-typescript'
import { parse as parseYaml } from 'yaml'

type Json = null | boolean | number | string | Json[] | { [key: string]: Json }
type Paths = Record<string, Record<string, { operationId?: string }>>

interface Api {
  /**
   * Where the provider publishes the spec, or, for blocklyd's, the command that pins one: its
   * releases carry the specs, and scripts/blocklyd.ts copies the pinned release's here.
   */
  source: string | { command: string }
  /** The vendored copy, relative to apps/control/src/infra. */
  spec: string
  types: string
  /** Mends a published spec the generator can't take as it is. */
  mend?: (spec: { paths: Paths }) => void
  /**
   * Whether a property with a default is required. Boat documents what it defaults, and a request
   * that omits such a property is the one it defaults; sending one can change what it means
   * (`environment` beside `noEnv`), so its types leave them optional.
   */
  defaultNonNullable?: boolean
}

const BLOCKLYD_RELEASE = 'bun scripts/blocklyd.ts bump <version>'

const APIS: Record<string, Api> = {
  boat: {
    source: 'https://docs.boat.dev/openapi/boat-v1.yaml',
    spec: 'boat/boat.openapi.json',
    types: 'boat/generated/boat.ts',
    defaultNonNullable: false,
  },
  // What a blocklyd node reads leaves out what it defaults, as boat's requests do.
  'blocklyd-reads': {
    source: { command: BLOCKLYD_RELEASE },
    spec: 'fleet/node-reads.openapi.json',
    types: 'fleet/generated/node-reads.ts',
    defaultNonNullable: false,
  },
  'blocklyd-writes': {
    source: { command: BLOCKLYD_RELEASE },
    spec: 'fleet/node-writes.openapi.json',
    types: 'fleet/generated/node-writes.ts',
  },
  fly: {
    source: 'https://docs.machines.dev/openapi.json',
    spec: 'fly/machines.openapi.json',
    types: 'fly/generated/machines.ts',
    mend: uniqueOperationIds,
  },
  modrinth: {
    source: 'https://docs.modrinth.com/openapi.yaml',
    spec: 'modrinth/labrinth.openapi.json',
    types: 'modrinth/generated/labrinth.ts',
  },
  hangar: {
    source: 'https://hangar.papermc.io/v3/api-docs',
    spec: 'hangar/hangar.openapi.json',
    types: 'hangar/generated/hangar.ts',
    mend: publicRoutesOnly,
  },
}

const INFRA = new URL('../../apps/control/src/infra/', import.meta.url)
const at = (path: string) => new URL(path, INFRA)
/** What brings an API's spec up to date. */
const refresh = (name: string, api: Api) =>
  typeof api.source === 'string' ? `bun run openapi:update ${name}` : api.source.command

const sortKeys = (value: Json): Json =>
  Array.isArray(value)
    ? value.map(sortKeys)
    : value !== null && typeof value === 'object'
      ? Object.fromEntries(
          Object.keys(value)
            .sort()
            .map((k) => [k, sortKeys(value[k] as Json)]),
        )
      : value
/** Sorted keys, two-space indent, JSON whatever the source format: an update reads as a diff. */
const canonical = (spec: Json) => `${JSON.stringify(sortKeys(spec), null, 2)}\n`

/**
 * Fly's spec breaks one OpenAPI rule: PUT and PATCH on a machine's metadata share an operationId,
 * which the generator refuses. A repeated id gets its method appended; paths and schemas stay
 * exactly as Fly publishes them.
 */
function uniqueOperationIds(spec: { paths: Paths }) {
  const seen = new Set<string>()
  for (const item of Object.values(spec.paths))
    for (const [method, operation] of Object.entries(item)) {
      if (typeof operation?.operationId !== 'string') continue
      if (seen.has(operation.operationId)) operation.operationId = `${operation.operationId}_${method}`
      seen.add(operation.operationId)
    }
}

/**
 * Hangar's spec also lists its site's own routes, which it says "should be considered internal,
 * and can change at a moment's notice. Do not use them". Only `/api/v1` is typed, without the
 * deprecated `{author}/` copies of its routes, which repeat the current ones' operationIds.
 */
function publicRoutesOnly(spec: { paths: Paths }) {
  for (const [path, item] of Object.entries(spec.paths)) {
    const deprecated = Object.values(item).every(
      (operation) => (operation as { deprecated?: boolean }).deprecated,
    )
    if (!path.startsWith('/api/v1/') || deprecated) delete spec.paths[path]
  }
}

async function render(name: string, api: Api, specText: string): Promise<string> {
  const sha256 = createHash('sha256').update(specText).digest('hex')
  const spec = JSON.parse(specText)
  api.mend?.(spec)
  return [
    `// Generated from infra/${api.spec} (sha256 ${sha256}) by tools/openapi.`,
    `// Do not edit. Regenerate with \`bun run openapi:generate\`; update the spec with \`${refresh(name, api)}\`.`,
    '',
    astToString(await openapiTS(spec, { defaultNonNullable: api.defaultNonNullable ?? true })),
  ].join('\n')
}

async function fetchSpec(source: string): Promise<Json> {
  const response = await fetch(source)
  if (!response.ok) throw new Error(`${source} answered ${response.status}`)
  const text = await response.text()
  return (source.endsWith('.json') ? JSON.parse(text) : parseYaml(text)) as Json
}

const [mode, only] = process.argv.slice(2)
if (mode === 'update') {
  const api = only === undefined ? undefined : APIS[only]
  if (api === undefined || typeof api.source !== 'string') {
    const published = Object.keys(APIS).filter((name) => typeof APIS[name]?.source === 'string')
    console.error(`usage: bun tools/openapi/openapi.ts update <${published.join(' | ')}>`)
    process.exit(2)
  }
  writeFileSync(at(api.spec), canonical(await fetchSpec(api.source)))
  writeFileSync(at(api.types), await render(only as string, api, readFileSync(at(api.spec), 'utf8')))
  console.warn(`Updated infra/${api.spec} and regenerated infra/${api.types}`)
} else if (mode === 'generate') {
  for (const [name, api] of Object.entries(APIS)) {
    writeFileSync(at(api.types), await render(name, api, readFileSync(at(api.spec), 'utf8')))
    console.warn(`Wrote infra/${api.types}`)
  }
} else if (mode === 'check') {
  const problems: string[] = []
  for (const [name, api] of Object.entries(APIS)) {
    const specText = readFileSync(at(api.spec), 'utf8')
    if (canonical(JSON.parse(specText) as Json) !== specText)
      problems.push(`infra/${api.spec} is not in canonical form; run \`${refresh(name, api)}\``)
    if (readFileSync(at(api.types), 'utf8') !== (await render(name, api, specText)))
      problems.push(`infra/${api.types} does not match its vendored spec; run \`bun run openapi:generate\``)
  }
  if (problems.length > 0) {
    console.error(`API contract drift:\n  ${problems.join('\n  ')}`)
    process.exit(1)
  }
  console.warn(`API types match their pinned specs (${Object.keys(APIS).join(', ')}).`)
} else {
  console.error('usage: bun tools/openapi/openapi.ts generate | check | update <api>')
  process.exit(2)
}
