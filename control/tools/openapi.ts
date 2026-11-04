// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { createHash } from 'node:crypto'
/** Generate node API types from this repository's Rust schemas and fail on committed drift. */
import { readFileSync, writeFileSync } from 'node:fs'
import openapiTS, { astToString } from 'openapi-typescript'

const mode = process.argv[2]
if (mode !== 'generate' && mode !== 'check') throw new Error('usage: openapi.ts generate | check')
for (const direction of ['reads', 'writes']) {
  const source = new URL(`../../openapi/node-${direction}.openapi.json`, import.meta.url)
  const target = new URL(`../src/generated/node-${direction}.ts`, import.meta.url)
  const spec = readFileSync(source, 'utf8')
  const generated = [
    '// SPDX-FileCopyrightText: 2026 The Cubepals Authors',
    '// SPDX-License-Identifier: FSL-1.1-ALv2',
    `// Generated from openapi/node-${direction}.openapi.json; sha256 ${createHash('sha256').update(spec).digest('hex')}.`,
    astToString(
      await openapiTS(JSON.parse(spec), {
        defaultNonNullable: direction === 'writes',
      }),
    ),
  ].join('\n')
  if (mode === 'generate') writeFileSync(target, generated)
  else if (readFileSync(target, 'utf8') !== generated) throw new Error(`API type drift: ${target.pathname}`)
}
console.log(`Node API types: ${mode}`)
