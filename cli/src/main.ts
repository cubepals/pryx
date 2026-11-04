// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

import { node } from './node.ts'

const [group, ...args] = process.argv.slice(2)
try {
  if (group !== 'node')
    throw new Error('usage: pryx node ls|get|token|drain|undrain|lost|reinstate|retire|events|summary')
  console.log(JSON.stringify(await node(args), null, 2))
} catch (error) {
  console.error(error instanceof Error ? error.message : 'Command failed')
  process.exitCode = 1
}
