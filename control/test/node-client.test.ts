// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * The node client on its own: a streamed body read line by line, and calls to a node over mutual
 * TLS that doesn't answer at first, or ever, which are sent again only when that is safe.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { createServer, type Server } from 'node:net'
import { Readable } from 'node:stream'
import { createServer as createTlsServer } from 'node:tls'
import { StatusCodes } from 'http-status-codes'
import { controlPlaneName, nodeName } from '../src/ca.ts'
import { SECOND_MS } from '../src/durations.ts'
import { NodeClient, NodeRefused, NodeUnreachable, ndjsonLines, repeatable } from '../src/node-client.ts'
import { testCa } from './nodes.ts'

/** A body that arrives in these chunks of bytes, as reads from a socket hand it over. */
const body = (...chunks: Buffer[]) => Readable.from(chunks, { objectMode: false })

const read = async (lines: AsyncIterable<string>) => {
  const out: string[] = []
  for await (const line of lines) out.push(line)
  return out
}

const going = () => new AbortController().signal

describe('ndjsonLines', () => {
  test('a character whose bytes two chunks split arrives whole', async () => {
    for (const char of ['§', 'ë', '🙂']) {
      const line = JSON.stringify({
        stream: 'stdout',
        line: `${char}aAlex joined`,
      })
      const bytes = Buffer.from(`${line}\n`)
      const at = bytes.indexOf(Buffer.from(char)) + 1
      const split = body(bytes.subarray(0, at), bytes.subarray(at))
      expect(await read(ndjsonLines(split, going()))).toEqual([line])
    }
  })

  test('blank lines are skipped, and the last line needs no newline', async () => {
    const chunks = ['{"a":1}\n\n  \n{"b":', '2}\n{"c":3}'].map((text) => Buffer.from(text))
    expect(await read(ndjsonLines(body(...chunks), going()))).toEqual(['{"a":1}', '{"b":2}', '{"c":3}'])
  })

  test('a body that fails ends with its error, unless the request was called off', async () => {
    const failing = () => {
      const stream = new Readable({ read() {} })
      stream.push(Buffer.from('{"a":1}\n'))
      setTimeout(() => stream.destroy(new Error('connection reset')), 0.01 * SECOND_MS)
      return stream
    }
    await expect(read(ndjsonLines(failing(), going()))).rejects.toThrow('connection reset')
    const off = new AbortController()
    off.abort()
    expect(await read(ndjsonLines(failing(), off.signal))).toEqual(['{"a":1}'])
  })
})

/**
 * A node that drops the first `drops` connections before a word is said, as pryxd restarting
 * or a keep-alive connection it had closed does, and answers every later request with `status`.
 */
async function flakyNode(status: number): Promise<{
  client: NodeClient
  address: { id: string; apiAddress: string }
  drop: (n: number) => void
  connections: () => number
  close: () => void
}> {
  const deployment = 'retries'
  const ca = await testCa(deployment)
  const id = '7a0c1d5e-0000-4000-8000-000000000001'
  const server = await ca.identity(nodeName(id, deployment), 'server', 1)
  const own = await ca.identity(controlPlaneName(deployment), 'client', 1)
  const tls = createTlsServer({
    key: server.keyPem,
    cert: server.certPem,
    ca: ca.pem,
    requestCert: true,
    rejectUnauthorized: true,
  })
  tls.on('secureConnection', (socket) => {
    socket.once('data', () => {
      const body = status < StatusCodes.MULTIPLE_CHOICES ? '{"ok":true}' : ''
      socket.end(
        `HTTP/1.1 ${status} X\r\ncontent-type: application/json\r\ncontent-length: ${body.length}\r\nconnection: close\r\n\r\n${body}`,
      )
    })
  })
  let drops = 0
  let seen = 0
  const tcp: Server = createServer((socket) => {
    seen++
    if (drops > 0) {
      drops--
      socket.destroy()
      return
    }
    tls.emit('connection', socket)
  })
  await new Promise<void>((resolve) => tcp.listen(0, '127.0.0.1', resolve))
  const port = (tcp.address() as { port: number }).port
  const client = new NodeClient({ caPem: ca.pem, certPem: own.certPem, keyPem: own.keyPem }, deployment)
  return {
    client,
    address: { id, apiAddress: `127.0.0.1:${port}` },
    drop: (n) => {
      drops = n
      seen = 0
    },
    connections: () => seen,
    close: () => {
      client.close()
      tcp.close()
    },
  }
}

describe('a node that does not answer', () => {
  let node: Awaited<ReturnType<typeof flakyNode>>
  beforeAll(async () => {
    node = await flakyNode(200)
  })
  afterAll(() => node.close())

  test('reads, a PUT, a start and a stop may be sent again; nothing else is', () => {
    expect(repeatable('GET', '/v1/workloads/w')).toBe(true)
    expect(repeatable('PUT', '/v1/workloads/w')).toBe(true)
    expect(repeatable('POST', '/v1/workloads/w/start')).toBe(true)
    expect(repeatable('POST', '/v1/workloads/w/stop')).toBe(true)
    for (const verb of ['kill', 'fence', 'exec', 'restore', 'snapshots'])
      expect(repeatable('POST', `/v1/workloads/w/${verb}`)).toBe(false)
    expect(repeatable('DELETE', '/v1/workloads/w?data=keep')).toBe(false)
  })

  test('a call safe to repeat is sent again, and the answer that comes is the answer', async () => {
    node.drop(2)
    expect(
      await node.client.call<unknown>(node.address, 'POST', '/v1/workloads/w/stop', { epoch: 1 }),
    ).toEqual({
      ok: true,
    })
    expect(node.connections()).toBe(3)
  })

  test('one that is not safe to repeat is sent once', async () => {
    node.drop(1)
    await expect(
      node.client.call(node.address, 'POST', '/v1/workloads/w/kill', {
        epoch: 1,
      }),
    ).rejects.toBeInstanceOf(NodeUnreachable)
    expect(node.connections()).toBe(1)
  })

  test(
    'a node that never answers fails the call after its last attempt, as unreachable',
    async () => {
      node.drop(100)
      const error = await node.client.call(node.address, 'GET', '/v1/workloads/w').catch((e: unknown) => e)
      expect(error).toBeInstanceOf(NodeUnreachable)
      expect(node.connections()).toBe(4)
    },
    10 * SECOND_MS,
  )

  test("the call's timeout bounds its attempts", async () => {
    node.drop(100)
    const error = await node.client
      .call(node.address, 'GET', '/v1/workloads/w', { timeoutMs: SECOND_MS })
      .catch((e: unknown) => e)
    expect(error).toBeInstanceOf(NodeUnreachable)
    expect(node.connections()).toBe(2)
  })

  test('a refusal is an answer, and is not sent again', async () => {
    const refusing = await flakyNode(404)
    try {
      const error = await refusing.client
        .call(refusing.address, 'GET', '/v1/workloads/w')
        .catch((e: unknown) => e)
      expect(error).toBeInstanceOf(NodeRefused)
      expect((error as NodeRefused).status).toBe(StatusCodes.NOT_FOUND)
      expect(refusing.connections()).toBe(1)
    } finally {
      refusing.close()
    }
  })
})
