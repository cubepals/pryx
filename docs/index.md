---
title: pryx
description: Sandboxes on Linux hosts you own.
template: splash
hero:
  title: Sandboxes on Linux hosts you own
  tagline: Run, sleep, snapshot and connect with the isolation your host can enforce.
  actions:
    - text: Host pryx
      link: hosting/
    - text: Connect your code
      link: connect/
      variant: secondary
---

## One command to install the CLI

```sh
curl -fsSL https://github.com/cubepals/pryx/releases/latest/download/install.sh | sh
```

The current installer supports Linux x86_64. Running the current one-box server on your own
machine requires root, systemd and its container runtime. Managed-service images can use the
process tier without a nested engine when their kernel enforces its policy.
[Choose a host](hosting.md) for the exact prerequisites and the welcome page.

## Host where your state lives

[Deploy on Render](https://render.com/deploy?repo=https://github.com/cubepals/pryx) ·
[Railway](host/railway.md) · [Fly.io](host/fly.md) ·
[Koyeb](host/koyeb.md) · [Coolify](host/coolify.md) ·
[A machine of your own](host/own-machine.md)

Render builds the source. The other links prepare the existing platform files; no catalog
publication is implied. Keep the persistent volume. Read the reported tier before requesting
images, browsers, desktops or virtual machines.

## One API, several ways in

Use TypeScript, Python, the CLI, GitHub Actions, MCP or HTTP. pryx keeps scheduling in control
and host policy on the node. A named isolation or capability is enforced or refused.

[Explore the API](api.md) · [Compare tiers](tiers.md) · [Read the documentation](README.md)
