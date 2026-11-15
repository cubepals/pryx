// SPDX-License-Identifier: FSL-1.1-ALv2
/** Temporary Git/filesystem fixtures cover released source and immutable static exports.
 * No external host is contacted and no repository release or remote ref is changed. */
import { expect, test } from 'bun:test'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileDigests, sourceGit } from '../../site/tools/artifact.ts'
import { assertRelease, publishSite } from '../../site/tools/publish.ts'
import { PRYX_VERSION } from '../src/generated/release.ts'

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), 'pryx-site-publish-'))
  const repository = join(directory, 'source')
  const artifact = join(directory, 'artifact')
  const webroot = join(directory, 'webroot')
  for (const path of [repository, artifact, webroot]) mkdirSync(path)
  const git = (...args: string[]) => sourceGit(repository, ...args)
  git('init', '--quiet')
  git('config', 'user.name', 'pryx test')
  git('config', 'user.email', 'test@example.test')
  git('config', 'commit.gpgsign', 'false')
  git('config', 'core.hooksPath', '/dev/null')
  writeFileSync(join(repository, 'source'), 'fixture')
  git('add', 'source')
  git('commit', '--quiet', '-m', 'Fixture')
  // This tag belongs only to the disposable fixture, never the checkout or a remote.
  git('tag', `v${PRYX_VERSION}`)
  writeFileSync(join(artifact, 'index.html'), '<h1>Fixture</h1>')
  writeFileSync(join(artifact, 'style.css'), 'body{color:teal}')
  const url = 'https://docs.example.test/manual/'
  const manifest = {
    version: PRYX_VERSION,
    base: `/manual/v${PRYX_VERSION}/`,
    address: url,
    commit: git('rev-parse', 'HEAD'),
    files: fileDigests(artifact),
  }
  return {
    directory,
    git,
    options: { repository, artifact, webroot, url, manifest },
    close: () => rmSync(directory, { recursive: true, force: true }),
  }
}

test('release publication is repeatable and retains previous directories', () => {
  const f = fixture()
  try {
    const prior = join(f.options.webroot, 'vprior')
    mkdirSync(prior)
    writeFileSync(join(prior, 'index.html'), 'Retained')
    const result = publishSite(f.options)
    expect(result.url).toBe(`${f.options.url}v${PRYX_VERSION}/`)
    expect(publishSite(f.options)).toEqual(result)
    const current = join(f.options.webroot, `v${PRYX_VERSION}`)
    expect(statSync(current).mode & 0o777).toBe(0o755)
    expect(statSync(join(current, 'index.html')).mode & 0o777).toBe(0o644)
    expect(readFileSync(join(prior, 'index.html'), 'utf8')).toBe('Retained')
    const pointer = readFileSync(join(f.options.webroot, 'index.html'), 'utf8')
    writeFileSync(join(current, 'index.html'), 'Changed')
    expect(() => publishSite(f.options)).toThrow('content changed')
    expect(readFileSync(join(f.options.webroot, 'index.html'), 'utf8')).toBe(pointer)
  } finally {
    f.close()
  }
})

test('extra or changed artifact files and mismatched addresses refuse publication', () => {
  const f = fixture()
  try {
    expect(() => publishSite({ ...f.options, url: 'https://docs.example.test/other/' })).toThrow(
      'addresses differ',
    )
    writeFileSync(join(f.options.artifact, 'unlisted.txt'), 'Fixture')
    expect(() => publishSite(f.options)).toThrow('inventory changed')
    rmSync(join(f.options.artifact, 'unlisted.txt'))
    writeFileSync(join(f.options.artifact, 'index.html'), 'Changed')
    expect(() => publishSite(f.options)).toThrow('content changed')
    expect(existsSync(join(f.options.webroot, `v${PRYX_VERSION}`))).toBe(false)
  } finally {
    f.close()
  }
})

test('publication refuses linked destinations and an unowned root index', () => {
  const f = fixture()
  try {
    const outside = join(f.directory, 'outside')
    mkdirSync(outside)
    writeFileSync(join(outside, 'index.html'), 'Untouched')
    const pointer = join(f.options.webroot, 'index.html')
    symlinkSync(join(outside, 'index.html'), pointer)
    expect(() => publishSite(f.options)).toThrow('linked root index')
    rmSync(pointer)
    writeFileSync(pointer, 'Unowned')
    expect(() => publishSite(f.options)).toThrow('unowned')
    rmSync(pointer)
    symlinkSync(outside, join(f.options.webroot, `v${PRYX_VERSION}`))
    expect(() => publishSite(f.options)).toThrow('existing directories')
    expect(readFileSync(join(outside, 'index.html'), 'utf8')).toBe('Untouched')
  } finally {
    f.close()
  }
})

test('dirty, untagged and different source commits refuse publication', () => {
  const f = fixture()
  try {
    const { repository, manifest } = f.options
    writeFileSync(join(repository, 'source'), 'Changed')
    expect(() => assertRelease(repository, manifest.version, manifest.commit)).toThrow('clean release')
    f.git('add', 'source')
    f.git('commit', '--quiet', '-m', 'Changed fixture')
    expect(() => assertRelease(repository, manifest.version, manifest.commit)).toThrow('exact release tag')
    f.git('tag', '-d', `v${PRYX_VERSION}`)
    expect(() => assertRelease(repository, manifest.version, manifest.commit)).toThrow('no local release tag')
  } finally {
    f.close()
  }
})

test('publication switch defaults off without a destination', async () => {
  const child = Bun.spawn([process.execPath, 'site/publish.ts'], {
    cwd: new URL('../../', import.meta.url).pathname,
    env: { ...process.env, PRYX_SITE_PUBLISH: '', PRYX_SITE_PUBLISH_DIR: '', PRYX_SITE_URL: '' },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  expect(await child.exited).toBe(0)
  expect(await new Response(child.stdout).text()).toContain('publication is off')
})

test('release guard ignores a caller Git-directory override', () => {
  const f = fixture()
  const previous = process.env.GIT_DIR
  try {
    process.env.GIT_DIR = join(f.directory, 'not-a-repository')
    expect(() =>
      assertRelease(f.options.repository, f.options.manifest.version, f.options.manifest.commit),
    ).not.toThrow()
  } finally {
    if (previous === undefined) delete process.env.GIT_DIR
    else process.env.GIT_DIR = previous
    f.close()
  }
})
