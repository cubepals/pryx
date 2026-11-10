// SPDX-License-Identifier: FSL-1.1-ALv2
/** Private image files and embedded services. Credentials are persisted once; regenerable node
 * configuration uses atomic replacement. Contents stay out of ordinary subprocess diagnostics. */
import { randomBytes, randomUUID } from 'node:crypto'
import {
  chmodSync,
  chownSync,
  closeSync,
  constants,
  existsSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join } from 'node:path'
import { FleetCa } from '../control/src/ca.ts'
import { SECOND_MS } from '../control/src/durations.ts'
import { ensureBucket } from '../control/src/store-bootstrap.ts'
import { bundleBinary, embeddedUser, postgresBinary, privateDirectory } from './local-layout.ts'

export const IMAGE_NODE_CONFIG = 'pryxd.toml'
// Tini 0.19.0's -p option accepts a signal name, as its --help documents.
export const PARENT_DEATH_SIGNAL = 'SIGTERM' satisfies NodeJS.Signals
function serviceIds() {
  return embeddedUser()
}
const secret = () => randomBytes(32).toString('hex')
export function createPrivateFile(
  path: string,
  contents: string | Uint8Array,
  replace = false,
  mode = 0o600,
) {
  const temporary = `${path}.next-${randomUUID()}`
  const fd = openSync(
    temporary,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
    mode,
  )
  try {
    writeFileSync(fd, contents)
    fsyncSync(fd)
  } finally {
    closeSync(fd)
  }
  let moved = false
  try {
    if (replace) {
      renameSync(temporary, path)
      moved = true
    } else linkSync(temporary, path)
  } finally {
    if (!moved) unlinkSync(temporary)
  }
  const directory = openSync(dirname(path), constants.O_RDONLY | constants.O_DIRECTORY)
  try {
    fsyncSync(directory)
  } finally {
    closeSync(directory)
  }
}
export async function credentials(root: string) {
  mkdirSync(root, { recursive: true, mode: 0o711 })
  const rootInfo = lstatSync(root)
  if (!rootInfo.isDirectory() || rootInfo.uid !== process.getuid?.())
    throw Object.assign(new Error('Unsafe state directory'), { code: 'UNSAFE_STATE' })
  chmodSync(root, process.env.PRYX_LOCAL_PRESET && process.getuid?.() !== 0 ? 0o700 : 0o711)
  const privateDir = privateDirectory(root)
  mkdirSync(privateDir, { recursive: true, mode: 0o700 })
  const privateInfo = lstatSync(privateDir)
  if (!privateInfo.isDirectory() || privateInfo.uid !== process.getuid?.() || privateInfo.mode & 0o077)
    throw Object.assign(new Error('Unsafe private directory'), { code: 'UNSAFE_PRIVATE' })
  const path = join(privateDir, 'credentials.json')
  if (!existsSync(path)) {
    const ca = await FleetCa.generate('pryx')
    createPrivateFile(
      path,
      JSON.stringify({
        caCert: ca.certPem,
        caKey: ca.keyPem,
        operator: secret(),
        edge: secret(),
        key: `pryx_${randomBytes(32).toString('base64url')}`,
        password: secret(),
        access: randomBytes(10).toString('hex'),
        storeSecret: secret(),
      }),
    )
  }
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  let encoded: string
  try {
    const info = fstatSync(fd)
    if (!info.isFile() || info.uid !== process.getuid?.() || info.mode & 0o077)
      throw Object.assign(new Error('Unsafe credential file'), { code: 'UNSAFE_CREDENTIALS' })
    encoded = readFileSync(fd, 'utf8')
  } finally {
    closeSync(fd)
  }
  return JSON.parse(encoded) as {
    caCert: string
    caKey: string
    operator: string
    edge: string
    key: string
    password: string
    access: string
    storeSecret: string
  }
}
export async function until(label: string, work: () => Promise<boolean>, seconds = 30) {
  const deadline = Date.now() + seconds * SECOND_MS
  do {
    if (await work().catch(() => false)) return
    await Bun.sleep(SECOND_MS / 100)
  } while (Date.now() < deadline)
  throw new Error(`${label} did not become ready`)
}
export function supervisedCommand(args: string[]) {
  return process.env.PRYX_LOCAL_PRESET
    ? [bundleBinary('tini', 'tini'), '-s', '-g', '-p', PARENT_DEATH_SIGNAL, '--', ...args]
    : args
}
export function serviceCommand(args: string[]) {
  const command =
    process.getuid?.() === 0
      ? [
          bundleBinary('setpriv', 'setpriv'),
          `--reuid=${serviceIds().uid}`,
          `--regid=${serviceIds().gid}`,
          '--clear-groups',
          ...args,
        ]
      : args
  return supervisedCommand(command)
}

export function privateLog(root: string, name: string) {
  const fd = openSync(
    join(privateDirectory(root), `${name}.log`),
    constants.O_WRONLY | constants.O_APPEND | constants.O_CREAT | constants.O_NOFOLLOW,
    0o600,
  )
  const info = fstatSync(fd)
  if (!info.isFile() || info.uid !== process.getuid?.() || info.mode & 0o077) {
    closeSync(fd)
    throw Object.assign(new Error('Unsafe private log'), { code: 'UNSAFE_LOG' })
  }
  return fd
}
export async function embedded(
  root: string,
  values: Awaited<ReturnType<typeof credentials>>,
  children: Bun.Subprocess[],
  options = { runtime: root, storePort: 9000, local: false },
) {
  let database = process.env.DATABASE_URL
  if (!database) {
    const directory = join(root, 'postgres')
    mkdirSync(directory, { recursive: true, mode: 0o700 })
    if (process.getuid?.() === 0) chownSync(directory, serviceIds().uid, serviceIds().gid)
    const data = join(directory, 'data')
    const socket = options.local ? join(options.runtime, 'postgres') : directory
    mkdirSync(socket, { recursive: true, mode: 0o700 })
    if (process.getuid?.() === 0) chownSync(socket, serviceIds().uid, serviceIds().gid)
    const environment = { ...process.env }
    if (process.getuid?.() !== 0 || options.local) {
      const ids =
        process.getuid?.() === 0 ? serviceIds() : { uid: process.getuid?.(), gid: process.getgid?.() }
      const passwd = join(options.local ? directory : privateDirectory(root), 'passwd'),
        group = join(options.local ? directory : privateDirectory(root), 'group')
      writeFileSync(passwd, `pryx:x:${ids.uid}:${ids.gid}:pryx:${directory}:/bin/sh\n`, { mode: 0o600 })
      writeFileSync(group, `pryx:x:${ids.gid}:\n`, { mode: 0o600 })
      if (process.getuid?.() === 0) {
        chownSync(passwd, serviceIds().uid, serviceIds().gid)
        chownSync(group, serviceIds().uid, serviceIds().gid)
      }
      Object.assign(environment, {
        ...(!process.env.PRYX_BUNDLE_DIR
          ? { LD_PRELOAD: '/usr/lib/x86_64-linux-gnu/libnss_wrapper.so' }
          : {}),
        NSS_WRAPPER_PASSWD: passwd,
        NSS_WRAPPER_GROUP: group,
      })
    }
    const log = privateLog(root, 'postgres')
    if (!existsSync(join(data, 'PG_VERSION'))) {
      const initialized = options.local ? join(directory, `data-next-${randomUUID()}`) : data
      const password = join(directory, 'password')
      writeFileSync(password, values.password, { mode: 0o600 })
      if (process.getuid?.() === 0) chownSync(password, serviceIds().uid, serviceIds().gid)
      const init = Bun.spawn(
        serviceCommand([
          postgresBinary('initdb'),
          '-D',
          initialized,
          '-U',
          'pryx',
          '--auth=scram-sha-256',
          `--pwfile=${password}`,
          '--no-instructions',
          ...(options.local ? ['--locale=C', '--encoding=UTF8'] : []),
        ]),
        { env: environment, stdout: log, stderr: log },
      )
      if (await init.exited) throw new Error('Embedded Postgres initialization failed')
      if (options.local) renameSync(initialized, data)
    }
    const postgres = Bun.spawn(
      serviceCommand([
        postgresBinary('postgres'),
        '-D',
        data,
        '-h',
        options.local ? '' : '127.0.0.1',
        '-p',
        '5432',
        '-k',
        socket,
      ]),
      { env: environment, stdout: log, stderr: log },
    )
    children.push(postgres)
    await until(
      'Embedded Postgres',
      async () =>
        Bun.spawnSync(
          [
            postgresBinary('pg_isready'),
            '-h',
            options.local ? socket : '127.0.0.1',
            '-p',
            '5432',
            '-U',
            'pryx',
          ],
          { stdout: 'ignore', stderr: 'ignore' },
        ).exitCode === 0,
    )
    const created = Bun.spawn(
      serviceCommand([
        postgresBinary('createdb'),
        '-h',
        options.local ? socket : '127.0.0.1',
        '-U',
        'pryx',
        'pryx',
      ]),
      { env: { ...environment, PGPASSWORD: values.password }, stdout: log, stderr: log },
    )
    // Existing databases are normal on restart. A subsequent authenticated control connection
    // must succeed; no startup credential is written to a command line.
    await created.exited
    database = options.local
      ? `postgres://pryx:${values.password}@localhost/pryx?host=${encodeURIComponent(socket)}`
      : `postgres://pryx:${values.password}@127.0.0.1:5432/pryx`
  }
  const external = Boolean(process.env.S3_ENDPOINT)
  const s3 = {
    endpoint: process.env.S3_ENDPOINT ?? `http://127.0.0.1:${options.storePort}`,
    bucket: process.env.S3_BUCKET ?? 'pryx-snapshots',
    accessKeyId: process.env.S3_ACCESS_KEY ?? values.access,
    secretAccessKey: process.env.S3_SECRET_KEY ?? values.storeSecret,
  }
  if (!external) {
    const directory = join(root, 'objects')
    mkdirSync(directory, { recursive: true, mode: 0o700 })
    if (process.getuid?.() === 0) chownSync(directory, serviceIds().uid, serviceIds().gid)
    const log = privateLog(root, 'store')
    children.push(
      Bun.spawn(serviceCommand([bundleBinary('rustfs', '/usr/local/bin/rustfs'), directory]), {
        env: {
          ...process.env,
          RUSTFS_ADDRESS: `127.0.0.1:${options.storePort}`,
          RUSTFS_CONSOLE_ENABLE: 'false',
          RUSTFS_ACCESS_KEY: s3.accessKeyId,
          RUSTFS_SECRET_KEY: s3.secretAccessKey,
        },
        stdout: log,
        stderr: log,
      }),
    )
  }
  await until('Object store', async () => {
    await ensureBucket(s3)
    return true
  })
  return { database, s3 }
}
