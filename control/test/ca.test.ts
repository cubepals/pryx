// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/** ca test behavior and its local regression coverage. */

import 'reflect-metadata'
import { describe, expect, test } from 'bun:test'
import { webcrypto } from 'node:crypto'
import * as x509 from '@peculiar/x509'
import { controlPlaneName, endpointName, FleetCa, isClientCertFor, nodeName, pemSha256 } from '../src/ca.ts'

const ALG = { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' }

async function csr(): Promise<string> {
  const keys = (await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])) as CryptoKeyPair
  const request = await x509.Pkcs10CertificateRequestGenerator.create({
    name: 'CN=whatever-the-node-says',
    keys,
    signingAlgorithm: ALG,
  })
  return request.toString('pem')
}

async function freshCa(deployment = 'test'): Promise<FleetCa> {
  const { certPem, keyPem } = await FleetCa.generate(deployment)
  return FleetCa.fromPem(certPem, keyPem)
}

describe('the fleet CA', () => {
  test('issues a node two certificates for its own key, under a name the control plane chooses', async () => {
    const ca = await freshCa()
    const issued = await ca.issueForNode(await csr(), 'node-1', 'test', 30)
    const server = new x509.X509Certificate(issued.server.certPem)
    const client = new x509.X509Certificate(issued.client.certPem)
    expect(server.subject).toBe(`CN=${nodeName('node-1', 'test')}`)
    expect(
      await server.verify({
        publicKey: ca.cert.publicKey,
        signatureOnly: true,
      }),
    ).toBe(true)
    expect(server.publicKey.rawData).toEqual(client.publicKey.rawData)
    expect(isClientCertFor(Buffer.from(client.rawData), nodeName('node-1', 'test'))).toBe(true)
    expect(isClientCertFor(Buffer.from(server.rawData), nodeName('node-1', 'test'))).toBe(false)
    expect(isClientCertFor(Buffer.from(client.rawData), nodeName('node-2', 'test'))).toBe(false)
    expect(issued.client.sha256).toBe(pemSha256(issued.client.certPem))
  })

  test('refuses a request not signed by its own key', async () => {
    const ca = await freshCa()
    const good = await csr()
    // The request's last byte, inside its signature, flipped.
    const bytes = Buffer.from(good.replace(/-----[^-]+-----/g, '').replace(/\s+/g, ''), 'base64')
    bytes[bytes.length - 1] = (bytes[bytes.length - 1] ?? 0) ^ 0x01
    const tampered = `-----BEGIN CERTIFICATE REQUEST-----\n${bytes.toString('base64')}\n-----END CERTIFICATE REQUEST-----\n`
    await expect(ca.issueForNode(tampered, 'n', 'test', 30)).rejects.toThrow()
  })

  test('comes back the same from the PEMs configuration holds, and refuses a key that is not its own', async () => {
    const { certPem, keyPem } = await FleetCa.generate('test')
    const ca = await FleetCa.fromPem(certPem, keyPem)
    const id = await ca.identity(controlPlaneName('test'), 'client', 1)
    const parsed = await FleetCa.fromPem(certPem, keyPem)
    expect(
      await new x509.X509Certificate(id.certPem).verify({
        publicKey: parsed.cert.publicKey,
        signatureOnly: true,
      }),
    ).toBe(true)
    const other = await FleetCa.generate('other')
    await expect(FleetCa.fromPem(certPem, other.keyPem)).rejects.toThrow('is not the key')
  })

  test('names the node endpoint by the addresses nodes dial it by', async () => {
    const ca = await freshCa()
    const endpoint = await ca.identity(endpointName('test'), 'server', 7, {
      ips: ['10.0.0.1'],
      dns: ['fleet.internal'],
    })
    const names = new x509.X509Certificate(endpoint.certPem).getExtension(
      x509.SubjectAlternativeNameExtension,
    )
    const values = names?.names.toJSON().map((n) => n.value)
    expect(values).toEqual([endpointName('test'), 'fleet.internal', '10.0.0.1'])
  })
})
