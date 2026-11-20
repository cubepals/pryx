#!/bin/bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Prove the first server install over publicly routed TLS with local Pebble, including renewal
# without a service restart. Trust Pebble only in the clients launched by this fixture.
set -euo pipefail
set +x
umask 077
unset PRYX_API_URL PRYX_API_KEY PRYX_CONFIG

pebble_pid= pebble_dns_pid= hosts_backup=
cleanup() {
  local pid executable status=0
  for pid in "$pebble_pid" "$pebble_dns_pid"; do
    [[ $pid =~ ^[1-9][0-9]*$ ]] || continue
    executable=$(readlink -- "/proc/$pid/exe" || true)
    case "$executable" in
      "$fixture"/pebble*/linux/amd64/pebble*)
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
        ;;
    esac
  done
  if [ -n "$hosts_backup" ]; then sudo cp -- "$hosts_backup" /etc/hosts || status=$?; fi
  rm -rf -- "$fixture" || status=$?
  return "$status"
}

if [ "${1:-}" = cleanup ]; then
  fixture=${PRYX_PEBBLE_FIXTURE_DIRECTORY:-}
  [ -n "$fixture" ] && [ -e "$fixture" ] || exit 0
  if [[ $(dirname -- "$fixture") != "${RUNNER_TEMP:?}" ||
    $(basename -- "$fixture") != pryx-pebble.?????? || ! -d $fixture || -L $fixture || ! -O $fixture ]]; then
    echo 'Unsafe Pebble fixture directory' >&2
    exit 1
  fi
  read -r pebble_pid pebble_dns_pid < "$fixture/pids"
  hosts_backup=$fixture/hosts
  cleanup
  exit
fi

readonly release=${1:?loopback release URL required}
case "$release" in
  http://127.0.0.1:*) ;;
  *) echo 'The ACME acceptance requires loopback release artifacts' >&2; exit 1 ;;
esac
readonly domain=sandboxes.example.com
readonly pebble_directory=https://127.0.0.1:14000/dir
readonly pebble_validation_port=443
fixture=$(mktemp -d "${RUNNER_TEMP:?requires a CI runner temporary directory}/pryx-pebble.XXXXXX")
readonly fixture
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

. "$(dirname -- "${BASH_SOURCE[0]}")/pebble-loopback.sh"
start_pebble
printf '%s %s\n' "$pebble_pid" "$pebble_dns_pid" > "$fixture/pids"
cp /etc/hosts "$fixture/hosts"
hosts_backup=$fixture/hosts
printf '127.0.0.1 %s\n' "$domain" | sudo tee -a /etc/hosts >/dev/null

started=$SECONDS
output=$(curl --noproxy '*' --retry 5 --retry-connrefused -fsS "$release/install.sh" |
  sudo "PRYX_RELEASE_BASE_URL=$release" "PRYX_DOMAIN=$domain" \
    "PRYX_ACME_TEST_DIRECTORY=$pebble_directory" "PRYX_ACME_TEST_CA=$fixture/transport-ca.pem" \
    "PRYX_ACME_TEST_ROOT=$fixture/issuance-root.pem" sh)
grep -Fxq "export PRYX_API_URL=https://$domain" <<< "$output"
[ "$(grep -c '^export PRYX_' <<< "$output")" = 3 ]

api_status() {
  curl --noproxy '*' --resolve "$domain:443:127.0.0.1" --cacert "$fixture/issuance-root.pem" \
    --connect-timeout 2 --max-time 5 -sS -o "$fixture/api.json" -w '%{http_code}' \
    "https://$domain/v1/sandboxes"
}
tls_cert() {
  timeout 5 openssl s_client -connect 127.0.0.1:443 -servername "$domain" -verify_hostname "$domain" \
    -verify_return_error -CAfile "$fixture/issuance-root.pem" </dev/null 2>/dev/null |
    openssl x509 "$@" -noout 2>/dev/null
}
cat /etc/ssl/certs/ca-certificates.crt "$fixture/issuance-root.pem" > "$fixture/client-trust.pem"
export NODE_EXTRA_CA_CERTS="$fixture/issuance-root.pem" SSL_CERT_FILE="$fixture/client-trust.pem"
export NO_PROXY="${NO_PROXY:+$NO_PROXY,}$domain,127.0.0.1"
export no_proxy="${no_proxy:+$no_proxy,}$domain,127.0.0.1"
unauthorized=$(python3 -c 'from http import HTTPStatus; print(HTTPStatus.UNAUTHORIZED.value)')
[ "$(api_status)" = "$unauthorized" ]
jq -e '.error.code == "unauthorized"' "$fixture/api.json" >/dev/null
if curl --noproxy '*' --resolve "$domain:443:127.0.0.1" --cacert /etc/ssl/certs/ca-certificates.crt \
  --max-time 5 -sS -o /dev/null \
  "https://$domain/v1/sandboxes" 2>/dev/null; then
  echo 'The API was trusted without the Pebble issuance root' >&2
  exit 1
fi
plain=$(curl --noproxy '*' --resolve "$domain:80:127.0.0.1" --max-time 3 -sS \
  -o /dev/null -w '%{http_code}' "http://$domain/v1/sandboxes" 2>/dev/null || true)
case "$plain" in
  2*|"$unauthorized") echo 'The plaintext front reached the public API' >&2; exit 1 ;;
esac
for socket in /run/pryx-edge/api.sock /run/pryx-api-tls/tls.sock; do
  [ "$(sudo stat -c '%G:%a' "$socket")" = 'pryx:660' ]
  [ "$(sudo stat -c '%G:%a' "$(dirname "$socket")")" = 'pryx:750' ]
done

# A later certificate must be trusted for the intended hostname before the initial one expires.
invocation=$(systemctl show --property=InvocationID --value pryx-api-tls.service)
[ -n "$invocation" ]
first=$(tls_cert -serial -enddate)
first_serial=$(sed -n 's/^serial=//p' <<< "$first")
expires_at=$(date -u -d "$(sed -n 's/^notAfter=//p' <<< "$first")" +%s)
[ -n "$first_serial" ]
pryx sandbox ls | jq -e '.sandboxes | type == "array"' >/dev/null
renewed=false
while [ "$(date +%s)" -lt "$expires_at" ]; do
  current=$(tls_cert -serial || true)
  current_serial=$(sed -n 's/^serial=//p' <<< "$current")
  if [ -n "$current_serial" ] && [ "$current_serial" != "$first_serial" ] &&
    [ "$(date +%s)" -lt "$expires_at" ]; then
    renewed=true
    break
  fi
  sleep 1
done
if [ "$renewed" = false ]; then
  echo 'The API did not serve a renewed Pebble certificate before expiry' >&2
  exit 1
fi
[ "$(systemctl show --property=InvocationID --value pryx-api-tls.service)" = "$invocation" ]
[ "$(api_status)" = "$unauthorized" ]
[ "$(pryx run --image alpine:3.22 -- echo hello)" = hello ]
made=$(pryx sandbox create --image alpine:3.22 --idle 5m)
pryx sandbox rm "$(jq -er .id <<< "$made")" >/dev/null

second=$(sudo "PRYX_RELEASE_BASE_URL=$release" pryx up --domain "$domain")
grep -Fxq "export PRYX_API_URL=https://$domain" <<< "$second"
grep -Fxq 'export PRYX_API_KEY=$(pryx config key)' <<< "$second"
[ "$(grep -c '^export PRYX_' <<< "$second")" = 3 ]
[ "$(pryx run --image alpine:3.22 -- echo hello)" = hello ]

# The subsequent CLI, Action and SDK steps use the disposable installation's public API key.
key=$(sed -n 's/^export PRYX_API_KEY=//p' <<< "$output")
[[ $key == pryx_* ]]
printf '::add-mask::%s\n' "$key"
example=$(
  cd sdk/typescript
  PRYX_API_URL="https://$domain" PRYX_API_KEY="$key" \
    bun ../../control/tools/readme-example.ts "$GITHUB_WORKSPACE/README.md" ts bun -
)
[ "$example" = hello ]
printf 'key=%s\n' "$key" >> "${GITHUB_OUTPUT:?requires a CI step output}"
printf 'Pebble validated the API certificate, renewed it before expiry and served HTTPS sandbox calls in %s seconds\n' \
  "$((SECONDS - started))" | tee -a "${GITHUB_STEP_SUMMARY:?requires a CI step summary}"

# Other SDK/Action steps start at the private API and receive agent URLs on this HTTPS hostname.
printf '%s\n' "PRYX_PEBBLE_FIXTURE_DIRECTORY=$fixture" "NODE_EXTRA_CA_CERTS=$NODE_EXTRA_CA_CERTS" \
  "SSL_CERT_FILE=$SSL_CERT_FILE" "NO_PROXY=$NO_PROXY" "no_proxy=$no_proxy" >> "${GITHUB_ENV:?}"
trap - EXIT
