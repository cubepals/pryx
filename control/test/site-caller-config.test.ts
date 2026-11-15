// SPDX-License-Identifier: FSL-1.1-ALv2
/** Mutate real guide blocks to catch configurations the reference transport would otherwise
 * accept through ambient credentials or ignored vendor fields. No agent account is involved. */
import { expect, test } from 'bun:test'
import { ZodError } from 'zod'
import { callerBlocks } from '../../site/tools/sources.ts'
import { siteActionStep, siteMcpConfiguration } from '../tools/site-callers.ts'

const guides = callerBlocks()
const environment = { PRYX_API_URL: 'https://api.example.test', PRYX_API_KEY: 'site-config-fixture' }
function guide(id: string) {
  const page = guides.find((page) => page.id === id)
  const block = page?.codes[0]
  if (!page || !block) throw new Error('Canonical guide missing')
  return { id: page.id, ...block }
}

test('every canonical MCP guide supplies its own complete credential environment', () => {
  for (const page of guides.filter((page) => page.id.startsWith('connect/mcp'))) {
    const block = guide(page.id)
    expect(siteMcpConfiguration(block.id, block.language, block.code, environment)).toEqual({
      command: 'pryx',
      args: ['mcp'],
      env: environment,
    })
  }
})

test('Codex cannot pass through missing or duplicate environment grants', () => {
  const block = guide('connect/mcp/codex')
  const missing = block.code.replace(/^env_vars.*$/m, '')
  expect(() => siteMcpConfiguration(block.id, block.language, missing, environment)).toThrow(ZodError)
  const duplicate = block.code.replace('"PRYX_API_KEY"', '"PRYX_API_URL"')
  expect(() => siteMcpConfiguration(block.id, block.language, duplicate, environment)).toThrow(ZodError)
})

test('Cursor needs its documented transport and variable syntax', () => {
  const block = guide('connect/mcp/cursor')
  const config = JSON.parse(block.code)
  delete config.mcpServers.pryx.type
  expect(() => siteMcpConfiguration(block.id, block.language, JSON.stringify(config), environment)).toThrow(
    ZodError,
  )
  expect(() =>
    siteMcpConfiguration(block.id, block.language, block.code.replaceAll('${env:', '${'), environment),
  ).toThrow(ZodError)
})

test('missing MCP environment mappings and disabled servers cannot be ignored', () => {
  const block = guide('connect/mcp/claude')
  const config = JSON.parse(block.code)
  delete config.mcpServers.pryx.env.PRYX_API_KEY
  expect(() => siteMcpConfiguration(block.id, block.language, JSON.stringify(config), environment)).toThrow(
    ZodError,
  )
  config.mcpServers.pryx.env.PRYX_API_KEY = '$' + '{PRYX_API_KEY}'
  config.mcpServers.pryx.disabled = true
  expect(() => siteMcpConfiguration(block.id, block.language, JSON.stringify(config), environment)).toThrow(
    ZodError,
  )
  expect(() => siteMcpConfiguration(block.id, block.language, block.code, {})).toThrow(ZodError)
})

test('Copilot tool filtering and additional servers must be executed or refused', () => {
  const block = guide('connect/mcp/copilot')
  const config = JSON.parse(block.code)
  config.mcpServers.pryx.tools = []
  expect(() => siteMcpConfiguration(block.id, block.language, JSON.stringify(config), environment)).toThrow(
    ZodError,
  )
  config.mcpServers.pryx.tools = ['*']
  config.mcpServers.other = { command: 'false' }
  expect(() => siteMcpConfiguration(block.id, block.language, JSON.stringify(config), environment)).toThrow(
    ZodError,
  )
})

test('Action blocks cannot hide unexecuted steps, conditions or inputs', () => {
  const block = guide('connect/action')
  expect(siteActionStep(block.code).with.run).toBe('echo hello')
  expect(() => siteActionStep(`${block.code}\n- run: exit 1\n`)).toThrow(ZodError)
  expect(() => siteActionStep(block.code.replace('  with:', '  if: false\n  with:'))).toThrow(ZodError)
  expect(() => siteActionStep(`${block.code}\n    snapshot: 'true'\n`)).toThrow(ZodError)
})
