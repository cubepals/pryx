// SPDX-License-Identifier: FSL-1.1-ALv2
/** Shell integration preserves existing user text and modes and never follows a profile link. */
import { expect, test } from 'bun:test'
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { installProfiles, removeProfiles } from '../src/up-local-profile.ts'
import { userPaths } from '../src/up-paths.ts'

test('PATH integration is idempotent and restores existing profile bytes and mode', () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-profile-'))
  const paths = userPaths({}, home)
  const profile = join(home, '.profile')
  const original = 'export EDITOR=vi\n# custom end without newline'
  try {
    writeFileSync(profile, original)
    chmodSync(profile, 0o640)
    const owned = installProfiles(paths, [], '/bin/sh')
    expect(owned).toEqual([{ path: profile, created: false }])
    const first = readFileSync(profile, 'utf8')
    expect(first).toContain(original)
    expect(first).toContain('$HOME/.local/bin')
    expect(installProfiles(paths, owned, '/bin/sh')).toEqual(owned)
    expect(readFileSync(profile, 'utf8')).toBe(first)
    removeProfiles(paths, owned)
    expect(readFileSync(profile, 'utf8')).toBe(original)
    expect(statSync(profile).mode & 0o777).toBe(0o640)
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})
test('purge removes a newly created profile and refuses an unrelated target', () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-profile-new-'))
  const paths = userPaths({}, home)
  try {
    const owned = installProfiles(paths, [], '/bin/sh')
    expect(owned[0]?.created).toBe(true)
    expect(() => removeProfiles(paths, [{ path: join(home, 'unrelated'), created: true }])).toThrow(
      'Invalid managed shell profile',
    )
    expect(existsSync(join(home, '.profile'))).toBe(true)
    removeProfiles(paths, owned)
    expect(existsSync(join(home, '.profile'))).toBe(false)
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})
test('a profile symlink leaves its target unchanged', () => {
  const home = mkdtempSync(join(tmpdir(), 'pryx-profile-link-'))
  const paths = userPaths({}, home)
  const target = join(home, 'dotfiles')
  try {
    writeFileSync(target, 'keep this text')
    symlinkSync(target, join(home, '.profile'))
    expect(installProfiles(paths, [], '/bin/sh')).toEqual([])
    expect(readFileSync(target, 'utf8')).toBe('keep this text')
  } finally {
    rmSync(home, { recursive: true, force: true })
  }
})
