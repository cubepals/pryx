<!-- SPDX-FileCopyrightText: 2026 The Cubepals Authors -->
<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->

# Operating a fleet

How to run the `fleet` runtime: Blockly's servers on Linux hosts it manages directly, each with
[blocklyd](https://github.com/cubepals/blocklyd), which is its own repository. Why it works this way is [fleet.md](fleet.md).

Everything an operator does goes through `bun scripts/fleet.ts` (the operator API on the control
plane's internal listener) or the node itself. Set, on the machine you operate from:

```sh
export OPERATOR_API=http://<control plane's internal address>:<internal port>   # private network only
export OPERATOR_TOKEN=…              # from the deployment's secrets; never on a node
export OPERATOR=<your name>          # what the ledger records as who acted
```

A fleet usually starts beside the cloud runtime the deployment already runs, as a canary
([runtimes.md](runtimes.md)): `RUNTIME_PROVIDERS=fly,fleet`, a few servers moved to it or a share of
new ones sent there by a rule, and `bun scripts/runtimes.ts report` to see what it ran.

## Contents

1. [A new deployment](#1-a-new-deployment)
2. [Adding a node](#2-adding-a-node)
3. [Day to day](#3-day-to-day)
4. [A node stops answering](#4-a-node-stops-answering)
5. [Declaring a node lost](#5-declaring-a-node-lost)
6. [A lost node comes back](#6-a-lost-node-comes-back)
7. [Retiring a node](#7-retiring-a-node)
8. [Certificates](#8-certificates)
9. [Upgrading blocklyd](#9-upgrading-blocklyd)
10. [Disk](#10-disk)
11. [Two hosts with one identity](#11-two-hosts-with-one-identity)
12. [What to watch](#12-what-to-watch)
- [Appendix: adding a node by hand](#appendix-adding-a-node-by-hand)

## 1. A new deployment

1. **A private network** joining the control plane, the edge and every future node (WireGuard, or
   the provider's private network). Nothing below needs a public address on a node. For nodes on
   Hetzner Cloud, the environment's `fleet_hetzner.network_id` names it; left out, the stack makes
   one (10.0.0.0/16, a subnet in each network zone its nodes are in), which the control plane and
   the edge must then reach.
2. **The fleet CA**, once per deployment:

   ```sh
   bun scripts/fleet.ts ca production ./fleet-ca
   ```

   Put `ca.pem` and `ca.key` into the deployment's secrets as `FLEET_CA_CERT` and `FLEET_CA_KEY`
   (PEM; `\n` escapes are accepted), then delete `ca.key` from disk. `ca.pem` is public: every node
   gets it.
3. **An operator token**: `openssl rand -base64 48`, as `OPERATOR_TOKEN`.
4. **The control plane's configuration** ([configuration.md](configuration.md#fleet)): `fleet` in
   `RUNTIME_PROVIDERS` beside the default runtime (or `RUNTIME_PROVIDER=fleet` for a fleet alone),
   an archive store (required: backups and moves go through it), `FLEET_ENDPOINT_HOSTS` (the names
   and addresses nodes dial the node endpoint by, for its certificate), and `FLEET_REGION_MAP` from
   product regions to fleet regions (`eu:fsn1`).
5. Deploy. The api role listens for nodes on `FLEET_NODE_LISTEN` (8443); open it on the private
   network only.

## 2. Adding a node

**On Hetzner Cloud**, from where you operate, with what an apply of the environment needs
([configuration.md](configuration.md#applying-an-environment)) and `HCLOUD_TOKEN`:

```sh
bun scripts/fleet.ts add fsn1 --type ccx33 --price-cents <cents> --dir infra/terraform/environments/production
```

It mints a token for the fleet region `fsn1` with the node's price on it (`monthly_cost_cents`,
from `--price-cents`: what the type costs a month, in whole US cents), lists the node in the
environment's `fleet-nodes.auto.tfvars.json` (as `fsn1-ccx33-1`, unless `--name` says otherwise), and runs `terraform apply` for that node alone,
which shows its plan and asks before it buys anything (`--yes` doesn't ask). The server
([`modules/fleet-node`](../infra/terraform/modules/fleet-node/main.tf)) is on the private network,
behind a firewall that admits nothing from the internet (`fleet_hetzner.edge_sources` opens the
game ports to an edge out there) and has no outbound rules, so what the node and its servers
dial ([fleet.md §11](fleet.md#11-networking-and-the-edge)) is open, and runs the line below at
first boot. `add` waits until the node beats `healthy` and says so. Commit the tfvars change.

**An edge on Fly reaching Hetzner nodes over the internet.** Every edge machine dials a node from
its region's egress addresses, and the firewall lets in only what `edge_sources` lists. Without
egress addresses of its own, a Fly machine dials out from its host's address, which changes when
the machine moves, so give the edge app a pair in every region `edge_regions` lists, and list them
all:

```sh
fly ips allocate-egress --app bly-prod-edge --region fra   # and again for iad, and each region
fly ips list --app bly-prod-edge                             # the egress addresses, by region
```

In `config.auto.tfvars.json`, `"fleet_hetzner": { "edge_sources": [...] }` holds each of them as
`<v4>/32` and `<v6>/128`. New egress addresses take a few minutes to reach machines already running. A region added to `edge_regions` later needs its own pair listed before
the apply that adds it: `production.ts apply` checks, and stops before Terraform with what is
missing, whenever `edge_sources` lists anything.

The Hetzner location is the region's name when that is one (`--location` otherwise). The join
line is spent at first boot, so later applies need it no more and never rebuild the host for it.

`bun scripts/fleet.ts remove <node>` takes one back out: it drains the node, waits until its
servers have moved off (or lists the ones that haven't), retires it, takes its entry out and
deletes the server.

**On any other host**, the line below.

**The host.** A Linux machine on the private network, with an empty `/etc/machine-id` in its image
(so each host makes its own at first boot) and a firewall that allows the node's API and game ports
only from the private network. Outbound, the host and its servers need HTTPS to the node
endpoint, the artifact endpoint (`ARTIFACTS_RUNTIME_FACING_URL`), the archive store's presigned
host, and Mojang and the mod hosts ([fleet.md §11](fleet.md#11-networking-and-the-edge) says
which reaches what, and why); a firewall that filters outbound traffic allows them. XFS with
reflink (the default for `mkfs.xfs`) for `/var/lib/blocklyd` makes snapshots free until the world
changes, though each still needs room for a full copy above the floor; ext4 works, with full
copies.

**The line**, from where you operate:

```sh
bun scripts/fleet.ts token fsn1 --label monthly_cost_cents=<cents>
```

It prints one line to paste on the host, as root:

```sh
d=$(mktemp -d) && curl -fsSk 'https://10.0.0.2:8443/fleet/v1/ca.pem' -o "$d/ca.pem" && echo "<sha256 of the CA>  $d/ca.pem" | sha256sum -c --quiet && curl -fsS --cacert "$d/ca.pem" 'https://10.0.0.2:8443/fleet/v1/join.sh' | sh -s -- bk1.… "$d/ca.pem"
```

It joins one host to the fleet region `fsn1` (what `FLEET_REGION_MAP` maps `eu` to; a region it
doesn't map to is refused), within an hour (`--ttl`, at most a week). Treat it as a password until
then. On the host it:

1. fetches the fleet CA from the node endpoint and stops unless it matches the hash in the line:
   nothing runs before that;
2. fetches `join.sh` over TLS checked against that CA, which installs Docker from Docker's apt
   repository if it is missing (Debian and Ubuntu), gives Docker
   [`daemon.json`](../apps/control/src/infra/fleet/daemon.json) if it has none (`live-restore` so a
   Docker restart doesn't stop servers; inter-container traffic off; bounded logs), and installs
   the blocklyd the control plane serves, once its sha256 matches, at
   `/var/lib/blocklyd/bin/blocklyd` with `/usr/local/bin/blocklyd` linked to it, where it can
   [upgrade itself](#9-upgrading-blocklyd);
3. runs `blocklyd join`, which writes `/etc/blocklyd/blocklyd.toml`, the CA, the token (0600) and
   the unit [`deploy/blocklyd.service`](https://github.com/cubepals/blocklyd/blob/main/deploy/blocklyd.service), starts blocklyd,
   and prints what it worked out from the host:

```text
Worked out from this host; set any of them in /etc/blocklyd/blocklyd.toml to override:
  api.listen = "10.0.0.5:7443"    (the host's private address, on enp7s0)
  network.edge_ips = ["10.0.0.5"]    (the host's private address, on enp7s0)
  network.control_ips = ["10.0.0.5"]    (the host's private address, on enp7s0)
  capacity.reserved_memory_mb = 4096    (a sixteenth of the host's 64000 MB, and at least 2048)
```

The address is the host's one private address: RFC 1918, 100.64.0.0/10, or a WireGuard
interface's. A host with several, or none, is asked which before anything is written: paste the
line again with `--address <ip>` added to its end. The configuration holds only the deployment and
the endpoint, so the rest is worked out again at each start, and anything you set in it wins
([examples/blocklyd.toml](https://github.com/cubepals/blocklyd/blob/main/examples/blocklyd.toml) lists every key). Pasting the
line on a host that is already a node of this deployment changes nothing.

`blocklyd doctor` says what is wrong with a host and what to do about each thing: Docker and its
settings, the state directory, the ports, the machine id, the clock, the listeners. On a host with
no servers yet, `blocklyd doctor --fix` makes only safe changes (daemon.json merged over the one
there, kept as `.bak`; the state directory) and prints how to undo each. The unit runs
`doctor --preflight` before every start, which refuses only an invalid config or a state directory
blocklyd can't own, and logs the rest as warnings.

blocklyd makes its key, enrolls, deletes the token, and beats. `bun scripts/fleet.ts nodes` shows
it `active` and `healthy` within seconds, and placement uses it at once.

One node is a complete deployment. More are the same `add` or the same line, in the same or other
regions. A host set up by hand is the [appendix](#appendix-adding-a-node-by-hand).

## 3. Day to day

```sh
bun scripts/fleet.ts nodes                 # lifecycle, health, last beat, servers and memory running/placed, disk, version
bun scripts/fleet.ts node <node>           # one node: its placements and recent events
bun scripts/fleet.ts placements --node <node>
bun scripts/fleet.ts placement <server>    # where a server is, its epoch, its history, what its node says of it
bun scripts/fleet.ts events --server <server>
bun scripts/fleet.ts summary
bun scripts/fleet.ts upgrades              # the blocklyd rollout: each node, its version, its state (§9)
```

A node's report of a server carries what it found wrong and can't mend itself, under `issues`: a
restart after a failure it refused for want of room (`insufficient_capacity`; the server stays
crashed until a start fits), or data over the size its plan promised (`over_storage`).

`summary`'s `endpoint` figures (heartbeats, refusals, latency) are those of the api process that
answered, since it started: every api process serves the node endpoint and counts only what it
served. With several, nodes spread across them, and each call may reach a different one.

- **Drain** a node before maintenance that will take it away for long: `drain <node>`. Nothing new
  is placed there, and the application's relocation sweep moves its servers off, up to three a
  minute: stopped ones at once, running ones only while nobody plays on them, each once a node has
  room for it. A server whose move was refused or failed is tried again 15 minutes later, then
  twice as long after each refusal in a row, up to a day; one its node can't send (too big for one
  upload, on a node without parts) stays, keeps the node from emptying, and says so each time
  (`placement.move_declined`).
- **Move** a server: `move <server> [--to <node>]`. As for any relocation, the application
  snapshots it while it runs, then it is stopped, moved and started again where it landed. It
  moves at once, even with players on it. A move with no room anywhere is refused (`RuntimeFull`)
  before anything stops: the server stays where it is, running if it ran, and the move's operation
  says why. Ask again once there is room. A world larger than one upload to the archive store
  carries moves in parts. Only one whose node can't send parts (blocklyd without the
  `multipart-upload` feature: upgrade it) can't move between nodes at all: the move is declined
  (`placement.move_declined`), the request dropped, and the server runs again where it was.
- **Undrain**: `undrain <node>`.
- **A node's labels**, its price among them: `label <node> monthly_cost_cents=<cents> [key=value]…
  [--remove key]…`. A node's labels come from its configuration and its token at enrollment;
  neither its heartbeats nor a re-enrollment change them, so correct a price here when the
  provider's changes. `monthly_cost_cents` is a whole number of US cents. The cost views use it at
  once (other processes within seconds), and `node.labels_changed` records the labels before and
  after, and who changed them.
- **A full node** needs nothing from you. Stopped servers hold no memory; a server asleep with
  its memory holds it as a running one does, and a reboot of its node drops what it held. When one
  wakes and the servers running on its node leave no room, it moves to a node in its region with
  room before it starts (`placement.moved_for_room` in `events`). If that happens most evenings,
  the region needs another node, or a lower `FLEET_MEMORY_OVERCOMMIT`.

A **reboot** needs none of this: servers running when the host goes down start again once it is
back and has heard from the control plane. Expect a minute or two of downtime per server.

## 4. A node stops answering

It reads `suspect` after 15 s and `unavailable` after 45 s. Its servers read `unknown` to the
application, which waits; nothing is moved or rebuilt on its own.

1. `bun scripts/fleet.ts node <node>`: `lastProbe` says whether the host answers TCP on the API
   port (blocklyd is down) or not at all (the host or the network is).
2. Look at the host through the provider (console, power state, network).
3. If the host is up: `journalctl -u blocklyd` (each start logs doctor's warnings), `blocklyd doctor`
   for what is wrong now, `systemctl restart blocklyd`. Servers kept running regardless.
4. If it is rebooting: wait.
5. If it is gone, or won't be back soon: section 5.

## 5. Declaring a node lost

This is the one-way door: the node's servers are rebuilt elsewhere from their last uploaded
snapshots, and anything played after those is lost unless the host comes back.

**First make sure the host can't run anything.** A node that is only cut off keeps its servers
running, and players connected to them would play a fork. Fence it in a way that holds:

- power it off in the provider's console or API, where that stays off (Hetzner Cloud, Vultr);
- or boot it into the provider's rescue system (Hetzner dedicated; a reset alone isn't enough);
- or remove it from the private network and the edge's reach (OVHcloud, whose power-off is emulated
  and slow).

Then:

```sh
bun scripts/fleet.ts lost <node> --fenced-by "powered off in the Hetzner console" --reason "disk failure"
```

A node that still beats is refused unless `--force`. Its servers become `displaced`. No sooner
than ten minutes after that, the application rebuilds each on another node from its newest
uploaded snapshot, up to three a minute, and emails its owner what was lost
(`server.rebuilt_from_backup`). With no other node to go to, they wait until there is one with
room: the sweep asks every minute, and rebuilds each as soon as it can.

## 6. A lost node comes back

When it beats again, its copies of servers that were rebuilt elsewhere are **fenced**: stopped,
kept, never started. Each one is reported (`fork.detected`) without looking at its data: it may
hold play newer than the rebuild, which its owner may want. Its copies of snapshots the
application deleted while it was away are deleted from it then; nothing of its servers is.

- **Its servers weren't rebuilt yet** (no other node, or you were quick): `reinstate <node>`. They
  are placed there again, and resume.
- **They were rebuilt**: copy anything an owner asks for out of the fenced copies
  (`/var/lib/blocklyd/workloads/<server>/data` on the host), then retire the node (section 7) or
  reinstate it empty.

## 7. Retiring a node

`retire <node>` takes a node out of the fleet for good: it must hold no placements (drain it and
move its servers first). Its certificates stop working at once, and the fleet never asks it for
anything again, so whatever it still holds (copies of deleted snapshots among it) goes only with
the host. Wipe the host before reusing it; a reinstalled host enrolls as a new node.

## 8. Certificates

Renewal is automatic: within ten days of their end, nodes make a new key and get new certificates
over their current ones. `node <node>` and `nodes --json` show when a node's certificates end
(`certificateExpiresAt`).

An enrollment whose answer was lost on the way needs nothing either: the node asks again with the
same token and key until the token expires, and gets its certificates (`node.enrollment_repeated`).
Past the token's expiry, give it a new one.

A node offline past its certificates' end can't renew, and one whose `/var/lib/blocklyd/identity`
was lost can't prove who it is. Either way, **re-enroll it under its own id**, so it keeps its
servers (their containers carry the node's id, which is how blocklyd recognises them). Where you
operate:

```sh
bun scripts/fleet.ts token --node <node>
```

It prints one line. Paste it on that node's host, as root. It is the line that adds a node
(section 2), and its token names the node, so `blocklyd join` knows this host is that node and:

1. runs `blocklyd doctor --preflight`, shows what it finds, and stops on a failure;
2. stops blocklyd and moves `identity` to `identity.old-<time>`;
3. writes the token, and the fleet CA when the token names another one than the host has;
4. starts blocklyd, waits for the first heartbeat the control plane accepts, then removes
   `identity.old-*`.

Like every pasted line, it first installs the blocklyd the control plane serves.

If it can't enroll within two minutes, it puts the old identity (and CA) back, starts blocklyd on
them, keeps what the attempt left as `identity.failed-<time>`, and says so: see
`journalctl -u blocklyd`, then paste the line again, or a new one if this one was used. If it
enrolled but its first heartbeat is late, it keeps the new identity, since enrolling ended the old
certificates, leaves the old one in `identity.old-<time>`, and says so; blocklyd keeps beating on
its own, and `node <node>` shows when it is healthy. A host
that is another node is refused, and told which line it needs. A host whose identity is gone
entirely takes the line too: with nothing to swap, it joins as on a new host, under the node's id.

The node gets new certificates for a new key under the same id; the old ones stop working at once
(`node.reenrolled`, which says whether it is the same machine). Its placements, lifecycle and
history are unchanged, and its servers are adopted where they are. Its session starts afresh, and
any quarantine is cleared. A host reinstalled with an empty disk is different: it has none of its
servers' data, so it joins with an ordinary token as a new node, and the old one is declared lost
(section 5).

### Replacing the fleet CA

When its key may have leaked. The control plane holds one CA at a time, and a node trusts only the
one it was given, so a new CA cuts every node off from the control plane until that node enrolls
again under it. Servers keep running throughout, but meanwhile their nodes don't report (the
application reads their servers `unknown`), nothing is placed or moved there, and once their lease
runs out nodes restart nothing on their own. Do it as maintenance, with the hosts at hand:

1. A new CA, where you operate: `bun scripts/fleet.ts ca <deployment> ./fleet-ca-new`.
2. Its `ca.pem` and `ca.key` replace `FLEET_CA_CERT` and `FLEET_CA_KEY` in the deployment's secrets;
   delete the key file, and deploy. From then on the node endpoint and the control plane's own
   client certificate come from the new CA, and every node's heartbeats fail.
3. `bun scripts/fleet.ts rotate-ca` prints one line per node that isn't retired, each under a
   comment naming its node.
4. Paste each line on its node's host, as root. The token names the new CA, so `join` fetches it,
   checks it against the token's hash, puts it where `fleet.ca` points, and re-enrolls the node as
   above; if it can't enroll, the old CA comes back with the old identity.
5. `bun scripts/fleet.ts nodes` until every node is `healthy`.

What isn't supported: trusting the old and the new CA side by side, so that nodes could move over
one at a time without losing contact (the control plane loads one CA, and a node's identity holds
one); and moving to a new CA through renewal, since a node refuses a renewal answer naming another
CA. A node that can't be reached to re-enroll stays cut off until it is.

## 9. Upgrading blocklyd

Deploying the control plane upgrades the fleet. Its image carries the blocklyd release it pins
(`bun scripts/blocklyd.ts version`), and the node endpoint offers it, in heartbeat answers, to the
nodes on an older version: one node per region at a time, regions side by side. So a rollout starts
when a commit that moves the pin (`bun scripts/blocklyd.ts bump <version>`) is deployed; nodes
already on it are left as they are. A node offered it:

1. takes the offer only with the release's signature (`blocklyd.sig`, beside the binary in the
   image) and only of a newer version, if it lists `signed-upgrade`; downloads it over its own
   mutual TLS and keeps it only if its sha256 is the one offered and it runs, saying it is the
   version offered;
2. keeps the binary it ran as `/var/lib/blocklyd/bin/blocklyd.prev`, puts the new one in its place
   (`/usr/local/bin/blocklyd` links to it), and restarts into it. Servers keep running;
3. is on trial: the new blocklyd must reconcile and have a heartbeat accepted within two minutes.
   If it doesn't, or can't start at all, the one before is put back, and the node reports why.

The next node in the region is offered it once this one beats `healthy` on the new version. A node
that fails, or isn't healthy on the new version 15 minutes after the offer, stops its region's
rollout; other regions carry on. Draining, lost and quarantined nodes are left out until they are
active again.

```sh
bun scripts/fleet.ts upgrades
```

lists each node with its version and its state: `current`, `upgraded`, `offered`, `waiting`,
`failed` (and why), or `skipped` (and why). After a failure, read why in `events --node <node>` and
the host's journal, fix it, and `bun scripts/fleet.ts retry-upgrade <node>` offers it again, which
resumes the region. The events are `node.upgrade_offered`, `node.upgraded`, `node.upgrade_failed`
and `node.upgrade_retried`. `FLEET_UPGRADES=off` ([configuration](configuration.md)) offers
nothing; nodes keep what they run.

On a host, `sudo blocklyd upgrade` does the same at once, without waiting for its turn, and
restarts into it (`--no-restart` leaves the restart to you). To go back by hand, set
`FLEET_UPGRADES=off` first, or the newer one is offered again, then
`mv /var/lib/blocklyd/bin/blocklyd.prev /var/lib/blocklyd/bin/blocklyd && systemctl restart blocklyd`.

**A node from before blocklyd upgraded itself** is `skipped` ("it upgrades by hand"), once. Fetch
the new binary and its sha256 from the node endpoint (`curl --cacert /etc/blocklyd/fleet-ca.pem`
`…/fleet/v1/blocklyd` and `…/fleet/v1/blocklyd.sha256`, then `sha256sum -c`), and:

```sh
install -D -m 0755 blocklyd /var/lib/blocklyd/bin/blocklyd
ln -sfn /var/lib/blocklyd/bin/blocklyd /usr/local/bin/blocklyd
cp blocklyd.service /etc/systemd/system/blocklyd.service   # the release's, for its ExecStopPost
systemctl daemon-reload && systemctl restart blocklyd
```

**A node that joined before blocklyd 0.3.5** keeps a unit without `blocklyd isolate`, the rules
that keep its servers off the host and off each other's ports ([fleet.md
§12](fleet.md#12-isolation)): `join` writes the unit once, and neither an upgrade nor joining
again (§8) rewrites it. Once the node runs 0.3.5, on its host, as root, with the release's
`blocklyd.service` (a file of [its release](https://github.com/cubepals/blocklyd/releases/tag/v0.3.5)):

```sh
apt-get install -y nftables
cp blocklyd.service /etc/systemd/system/blocklyd.service
systemctl daemon-reload && systemctl restart blocklyd
```

The unit then puts the rules in place before every start, and doesn't start without them.

From then on it upgrades itself. If the control plane's new version needs a feature (see
`features` in [protocol.md](https://github.com/cubepals/blocklyd/blob/main/docs/protocol.md)), it places only on nodes that have
it, and older nodes keep their servers until they upgrade.

## 10. Disk

Each node keeps `min_free_disk_mb` (5 GB) free: creates, snapshots, exports and restores are
refused below it (`insufficient_disk`); running servers are untouched. Space goes to worlds, local
snapshot copies (one or two per server: older ones go once uploaded), and the trash (deleted data,
kept 24 hours). `nodes` shows free space; `/var/lib/blocklyd/trash` can be emptied by hand in an
emergency.

## 11. Two hosts with one identity

`node.duplicate_identity` and a `quarantined` node mean two hosts beat with one identity, usually
a disk image copied after enrollment. Nothing new goes there. Find the copy (the event names both
sessions), stop its blocklyd and wipe its `/var/lib/blocklyd/identity`, then
`clear-quarantine <node>`.

## 12. What to watch

From `events` and each node's `/metrics` ([fleet.md §14](fleet.md#14-observability)):

- a node `unavailable` for more than five minutes;
- `blocklyd_fleet_lease_remaining_seconds` at 0 on an active node (it can't hear the control
  plane);
- `snapshot.upload_failed`. With `code: archive_too_large` it is a world bigger than one upload
  to the store carries, on a node that can't send it in parts (blocklyd without the
  `multipart-upload` feature): its backups stay on its node and it can't move between nodes, so it
  is lost with its host until the node is upgraded;
- uploads in parts left behind. One the control plane began and couldn't finish is aborted at
  once; one a crash cut short isn't, and its parts are stored, and billed, until the bucket aborts
  incomplete multipart uploads: R2 does after 7 days by default, and a lifecycle rule makes that
  sooner (`wrangler r2 bucket lifecycle add <bucket> --abort-multipart-days 1`);
- `placement.abandoned` with `code: never_made`: a new server's placement its node never got, let go
  after 30 minutes. Now and then is a network blip; often is a node or path to look at;
- any `fork.detected`, `copy.orphaned`, `epoch.conflict`, `node.duplicate_identity`;
- free disk near the floor. Worlds stay on their nodes while they sleep, so disk fills before
  memory, though servers asleep with their memory hold that too;
- `placement.moved_for_room` most evenings, or starts refused with `RuntimeFull`: the region is
  short of room to run;
- certificates within three days of their end.

## Appendix: adding a node by hand

What the line in [§2](#2-adding-a-node) does, step by step, for a host it can't run on.

**The host.** A Linux machine with:

- Docker Engine, configured with [`daemon.json`](../apps/control/src/infra/fleet/daemon.json)
  (`live-restore` so a Docker restart doesn't stop servers; inter-container traffic off; bounded
  logs). XFS with reflink (the default for `mkfs.xfs`) for `/var/lib/blocklyd` makes snapshots
  free until the world changes, though each still needs room for a full copy above the floor;
  ext4 works, with full copies.
- An empty `/etc/machine-id` in the image, so each host makes its own at first boot.
- Its private address, and a firewall that allows the node's API and game ports only from the
  private network, and lets the host and its servers out over HTTPS to the node endpoint, the
  artifact endpoint (`ARTIFACTS_RUNTIME_FACING_URL`), the archive store's presigned host, and
  Mojang and the mod hosts ([fleet.md §11](fleet.md#11-networking-and-the-edge)).

**blocklyd.** The static binary at `/var/lib/blocklyd/bin/blocklyd`, linked from
`/usr/local/bin/blocklyd` so it can [upgrade itself](#9-upgrading-blocklyd), checked against its
`.sha256`: the
node endpoint serves the release the control plane pins, at `/fleet/v1/blocklyd` and
`/fleet/v1/blocklyd.sha256` (`curl --cacert` with the fleet CA's `ca.pem`). Then the unit
[`deploy/blocklyd.service`](https://github.com/cubepals/blocklyd/blob/main/deploy/blocklyd.service), and
`/etc/blocklyd/blocklyd.toml`:

```toml
deployment_id = "production"

[api]
listen = "10.0.0.5:7443"                       # the private address

[network]
edge_ips = ["10.0.0.5"]                         # where the edge reaches game ports
control_ips = ["10.0.0.5"]                      # where the control plane reaches consoles

[capacity]
reserved_memory_mb = 2048                       # kept for the host; left out, a sixteenth of it

[fleet]
url = "https://10.0.0.2:8443"                   # the node endpoint
ca = "/etc/blocklyd/fleet-ca.pem"
enrollment_token_file = "/var/lib/blocklyd/enrollment-token"
labels = { monthly_cost_cents = "<cents>", provider = "hetzner" }
```

([examples/blocklyd.toml](https://github.com/cubepals/blocklyd/blob/main/examples/blocklyd.toml) lists every key.)

**The token**, from where you operate:

```sh
bun scripts/fleet.ts token fsn1 --label monthly_cost_cents=<cents> --out token.txt
```

It enrolls one node into the fleet region `fsn1` (what `FLEET_REGION_MAP` maps `eu` to), within an
hour (`--ttl`, at most a week). `--out` writes the `bk1.` form, which names this deployment and
its CA too, so blocklyd checks both before it enrolls; a bare token works as well. Copy it to the
host as `/var/lib/blocklyd/enrollment-token` (mode 0600), with the CA certificate, then:

```sh
blocklyd check-config --config /etc/blocklyd/blocklyd.toml
systemctl enable --now blocklyd
```

blocklyd makes its key, enrolls, deletes the token, and beats. `bun scripts/fleet.ts nodes` shows
it `active` and `healthy` within seconds, and placement uses it at once.
