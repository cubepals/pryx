// SPDX-License-Identifier: FSL-1.1-ALv2
/** Explicit v1 node-wire probes for lifecycle acceptance. Production control uses the agent;
 * these probes keep assertions on the published node exec endpoint while that protocol is v1. */
import { SECOND_MS } from '../src/durations.ts'
import { decodeHandle } from '../src/handle.ts'
import type { NodeClient } from '../src/node-client.ts'
import type { FleetRuntime } from '../src/runtime-engine.ts'
import type { RuntimeHandle } from '../src/runtime-model.ts'
import type { ExecResponse } from '../src/wire.ts'
export async function nodeCommand(
  runtime: Pick<FleetRuntime, 'home'>,
  nodes: NodeClient,
  handle: RuntimeHandle,
  command: readonly string[],
  timeoutSeconds: number,
) {
  const ref = decodeHandle(handle),
    node = await runtime.home(handle)
  if (!node) throw new Error('Workload has no node for its wire probe')
  const result = await nodes.call<ExecResponse>(node, 'POST', `/v1/workloads/${ref.key}/exec`, {
    body: { command: [...command], timeoutSeconds },
    epoch: ref.epoch,
    timeoutMs: (timeoutSeconds + 15) * SECOND_MS,
  })
  if (result.timedOut) throw new Error('The command timed out')
  return { exitCode: result.exitCode ?? -1, stdout: result.stdout, stderr: result.stderr }
}
