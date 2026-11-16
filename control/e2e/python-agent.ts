// SPDX-License-Identifier: FSL-1.1-ALv2
/** Run the Python acceptance child with failure-only evidence and retire it before deployment.
 * The stdlib owner keeps its child PID reserved until the complete process group is killed. */
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'

export const PYTHON_AGENT_BUDGET_MS = 4 * MINUTE_MS
// Deployment stop already permits ten seconds; reserve a further five for retirement/evidence.
const CLEANUP_RESERVE_MS = 15 * SECOND_MS
const RETIRE_MS = SECOND_MS
const CAPTURE_MS = SECOND_MS / 10

export async function pythonAgent(options: {
  env: Record<string, string | undefined>
  deadline: number
  close: () => Promise<void>
  python?: string
  target?: string
}) {
  let child: ReturnType<typeof Bun.spawn> | undefined
  let cancelled: NodeJS.Signals | undefined
  const handlers = new Map<NodeJS.Signals, () => void>()
  for (const value of ['SIGTERM', 'SIGINT'] as const) {
    const handler = () => {
      cancelled ??= value
      if (child && child.exitCode === null) child.kill(value)
    }
    handlers.set(value, handler)
    process.on(value, handler)
  }
  try {
    const remaining = options.deadline - performance.now() - CLEANUP_RESERVE_MS
    if (remaining <= 0) throw new Error('Python agent has no remaining acceptance budget')
    child = Bun.spawn(
      [
        options.python ?? process.env.PRYX_PYTHON ?? 'python3',
        'sdk/python/e2e/agent_evidence.py',
        options.target ?? 'sdk/python/e2e/agent.py',
        String(remaining / SECOND_MS),
        String(RETIRE_MS / SECOND_MS),
        String(CAPTURE_MS / SECOND_MS),
      ],
      { env: options.env, stdout: 'inherit', stderr: 'inherit' },
    )
    return await child.exited
  } finally {
    try {
      if (child && child.exitCode === null) {
        child.kill('SIGTERM')
        await child.exited
      }
      await options.close()
    } finally {
      for (const [value, handler] of handlers) process.off(value, handler)
      // Match the caller's original cancellation only after child and deployment cleanup.
      if (cancelled) process.kill(process.pid, cancelled)
    }
  }
}
