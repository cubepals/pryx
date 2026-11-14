# DigitalOcean

App Platform has no persistent volumes. Managed Postgres and Spaces cannot retain this
release's fleet identity or local sandbox data. A deploy.template.yaml or App Platform button
would silently lose state on replacement, so no runnable App Platform spec is shipped.
Use the [cloud-init file](../cloud-init/user-data.yaml) on a rented machine instead.
See [hosting](../../../docs/hosting.md#digitalocean-app-platform) for the three storage paths checked.
