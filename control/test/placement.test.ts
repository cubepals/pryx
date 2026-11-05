// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** A node; unless `over` says otherwise, every server placed on it runs. */

import { describe, expect, test } from 'bun:test'
import {
  cpuRequest,
  DEFAULTS,
  type NodeView,
  type PlacementRequest,
  selectNode,
  startRefusal,
} from '../src/placement.ts'

function node(
  id: string,
  over: Partial<NodeView> = {},
  allocated: Partial<NodeView['allocated']> = {},
): NodeView {
  const placed = {
    memoryMb: 0,
    cpuMillis: 0,
    diskGb: 0,
    workloads: 0,
    ...allocated,
  }
  return {
    id,
    name: id,
    regionKey: 'eu',
    lifecycle: 'active',
    health: 'healthy',
    quarantined: false,
    features: ['placement-epochs', 'data-transfer', 'local-snapshots'],
    capacity: {
      allocatableMemoryMb: 16_384,
      cpus: 8,
      reservedCpuMillis: 1000,
      diskTotalBytes: 400 * 1024 ** 3,
      diskAvailableBytes: 200 * 1024 ** 3,
      minFreeDiskMb: 2048,
      portsTotal: 100,
      portsAllocated: 0,
    },
    allocated: placed,
    running: {
      memoryMb: placed.memoryMb,
      cpuMillis: placed.cpuMillis,
      workloads: placed.workloads,
    },
    ...over,
  }
}

const asleep = { memoryMb: 0, cpuMillis: 0, workloads: 0 }

const request = (over: Partial<PlacementRequest> = {}): PlacementRequest => ({
  memoryMb: 3072,
  cpuMillis: 1000,
  diskGb: 10,
  ports: 2,
  regionKey: 'eu',
  needs: ['placement-epochs'],
  ...over,
})

describe('selectNode', () => {
  test('process and container requests cannot cross isolation tiers', () => {
    const process = node('process', { features: ['placement-epochs', 'process-tier'] })
    expect(selectNode(request(), [process]).node).toBeNull()
    expect(selectNode(request({ isolation: 'process' }), [node('container')]).node).toBeNull()
    expect(selectNode(request({ isolation: 'process' }), [process]).node?.id).toBe('process')
  })
  test('wasm placement requires its tier and requested package feature', () => {
    const wasm = node('wasm', { features: ['placement-epochs', 'wasm-tier', 'wasm-packages'] })
    const process = node('process', { features: ['placement-epochs', 'process-tier'] })
    const container = node('container')
    for (const isolation of ['container', 'process', 'wasm'] as const) {
      const chosen = selectNode(request({ isolation }), [wasm, process, container]).node
      expect(chosen?.id).toBe(isolation)
    }
    const old = node('wasm-old', { features: ['placement-epochs', 'wasm-tier'] })
    expect(selectNode(request({ isolation: 'wasm', needs: ['wasm-packages'] }), [old]).node).toBeNull()
    expect(selectNode(request({ isolation: 'wasm', needs: ['wasm-packages'] }), [wasm]).node?.id).toBe('wasm')
  })
  test('only an active, healthy node in the region with the features can be chosen', () => {
    const nodes = [
      node('a', { lifecycle: 'draining' }),
      node('b', { health: 'suspect' }),
      node('c', { regionKey: 'us' }),
      node('d', { features: [] }),
      node('e', { quarantined: true }),
      node('f', { lifecycle: 'lost', health: 'unavailable' }),
    ]
    const decision = selectNode(request(), nodes)
    expect(decision.node).toBeNull()
    expect(decision.considered.map((c) => c.reason)).toEqual([
      'lifecycle draining',
      'health suspect',
      'region us',
      'lacks placement-epochs',
      'quarantined: duplicate identity',
      'lifecycle lost',
    ])
    expect(selectNode(request(), [...nodes, node('g')]).node?.id).toBe('g')
  })

  test('counts the ledger, not what the node says it uses, and keeps the headroom', () => {
    const full = node('a', {}, { memoryMb: 14_000 })
    expect(selectNode(request(), [full]).node).toBeNull()
    const roomy = node('b', {}, { memoryMb: 10_000 })
    expect(selectNode(request(), [roomy]).node?.id).toBe('b')
    // 16384 − 10000 − 3072 headroom = 3312 ≥ 3072, but not with a bigger headroom.
    expect(selectNode(request(), [roomy], { ...DEFAULTS, headroomMb: 3400 }).node).toBeNull()
  })

  test('refuses on CPU even when memory fits', () => {
    // 8 cores − 1 reserved = 7000m; 6500m promised leaves 500m.
    const busy = node('a', {}, { memoryMb: 0, cpuMillis: 6500 })
    const decision = selectNode(request(), [busy])
    expect(decision.node).toBeNull()
    expect(decision.considered[0]?.reason).toStartWith('cpu:')
    expect(selectNode(request(), [busy], { ...DEFAULTS, cpuOvercommit: 1.5 }).node?.id).toBe('a')
  })

  test('refuses on disk, and on ports', () => {
    const disk = node('a', {
      capacity: { ...node('x').capacity, diskAvailableBytes: 5 * 1024 ** 3 },
    })
    expect(selectNode(request(), [disk]).considered[0]?.reason).toStartWith('disk:')
    // Room on the disk today, but its data sets were promised all of it.
    const promised = node('c', {}, { diskGb: 395 })
    expect(selectNode(request(), [promised]).considered[0]?.reason).toStartWith('disk: ')
    expect(selectNode(request(), [promised], { ...DEFAULTS, diskOvercommit: 2 }).node?.id).toBe('c')
    const ports = node('b', {
      capacity: { ...node('x').capacity, portsAllocated: 99 },
    })
    expect(selectNode(request(), [ports]).considered[0]?.reason).toStartWith('ports:')
  })

  test('binpack fills the fullest node that fits; spread the emptiest', () => {
    const nodes = [node('a', {}, { memoryMb: 9000 }), node('b', {}, { memoryMb: 2000 }), node('c')]
    expect(selectNode(request(), nodes, { ...DEFAULTS, policy: 'binpack' }).node?.id).toBe('a')
    expect(selectNode(request(), nodes, { ...DEFAULTS, policy: 'spread' }).node?.id).toBe('c')
  })

  test('balanced packs memory, but not onto a node past the CPU pressure line', () => {
    // Holding memory for every placed server: a is fullest on memory, but 5000m + 1000m of 7000m
    // is 86% of its CPU; b is roomier and calm.
    const held = { ...DEFAULTS, memoryOvercommit: 1 }
    const nodes = [
      node('a', {}, { memoryMb: 9000, cpuMillis: 5000 }),
      node('b', {}, { memoryMb: 4000 }),
      node('c'),
    ]
    expect(selectNode(request(), nodes, held).node?.id).toBe('b')
    // With no calm node left, the least pressed one.
    const hot = [node('a', {}, { cpuMillis: 5500 }), node('b', {}, { cpuMillis: 5000 })]
    expect(selectNode(request(), hot, held).node?.id).toBe('b')
    // Placed CPU counts against what may be placed: with a 4× overcommit, a's 6000m is 21%.
    expect(selectNode(request(), nodes).node?.id).toBe('a')
  })

  test('a sleeping server holds no memory; a running one does', () => {
    // 14 000 MB placed on a 16 384 MB node, all of it asleep: room to run, and to place.
    const sleeping = node('a', { running: asleep }, { memoryMb: 14_000 })
    expect(selectNode(request(), [sleeping]).node?.id).toBe('a')
    // Holding memory for every placed server, as the fleet did, it is full.
    expect(
      selectNode(request(), [sleeping], { ...DEFAULTS, memoryOvercommit: 1 }).considered[0]?.reason,
    ).toBe('memory: 2384 MB left to place sandboxes in, needs 3072')
    // The same sandboxes running leave no room to start another, however few are placed.
    const busy = node(
      'b',
      { running: { memoryMb: 14_000, cpuMillis: 0, workloads: 4 } },
      { memoryMb: 14_000 },
    )
    expect(selectNode(request(), [busy]).considered[0]?.reason).toBe(
      'memory: 2384 MB free after headroom, needs 3072',
    )
    // What may be placed is capped all the same: 4 × 16 384 MB.
    const crowded = node('c', { running: asleep }, { memoryMb: 63_000 })
    expect(selectNode(request(), [crowded]).considered[0]?.reason).toBe(
      'memory: 2536 MB left to place sandboxes in, needs 3072',
    )
  })

  test('placement scores what is placed, not what happens to run as it is asked', () => {
    // a: more placed, nothing running now; b: less placed, more running now.
    const nodes = [
      node('a', { running: asleep }, { memoryMb: 30_000, cpuMillis: 7000 }),
      node(
        'b',
        { running: { memoryMb: 10_000, cpuMillis: 2500, workloads: 3 } },
        { memoryMb: 12_000, cpuMillis: 3000 },
      ),
    ]
    expect(selectNode(request(), nodes, { ...DEFAULTS, policy: 'binpack' }).node?.id).toBe('a')
    expect(selectNode(request(), nodes, { ...DEFAULTS, policy: 'spread' }).node?.id).toBe('b')
  })

  test('ties break on the node id, so the same inputs give the same answer', () => {
    expect(selectNode(request(), [node('b'), node('a')]).node?.id).toBe('a')
    expect(selectNode(request(), [node('a'), node('b')]).node?.id).toBe('a')
  })

  test('a move never picks its own source', () => {
    expect(selectNode(request({ avoid: ['a'] }), [node('a'), node('b')]).node?.id).toBe('b')
  })

  test('an operator can name the one node a move goes to, and its refusal says so', () => {
    const decision = selectNode(request({ only: 'c' }), [
      node('a'),
      node('b'),
      node('c', { lifecycle: 'draining' }),
    ])
    expect(decision.node).toBeNull()
    expect(decision.considered.map((c) => c.reason)).toEqual([
      'not the node asked for',
      'not the node asked for',
      'lifecycle draining',
    ])
    expect(selectNode(request({ only: 'b' }), [node('a'), node('b')]).node?.id).toBe('b')
  })

  test('one node is a whole fleet: every server that fits goes there', () => {
    const one = node('only', {
      capacity: {
        ...node('x').capacity,
        allocatableMemoryMb: 60_000,
        cpus: 16,
        diskTotalBytes: 2000 * 1024 ** 3,
        diskAvailableBytes: 1500 * 1024 ** 3,
      },
    })
    const each = { memoryMb: 3072, cpuMillis: cpuRequest(3072, 250) }
    /** Servers placed one by one until the node refuses: all running, or all asleep. */
    const fill = (running: boolean, options = DEFAULTS) => {
      let allocated = { memoryMb: 0, cpuMillis: 0, diskGb: 0, workloads: 0 }
      let runs = asleep
      for (;;) {
        if (
          selectNode(request({ cpuMillis: each.cpuMillis }), [{ ...one, allocated, running: runs }], options)
            .node === null
        )
          return allocated.workloads
        allocated = {
          memoryMb: allocated.memoryMb + each.memoryMb,
          cpuMillis: allocated.cpuMillis + each.cpuMillis,
          diskGb: allocated.diskGb + 10,
          workloads: allocated.workloads + 1,
        }
        if (running)
          runs = {
            memoryMb: allocated.memoryMb,
            cpuMillis: allocated.cpuMillis,
            workloads: allocated.workloads,
          }
      }
    }
    // Running: 60 000 MB holds 19 sandboxes of 3 GB; 15 000m of CPU holds 19 at 768m each.
    expect(fill(true)).toBe(19)
    // Asleep: 4 × 60 000 MB holds 78, which their disks fit.
    expect(fill(false)).toBe(78)
    // Holding memory for every placed server, as the fleet did, 19 again.
    expect(fill(false, { ...DEFAULTS, memoryOvercommit: 1 })).toBe(19)
  })

  test('a placed server starts where it is when the sandboxes running there leave it room', () => {
    const start = { memoryMb: 3072, cpuMillis: 768 }
    // 16 384 − 13 000 = 3384 MB free: room, and no headroom kept for a start.
    const roomy = node('a', {
      running: { memoryMb: 13_000, cpuMillis: 3000, workloads: 4 },
    })
    expect(startRefusal(roomy, start, DEFAULTS)).toBeNull()
    const full = node('b', {
      running: { memoryMb: 14_000, cpuMillis: 3000, workloads: 4 },
    })
    expect(startRefusal(full, start, DEFAULTS)).toBe(
      'memory: 2384 MB free beside the sandboxes running there, needs 3072',
    )
    // A retried start holds its share already.
    expect(startRefusal(full, start, DEFAULTS, true)).toBeNull()
    const hot = node('c', {
      running: { memoryMb: 0, cpuMillis: 6500, workloads: 8 },
    })
    expect(startRefusal(hot, start, DEFAULTS)).toStartWith('cpu:')
  })

  test('a CPU request follows memory when the spec names none', () => {
    expect(cpuRequest(3072, 333)).toBe(999)
    expect(cpuRequest(1024, 500)).toBe(500)
  })
})
