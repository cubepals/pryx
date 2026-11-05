// SPDX-License-Identifier: FSL-1.1-ALv2
/** Private image credentials and embedded services. Credentials are persisted once, with mode
 * 0600, and never included in subprocess diagnostics or the ordinary startup log. */
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
  unlinkSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join } from 'node:path'
import { FleetCa } from '../control/src/ca.ts'
import { SECOND_MS } from '../control/src/durations.ts'
import { ensureBucket } from '../control/src/store-bootstrap.ts'

function serviceIds() {
  const entry = readFileSync('/etc/passwd', 'utf8')
    .split('\n')
    .find((line) => line.startsWith('postgres:'))
  if (!entry) throw new Error('The embedded service user is missing')
  const fields = entry.split(':')
  return { uid: Number(fields[2]), gid: Number(fields[3]) }
}
const secret = () => randomBytes(32).toString('hex')
export function createPrivateFile(path: string, contents: string) {
  const temporary = `${path}.next-${randomUUID()}`
  const fd = openSync(
    temporary,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
    0o600,
  )
  try {
    writeFileSync(fd, contents)
    fsyncSync(fd)
  } finally {
    closeSync(fd)
  }
  try {
    linkSync(temporary, path)
  } finally {
    unlinkSync(temporary)
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
    throw new Error('Unsafe state directory')
  chmodSync(root, 0o711)
  const privateDir = join(root, 'private')
  mkdirSync(privateDir, { recursive: true, mode: 0o700 })
  const privateInfo = lstatSync(privateDir)
  if (!privateInfo.isDirectory() || privateInfo.uid !== process.getuid?.() || privateInfo.mode & 0o077)
    throw new Error('Unsafe private directory')
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
      throw new Error('Unsafe credential file')
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
export function serviceCommand(args: string[]) {
  return process.getuid?.() === 0
    ? ['setpriv', `--reuid=${serviceIds().uid}`, `--regid=${serviceIds().gid}`, '--clear-groups', ...args]
    : args
}
export function privateLog(root: string, name: string) {
  return openSync(join(root, 'private', `${name}.log`), 'a', 0o600)
}
export async function embedded(
  root: string,
  values: Awaited<ReturnType<typeof credentials>>,
  children: Bun.Subprocess[],
) {
  let database = process.env.DATABASE_URL
  if (!database) {
    const directory = join(root, 'postgres')
    mkdirSync(directory, { recursive: true, mode: 0o700 })
    if (process.getuid?.() === 0) chownSync(directory, serviceIds().uid, serviceIds().gid)
    const data = join(directory, 'data')
    const environment = { ...process.env }
    if (process.getuid?.() !== 0) {
      const passwd = join(root, 'private', 'passwd'),
        group = join(root, 'private', 'group')
      writeFileSync(
        passwd,
        `pryx:x:${process.getuid?.()}:${process.getgid?.()}:pryx:${directory}:/bin/sh\n`,
        { mode: 0o600 },
      )
      writeFileSync(group, `pryx:x:${process.getgid?.()}:\n`, { mode: 0o600 })
      Object.assign(environment, {
        LD_PRELOAD: '/usr/lib/x86_64-linux-gnu/libnss_wrapper.so',
        NSS_WRAPPER_PASSWD: passwd,
        NSS_WRAPPER_GROUP: group,
      })
    }
    const log = privateLog(root, 'postgres')
    if (!existsSync(join(data, 'PG_VERSION'))) {
      const password = join(directory, 'password')
      writeFileSync(password, values.password, { mode: 0o600 })
      if (process.getuid?.() === 0) chownSync(password, serviceIds().uid, serviceIds().gid)
      const init = Bun.spawn(
        serviceCommand([
          '/usr/lib/postgresql/17/bin/initdb',
          '-D',
          data,
          '-U',
          'pryx',
          '--auth-local=trust',
          '--auth-host=scram-sha-256',
          `--pwfile=${password}`,
          '--no-instructions',
        ]),
        { env: environment, stdout: log, stderr: log },
      )
      if (await init.exited) throw new Error('Embedded Postgres initialization failed')
    }
    const postgres = Bun.spawn(
      serviceCommand([
        '/usr/lib/postgresql/17/bin/postgres',
        '-D',
        data,
        '-h',
        '127.0.0.1',
        '-p',
        '5432',
        '-k',
        directory,
      ]),
      { env: environment, stdout: log, stderr: log },
    )
    children.push(postgres)
    await until(
      'Embedded Postgres',
      async () =>
        Bun.spawnSync(
          ['/usr/lib/postgresql/17/bin/pg_isready', '-h', '127.0.0.1', '-p', '5432', '-U', 'pryx'],
          { stdout: 'ignore', stderr: 'ignore' },
        ).exitCode === 0,
    )
    const created = Bun.spawn(
      serviceCommand(['/usr/lib/postgresql/17/bin/createdb', '-h', '127.0.0.1', '-U', 'pryx', 'pryx']),
      { env: { ...environment, PGPASSWORD: values.password }, stdout: log, stderr: log },
    )
    // Existing databases are normal on restart. A subsequent authenticated control connection
    // must succeed; no startup credential is written to a command line.
    await created.exited
    database = `postgres://pryx:${values.password}@127.0.0.1:5432/pryx`
  }
  const external = Boolean(process.env.S3_ENDPOINT)
  const s3 = {
    endpoint: process.env.S3_ENDPOINT ?? 'http://127.0.0.1:9000',
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
      Bun.spawn(serviceCommand(['/usr/local/bin/rustfs', directory]), {
        env: {
          ...process.env,
          RUSTFS_ADDRESS: '127.0.0.1:9000',
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
