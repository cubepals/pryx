// SPDX-License-Identifier: FSL-1.1-ALv2

/** Cover the installer boundary for production hostnames and the separate loopback test trust. */
import { expect, test } from 'bun:test'
import { caddyConfig, dnsDomain, pebbleOptions } from '../src/up-acme-config.ts'

test('server names use unambiguous DNS hostnames', () => {
  expect(dnsDomain('Sandboxes.Example.com')).toBe('sandboxes.example.com')
  expect(dnsDomain('localhost')).toBe('localhost')
  for (const invalid of [
    'example',
    'example.com:443',
    'a..com',
    'a-.com',
    '*.example.com',
    '127.0.0.1',
    'a.1',
  ])
    expect(() => dnsDomain(invalid)).toThrow('--domain is a DNS hostname')
})

test('the ACME fixture refuses partial settings and non-loopback directories before reading a CA', () => {
  expect(() => pebbleOptions({ PRYX_ACME_TEST_DIRECTORY: 'https://127.0.0.1/dir' })).toThrow(
    'requires a directory and both public CA files',
  )
  for (const invalid of ['https://example.com/dir', 'http://127.0.0.1/dir', 'https://localhost/dir']) {
    expect(() =>
      pebbleOptions({
        PRYX_ACME_TEST_DIRECTORY: invalid,
        PRYX_ACME_TEST_CA: '/not-opened',
        PRYX_ACME_TEST_ROOT: '/not-opened',
      }),
    ).toThrow('must be a loopback HTTPS URL')
  }
})

test('the test issuer and its fallback both use Pebble, with production renewal settings intact', () => {
  const domain = 'api.example.com'
  const directory = 'https://127.0.0.1:14000/dir'
  const production = caddyConfig(domain)
  const fixture = caddyConfig(domain, { directory, issuer: '', root: '' })
  expect(production.apps.tls.automation).not.toHaveProperty('renew_interval')
  expect(production.apps.tls.automation.policies[0]?.issuers[0]).not.toHaveProperty('ca')
  expect(fixture.apps.tls.automation.policies[0]?.issuers[0]).toMatchObject({
    ca: directory,
    test_ca: directory,
  })
  expect(production.apps.http.servers.api.routes[0]?.match).toEqual([{ host: [domain] }])
})
