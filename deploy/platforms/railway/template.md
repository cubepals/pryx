# Railway template fields

No account or template has been published. In Template Composer add one image service:

| Field | Value |
| --- | --- |
| Name | pryx |
| Image | ghcr.io/cubepals/pryx:0.1.0 |
| Replicas | 1 |
| Volume mount | /var/lib/pryx |
| Public networking | Generate domain, target port 8080 |
| Health check | /healthz |
| PORT | 8080 |
| PRYX_HOSTING_PLATFORM | railway |
| PRYX_BOOTSTRAP_API_KEY | ${{secret(64)}} |
| PRYX_PUBLIC_URL | https://${{RAILWAY_PUBLIC_DOMAIN}} |

Give the service enough memory for embedded Postgres, store and sandboxes (start with 2 GiB).
The generated key stays in service Variables. Keep the volume across redeploys.
Once a maintainer publishes the template, its returned deploy URL is the button target;
no invented template ID is usable. See [hosting](../../../docs/hosting.md#railway).
