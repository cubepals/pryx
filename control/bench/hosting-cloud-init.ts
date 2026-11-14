// SPDX-License-Identifier: FSL-1.1-ALv2
/** Boot checksum-pinned cloud distributions with the shipped user-data, using unreleased local
 * artifacts. QEMU, cloud-init and Docker remain separate programs; no provider is contacted. */

import { randomBytes } from 'node:crypto'
import {
  accessSync,
  appendFileSync,
  closeSync,
  constants,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import images from '../../deploy/platforms/cloud-init/images.json'
import { MINUTE_MS, SECOND_MS } from '../src/durations.ts'
import { WELCOME_DETAILS_PATH } from '../src/welcome-path.ts'

async function command(args: string[], env = process.env) {
  const child = Bun.spawn(args, { env, stdout: 'pipe', stderr: 'pipe' })
  const [output, errors] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  if (await child.exited) throw new Error(`${args[0]} failed: ${errors.slice(-2048)}`)
  return output
}
if (!(await command(['qemu-system-x86_64', '--version'])).includes('version 8.2.2'))
  throw new Error('Expected pinned QEMU 8.2.2')
for (const [name, version] of [
  ['cloud-image-utils', '0.33'],
  ['genisoimage', '1.1.11'],
]) {
  if (!(await command(['dpkg-query', '-W', `-f=\${Version}`, name as string])).includes(version as string))
    throw new Error(`Expected pinned ${name} ${version}`)
}
const root = mkdtempSync(join(tmpdir(), 'pryx-cloud-init-'))
const summaries: string[] = []
try {
  const shared = join(root, 'artifacts')
  mkdirSync(shared)
  const archive = process.env.PRYX_CLOUD_INIT_IMAGE_ARCHIVE
  if (!archive) throw new Error('Set PRYX_CLOUD_INIT_IMAGE_ARCHIVE to the existing release-image archive')
  copyFileSync(archive, join(shared, 'image.tar.gz'), constants.COPYFILE_FICLONE)
  for (const name of ['pryx', 'SHA256SUMS', 'install.sh'])
    copyFileSync(resolve('dist', name), join(shared, name))
  copyFileSync(process.execPath, join(shared, 'bun'))
  await command([
    process.execPath,
    'build',
    '--target=bun',
    'control/tools/hosting-prepare-host.ts',
    '--outfile',
    join(shared, 'prepare-host.js'),
  ])
  let acceleration = 'tcg,thread=multi'
  try {
    accessSync('/dev/kvm', constants.R_OK | constants.W_OK)
    acceleration = 'kvm'
  } catch {}

  for (const image of images) {
    const directory = join(root, image.name.replaceAll(' ', '-'))
    mkdirSync(directory)
    const base = join(directory, 'base.img'),
      disk = join(directory, 'disk.qcow2')
    await command([
      'curl',
      '--config',
      '.github/download.curlrc',
      '--proto',
      '=https',
      '--proto-redir',
      '=https',
      '-sSL',
      image.url,
      '-o',
      base,
    ])
    const check = Bun.spawn(['sha256sum', '--check', '--status'], {
      stdin: 'pipe',
      stdout: 'ignore',
      stderr: 'ignore',
    })
    check.stdin.write(`${image.sha256}  ${base}\n`)
    check.stdin.end()
    if (await check.exited) throw new Error(`${image.name} cloud image digest mismatch`)
    await command(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', base, disk, '32G'])
    const key = `pryx_${randomBytes(32).toString('base64url')}`
    const port = 18081,
      origin = `http://127.0.0.1:${port}`
    const data = Bun.YAML.parse(readFileSync('deploy/platforms/cloud-init/user-data.yaml', 'utf8')) as {
      packages: string[]
      bootcmd?: string[][]
      runcmd: string[][]
    }
    const shipped = data.runcmd[0]
    if (!shipped || shipped[0] !== 'sh' || shipped[1] !== '-c')
      throw new Error('Cloud-init command shape changed')
    const shell = shipped[2]
      ?.replace(
        'https://github.com/cubepals/pryx/releases/latest/download/install.sh',
        'http://127.0.0.1:9876/install.sh',
      )
      .replace('pryx up --image', 'pryx up --image pryx-one-box-release')
    if (!shell || shell === shipped[2]) throw new Error('Cloud-init artifact substitution failed')
    data.packages.push('python3')
    data.bootcmd = [
      ['mkdir', '-p', '/mnt/pryx-artifacts'],
      [
        'mount',
        '-t',
        '9p',
        '-o',
        'trans=virtio,version=9p2000.L,ro',
        'pryx-artifacts',
        '/mnt/pryx-artifacts',
      ],
    ]
    data.runcmd = [
      ['/mnt/pryx-artifacts/bun', '/mnt/pryx-artifacts/prepare-host.js'],
      ['docker', 'load', '-i', '/mnt/pryx-artifacts/image.tar.gz'],
      [
        'systemd-run',
        '--unit=pryx-ci-artifacts',
        'python3',
        '-m',
        'http.server',
        '9876',
        '--bind',
        '127.0.0.1',
        '--directory',
        '/mnt/pryx-artifacts',
      ],
      [
        'env',
        'PRYX_RELEASE_BASE_URL=http://127.0.0.1:9876',
        `PRYX_BOOTSTRAP_API_KEY=${key}`,
        `PRYX_PUBLIC_URL=${origin}`,
        'sh',
        '-c',
        shell,
      ],
    ]
    const userData = join(directory, 'user-data'),
      seed = join(directory, 'seed.img')
    writeFileSync(userData, `#cloud-config\n${Bun.YAML.stringify(data)}`, { mode: 0o600 })
    await command(['cloud-init', 'schema', '--config-file', userData])
    await command(['cloud-localds', seed, userData])
    const serial = join(directory, 'serial.log'),
      fd = openSync(serial, 'w', 0o600)
    const vm = Bun.spawn(
      [
        'qemu-system-x86_64',
        '-accel',
        acceleration,
        '-cpu',
        'max',
        '-m',
        '4096',
        '-smp',
        '2',
        '-nographic',
        '-no-reboot',
        '-drive',
        `file=${disk},format=qcow2,if=virtio`,
        '-drive',
        `file=${seed},format=raw,if=virtio,readonly=on`,
        '-virtfs',
        `local,path=${shared},mount_tag=pryx-artifacts,security_model=none,readonly=on`,
        '-netdev',
        `user,id=net0,hostfwd=tcp:127.0.0.1:${port}-:8080`,
        '-device',
        'virtio-net-pci,netdev=net0',
      ],
      { stdin: 'ignore', stdout: fd, stderr: fd },
    )
    closeSync(fd)
    try {
      const deadline = Date.now() + 25 * MINUTE_MS
      let healthy = false
      while (Date.now() < deadline && vm.exitCode === null) {
        const response = await fetch(`${origin}/healthz`).catch(() => undefined)
        if (response?.ok) {
          healthy = true
          break
        }
        await Bun.sleep(5 * SECOND_MS)
      }
      if (!healthy)
        throw new Error(
          `${image.name} cloud-init failed: ${readFileSync(serial, 'utf8').replaceAll(key, '[redacted]').slice(-12000)}`,
        )
      const page = await fetch(origin)
      if (!page.ok || !(await page.text()).includes('pryx runs here'))
        throw new Error('Boot did not end at welcome')
      const privatePage = await fetch(`${origin}${WELCOME_DETAILS_PATH}`, {
        headers: { authorization: `Bearer ${key}` },
      })
      if (!privatePage.ok) throw new Error('Boot key rejected')
      const output = await command(
        [resolve('dist/pryx'), 'run', '--image', 'alpine:3.22', '--', 'echo', 'hello'],
        { ...process.env, PRYX_API_URL: origin, PRYX_API_KEY: key },
      )
      if (output.trim() !== 'hello') throw new Error('Cloud-init hello differs')
      const summary = `${image.name} ${image.sha256}: boot, cloud-init install, welcome, authentication and hello pass (${acceleration})`
      summaries.push(summary)
      console.log(summary)
    } finally {
      vm.kill()
      await vm.exited
      rmSync(directory, { recursive: true, force: true })
    }
  }
} finally {
  rmSync(root, { recursive: true, force: true })
}
if (process.env.GITHUB_STEP_SUMMARY)
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summaries.join('\n')}\n`)
