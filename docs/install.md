<!-- SPDX-License-Identifier: FSL-1.1-ALv2 -->
# Install pryx

The installer supports Linux x86_64. It measures the host, selects a preset and
starts without questions. `pryx up --check` prints its plan without downloads or
configuration changes; `--preset user|container|server|node` overrides detection.
The first successful start prints the API URL, initial key, available tier and
capabilities that need host changes. Credentials are also saved privately.

## Laptop or VM without root

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
```

An ordinary account gets the user preset without sudo, Docker or a system manager.
The signed release bundle includes PostgreSQL, RustFS, control, edge, pryxd and its
agent. Data, configuration, state and runtime sockets follow absolute XDG paths
below home; defaults are `~/.local/share/pryx`, `~/.config/pryx`,
`~/.local/state/pryx` and `~/.cache/pryx-runtime/pryx`. An XDG path escaping home
is refused; the runtime falls back below home when `/run/user` is outside it.
The service listens on loopback port 7331, or another free unprivileged port when it is busy. Sandboxes use authenticated path aliases;
a domain is optional. Process isolation requires enforceable Landlock ABI 3. On compatible GNU hosts without it, the user preset selects and reports Wasm instead; explicitly selected unsupported backends are refused.

Where the systemd user manager and noninteractive linger authorization work, pryx
installs `~/.config/systemd/user/pryx.service` and starts it. Otherwise the command
stays in the foreground and explains how to keep it running after logout. In a
second terminal, run `~/.local/bin/pryx run -- echo hello`. New POSIX shell sessions
find `pryx` through an installed PATH block; linked, unsafe or read-only shell
profiles are left alone. The absolute command works in the current shell.
User sandboxes share your UID and have no enforced
resource limits: use them for your work, not strangers' code.

`pryx upgrade` authenticates the latest release and retains state and credentials.
With foreground supervision it stays attached to the upgraded service. Deployment
requirements are retained across user-manager restarts and upgrades; a backend
change requires an explicit data migration.
`pryx down` stops the service and its retained compute, keeping data.
`pryx down --purge` also removes the managed bundle, configuration, state, cache,
bootstrap, CLI shortcut, exact PATH block and user unit, prints the removed paths and reverses linger
only when this installation enabled it. Save backups before purging.

## Server with root

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sudo sh
```

A systemd host gets Docker, nftables, persistent database/object-store Docker volumes,
control/edge units and its enrolled local node. Debian and Ubuntu can install missing
packages automatically; other distributions need Docker and nftables installed and
receive their names before any provisioning. Server sandbox placement requires
private network/PID namespaces and enforced CPU/memory limits. The main daemon's
unit hardening remains in force. A domainless server uses loopback port 7331 and
path aliases; `pryx up --preset server --domain sandboxes.example.com` selects a
domain at first installation. Existing domains stay immutable. See
[operations](operations.md) for DNS, TLS, server upgrades and explicit data removal.

## Inside a container

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
```

An ordinary account still gets user. Root in a container gets container, with
managed files under `/opt/pryx`, `/etc/pryx-local`, `/var/lib/pryx` and
`/run/pryx-local`. Embedded PostgreSQL drops to its dedicated nonzero UID.
Process confinement must actually pass; missing kernel syscalls cause an actionable
refusal. A mounted Docker socket alone does not establish correct host bind paths
or firewall coverage. Use the server preset on the engine host for image workloads.

## Platform that runs images

```sh
docker run -p 8080:8080 -v pryx:/var/lib/pryx ghcr.io/cubepals/pryx:0.1.0
```

The same supervisor runs inside the release image. The first key appears once in
its log. Keep the named volume; configure a trusted TLS terminator for remote
access. See [deployment](deploy.md) for Render, external stores and image tiers.

## Node joining a fleet

```sh
sudo env PRYX_PRESET=node PRYX_JOIN_TOKEN='<fleet token>' sh -c 'curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh'
```

The node preset uses the existing root/systemd fleet join. It installs the verifier
and uses the supplied fleet token, verifying the signed daemon/agent bootstrap
independently of the fleet before joining.
The token is an operator credential: keep it out of shell history and CI output.
See [operations](operations.md) for issuing a token and node policy.

Set `PRYX_NO_UP=1` on the installer process to install only the CLI. No production
release is published by branch acceptance; preview builds use an isolated fixture
signing identity and cannot replace production release keys.
