---
title: Host on your own machine
---

Supply [the shipped cloud-init user data](../../deploy/platforms/cloud-init/user-data.yaml)
to a Linux machine with persistent disk. It runs the existing installer and image startup once.
The checksum-pinned Ubuntu 24.04 and 26.04 paths pass [booted CI acceptance](../checks/k28.md#booted-machines).
No live provider deployment or public HTTPS installation is claimed.

On current main this server path needs root, systemd, Docker and the host firewall policy.
The automatic rootless server preset depends on the [any-host installer](../any-host.md).
The CLI's [one command](../index.md#one-command-to-install-the-cli) installs the client;
it does not imply those server prerequisites are present. The image uses container isolation
when that engine and policy are available; a requested unavailable capability is refused.

Keep the state directory. The current default service address is HTTP on loopback: use an SSH
tunnel to reach it. Public HTTPS requires your existing DNS/TLS setup and `PRYX_PUBLIC_URL`.
See [operations](../operations.md) and [the full machine setup](../hosting.md#a-machine-of-your-own).

Open the service address through that tunnel or your verified HTTPS endpoint. Welcome states
the first-start key location in the journal and the configured tier. Enter the key there,
then choose a [caller guide](../connect.md).
