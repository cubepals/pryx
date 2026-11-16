// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real Python child ownership and private failure-only positions, without a deployment.
 * The PTY descendant escapes the initial group and ignores graceful retirement signals. */
import { expect, test } from 'bun:test'
import { randomBytes } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer, type Socket } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PYTHON_AGENT_BUDGET_MS, pythonAgent } from '../e2e/python-agent.ts'
import { SECOND_MS } from '../src/durations.ts'

const python = Bun.which('python3')
if (!python) throw new Error('Python is required for acceptance child ownership')
const target = 'control/test/fixtures/python-agent-evidence.py'

async function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'python-agent-evidence-'))
  const sockets = new Set<Socket>()
  const server = createServer((socket) => {
    sockets.add(socket)
    socket.on('close', () => sockets.delete(socket))
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('Missing fixture listener')
  const secret = randomBytes(32).toString('hex')
  const env = {
    FIXTURE_MODE: 'stall',
    FIXTURE_SECRET: secret,
    FIXTURE_PIDS: join(root, 'pids'),
    FIXTURE_PORT: String(address.port),
  }
  return {
    root,
    secret,
    env,
    pids: () => JSON.parse(readFileSync(env.FIXTURE_PIDS, 'utf8')) as number[],
    async ready() {
      const deadline = performance.now() + 3 * SECOND_MS
      while (!existsSync(env.FIXTURE_PIDS) || sockets.size === 0) {
        if (performance.now() >= deadline) throw new Error('Python fixture was not ready')
        await Bun.sleep(SECOND_MS / 100)
      }
    },
    close() {
      for (const socket of sockets) socket.destroy()
      server.close()
      rmSync(root, { recursive: true, force: true })
    },
  }
}

function assertRetired(pids: number[]) {
  for (const pid of pids) expect(existsSync(`/proc/${pid}`)).toBe(false)
}

test('Python acceptance success keeps output and closes before deployment', async () => {
  const f = await fixture()
  try {
    const owner = Bun.spawn([python, 'sdk/python/e2e/agent_evidence.py', target, '1', '1', '0.1'], {
      env: { ...f.env, FIXTURE_MODE: 'success' },
      stdout: 'pipe',
      stderr: 'pipe',
    })
    expect(await owner.exited).toBe(0)
    expect(await new Response(owner.stdout).text()).toBe('unchanged success\n')
    expect(await new Response(owner.stderr).text()).toBe('')
    let closed = false
    expect(
      await pythonAgent({
        env: { ...f.env, FIXTURE_MODE: 'success' },
        target,
        python,
        deadline: performance.now() + PYTHON_AGENT_BUDGET_MS,
        close: async () => {
          closed = true
        },
      }),
    ).toBe(0)
    expect(closed).toBe(true)
  } finally {
    f.close()
  }
})

for (const cancellation of [undefined, 'SIGTERM', 'SIGINT'] as const) {
  test(`Python acceptance retires a stalled PTY tree on ${cancellation ?? 'deadline'}`, async () => {
    const f = await fixture()
    let owner: Bun.Subprocess<'ignore', 'pipe', 'pipe'> | undefined
    try {
      owner = Bun.spawn(
        [python, 'sdk/python/e2e/agent_evidence.py', target, cancellation ? '10' : '0.5', '0.1', '0.1'],
        { env: f.env, stdout: 'pipe', stderr: 'pipe' },
      )
      await f.ready()
      const started = performance.now()
      if (cancellation) owner.kill(cancellation)
      expect(await owner.exited).not.toBe(0)
      expect(performance.now() - started).toBeLessThan(2 * SECOND_MS)
      const output = await new Response(owner.stdout).text()
      const error = await new Response(owner.stderr).text()
      expect(output).toBe('')
      expect(error).toMatch(/Python agent (timeout|cancelled): agent.py:[0-9]+; phase [0-9.]+s/)
      expect(error).toContain('in blocked_socket')
      expect(error).toContain('in join')
      expect(error.includes(f.secret)).toBe(false)
      expect(error).not.toContain('fixture.invalid')
      expect(error).not.toContain('FIXTURE_SECRET')
      for (const line of error.trim().split('\n').slice(1)) {
        expect(line).toMatch(/^ {2}File "[^"\r\n]+", line [0-9]+ in [^\r\n]+$/)
      }
      assertRetired(f.pids())
    } finally {
      if (owner && owner.exitCode === null) {
        owner.kill('SIGTERM')
        await owner.exited
      }
      f.close()
    }
  })
}

for (const cancellation of ['SIGTERM', 'SIGINT'] as const) {
  test(`Bun ${cancellation} retires Python and PTY before deployment cleanup`, async () => {
    const f = await fixture()
    const closed = join(f.root, 'closed')
    const runner = join(f.root, 'runner.ts')
    writeFileSync(
      runner,
      `import { pythonAgent } from ${JSON.stringify(new URL('../e2e/python-agent.ts', import.meta.url).pathname)}
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
await pythonAgent({
  python: ${JSON.stringify(python)}, env: ${JSON.stringify(f.env)}, target: ${JSON.stringify(target)}, deadline: performance.now()+${PYTHON_AGENT_BUDGET_MS},
  close: async () => {
    const pids = JSON.parse(readFileSync(${JSON.stringify(f.env.FIXTURE_PIDS)},'utf8'))
    if (pids.some((pid) => existsSync('/proc/'+pid))) throw new Error('Child outlived deployment')
    writeFileSync(${JSON.stringify(closed)}, 'closed')
  }
})
`,
    )
    const child = Bun.spawn([process.execPath, runner], { stdout: 'pipe', stderr: 'pipe' })
    try {
      await f.ready()
      child.kill(cancellation)
      expect(await child.exited).not.toBe(0)
      expect(readFileSync(closed, 'utf8')).toBe('closed')
      assertRetired(f.pids())
      expect((await new Response(child.stderr).text()).includes(f.secret)).toBe(false)
    } finally {
      if (child.exitCode === null) {
        child.kill('SIGTERM')
        await child.exited
      }
      f.close()
    }
  })
}

test('Python bootstrap without registration never receives the evidence signal', async () => {
  const f = await fixture()
  try {
    const owner = Bun.spawn([python, target, 'bootstrap'], {
      env: f.env,
      stdout: 'pipe',
      stderr: 'pipe',
    })
    expect(await owner.exited).not.toBe(0)
    expect(await new Response(owner.stdout).text()).toBe('')
    expect(await new Response(owner.stderr).text()).toMatch(/Python agent timeout: bootstrap;/)
    expect(existsSync(`${f.env.FIXTURE_PIDS}.unexpected`)).toBe(false)
    assertRetired(f.pids())
  } finally {
    f.close()
  }
})

test('Python nonzero completion remains a failure with a fixed position', async () => {
  const f = await fixture()
  try {
    const owner = Bun.spawn([python, 'sdk/python/e2e/agent_evidence.py', target, '1', '1', '0.1'], {
      env: { ...f.env, FIXTURE_MODE: 'failure' },
      stdout: 'pipe',
      stderr: 'pipe',
    })
    expect(await owner.exited).toBe(7)
    expect(await new Response(owner.stdout).text()).toBe('')
    expect(await new Response(owner.stderr).text()).toMatch(/Python agent failed: agent.py:[0-9]+;/)
  } finally {
    f.close()
  }
})
