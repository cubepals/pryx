<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# Deploy the release image

```sh
docker run -p 8080:8080 ghcr.io/cubepals/pryx:0.1.0
```

The first successful start prints the initial API key once. Set PRYX_API_URL to
`http://localhost:8080` and PRYX_API_KEY to that key in the CLI or SDK environment.
`pryx run -- echo hello` uses the bundled userland. `pryx run --image alpine:3.22 -- echo hello`
is refused on the process tier with the image error documented in [process-tier.md](process-tier.md).
The image also contains agentOS VMs. Set `PRYX_DEFAULT_TIER=wasm` at image startup to use them
without an engine or added privileges. A usable Docker socket selects container automatically;
`PRYX_DEFAULT_TIER=container|process|wasm` overrides that choice. Sandbox requests select a
provided tier with `isolation`; they cannot enable a missing host runtime. Wasm refuses an image
and accepts optional `packages` registry names instead. See [tiers.md](tiers.md) for the measured
default and capabilities and [agentos.md](agentos.md) for persistence and ingress limits.
The [one-command installer](install.md) runs the same supervisor as an ordinary user or provisions a systemd server as root.

Docker creates the image's state volume automatically. For a named persistent volume:

```sh
docker run --name pryx -p 8080:8080 -v pryx:/var/lib/pryx ghcr.io/cubepals/pryx:0.1.0
```

`docker compose up --build` uses the root docker-compose.yml with a named state volume.
`docker compose logs pryx` shows the initial key on first start. A fresh image can use
`--user 1000:1000`; it then reports shared UID isolation and keeps that user's state in its
own directory in the volume. Restart with the same UID and volume. No privileged flag,
engine socket or seccomp override is needed for the process tier. Embedded Postgres requires
its private password on both TCP and Unix sockets.

The only public listener is HTTP port 8080 (or PORT). Set PRYX_PUBLIC_URL to the externally
reachable HTTP or HTTPS origin behind your platform's TLS terminator. Agent URLs use an
authenticated `/sandboxes/<id>/_pryx/` alias; exposed application ports use public
`/sandboxes/<id>/ports/<port>/` aliases. Applications using those aliases must support their
URL prefix. Declared HTTP ports use the public hostnames returned by the API; configure DNS
and preserve the Host header through the TLS terminator. The configured PRYX_PUBLIC_URL
hostname stays reserved for control routing, including when a declared hostname collides.
TLS and raw TCP fronts are disabled in the image. Standalone host edges
can still use wildcard DNS, certificates and raw fronts as documented in [agent.md](agent.md).

New image state derives its private node listener pool below the kernel's outgoing ephemeral
range without changing sysctls. Restart preserves the stored pool, including older implicit
pools. The image honors the edge's `PRYX_EDGE_*` admission and greeting settings while keeping
embedded service listeners private; normal limits remain the defaults.

Without DATABASE_URL the image starts private Postgres in the volume. With DATABASE_URL it
uses that existing Postgres and applies pryx's migrations. Without S3_ENDPOINT it starts a
private S3-compatible store in the volume. To use external S3, set S3_ENDPOINT, S3_BUCKET,
S3_ACCESS_KEY and S3_SECRET_KEY together; normal TLS certificate checks remain enabled.
Keep the state volume even with external stores: it also holds fleet identity, private
credentials and local sandbox data. Back up the database, object store and state together.

For Render, this repository includes render.yaml, a public Blueprint with an ordinary Docker
web service, persistent state disk and managed Postgres:

1. Open Render's **New → Blueprint**, connect your fork of this public repository, select the
   branch containing render.yaml, and apply the Blueprint.
2. Keep its Standard web-service plan and disk; the bundled service and a default sandbox need
   more memory than a 512 MiB instance. Render creates the Postgres add-on and supplies its
   connection string through the Blueprint's DATABASE_URL reference.
3. Wait for `/healthz` to become healthy, then read the first API key in the service log.
   The image uses Render's RENDER_EXTERNAL_URL automatically, so agent and port aliases use
   the service's verified HTTPS origin. Set PRYX_PUBLIC_URL explicitly when using a custom domain.
4. Point the CLI/SDK at the service origin with that key. Run `pryx run -- echo hello` and
   create sandboxes without an image. Keep the disk attached across deployments.

The process tier is Linux-only and requires the kernel policy described in process-tier.md.
The image fails closed when the platform cannot enforce it. Container-tier installations
add a usable host Docker socket and the host firewall policy; they require an operator's
host permissions and do not change the ordinary-container process proof.

On a fresh Linux host, `sudo pryx up --image --domain sandboxes.example.com` installs the same
release image as a systemd service. An explicit image reference may follow `--image`. This
operator path installs the host Docker/firewall policy and mounts its engine socket; its
container requires NET_ADMIN to apply that policy. It is separate from the ordinary process
start above. Repeated installation retains credentials. Existing binary installations must
first be stopped and their data migrated; the installer refuses an active binary deployment.

Host installation reserves its configured workload ports in Linux's
`ip_local_reserved_ports`, preserving existing reservations and writing a sysctl drop-in.
This prevents outgoing connections from taking a stopped sandbox's published port.
For an existing manually installed node, run `sudo pryxd reserve-ports --config /etc/pryxd/pryxd.toml`.
