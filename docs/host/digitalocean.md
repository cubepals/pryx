---
title: Host on DigitalOcean
---

[Use a rented machine](own-machine.md).

App Platform has no persistent volumes. Its three investigated storage paths do not preserve the complete install. Use the shipped cloud-init machine path to retain state; no stateless App Platform button is provided.

Read [the validated setup and its evidence](../hosting.md#digitalocean-app-platform) before deploying.
The rented-machine path installs its container engine and host firewall policy; it attaches no
KVM device. Container admission must enforce that policy. See the [machine guide](own-machine.md)
and [tier report](../hosting.md#tiers-and-guarantees) for the actual boundary. Live provider
deployment has not run.

Open the service's address when it is healthy. Its welcome page states the key location and
configured tier; enter the key there to connect, then choose a [caller guide](../connect.md).
