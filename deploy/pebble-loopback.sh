#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Start the pinned Pebble CA and its DNS resolver on loopback for installed-server acceptance.
# The caller owns a private fixture directory, process cleanup, and client trust.

start_pebble() {
  local version=v2.10.3
  local server=pebble-linux-amd64
  local resolver=pebble-challtestsrv-linux-amd64
  local asset
  for asset in "$server" "$resolver"; do
    curl --proto '=https' --proto-redir '=https' --retry 3 --retry-all-errors --max-time 60 \
      -fsSL "https://github.com/letsencrypt/pebble/releases/download/$version/$asset.tar.gz" \
      -o "$fixture/$asset.tar.gz"
  done
  (
    cd "$fixture"
    printf '%s  %s.tar.gz\n' \
      7e33cf3e426b67e9e647c44e8cc91683caa8ab331f7a99e5e27fcee291c4c7e2 "$server" \
      00b2c5ef525cd8ad56cda898a7cac6169a10b0fd3f2a7456fe0cec92e7b36cb6 "$resolver" |
      sha256sum --check --strict
    tar -xzf "$server.tar.gz" "$server/linux/amd64/pebble"
    tar -xzf "$resolver.tar.gz" "$resolver/linux/amd64/pebble-challtestsrv"
    chmod 0700 "$server/linux/amd64/pebble" "$resolver/linux/amd64/pebble-challtestsrv"

    # This CA authenticates the ACME transport. Pebble independently generates its issuance root.
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -noenc -days 1 \
      -subj '/CN=pryx Pebble transport root' -keyout transport-ca.key -out transport-ca.pem \
      -addext 'basicConstraints=critical,CA:TRUE' 2>/dev/null
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -noenc -subj '/CN=localhost' \
      -keyout directory.key -out directory.csr 2>/dev/null
    printf 'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\n' > directory.extensions
    openssl x509 -req -in directory.csr -CA transport-ca.pem -CAkey transport-ca.key \
      -out directory.pem -extfile directory.extensions -days 1 2>/dev/null
    cat > pebble.json <<JSON
{"pebble":{
  "listenAddress":"127.0.0.1:14000",
  "managementListenAddress":"127.0.0.1:15000",
  "certificate":"directory.pem",
  "privateKey":"directory.key",
  "tlsPort":$pebble_validation_port,
  "keyAlgorithm":"ecdsa",
  "profiles":{"default":{"description":"Renewal acceptance","validityPeriod":15}}
}}
JSON
  )

  "$fixture/$resolver/linux/amd64/pebble-challtestsrv" -defaultIPv6 '' -dnsserver 127.0.0.1:8053 \
    -management 127.0.0.1:8055 -http01 '' -https01 '' -tlsalpn01 '' -doh '' >/dev/null 2>&1 &
  pebble_dns_pid=$!
  curl --noproxy '*' --retry 10 --retry-connrefused --retry-delay 1 --max-time 3 \
    -fsS -d '{"ip":"127.0.0.1"}' http://127.0.0.1:8055/set-default-ipv4 >/dev/null

  (
    cd "$fixture"
    export PEBBLE_AUTHZREUSE=0 PEBBLE_VA_NOSLEEP=1 PEBBLE_WFE_NONCEREJECT=0
    unset PEBBLE_VA_ALWAYS_VALID
    exec "$server/linux/amd64/pebble" -config pebble.json -dnsserver 127.0.0.1:8053
  ) >/dev/null 2>&1 &
  pebble_pid=$!
  curl --noproxy '*' --cacert "$fixture/transport-ca.pem" --retry 10 --retry-connrefused \
    --retry-delay 1 --max-time 3 -fsS https://127.0.0.1:15000/roots/0 -o "$fixture/issuance-root.pem"
  curl --noproxy '*' --cacert "$fixture/transport-ca.pem" --max-time 3 -fsS "$pebble_directory" >/dev/null
}
