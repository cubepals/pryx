# Desktop, browser, SSH and bucket tools

Native tools run on the **container** tier, as the sandbox user. The process and Wasm tiers
refuse these tools by name. They wrap existing programs; the agent owns scoped routes and the
edge owns access. `toolsVersion` is separate from the stable agent API version.

```sh
pryx template create desktop
pryx template create browser
pryx template create tools
```

Use a released version of the three prepared images, or `--image` for your built image.
The native-tools image contains OpenSSH, Mountpoint S3 and rclone, and the desktop dependencies;
the desktop image selects its X11 start command. The browser image uses the official Playwright
image as a donor, retaining only its open-source headless shell and FFmpeg, and publishes a
sanitized flattened filesystem. No published layer contains the donor's proprietary Widevine
module. Templates retain complete upstream notices and source URLs; the licence inventories
are in [checks/measurements](checks/measurements/).

Configure the node explicitly before scheduling these tools. Browser Chromium requires user
namespaces. FUSE additionally requires the real `/dev/fuse` character device. Install the
[host profile](../deploy/native-tools-seccomp.json) as a protected root-owned file:

```sh
sudo install -m 600 deploy/native-tools-seccomp.json /etc/pryx/native-tools-seccomp.json
```

```toml
[docker]
native_tools_seccomp = "/etc/pryx/native-tools-seccomp.json"
native_fuse = true
# On AppArmor hosts, load the provided profile in enforcing mode first.
native_tools_apparmor = "pryx-native-tools"
```

On an AppArmor host, also load `deploy/pryx-native-tools.apparmor` with
`sudo apparmor_parser -r deploy/pryx-native-tools.apparmor`. It is derived from Docker's Moby
profile and retains its other denials. The node requires the selected name to already be in
enforcing mode; `unconfined` and complain-mode profiles are refused. On hosts without AppArmor,
omit `native_tools_apparmor`. The default policy is unchanged.

The host seccomp profile wraps Playwright 1.65.0's Apache-2.0 seccomp profile and explicitly permits
mount/unmount and chroot syscalls. Chromium chroots inside its own user namespace; the kernel
still requires that namespace's CAP_SYS_CHROOT, which ordinary commands do not have.
FUSE uses a nested user/mount namespace; the sandbox never receives
SYS_ADMIN in the host or original Docker namespace. Only a mount worker retains that capability
inside its own namespace, sharing just the approved target. Ordinary commands/PTYs drop it.
A bounded 16-MiB private `/mnt` tmpfs holds mountpoint metadata while the image root stays read-only.
Native FUSE requires a non-root sandbox user; the prepared native templates use UID 1000.
Namespace setup runs only for native templates or creates with mounts. Ordinary images,
including root images, keep their normal bootstrap on a host that approves FUSE. Default hosts keep
all capabilities dropped, read-only root, no-new-privileges and their existing seccomp policy.
A request cannot select a host profile or a device. Hosts without this opt-in do not advertise
`native-fuse-mounts`, so placement refuses mounted requests before starting compute.

The host must permit nested user namespaces under its own LSM/AppArmor policy; this setting
does not turn off an LSM. A host that refuses that operation must be configured by its operator.
Chromium always starts with `chromiumSandbox: true`; there is no insecure fallback.

```ts
const box = await pryx.sandbox({ template: 'desktop', units: 2 })
const { url } = await box.desktop()
const png = await box.desktop.screenshot()
await box.desktop.click(100, 100)
await box.desktop.type('hello')
await box.desktop.key('Return')
await box.desktop.move(120, 100)
await box.desktop.scroll(0, 1)
```

The display defaults to 1280×720; set `PRYX_DESKTOP_WIDTH`/`PRYX_DESKTOP_HEIGHT` between 64 and
8192 in the template environment. Xvfb disables TCP. x11vnc binds only loopback, and websockify
binds loopback on 6080. `desktop()` exposes noVNC with a fresh random capability in the URL
fragment, valid for at most five minutes and no longer than the agent token. Minting another
URL rotates access. The WebSocket must present that token; websockify's TokenFile adapter also
checks wall-clock expiry. Requests are not logged with tokens. An authenticated MCP caller can
use `screenshot`, `click`, `type` and `key`; screenshots are native PNG image content.

```ts
const browser = await pryx.sandbox({ template: 'browser', units: 4 })
const { url: cdp } = await browser.browser()
// Official Playwright: chromium.connectOverCDP(cdp)
const page = await browser.browser.screenshot('https://example.com')
```

CDP remains on loopback 9222. The exposed 9223 HTTP/WebSocket proxy requires a random expiring
capability, delegates forwarding to http-proxy, and rewrites discovery for the edge URL prefix.
Keep the returned URL private. New connections fail after expiry/rotation. Existing browser/VNC
WebSockets can live until disconnection, as other authenticated streaming sessions do.
Screenshots require HTTP(S) URLs without embedded credentials and return bounded PNG bytes.
The browser's network access is the sandbox's own network policy.

```ts
const tools = await pryx.sandbox({ template: 'tools', idle: '10m' })
const { host, port, user, key, hostKey } = await tools.ssh()
// Pin hostKey and pass key to an SSH client in memory.
await tools.ssh.revoke()
```

`pryx sandbox ssh <id>` wraps OpenSSH, loads the one-time private key into a dedicated local
ssh-agent over stdin, pins the returned host key, and revokes on exit. It never writes the
client private key. Each call creates an independent Ed25519 key and listener. Raw SSH is
routed through the edge's hold/wake front with a session-bound preface. Raw slots are absent
from HTTP previews and disk-restored assignments. Revocation retires routes and closes sessions.
Keys expire at the earlier of the scoped capability, five minutes and **idle from issue**;
this conservative bound ensures a sandbox that sleeps past idle cannot revive access.
OpenSSH checks wall-clock public-key expiry even immediately after wake. A stop/new agent
invalidates old raw slots; request a new lease. Only public lease metadata reaches control.
Ordinary serialization of SDK SSH handles omits `key`.

```ts
const mounted = await pryx.sandbox({
  image: 'ghcr.io/cubepals/pryx-tools:<release-version>',
  mounts: [{ bucket: 's3://my-bucket/prefix', path: '/mnt/data', readOnly: false,
    credentials: { accessKeyId: process.env.S3_ACCESS_KEY!,
      secretAccessKey: process.env.S3_SECRET_KEY!, region: 'us-east-1',
      endpoint: 'https://storage.example.com' } }],
})
console.log(await mounted.mounts()) // source/path/backend/state, never credentials
```

A mounts-only create uses the prepared tools image when no source is supplied; production
operators should pin the released image explicitly. Mountpoint S3 is preferred; an image with
rclone but without Mountpoint uses rclone mount. Both require working FUSE and obey the same host
policy. Their normal filesystem semantics apply: Mountpoint does not promise full POSIX behavior;
rclone buffers writes in a bounded private temporary cache. The agent refuses unsupported tiers
and unavailable workers instead of emulating a mount. Paths must be below `/mnt` or persistent
data and pass the descriptor-relative no-follow walk. Roots and agent state cannot be covered.

Control seals credentials with Node's AES-256-GCM and the operator seal key, binding ciphertext
to sandbox/project. Specs, logs, listings, operation responses and data archives contain no mount
credentials. The agent receives them over its scoped transport and passes them only in mount
worker environment, with ambient AWS config/metadata providers disabled. It serves health while
holding the entrypoint until all initial mounts are ready. A failed create is destroyed; a dead
worker is reported and requires an explicit restart, never an implicit remount. The running-state hook initializes mounts on start/wake, and mounted-only maintenance handles autonomous
agent restarts. Ready agents receive no repeated credential delivery. Mounted snapshots require explicit credentials
to fork. Deletion erases sealed credentials. Preserve the operator seal key across control restarts.

Mount workers run as the sandbox's own uid, so the image installs `mount-s3`, `rclone` and
`fusermount3` execute-only (mode 0111). The kernel then leaves a worker non-dumpable after exec,
and sandbox commands, which hold no capability, cannot read its `/proc` environment or memory or
trace it; the credentials never appear in its command line. A worker refuses to start a readable
helper, so a custom image that installs one readable fails its mount instead of exposing the
credentials. This does not hold against a same-uid process with `CAP_SYS_PTRACE`, which no sandbox
command is given.

Python exposes the same callable handles in sync and async clients: `box.desktop()`,
`box.desktop.screenshot()`, `box.browser()`, `box.browser.screenshot(url)`, `box.ssh()`,
`box.ssh.revoke()` and `box.mounts()`; await them in an async context. Credentials use the public
camelCase mount fields inside the `mounts` list. See [speed](speed.md) for measured readiness.
