# pryx

Sandboxes on Linux hosts you own. [Compare capabilities and licences](docs/compare.md).

Install on Linux x86_64 with systemd:

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
sudo pryx up --domain sandboxes.example.com
pryx run --image alpine:3.22 -- echo hello
pryx sandbox create --image alpine:3.22 --idle 5m
```

`up` saves your API credential privately. Point sandbox DNS at the edge;
[operations](docs/operations.md) covers TLS, host policy, upgrades and recovery.

Install `@cubepals/pryx` with your package manager, then set the API URL and key:

```ts
import { Pryx } from '@cubepals/pryx'
const { PRYX_API_URL: url, PRYX_API_KEY: key } = process.env
if (!url || !key) throw new Error('Set PRYX_API_URL and PRYX_API_KEY')
const pryx = new Pryx({ url, key })
const box = await pryx.sandbox({ image: 'node:22', idle: '10m' })
try {
  await box.write('hello.js', 'console.log("hello")')
  const result = await box.run('node /data/hello.js')
  console.log(result.stdout); if (result.code !== 0) throw new Error('Command failed')
} finally { await box.delete() }
```

Container: Docker isolation and images; choose it for untrusted workloads.
Process: native processes with host restrictions; no container image required.
Wasm: agentOS WebAssembly VMs with registry packages; no container image required.

[TypeScript](sdk/typescript/README.md) · [Python](sdk/python/README.md) ·
[Documentation](docs/README.md) · [Architecture](docs/architecture.md) ·
[Tier comparison](docs/tiers.md) · [Contributing](CONTRIBUTING.md).
Raw HTTP access is defined by the [OpenAPI document](control/openapi/pryx.openapi.json).

[FSL-1.1-ALv2](LICENSE.md); see [licensing](docs/protocol.md#format-names) for format history.

pryxd 0.1.0 continues blocklyd 0.3.x; 0.2.2 and earlier were AGPL-3.0-only.
