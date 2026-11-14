---
title: Host on Render
---

[Deploy on Render](https://render.com/deploy?repo=https://github.com/cubepals/pryx).

It builds the repository Dockerfile and retains its state disk and managed Postgres. Find the generated key in the service Environment settings.

Read [the validated setup and its evidence](../hosting.md#render) before deploying.
Managed-service images use the process policy where the kernel enforces Landlock and seccomp;
these files attach no nested engine or KVM. Shared-UID workloads require mutual trust, and
internet-enabled process workloads share host networking. Live provider deployments have not run.
[The tier report and limits](../hosting.md#tiers-and-guarantees) determine what is available.

Open the service's address when it is healthy. Its welcome page states the key location and
configured tier; enter the key there to connect, then choose a [caller guide](../connect.md).
