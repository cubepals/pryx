// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real installed one-box project admission exercise. Runs as the installer user with root-only configuration access in CI, never prints credentials. */
import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { cpus, totalmem } from 'node:os'
import { Pryx, PryxError } from '@cubepals/pryx'
import { StatusCodes } from 'http-status-codes'
import { readEnvironment } from '../../cli/src/up-files.ts'
import { UNIT_DEFAULTS } from '../src/project-model.ts'

const config = readEnvironment('/etc/pryx/control.env')
const token = config.PRYX_OPERATOR_TOKEN
if (!token) throw new Error('Run against the installed one-box configuration as root')
async function operator(path: string, method = 'GET', body?: unknown) {
  const response = await fetch(`http://127.0.0.1:8081${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    ...(body ? { body: JSON.stringify(body) } : {}),
  })
  if (!response.ok) throw new Error(`Operator measurement refused: ${response.status}`)
  return response.json()
}
const capacity = (await operator('/fleet/v1/nodes')) as {
  nodes: Array<{
    id: string
    unitsFree: number
    memoryMb: { total: number; reservedForHost: number }
    cpu: { cores: number; reservedForHostMillis: number }
  }>
}
console.log(`PROJECT_CAPACITY ${JSON.stringify(capacity)}`)
const reports: unknown[] = []
for (let index = 0; index < 10; index++) {
  const project = (await operator('/operator/v1/projects', 'POST', {
    name: `measurement-${index}`,
    key: true,
    maxUnits: 5,
    maxRunning: 5,
  })) as { id: string; key: { key: string } }
  const client = new Pryx({ url: 'http://127.0.0.1:8082', key: project.key.key })
  const boxes = []
  try {
    for (let unit = 0; unit < 5; unit++) {
      const box = await client.sandbox({ image: 'alpine:3.22', units: 1 })
      boxes.push(box)
      assert.equal((await box.run(['echo', `project-${index}-${unit}`])).stdout, `project-${index}-${unit}\n`)
    }
    let refusal: unknown
    try {
      await client.sandbox({ image: 'alpine:3.22', units: 1 })
      throw new Error('Quota was not enforced')
    } catch (error) {
      if (!(error instanceof PryxError)) throw error
      assert.equal(error.status, StatusCodes.FORBIDDEN)
      assert.equal(error.code, 'limit_exceeded')
      assert.deepEqual(error.details, { name: 'maxUnits', limit: 5, current: 5 })
      refusal = { error: { code: error.code, message: error.message, details: error.details } }
    }
    const usage = await client.project()
    assert.equal(usage.usage.units, 5)
    reports.push({ project: project.id, sandboxes: boxes.length, usage: usage.usage, refusal })
    console.log(`PROJECT_REFUSAL ${JSON.stringify(refusal)}`)
  } finally {
    for (const box of boxes) await box.delete()
    await operator(`/operator/v1/projects/${project.id}`, 'DELETE')
  }
}
const report = {
  at: new Date().toISOString(),
  machine: { memoryBytes: totalmem(), cpus: cpus().length },
  unit: UNIT_DEFAULTS,
  capacity,
  projects: reports,
  totalSandboxes: 50,
  simultaneousSandboxes: 5,
  physical16GiBHostMeasured: false,
  explanation:
    'Ten successive groups of five preserve real CPU and memory admission on the available runner. The host capacity above is measured; a physical 16 GiB host is not available.',
}
writeFileSync(process.argv[2] ?? 'projects-one-box.json', JSON.stringify(report, null, 2) + '\n')
console.log(`PROJECT_MEASUREMENT ${JSON.stringify(report)}`)
