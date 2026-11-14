// SPDX-License-Identifier: FSL-1.1-ALv2
/** Validate platform-owned syntax with the owner's schema or tool. Dashboard-only formats
 * have no offline validator; account-required checks are reported separately in hosting.md. */
import { createHash } from 'node:crypto'
import { appendFileSync } from 'node:fs'
import Ajv2020 from 'ajv/dist/2020.js'
import addFormats from 'ajv-formats'

const response = await fetch('https://render.com/schema/render.yaml.json')
if (!response.ok) throw new Error('Render schema unavailable')
const schema = await response.text()
const ajv = new Ajv2020({ strict: false, allErrors: true })
addFormats(ajv)
const validate = ajv.compile(JSON.parse(schema))
if (!validate(Bun.YAML.parse(await Bun.file('render.yaml').text())))
  throw new Error(`Render Blueprint invalid: ${JSON.stringify(validate.errors)}`)
console.log('Render owner schema: pass')
Bun.TOML.parse(await Bun.file('deploy/platforms/fly/fly.toml').text())
const compose = Bun.spawnSync(
  ['docker', 'compose', '-f', 'deploy/platforms/coolify/compose.yaml', 'config', '--quiet'],
  {
    env: {
      ...process.env,
      SERVICE_URL_PRYX: 'https://pryx.example.test',
      SERVICE_URL_PRYX_8080: 'https://pryx.example.test',
      SERVICE_PASSWORD_64_PRYX: 'validator-placeholder',
    },
    stdout: 'inherit',
    stderr: 'inherit',
  },
)
if (compose.exitCode) throw new Error('Coolify Compose invalid')
const cloud = Bun.spawnSync(
  ['cloud-init', 'schema', '--config-file', 'deploy/platforms/cloud-init/user-data.yaml'],
  { stdout: 'inherit', stderr: 'inherit' },
)
if (cloud.exitCode) throw new Error('cloud-init schema invalid')
const summary = `Render owner schema SHA-256 ${createHash('sha256').update(schema).digest('hex')}: pass; Fly TOML syntax: pass (remote validator not run); Coolify Docker Compose: pass; cloud-init schema: pass. Dashboard-only Railway/Koyeb and unsupported App Platform: no validator run.\n`
console.log(summary)
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary)
