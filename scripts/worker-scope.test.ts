import { expect, test } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

type Workflow = {
  on: { push: { branches: string[] } }
  concurrency: { group: string; 'cancel-in-progress': string }
  jobs: { changes: { steps: { id?: string; run: string }[] } }
}

const workflows = ['ci', 'any-host', 'engines', 'any-host-vm'] as const
const branches = [
  ['k0', true],
  ['k9-feature', true],
  ['K3-topic/nested/fix', true],
  ['codex/topic', true],
  ['CoDeX/nested/topic', true],
  ['k', false],
  ['kernel-docs', false],
  ['topic/k8', false],
  ['codex', false],
  ['codex-other/topic', false],
  ['any-host', false],
  ['main', false],
] as const

function loadWorkflow(name: string): Workflow {
  const path = join(import.meta.dir, '..', '.github', 'workflows', `${name}.yml`)
  return Bun.YAML.parse(readFileSync(path, 'utf8')) as Workflow
}

test('CI selects quick worker pushes, skips same-repository worker PRs and fully tests forks and dispatches', () => {
  const step = loadWorkflow('ci').jobs.changes.steps.find(({ id }) => id === 'scope')
  if (!step) throw new Error('Missing CI scope step')
  const dir = mkdtempSync(join(tmpdir(), 'pryx-worker-scope-'))
  const output = join(dir, 'output')
  try {
    for (const [branch, worker] of branches) {
      const events = [
        ['push', branch, '', 'false', worker ? 'quick' : 'full'],
        ['pull_request', '62/merge', branch, 'true', worker ? 'none' : 'full'],
        ['pull_request', '62/merge', branch, 'false', 'full'],
        ['workflow_dispatch', branch, '', 'false', 'full'],
      ] as const
      for (const [event, ref, head, sameRepo, scope] of events) {
        writeFileSync(output, '')
        const run = Bun.spawnSync(['bash', '-e', '-o', 'pipefail', '-c', step.run], {
          env: {
            ...process.env,
            EVENT: event,
            BRANCH: ref,
            HEAD: head,
            SAME_REPOSITORY: sameRepo,
            GITHUB_OUTPUT: output,
          },
        })
        expect(run.exitCode, run.stderr.toString()).toBe(0)
        expect(readFileSync(output, 'utf8').trim(), `${event} ${branch} same repo=${sameRepo}`).toBe(
          `scope=${scope}`,
        )
      }
    }
  } finally {
    rmSync(dir, { recursive: true })
  }
})

for (const name of workflows) {
  const workflow = loadWorkflow(name)
  test(`${name} pushes match worker and ordinary branches as intended`, () => {
    const patterns = workflow.on.push.branches.map((pattern) => new Bun.Glob(pattern))
    for (const [branch, worker] of branches) {
      expect(
        patterns.some((pattern) => pattern.match(branch)),
        branch,
      ).toBe(name === 'ci' || branch === 'any-host' || worker)
    }
  })

  test(`${name} uses the worker predicate for concurrency`, () => {
    const prefixes = [...Array.from({ length: 10 }, (_, i) => `k${i}`), 'codex/']
    const worker = prefixes.map((prefix) => `startsWith(github.ref, 'refs/heads/${prefix}')`).join(' || ')
    const vm = name === 'any-host-vm'
    const cancel = vm ? `github.event_name == 'pull_request' || ${worker}` : worker
    const dispatch = vm
      ? `\${{ github.event_name == 'workflow_dispatch' && !(${worker}) && 'dispatch-' || '' }}`
      : ''
    expect(workflow.concurrency['cancel-in-progress']).toBe(`\${{ ${cancel} }}`)
    expect(workflow.concurrency.group).toBe(`\${{ github.workflow }}-${dispatch}\${{ github.ref }}`)
  })
}
