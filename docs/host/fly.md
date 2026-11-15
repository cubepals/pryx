---
title: Host on Fly.io
---

[Use the Fly Launch configuration](../../deploy/platforms/fly/fly.toml).

Launch one Machine with the persistent volume. Fly chooses the app name. The initial key appears once in Monitoring → Logs; keep the volume on replacement.

Read [the validated setup and its evidence](../hosting.md#flyio) before deploying.
Managed-service images use the process policy where the kernel enforces Landlock and seccomp;
these files attach no nested engine or KVM. Shared-UID workloads require mutual trust, and
internet-enabled process workloads share host networking. Live provider deployments have not run.
[The tier report and limits](../hosting.md#tiers-and-guarantees) determine what is available.

Open the service's address when it is healthy. Its welcome page states the key location and
configured tier; enter the key there to connect, then choose a [caller guide](../connect.md).
