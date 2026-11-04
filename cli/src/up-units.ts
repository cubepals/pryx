// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

/** Systemd templates for private control/storage and public byte fronts. No template contains a secret. */
export function service(name: 'control' | 'edge') {
  return `[Unit]
Description=pryx ${name}
After=network-online.target pryx-postgres.service pryx-store.service
Wants=network-online.target
[Service]
User=pryx
Group=pryx
EnvironmentFile=/etc/pryx/${name}.env
ExecStart=/opt/pryx/pryx-${name}
Restart=always
RestartSec=2
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
${name === 'edge' ? 'AmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE' : ''}
[Install]
WantedBy=multi-user.target
`
}
export function container(name: 'postgres' | 'store', docker: string) {
  const postgres = name === 'postgres'
  const image = postgres ? 'mirror.gcr.io/library/postgres:17-alpine' : 'ghcr.io/rustfs/rustfs:1.0.0'
  const port = postgres ? '5432' : '9000',
    volume = postgres ? '/var/lib/postgresql/data' : '/data'
  const command = postgres
    ? 'exec /usr/local/bin/docker-entrypoint.sh postgres'
    : 'exec /usr/bin/rustfs /data'
  return `[Unit]
Description=pryx ${name} storage
After=docker.service
Requires=docker.service
[Service]
EnvironmentFile=/etc/pryx/${name}.env
ExecStart=${docker} run --rm --name pryx-${name} --label pryx.service=${name} --user root -p 127.0.0.1:${port}:${port} --mount type=volume,source=pryx-${name},target=${volume} --mount type=bind,source=/etc/pryx/${name}.env,target=/run/pryx.env,readonly --entrypoint /bin/sh ${image} -c "set -a; . /run/pryx.env; set +a; ${command}"
ExecStop=${docker} stop pryx-${name}
Restart=always
RestartSec=2
TimeoutStartSec=0
[Install]
WantedBy=multi-user.target
`
}
