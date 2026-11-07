// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Node requests for the small, cached-image fleet fixture, bounded before Bun's test deadline. */
import { SECOND_MS } from '../src/durations.ts'
import { type CallOptions, type ClientIdentity, type NodeAddress, NodeClient } from '../src/node-client.ts'

export class FleetClient extends NodeClient {
  constructor(
    identity: ClientIdentity,
    deployment: string,
    readonly callTimeoutMs = 30 * SECOND_MS,
  ) {
    super(identity, deployment)
  }

  override call<T>(node: NodeAddress, method: string, path: string, options: CallOptions = {}): Promise<T> {
    return super.call(node, method, path, {
      ...options,
      timeoutMs: Math.min(options.timeoutMs ?? this.callTimeoutMs, this.callTimeoutMs),
    })
  }
}
