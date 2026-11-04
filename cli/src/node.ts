// SPDX-FileCopyrightText: 2026 The Cubepals Authors
//
// SPDX-License-Identifier: FSL-1.1-ALv2

/**
 * Operating a fleet: the nodes the `fleet` runtime places servers on (docs/fleet-operations.md).
 *
 *   bun scripts/fleet.ts ca <deployment> <dir>     a new fleet CA, for the deployment's secrets
 *   bun scripts/fleet.ts token <region> [--label k=v]… [--ttl seconds] [--out file]
 *                                                  the one line to paste on a new host, as root: it
 *                                                  checks the fleet CA, installs Docker and blocklyd,
 *                                                  and joins the host to the fleet in that region.
 *                                                  --out writes only its one-time token (bk1.…), 0600
 *   bun scripts/fleet.ts token --node <node> [--out file]
 *                                                  the line that re-enrolls a node under its own id,
 *                                                  pasted on its host: its certificates ended, or its
 *                                                  identity directory was lost (§8)
 *   bun scripts/fleet.ts rotate-ca [--ttl seconds]   once the control plane runs a new fleet CA, one
 *                                                  such line per node that isn't retired (§8)
 *   bun scripts/fleet.ts add <region> --type <hetzner type> [--name <name>] [--location <location>]
 *                            --price-cents n [--label k=v]… [--dir <environment>] [--yes]
 *                                                  a Hetzner Cloud node, start to finish: mints its token
 *                                                  with its monthly price in US cents, lists
 *                                                  it in the environment's fleet-nodes.auto.tfvars.json,
 *                                                  applies it with terraform, and waits until it is healthy
 *   bun scripts/fleet.ts remove <node> [--dir <environment>] [--yes]
 *                                                  drains a node until it holds no servers, retires it,
 *                                                  takes it out of the tfvars file and deletes the server
 *   bun scripts/fleet.ts nodes [--all] [--json]    every node: lifecycle, health, room
 *   bun scripts/fleet.ts node <node>               one node, its servers and its recent events
 *   bun scripts/fleet.ts label <node> key=value… [--remove key]…
 *                                                  set or remove a node's labels, its monthly price
 *                                                  (monthly_cost_cents) among them
 *   bun scripts/fleet.ts drain|undrain <node>      take no new servers, or take them again
 *   bun scripts/fleet.ts lost <node> --fenced-by <how> --reason <why> [--force]
 *                                                  declare a node gone, once it is stopped for sure
 *   bun scripts/fleet.ts reinstate <node>          a lost node came back and its data is wanted
 *   bun scripts/fleet.ts retire <node>             an empty node leaves the fleet for good
 *   bun scripts/fleet.ts clear-quarantine <node>   after two hosts beat with one identity
 *   bun scripts/fleet.ts upgrades [--json]         the blocklyd rollout: each node, its version, its state
 *   bun scripts/fleet.ts retry-upgrade <node>      offer a node whose upgrade failed the release again,
 *                                                  which resumes its region's rollout (§9)
 *   bun scripts/fleet.ts placements [--node <node>]
 *   bun scripts/fleet.ts placement <server>
 *   bun scripts/fleet.ts move <server> [--to <node>]   through the server's own relocation
 *   bun scripts/fleet.ts events [--node <node>] [--server <server>] [--limit n]
 *   bun scripts/fleet.ts summary
 *
 * Everything but `ca` talks to the control plane's operator API, on its internal listener, which
 * only the deployment's private network reaches: OPERATOR_API is its base URL (http://host:port)
 * and OPERATOR_TOKEN its bearer token (FLEET_API and FLEET_OPERATOR_TOKEN are read too). OPERATOR
 * (or FLEET_OPERATOR, or USER) is who the fleet's ledger says acted. `lost` is the one-way door here: the node's servers are rebuilt elsewhere from their last
 * backup, so it asks for how the node was stopped, and refuses a node that still beats unless forced.
 *
 * `add` and `remove` also run terraform in the environment `--dir` names (or FLEET_TERRAFORM_DIR),
 * with whatever that environment's apply needs already set (docs/configuration.md) and HCLOUD_TOKEN.
 * `--wait` is how many minutes either waits for the fleet (20 by default, 30 for `remove`).
 */
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { MINUTE_MS } from '@blockly/contracts/durations'
import { FleetCa } from '../apps/control/src/infra/fleet/ca.ts'
import {
  applyNode,
  HETZNER_LOCATIONS,
  NODE_NAME,
  nextName,
  nodesFile,
  priceCents,
  readNodes,
  writeNodes,
} from './fleet-hetzner.ts'
import { flags, operatorCli } from './lib/operator-cli.ts'

const [command = 'help', ...rest] = process.argv.slice(2)

const { positional, options } = flags(rest)
const option = (name: string) => options.get(name)?.at(-1)

const { fail, api } = operatorCli('fleet', '/fleet/v1', 'docs/fleet-operations.md')

const say = (line: string) => process.stdout.write(`${line}\n`)
const show = (value: unknown) => say(JSON.stringify(value, null, 2))
/** Labels as `key=value` arguments. */
const labelsOf = (list: string[], what: string): Record<string, string> =>
  Object.fromEntries(
    list.map((pair) => {
      const at = pair.indexOf('=')
      if (at < 1) fail(`${what} takes key=value, not ${pair}`)
      return [pair.slice(0, at), pair.slice(at + 1)]
    }),
  )
const node = () => positional[0] ?? fail(`${command} needs a node id`)
const server = () => positional[0] ?? fail(`${command} needs a server id`)
const query = (pairs: Record<string, string | undefined>) => {
  const params = new URLSearchParams(
    Object.entries(pairs).filter((p): p is [string, string] => p[1] !== undefined),
  )
  return params.size === 0 ? '' : `?${params}`
}

interface NodeView {
  id: string
  name: string
  region: string
  lifecycle: string
  health: string
  quarantined: boolean
  lastHeartbeatSecondsAgo: number | null
  version: string | null
  servers: number
  serversRunning: number
  memoryMb: { allocatable: number | null; allocated: number; running: number }
  disk: { availableBytes: number | null }
}

interface Upgrades {
  release: { version: string; sha256: string } | null
  nodes: Array<{
    id: string
    name: string
    region: string
    version: string | null
    state: string
    since: string | null
    reason: string | null
  }>
}

function rollout(upgrades: Upgrades) {
  say(
    upgrades.release === null
      ? 'No rollout: FLEET_UPGRADES is off, or the control plane has no blocklyd to hand out.'
      : `Rolling out blocklyd ${upgrades.release.version}, one node per region at a time.`,
  )
  const rows = upgrades.nodes.map((n) => [
    n.id,
    n.name,
    n.region,
    n.version ?? '?',
    n.state,
    n.since ?? '',
    n.reason ?? '',
  ])
  const head = ['ID', 'NAME', 'REGION', 'VERSION', 'STATE', 'SINCE', 'WHY']
  const widths = head.map((h, i) => Math.max(h.length, ...rows.map((r) => (r[i] ?? '').length)))
  for (const row of [head, ...rows])
    say(
      row
        .map((cell, i) => cell.padEnd(widths[i] ?? 0))
        .join('  ')
        .trimEnd(),
    )
  if (upgrades.nodes.some((n) => n.state === 'failed'))
    say(
      'A failed node stops its region. See why in `fleet.ts events --node <node>`, then `fleet.ts retry-upgrade <node>`.',
    )
}

function table(nodes: NodeView[]) {
  const gb = (bytes: number | null) => (bytes === null ? '?' : `${Math.round(bytes / 1024 ** 3)} GB`)
  const rows = nodes.map((n) => [
    n.id,
    n.name,
    n.region,
    n.quarantined ? `${n.lifecycle} (quarantined)` : n.lifecycle,
    n.health,
    n.lastHeartbeatSecondsAgo === null ? '-' : `${n.lastHeartbeatSecondsAgo}s`,
    `${n.serversRunning}/${n.servers}`,
    `${n.memoryMb.running}/${n.memoryMb.allocatable ?? '?'} MB (${n.memoryMb.allocated} placed)`,
    gb(n.disk.availableBytes),
    n.version ?? '?',
  ])
  const head = [
    'ID',
    'NAME',
    'REGION',
    'LIFECYCLE',
    'HEALTH',
    'BEAT',
    'RUNNING/PLACED',
    'MEMORY RUNNING',
    'DISK FREE',
    'VERSION',
  ]
  const widths = head.map((h, i) => Math.max(h.length, ...rows.map((r) => (r[i] ?? '').length)))
  for (const row of [head, ...rows]) say(row.map((cell, i) => cell.padEnd(widths[i] ?? 0)).join('  '))
}

function terraformDir(): string {
  const dir = option('dir') ?? process.env.FLEET_TERRAFORM_DIR
  if (!dir) fail('say which environment: --dir infra/terraform/environments/<name>, or FLEET_TERRAFORM_DIR')
  if (!existsSync(nodesFile(dir)))
    fail(`${nodesFile(dir)} doesn't exist: --dir names an environment's directory`)
  if (!Bun.which('terraform'))
    fail('terraform is missing here: install it (1.11 or later), then run this again')
  return dir
}

const minutes = (fallback: number) => Number(option('wait') ?? fallback) * MINUTE_MS
const waited = (fallback: number) => {
  const n = Number(option('wait') ?? fallback)
  return `${n} minute${n === 1 ? '' : 's'}`
}

/** Until the node that joins under this name, and wasn't there before, beats healthy. */
async function untilHealthy(name: string, before: Set<string>): Promise<NodeView> {
  const deadline = Date.now() + minutes(20)
  let last = ''
  while (Date.now() < deadline) {
    const { nodes } = (await api('GET', '/nodes')) as { nodes: NodeView[] }
    const joined = nodes.find((n) => n.name === name && !before.has(n.id))
    const now = joined
      ? `${joined.id} is ${joined.lifecycle} and ${joined.health}`
      : `${name} hasn't joined yet`
    if (now !== last) say(`  ${now}`)
    last = now
    if (joined?.health === 'healthy') return joined
    await Bun.sleep(5000)
  }
  fail(
    `${name} isn't healthy after ${waited(20)} (${last}). On the host, /var/log/cloud-init-output.log ` +
      'says how the join line went, and blocklyd doctor what is wrong with the host.',
  )
}

async function add() {
  const region = positional[0] ?? fail('add <region> --type <hetzner type> [--name <name>]')
  const type = option('type')?.toLowerCase() ?? fail('add needs --type, a Hetzner Cloud type such as ccx33')
  const location =
    option('location') ??
    (HETZNER_LOCATIONS.includes(region)
      ? region
      : fail(
          `${region} isn't a Hetzner location: say which with --location (${HETZNER_LOCATIONS.join(', ')})`,
        ))
  const dir = terraformDir()
  const nodes = readNodes(dir)
  const name = option('name') ?? nextName(nodes, location, type)
  if (!NODE_NAME.test(name)) fail(`${name} can't name a host: lowercase letters, digits and dashes`)
  if (nodes[name] !== undefined) fail(`${name} is in ${nodesFile(dir)} already: choose another --name`)
  const cents =
    priceCents(option('price-cents')) ??
    fail(`add needs --price-cents: what a ${type} costs a month, in whole US cents`)

  const { nodes: listed } = (await api('GET', '/nodes?all=true')) as { nodes: NodeView[] }
  const labels = {
    ...labelsOf(options.get('label') ?? [], '--label'),
    monthly_cost_cents: String(cents),
  }
  const made = (await api('POST', '/tokens', { region, labels })) as {
    joinCommand?: string | null
    expiresAt: string
  }
  if (!made.joinCommand) fail('the control plane gave no join line: set FLEET_ENDPOINT_HOSTS on it')
  say(`Minted a token for ${region} (until ${made.expiresAt}), priced monthly_cost_cents=${cents}.`)
  writeNodes(dir, { ...nodes, [name]: { location, type } })
  say(`Listed ${name}, a ${type} in ${location}, in ${nodesFile(dir)}.`)

  if (applyNode(dir, name, { joinLine: made.joinCommand, yes: options.has('yes') }) !== 0)
    fail(
      `terraform didn't finish, so ${name} may not exist. Its entry stays in ${nodesFile(dir)} and its token ` +
        `expires unused: bun scripts/fleet.ts remove ${name} takes back whatever was made.`,
    )
  say(`Hetzner made ${name}; cloud-init runs its join line. Waiting for it to join:`)
  const joined = await untilHealthy(name, new Set(listed.map((n) => n.id)))
  say(`${name} is node ${joined.id} in ${joined.region}, healthy, on blocklyd ${joined.version ?? '?'}.`)
  say('Commit the tfvars change. Placement uses the node already.')
}

/** Until the node holds nothing, or the wait is over and what it still holds is the answer. */
async function untilEmpty(node: NodeView): Promise<void> {
  const deadline = Date.now() + minutes(30)
  let held: { workload: string; state: string; observed: string | null }[] = []
  let count = -1
  while (Date.now() < deadline) {
    const { placements } = (await api('GET', `/placements?node=${node.id}`)) as { placements: typeof held }
    held = placements.filter((p) => p.state !== 'released')
    if (held.length === 0) return
    if (held.length !== count)
      say(`  ${node.name} holds ${held.length} server${held.length === 1 ? '' : 's'}`)
    count = held.length
    await Bun.sleep(10_000)
  }
  for (const p of held) say(`  ${p.workload}: ${p.state}, ${p.observed ?? 'not reported'}`)
  fail(
    `${node.name} still holds these after ${waited(30)}. bun scripts/fleet.ts placement <server> says where one ` +
      `stands, and events --node ${node.id} why a move was declined. Move one with fleet.ts move <server>; a ` +
      'deleted server stays until it is purged. Then run remove again.',
  )
}

async function remove() {
  const which = positional[0] ?? fail('remove <node>: its id or its name')
  const dir = terraformDir()
  const nodes = readNodes(dir)
  const { nodes: listed } = (await api('GET', '/nodes?all=true')) as { nodes: NodeView[] }
  const found = listed.find((n) => (n.id === which || n.name === which) && n.lifecycle !== 'retired')
  const name = found?.name ?? which
  if (found === undefined && nodes[name] === undefined)
    fail(`${which} is neither a node of the fleet nor in ${nodesFile(dir)}`)

  if (found === undefined) say(`${name} never joined the fleet, so nothing drains.`)
  else {
    if (found.lifecycle === 'active') {
      await api('POST', `/nodes/${found.id}/drain`)
      say(`Drained ${name}: nothing new goes there, and its servers move off. Waiting until it holds none:`)
    }
    await untilEmpty(found)
    await api('POST', `/nodes/${found.id}/retire`)
    say(`Retired ${name} (${found.id}): its certificates no longer work.`)
  }

  if (nodes[name] === undefined) {
    say(`${name} isn't in ${nodesFile(dir)}: delete its host at its provider.`)
    return
  }
  const { [name]: _removed, ...rest } = nodes
  writeNodes(dir, rest)
  say(`Took ${name} out of ${nodesFile(dir)}.`)
  if (applyNode(dir, name, { yes: options.has('yes') }) !== 0)
    fail(`terraform didn't finish: the server ${name} may still exist. terraform apply in ${dir} deletes it.`)
  say(`Deleted the server ${name}. Commit the tfvars change.`)
}

switch (command) {
  case 'ca': {
    const [deployment, dir] = positional
    if (!deployment || !dir) fail('ca <deployment> <dir>')
    const { certPem, keyPem } = await FleetCa.generate(deployment)
    mkdirSync(dir, { recursive: true, mode: 0o700 })
    writeFileSync(join(dir, 'ca.pem'), certPem)
    writeFileSync(join(dir, 'ca.key'), keyPem, { mode: 0o600 })
    say(`Wrote ${join(dir, 'ca.pem')} and ${join(dir, 'ca.key')} (0600).`)
    say('Set FLEET_CA_CERT and FLEET_CA_KEY from them in the deployment’s secrets, then delete the key file.')
    say('ca.pem is public: every node gets it with its token.')
    break
  }
  case 'token': {
    const node = option('node')
    const region = positional[0] ?? (node ? undefined : fail('token <region>, or token --node <node>'))
    const labels = labelsOf(options.get('label') ?? [], '--label')
    const ttl = option('ttl')
    const made = (await api('POST', '/tokens', {
      ...(region ? { region } : {}),
      ...(node ? { node } : {}),
      labels,
      ...(ttl ? { ttlSeconds: Number(ttl) } : {}),
    })) as { token: string; joinToken?: string; joinCommand?: string | null; expiresAt: string }
    const token = made.joinToken ?? made.token
    const out = option('out')
    if (out) {
      writeFileSync(out, `${token}\n`, { mode: 0o600 })
      say(`Wrote the token to ${out} (0600); it works once, until ${made.expiresAt}.`)
    } else if (made.joinCommand) {
      say(made.joinCommand)
      console.error(
        node
          ? `Paste it on ${node}’s host, as root: it enrolls the node again under its own id, until ${made.expiresAt}. Treat it as a password until then.`
          : `Paste it on the new host, as root. It joins one host, until ${made.expiresAt}: treat it as a password until then.`,
      )
    } else {
      say(token)
      console.error(`One enrollment, until ${made.expiresAt}. Treat it as a password until then.`)
    }
    break
  }
  case 'add':
    await add()
    break
  case 'remove':
    await remove()
    break
  case 'rotate-ca': {
    const ttl = option('ttl')
    const listed = (await api('GET', '/nodes?all=true')) as { nodes: NodeView[] }
    const nodes = listed.nodes.filter((n) => n.lifecycle !== 'retired')
    if (nodes.length === 0) fail('no node to move to the new CA: every one is retired')
    let ca: string | undefined
    for (const n of nodes) {
      const made = (await api('POST', '/tokens', {
        node: n.id,
        labels: {},
        ...(ttl ? { ttlSeconds: Number(ttl) } : {}),
      })) as { joinToken?: string; joinCommand?: string | null; expiresAt: string }
      if (!made.joinToken || !made.joinCommand)
        fail(
          'the control plane names no node endpoint for hosts (FLEET_JOIN_URL), so it has no line to print',
        )
      ca ??= JSON.parse(Buffer.from(made.joinToken.slice(4), 'base64url').toString()).h
      say(`# ${n.id} (${n.name}), until ${made.expiresAt}`)
      say(made.joinCommand)
    }
    console.error(
      `Paste each line on its node’s host, as root. It moves the node to the fleet CA the control plane runs now (sha256 ${ca}) and enrolls it again under its own id, servers and all. Treat the lines as passwords until they expire.`,
    )
    break
  }
  case 'nodes': {
    const listed = (await api('GET', `/nodes${options.has('all') ? '?all=true' : ''}`)) as {
      nodes: NodeView[]
    }
    if (options.has('json')) show(listed)
    else table(listed.nodes)
    break
  }
  case 'node':
    show(await api('GET', `/nodes/${node()}`))
    break
  case 'label': {
    const set = labelsOf(positional.slice(1), 'label')
    const remove = options.get('remove') ?? []
    if (Object.keys(set).length === 0 && remove.length === 0) fail('label <node> key=value… [--remove key]…')
    show(await api('POST', `/nodes/${node()}/labels`, { set, remove }))
    break
  }
  case 'drain':
  case 'undrain':
  case 'reinstate':
  case 'retire':
  case 'clear-quarantine':
    show(await api('POST', `/nodes/${node()}/${command}`))
    break
  case 'upgrades': {
    const upgrades = (await api('GET', '/upgrades')) as Upgrades
    if (options.has('json')) show(upgrades)
    else rollout(upgrades)
    break
  }
  case 'retry-upgrade':
    show(await api('POST', `/nodes/${node()}/retry-upgrade`))
    break
  case 'lost': {
    const fencedBy = option('fenced-by')
    const reason = option('reason')
    if (!fencedBy || !reason)
      fail('lost <node> --fenced-by <how it was stopped> --reason <why it is lost> [--force]')
    show(await api('POST', `/nodes/${node()}/lost`, { fencedBy, reason, force: options.has('force') }))
    break
  }
  case 'placements':
    show(await api('GET', `/placements${query({ node: option('node') })}`))
    break
  case 'placement':
    show(await api('GET', `/placements/${server()}`))
    break
  case 'move':
    show(await api('POST', `/placements/${server()}/move`, { to: option('to') ?? null }))
    break
  case 'events':
    show(
      await api(
        'GET',
        `/events${query({ node: option('node'), server: option('server'), limit: option('limit') })}`,
      ),
    )
    break
  case 'summary':
    show(await api('GET', '/summary'))
    break
  default:
    say(
      'bun scripts/fleet.ts ca|token|rotate-ca|add|remove|nodes|node|label|drain|undrain|lost|reinstate|retire|clear-quarantine|upgrades|retry-upgrade|placements|placement|move|events|summary',
    )
    say('See the top of scripts/fleet.ts, and docs/fleet-operations.md.')
    if (command !== 'help') process.exit(1)
}
