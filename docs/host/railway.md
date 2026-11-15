---
title: Host on Railway
---

[Prepare the Railway fields](../../deploy/platforms/railway/template.md).

Paste the documented Composer fields, attach one state volume and generate a domain. Find the generated bootstrap key in service Variables. The account-owned template has not been published.

Read [the validated setup and its evidence](../hosting.md#railway) before deploying.
Managed-service images use the process policy where the kernel enforces Landlock and seccomp;
these files attach no nested engine or KVM. Shared-UID workloads require mutual trust, and
internet-enabled process workloads share host networking. Live provider deployments have not run.
[The tier report and limits](../hosting.md#tiers-and-guarantees) determine what is available.

Open the service's address when it is healthy. Its welcome page states the key location and
configured tier; enter the key there to connect, then choose a [caller guide](../connect.md).
