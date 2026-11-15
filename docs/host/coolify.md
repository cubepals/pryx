---
title: Host on Coolify
---

[Use the Compose service](../../deploy/platforms/coolify/compose.yaml).

Paste it into a Docker Compose Empty service on your existing Coolify host. Retain its named state volume. Coolify supplies the URL and generated key; find the key in service Configuration → Environment Variables.

Read [the validated setup and its evidence](../hosting.md#coolify) before deploying.
Managed-service images use the process policy where the kernel enforces Landlock and seccomp;
these files attach no nested engine or KVM. Shared-UID workloads require mutual trust, and
internet-enabled process workloads share host networking. Live provider deployments have not run.
[The tier report and limits](../hosting.md#tiers-and-guarantees) determine what is available.

Open the service's address when it is healthy. Its welcome page states the key location and
configured tier; enter the key there to connect, then choose a [caller guide](../connect.md).
