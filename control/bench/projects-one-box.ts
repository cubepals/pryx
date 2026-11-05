// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real installed one-box project admission exercise. Runs as the installer user with root-only
 * configuration access in CI. Partial results survive failure; credentials never enter reports. */
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { cpus, totalmem } from 'node:os'
import { Pryx, PryxError } from '@cubepals/pryx'
import { StatusCodes } from 'http-status-codes'
import { readEnvironment } from '../../cli/src/up-files.ts'
import type { OperatorNode } from '../src/operator-api.ts'

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
const measured = (await operator('/fleet/v1/nodes')) as { nodes: OperatorNode[] }
const capacity = {
  nodes: measured.nodes.map(({ id, unitsFree, unit, memoryMb, cpu, disk, ports }) => ({
    id,
    unitsFree,
    unit,
    memoryMb,
    cpu,
    disk,
    ports,
  })),
}
assert.equal(capacity.nodes.length, 1)
const quota = Math.min(5, capacity.nodes[0]?.unitsFree ?? 0)
assert.ok(quota > 0, 'The measured host must have room for at least one running unit')
console.log(`PROJECT_CAPACITY ${JSON.stringify(capacity)}`)
const reports: Record<string, unknown>[] = []
const report = {
  at: new Date().toISOString(),
  machine: { memoryBytes: totalmem(), cpus: cpus().length },
  unit: capacity.nodes[0]?.unit,
  capacity,
  kernel: {
    ephemeralPorts: readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').trim(),
    reservedPorts: readFileSync('/proc/sys/net/ipv4/ip_local_reserved_ports', 'utf8').trim(),
  },
  projects: reports,
  totalSandboxes: 0,
  ranSandboxes: 0,
  simultaneousSandboxes: 0,
  quota,
  completed: false,
  failure: undefined as { name: string; message: string; code?: string; status?: number } | undefined,
  physical16GiBHostMeasured: false,
  explanation:
    'Each of ten projects creates five stopped sandboxes and runs every one, rotating active work within its measured host unit capacity. Counts include acknowledged creations and validated command results only. Refusals exceed that project quota without relaxing host reservations. No 16 GiB capacity is inferred from the observed host.',
}
try {
  for (let index = 0; index < 10; index++) {
    const project = (await operator('/operator/v1/projects', 'POST', {
      name: `measurement-${index}`,
      key: true,
      maxUnits: quota,
      maxRunning: quota,
    })) as { id: string; key: { key: string } }
    const client = new Pryx({ url: 'http://127.0.0.1:8082', key: project.key.key })
    const boxes = []
    const partial: Record<string, unknown> = { project: project.id, sandboxes: 0, ran: 0, completed: false }
    reports.push(partial)
    try {
      for (let unit = 0; unit < 5; unit++) {
        const box = await client.sandbox({ image: 'alpine:3.22', units: 1, start: false })
        boxes.push(box)
        partial.sandboxes = boxes.length
        report.totalSandboxes++
      }
      const active = []
      let ran = 0
      for (const [unit, box] of boxes.entries()) {
        if (active.length === quota) await active.shift()?.stop()
        await box.start()
        active.push(box)
        report.simultaneousSandboxes = Math.max(report.simultaneousSandboxes, active.length)
        assert.equal(
          (await box.run(['echo', `project-${index}-${unit}`])).stdout,
          `project-${index}-${unit}\n`,
        )
        partial.ran = ++ran
        report.ranSandboxes++
      }
      let refusal: unknown
      try {
        await client.sandbox({ image: 'alpine:3.22', units: 1 })
        throw new Error('Quota was not enforced')
      } catch (error) {
        if (!(error instanceof PryxError)) throw error
        assert.equal(error.status, StatusCodes.FORBIDDEN)
        assert.equal(error.code, 'limit_exceeded')
        assert.deepEqual(error.details, { name: 'maxUnits', limit: quota, current: quota })
        refusal = { error: { code: error.code, message: error.message, details: error.details } }
      }
      const usage = await client.project()
      assert.equal(usage.usage.units, quota)
      Object.assign(partial, { usage: usage.usage, refusal, completed: true })
      console.log(`PROJECT_REFUSAL ${JSON.stringify(refusal)}`)
    } finally {
      for (const box of boxes) await box.delete()
      await operator(`/operator/v1/projects/${project.id}`, 'DELETE')
    }
  }
  assert.equal(report.totalSandboxes, 50)
  assert.equal(report.ranSandboxes, 50)
  report.completed = true
} catch (error) {
  report.failure =
    error instanceof Error
      ? {
          name: error.name,
          message: error.message,
          ...(error instanceof PryxError ? { code: error.code, status: error.status } : {}),
        }
      : { name: 'Error', message: 'Unknown measurement failure' }
  throw error
} finally {
  writeFileSync(process.argv[2] ?? 'projects-one-box.json', `${JSON.stringify(report, null, 2)}\n`)
  console.log(`PROJECT_MEASUREMENT ${JSON.stringify(report)}`)
}
