# Koyeb service fields

Prepare one web service from `ghcr.io/cubepals/pryx:0.1.0`, port `8080`, HTTP route `/`,
HTTP health check `/healthz`, one instance, at least 2 GiB memory. Create and attach a
persistent volume at `/var/lib/pryx` in the same region before the first start.
Set `PORT=8080` and `PRYX_HOSTING_PLATFORM=koyeb`. Keep privileged mode off.
No template secret generator is documented; omit PRYX_BOOTSTRAP_API_KEY and read the
once-only apiKey in the first successful service log. KOYEB_PUBLIC_DOMAIN supplies the origin.

Koyeb's deploy button cannot attach the required volume. Do not publish a stateless button:
prepare this service with its volume in the dashboard. See
[hosting](../../../docs/hosting.md#koyeb) for the storage and kernel boundaries.
