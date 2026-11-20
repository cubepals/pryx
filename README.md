# pryx

Sandboxes on Linux hosts you own. [Compare capabilities and licences](docs/compare.md).

## On your own Linux machine

Install on x86_64 or aarch64 as your ordinary user.

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
```

The installer starts pryx and prints its URL, key and available capabilities.
Run `~/.local/bin/pryx run -- echo hello` in another terminal. The user preset
needs neither root nor Docker; its shared UID is for one person's work.
[Installation](docs/install.md) covers servers, containers, platforms and fleet nodes;
[operations](docs/operations.md) covers TLS, host policy, upgrades and recovery.

## On a server

Point the domain’s DNS A record at the server and allow inbound TCP 443. The installer starts
pryx and obtains a TLS certificate for its API. pryx renews it automatically.

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sudo PRYX_DOMAIN=sandboxes.example.com sh
```

Install `@cubepals/pryx` with your package manager and set `PRYX_API_URL` and
`PRYX_API_KEY` to the values the installer prints.

```ts
import { Pryx } from '@cubepals/pryx'
const { PRYX_API_URL: url, PRYX_API_KEY: key } = process.env
if (!url || !key) throw new Error('Set PRYX_API_URL and PRYX_API_KEY')
const pryx = new Pryx({ url, key })
console.log((await pryx.run('alpine:3.22', ['echo', 'hello'])).stdout)
```

Container: Docker isolation and images; choose it for untrusted workloads.
Process: native processes with host restrictions; no container image required.
Wasm: agentOS WebAssembly VMs with registry packages; no container image required.
VM: a Firecracker virtual machine with its own kernel, from an amd64 image, where an ordinary
user can open `/dev/kvm`; root hosts don't serve it yet. See [virtual machines](docs/vm.md).

[TypeScript](sdk/typescript/README.md) · [Python](sdk/python/README.md) ·
[Documentation](docs/README.md) · [Architecture](docs/architecture.md) ·
[Tier comparison](docs/tiers.md) · [Contributing](CONTRIBUTING.md).
Raw HTTP access is defined by the [OpenAPI document](control/openapi/pryx.openapi.json).

[FSL-1.1-ALv2](LICENSE.md); see [licensing](docs/protocol.md#format-names) for format history.

pryxd 0.1.0 continues blocklyd 0.3.x; 0.2.2 and earlier were AGPL-3.0-only.
