// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Run Cubepals' existing structure ratchet over the mixed Rust and Bun repository. */

import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const source = 'https://raw.githubusercontent.com/cubepals/cubepals/e264c475d05aa31ec8871b2ceb8c421cf7370200/scripts/check-structure.ts'
const response = await fetch(source)
if (!response.ok) throw new Error(`Structure source returned ${response.status}`)
const original = await response.text()
if (!/^const SCOPE = .+$/m.test(original)) throw new Error('Structure source scope changed')
const expanded = original.replace(/^const SCOPE = .+$/m, "const SCOPE = new RegExp('^(src|tests|examples|benches|agent|control|cli|edge|sdk|integrations)/')")
mkdirSync('target', { recursive: true })
const path = join(process.cwd(), 'target', 'check-control-structure.ts')
writeFileSync(path, expanded)
const result = Bun.spawnSync([process.execPath, path], { stdio: ['inherit', 'inherit', 'inherit'] })
process.exit(result.exitCode)
