// SPDX-License-Identifier: FSL-1.1-ALv2
/** Real archive and signature failures must precede extraction or activation. These tests
 * do not substitute for the fresh-user and booted-distribution installer acceptance. */
import { expect, test } from 'bun:test'
import { generateKeyPairSync, sign } from 'node:crypto'
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  bundleStatement,
  HOST_BUNDLE,
  HOST_MACHINE,
  releaseAsset,
  releasedFor,
} from '../../control/src/bundle-release.ts'
import { PRYX_VERSION } from '../../control/src/generated/release.ts'
import { bundleFiles, hashFile } from '../src/bundle-download.ts'
import { stageBundle } from '../src/bundle-install.ts'
import { verifyBundleSignature } from '../src/bundle-trust.ts'
import { userPaths } from '../src/up-paths.ts'
import { selectPreset } from '../src/up-preset.ts'

async function fixture(architecture = HOST_MACHINE) {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-bundle-'))
  const home = join(directory, 'home'),
    source = join(directory, 'release')
  mkdirSync(home, { mode: 0o700 })
  mkdirSync(source)
  const keys = generateKeyPairSync('ed25519')
  const trustedKeys = keys.publicKey.export({ type: 'spki', format: 'pem' }).toString()
  const archive = join(source, HOST_BUNDLE)
  const resign = async () => {
    const digest = await hashFile(archive)
    writeFileSync(join(source, 'SHA256SUMS'), `${digest}  ${HOST_BUNDLE}\n`)
    writeFileSync(
      join(source, `${HOST_BUNDLE}.sig`),
      sign(null, Buffer.from(bundleStatement(PRYX_VERSION, digest)), keys.privateKey),
    )
    return digest
  }
  await Bun.Archive.write(
    archive,
    {
      'bundle.json': JSON.stringify({
        version: PRYX_VERSION,
        architecture,
        serviceUid: 1001,
        serviceGid: 1001,
        postgres: { major: '17', version: '17.11.0' },
      }),
      'bin/sentinel': 'must never execute during verification',
      'bin/vm-bundle.json': '{}',
    },
    { compress: 'gzip' },
  )
  const digest = await resign()
  return {
    directory,
    source,
    archive,
    trustedKeys,
    resign,
    digest,
    paths: userPaths({}, home),
    close: () => rmSync(directory, { recursive: true, force: true }),
  }
}

test('a real signed archive stages without activating or writing state', async () => {
  const f = await fixture()
  try {
    const staged = await stageBundle(f.paths, { source: f.source, trustedKeys: f.trustedKeys })
    expect(staged.digest).toBe(f.digest)
    expect(existsSync(join(staged.unpacked, 'bin/sentinel'))).toBe(true)
    expect(existsSync(f.paths.config)).toBe(false)
    expect(existsSync(f.paths.state)).toBe(false)
    expect(existsSync(f.paths.data)).toBe(false)
    rmSync(staged.temporary, { recursive: true })
  } finally {
    f.close()
  }
})

test("a signed bundle built for another machine is refused, and each machine's files are named", async () => {
  const f = await fixture(HOST_MACHINE === 'aarch64' ? 'x86_64' : 'aarch64')
  try {
    await expect(stageBundle(f.paths, { source: f.source, trustedKeys: f.trustedKeys })).rejects.toThrow(
      'Host bundle metadata does not match this release',
    )
  } finally {
    f.close()
  }
  expect(HOST_BUNDLE).toBe(`pryx-bundle-linux-${process.arch}.tar.gz`)
  expect(releaseAsset('pryxd', 'x64')).toBe('pryxd')
  expect(releaseAsset('pryxd.bootstrap.sig', 'x64')).toBe('pryxd.bootstrap.sig')
  expect(releaseAsset('pryxd', 'arm64')).toBe('pryxd-linux-arm64')
  expect(releaseAsset('pryxd.bootstrap.sig', 'arm64')).toBe('pryxd-linux-arm64.bootstrap.sig')
  expect(releaseAsset('pryx-control-assets.tar.gz', 'arm64')).toBe('pryx-control-assets-linux-arm64.tar.gz')
  expect(['x86_64', 'aarch64', 'riscv64'].map(releasedFor)).toEqual([true, true, false])
})

test('signed VM pins keep their readonly mode under a private installer umask', async () => {
  const f = await fixture()
  try {
    const script = `
      import {statSync,rmSync} from 'node:fs';
      import {join} from 'node:path';
      const {stageBundle}=await import(process.argv[1]);
      process.umask(0o077);
      const {paths,source,trustedKeys}=JSON.parse(process.argv[2]);
      const staged=await stageBundle(paths,{source,trustedKeys});
      console.log(statSync(join(staged.unpacked,'bin/vm-bundle.json')).mode & 0o777);
      rmSync(staged.temporary,{recursive:true});
    `
    const child = Bun.spawn(
      [
        process.execPath,
        '-e',
        script,
        join(import.meta.dir, '../src/bundle-install.ts'),
        JSON.stringify({ paths: f.paths, source: f.source, trustedKeys: f.trustedKeys }),
      ],
      { stdout: 'pipe', stderr: 'pipe' },
    )
    const [status, output, error] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])
    expect(error).toBe('')
    expect(status).toBe(0)
    expect(Number(output.trim())).toBe(0o444)
  } finally {
    f.close()
  }
})

test('matching attacker checksums cannot authenticate a changed archive', async () => {
  const f = await fixture()
  try {
    await Bun.Archive.write(f.archive, { sentinel: 'changed' }, { compress: 'gzip' })
    const digest = await hashFile(f.archive)
    writeFileSync(join(f.source, 'SHA256SUMS'), `${digest}  ${HOST_BUNDLE}\n`)
    await expect(stageBundle(f.paths, { source: f.source, trustedKeys: f.trustedKeys })).rejects.toThrow(
      'trusted release key',
    )
    expect(readdirSync(f.paths.cache)).toEqual([])
    expect(existsSync(f.paths.data)).toBe(false)
    const signature = sign(
      null,
      Buffer.from(bundleStatement('999.0.0', digest)),
      generateKeyPairSync('ed25519').privateKey,
    )
    expect(() => verifyBundleSignature(PRYX_VERSION, digest, signature, f.trustedKeys)).toThrow(
      'trusted release key',
    )
  } finally {
    f.close()
  }
})

test('even a signed archive cannot introduce a link outside its staging tree', async () => {
  const f = await fixture()
  try {
    const contents = join(f.directory, 'contents')
    mkdirSync(contents)
    symlinkSync(f.paths.home, join(contents, 'escape'))
    const packed = Bun.spawnSync(['tar', '-czf', f.archive, '-C', contents, '.'])
    expect(packed.exitCode).toBe(0)
    await f.resign()
    await expect(stageBundle(f.paths, { source: f.source, trustedKeys: f.trustedKeys })).rejects.toThrow(
      'links and special modes',
    )
    expect(readdirSync(f.paths.cache)).toEqual([])
  } finally {
    f.close()
  }
})

test('release transport refuses insecure external origins before fetching', async () => {
  const f = await fixture()
  try {
    await expect(bundleFiles(f.directory, undefined, 'http://example.invalid')).rejects.toThrow('HTTPS')
    expect(existsSync(join(f.directory, HOST_BUNDLE))).toBe(false)
  } finally {
    f.close()
  }
})

test('preset choice follows privilege and the host without questions', () => {
  expect(selectPreset(undefined, { uid: 1001, container: true, systemd: true })).toBe('user')
  expect(selectPreset(undefined, { uid: 0, container: true, systemd: true })).toBe('container')
  expect(selectPreset(undefined, { uid: 0, container: false, systemd: true })).toBe('server')
  expect(selectPreset('user', { uid: 1001, container: false, systemd: true })).toBe('user')
  expect(() => selectPreset(undefined, { uid: 0, container: false, systemd: false })).toThrow('ordinary user')
  expect(() => selectPreset('unknown')).toThrow('--preset')
})

test('XDG paths cannot redirect user installation beyond its home', () => {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-xdg-'))
  const home = join(directory, 'home'),
    outside = join(directory, 'outside')
  mkdirSync(home, { mode: 0o700 })
  mkdirSync(outside)
  try {
    expect(() => userPaths({ XDG_CONFIG_HOME: outside }, home)).toThrow('writes only below')
    expect(userPaths({ XDG_DATA_HOME: 'relative', XDG_RUNTIME_DIR: '/run/user/1001' }, home).data).toBe(
      join(home, '.local/share/pryx'),
    )
    expect(userPaths({ XDG_RUNTIME_DIR: '/run/user/1001' }, home).runtime).toStartWith(home)
    symlinkSync(outside, join(home, '.cache'))
    expect(() => userPaths({}, home)).toThrow('Managed path')
    expect(readdirSync(outside)).toEqual([])
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
