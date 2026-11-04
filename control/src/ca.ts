// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

// @peculiar/x509 needs a Reflect polyfill loaded before it, and since 2.0 leaves the choice to us.
import 'reflect-metadata'
import { createHash, X509Certificate as NodeX509, webcrypto } from 'node:crypto'
import { DAY_MS, MINUTE_MS } from '@blockly/contracts/durations'
import * as x509 from '@peculiar/x509'

/**
 * The fleet's certificate authority (docs/fleet.md, "Trust"). It issues:
 * - each node a serverAuth certificate (for its API) and a clientAuth one (for its heartbeats),
 *   both for the key the node made itself and never sent anywhere;
 * - the control plane its client identity (to call nodes) and the node endpoint's server one.
 *
 * Its key comes from configuration, which comes from the deployment's secret store; it is never
 * written to the database or to disk by the control plane. @peculiar/x509 does the X.509 work, as
 * it already does for the realtime certificate: no openssl binary, nothing shelled out.
 */

x509.cryptoProvider.set(webcrypto as unknown as Crypto)

const ALG = { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' }

export type Usage = 'server' | 'client'

export interface Issued {
  certPem: string
  /** sha256 of the DER certificate, hex: what the control plane pins per node. */
  sha256: string
  expiresAt: Date
}

export interface Identity extends Issued {
  keyPem: string
}

/** The name a node's certificates carry. The control plane decides it, never the node. */
export const nodeName = (nodeId: string, deployment: string) => `${nodeId}.nodes.${deployment}.fleet`
/** The name nodes allow to call their API. */
export const controlPlaneName = (deployment: string) => `control-plane.${deployment}.fleet`
/** The name the node endpoint's certificate carries, which nodes check. */
export const endpointName = (deployment: string) => `endpoint.${deployment}.fleet`

export function pemSha256(pem: string): string {
  return createHash('sha256').update(new NodeX509(pem).raw).digest('hex')
}

/**
 * sha256 of the key a certificate request is for: its SubjectPublicKeyInfo, DER. Every request a
 * node signs with one key has the same, whatever else differs between them.
 */
export function csrKeySha256(csrPem: string): string {
  const csr = new x509.Pkcs10CertificateRequest(csrPem)
  return createHash('sha256').update(Buffer.from(csr.publicKey.rawData)).digest('hex')
}

function serial(): string {
  const bytes = webcrypto.getRandomValues(new Uint8Array(16))
  bytes[0] = (bytes[0] ?? 0) & 0x7f
  return Buffer.from(bytes).toString('hex')
}

function pem(label: string, der: ArrayBuffer): string {
  const body = Buffer.from(der).toString('base64').replace(/.{64}/g, '$&\n')
  return `-----BEGIN ${label}-----\n${body}${body.endsWith('\n') ? '' : '\n'}-----END ${label}-----\n`
}

function der(pemText: string): ArrayBuffer {
  const body = pemText.replace(/-----[^-]+-----/g, '').replace(/\s+/g, '')
  const bytes = Buffer.from(body, 'base64')
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
}

export class FleetCa {
  readonly cert: x509.X509Certificate
  readonly pem: string
  readonly #key: CryptoKey

  private constructor(cert: x509.X509Certificate, key: CryptoKey) {
    this.cert = cert
    this.pem = cert.toString('pem')
    this.#key = key
  }

  /** The CA as configuration holds it: its certificate, and its PKCS#8 key. */
  static async fromPem(certPem: string, keyPem: string): Promise<FleetCa> {
    const cert = new x509.X509Certificate(certPem)
    const key = await webcrypto.subtle.importKey('pkcs8', der(keyPem), ALG, false, ['sign'])
    const ca = new FleetCa(cert, key)
    // A key that doesn't match the certificate would issue certificates nothing trusts.
    const probe = await ca.identity('probe', 'client', 1)
    if (!(await new x509.X509Certificate(probe.certPem).verify({ publicKey: cert.publicKey })))
      throw new Error("FLEET_CA_KEY is not the key of FLEET_CA_CERT's certificate")
    return ca
  }

  /** A new CA, for an operator to keep in the deployment's secrets (`scripts/fleet.ts ca`). */
  static async generate(deployment: string, years = 10): Promise<{ certPem: string; keyPem: string }> {
    const keys = (await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])) as CryptoKeyPair
    const now = Date.now()
    const cert = await x509.X509CertificateGenerator.createSelfSigned({
      serialNumber: serial(),
      name: `CN=Blockly fleet CA (${deployment})`,
      notBefore: new Date(now - MINUTE_MS),
      notAfter: new Date(now + years * 365 * DAY_MS),
      signingAlgorithm: ALG,
      keys,
      extensions: [
        new x509.BasicConstraintsExtension(true, 0, true),
        new x509.KeyUsagesExtension(x509.KeyUsageFlags.keyCertSign | x509.KeyUsageFlags.cRLSign, true),
        await x509.SubjectKeyIdentifierExtension.create(keys.publicKey),
      ],
    })
    return {
      certPem: cert.toString('pem'),
      keyPem: pem('PRIVATE KEY', await webcrypto.subtle.exportKey('pkcs8', keys.privateKey)),
    }
  }

  /** A certificate for `publicKey`, for one name and usage. */
  async issue(
    publicKey: x509.PublicKeyType,
    name: string,
    usage: Usage,
    days: number,
    extra: { ips?: readonly string[]; dns?: readonly string[] } = {},
  ): Promise<Issued> {
    const now = Date.now()
    const expiresAt = new Date(now + days * DAY_MS)
    const cert = await x509.X509CertificateGenerator.create({
      serialNumber: serial(),
      subject: `CN=${name}`,
      issuer: this.cert.subject,
      notBefore: new Date(now - MINUTE_MS),
      notAfter: expiresAt,
      signingAlgorithm: ALG,
      publicKey,
      signingKey: this.#key,
      extensions: [
        new x509.BasicConstraintsExtension(false, undefined, true),
        new x509.KeyUsagesExtension(x509.KeyUsageFlags.digitalSignature, true),
        new x509.ExtendedKeyUsageExtension(
          [usage === 'server' ? x509.ExtendedKeyUsage.serverAuth : x509.ExtendedKeyUsage.clientAuth],
          false,
        ),
        new x509.SubjectAlternativeNameExtension([
          { type: 'dns', value: name },
          ...(extra.dns ?? []).map((value) => ({ type: 'dns' as const, value })),
          ...(extra.ips ?? []).map((value) => ({ type: 'ip' as const, value })),
        ]),
        await x509.AuthorityKeyIdentifierExtension.create(this.cert),
      ],
    })
    const certPem = cert.toString('pem')
    return { certPem, sha256: pemSha256(certPem), expiresAt }
  }

  /**
   * A node's two certificates, for the key in its CSR. The CSR's signature proves the node holds
   * the key; its subject and names are ignored, since the control plane names nodes.
   */
  async issueForNode(csrPem: string, nodeId: string, deployment: string, days: number) {
    const csr = new x509.Pkcs10CertificateRequest(csrPem)
    if (!(await csr.verify())) throw new Error('the certificate request is not signed by its own key')
    const name = nodeName(nodeId, deployment)
    return {
      server: await this.issue(csr.publicKey, name, 'server', days),
      client: await this.issue(csr.publicKey, name, 'client', days),
    }
  }

  /** A new key and certificate, for the control plane's own identities. */
  async identity(
    name: string,
    usage: Usage,
    days: number,
    extra: { ips?: readonly string[]; dns?: readonly string[] } = {},
  ): Promise<Identity> {
    const keys = (await webcrypto.subtle.generateKey(ALG, true, ['sign', 'verify'])) as CryptoKeyPair
    const issued = await this.issue(keys.publicKey, name, usage, days, extra)
    return {
      ...issued,
      keyPem: pem('PRIVATE KEY', await webcrypto.subtle.exportKey('pkcs8', keys.privateKey)),
    }
  }
}

/** Whether a peer certificate is a clientAuth certificate for exactly `name`. */
export function isClientCertFor(certDer: Buffer, name: string): boolean {
  const cert = new NodeX509(certDer)
  const names = (cert.subjectAltName ?? '').split(',').map((s) => s.trim())
  const eku = cert.keyUsage ?? []
  return names.includes(`DNS:${name}`) && eku.includes(x509.ExtendedKeyUsage.clientAuth)
}
