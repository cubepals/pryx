#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Sourced by CI fixtures on disposable hosted runners. pryx_loopback_tls makes a one-day CA,
# adds it to the system store, and serves one directory of preview release assets over
# certificate-verified HTTPS on 127.0.0.1:9877, where the preview descriptors point. It sets
# pryx_loopback_tls_pid to the server's PID. No product install changes host trust; only CI does.
pryx_loopback_tls() {
  local directory=$1 trust=$2 log=$3
  mkdir -m 700 "$trust"
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj '/CN=Pryx CI installed tools CA' \
    -keyout "$trust/ca.key" -out "$trust/ca.crt" > "$trust/openssl.log" 2>&1
  openssl req -newkey rsa:2048 -nodes -subj '/CN=127.0.0.1' \
    -keyout "$trust/server.key" -out "$trust/server.csr" >> "$trust/openssl.log" 2>&1
  printf '%s\n' 'subjectAltName=IP:127.0.0.1' 'extendedKeyUsage=serverAuth' > "$trust/extensions"
  openssl x509 -req -in "$trust/server.csr" -CA "$trust/ca.crt" \
    -CAkey "$trust/ca.key" -CAcreateserial -days 1 -extfile "$trust/extensions" \
    -out "$trust/server.crt" >> "$trust/openssl.log" 2>&1
  sudo install -m 644 "$trust/ca.crt" /usr/local/share/ca-certificates/pryx-ci-installed-tools.crt
  sudo update-ca-certificates
  python3 deploy/tool-artifact-server.py --directory "$directory" \
    --certificate "$trust/server.crt" --key "$trust/server.key" > "$log" 2>&1 &
  # shellcheck disable=SC2034 # the script that sourced this one reads it
  pryx_loopback_tls_pid=$!
}
