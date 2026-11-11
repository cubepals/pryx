#!/usr/bin/env bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Start one pinned engine as the hosted runner's ordinary user. Root is used only to install
# its packages and its own AppArmor attachment; no pryx workload profile is relaxed. These
# isolated CI hosts and their temporary engine state are not an installer or a user preset.
set -euo pipefail

engine=${1:?docker or podman}
case "$engine" in docker|podman) ;; *) exit 2 ;; esac
[ "$(id -u)" != 0 ]
root="${RUNNER_TEMP:?}/pryx-engine-$engine"
mkdir -p "$root/bin" "$root/run" "$root/data" "$root/exec"
chmod 700 "$root/run" "$root/data" "$root/exec"
manifest=.github/engine-versions.json
export XDG_RUNTIME_DIR="$root/run"

fetch() {
  local url=$1 digest=$2 output=$3
  curl --retry 5 --retry-delay 3 --retry-connrefused --connect-timeout 20 --max-time 180 -fsSL "$url" -o "$output"
  printf '%s  %s\n' "$digest" "$output" | sha256sum -c -
}

sudo systemctl stop docker.service docker.socket
sudo rm -f /var/run/docker.sock
sudo apt-get update -o Acquire::Retries=3
sudo apt-get install -y uidmap slirp4netns

if [ "$engine" = docker ]; then
  fetch "$(jq -r .docker.url "$manifest")" "$(jq -r .docker.sha256 "$manifest")" "$root/docker.tgz"
  fetch "$(jq -r .docker.extrasUrl "$manifest")" "$(jq -r .docker.extrasSha256 "$manifest")" "$root/extras.tgz"
  tar -xzf "$root/docker.tgz" -C "$root/bin" --strip-components=1
  tar -xzf "$root/extras.tgz" -C "$root/bin" --strip-components=1
  "$root/bin/dockerd" --version | grep -F "$(jq -r .docker.version "$manifest")"
  "$root/bin/rootlesskit" --version | grep -F "$(jq -r .docker.rootlesskit "$manifest")"
  # Docker's Ubuntu rootless documentation prescribes an attachment permitting userns for
  # this exact executable. It gives no extra capabilities to pryxd or any sandbox.
  sudo install -m 755 "$root/bin/rootlesskit" /usr/bin/pryx-ci-rootlesskit
  rm "$root/bin/rootlesskit"
  ln -s /usr/bin/pryx-ci-rootlesskit "$root/bin/rootlesskit"
  cat > "$root/rootlesskit.apparmor" <<'PROFILE'
abi <abi/4.0>,
include <tunables/global>
/usr/bin/pryx-ci-rootlesskit flags=(unconfined) {
  userns,
}
PROFILE
  sudo apparmor_parser -r "$root/rootlesskit.apparmor"
  export PATH="$root/bin:$PATH"
  export XDG_RUNTIME_DIR="$root/run"
  export DOCKERD_ROOTLESS_ROOTLESSKIT_STATE_DIR="$root/rootlesskit"
  export DOCKERD_ROOTLESS_ROOTLESSKIT_NET=slirp4netns
  endpoint="unix://$root/run/docker.sock"
  "$root/bin/dockerd-rootless.sh" --host="$endpoint" --data-root="$root/data" \
    --exec-root="$root/exec" --pidfile="$root/engine.pid" --storage-driver=vfs \
    --exec-opt=native.cgroupdriver=cgroupfs > "$root/engine.log" 2>&1 &
  printf '%s\n' "$root/bin" >> "${GITHUB_PATH:?}"
  probe=("$root/bin/docker" --host "$endpoint")
else
  fetch "$(jq -r .podman.url "$manifest")" "$(jq -r .podman.sha256 "$manifest")" "$root/podman.deb"
  sudo apt-get install -y --allow-downgrades "$root/podman.deb"
  [ "$(dpkg-query -W -f='${Version}' podman)" = "$(jq -r .podman.version "$manifest")" ]
  endpoint="unix://$root/run/podman.sock"
  podman --root "$root/data" --runroot "$root/exec" --storage-driver=vfs \
    --cgroup-manager=cgroupfs --events-backend=file system service --time=0 "$endpoint" > "$root/engine.log" 2>&1 &
  probe=(podman --remote --url "$endpoint")
  # The existing fixture's failure cleanup uses Docker's CLI spelling. Keep its same
  # endpoint and operations while letting the engine's own client read its older API.
  cat > "$root/bin/docker" <<'CLIENT'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = --host ]
shift
engine_endpoint=${1:?}
shift
exec /usr/bin/podman --remote --url "$engine_endpoint" "$@"
CLIENT
  chmod 755 "$root/bin/docker"
  printf '%s\n' "$root/bin" >> "${GITHUB_PATH:?}"
fi
pid=$!
printf '%s\n' "$pid" > "$root/service.pid"
ready=false
for ((attempt=0; attempt<60; attempt++)); do
  if "${probe[@]}" info > "$root/info.txt" 2>&1; then ready=true; break; fi
  kill -0 "$pid" || break
  sleep 1
done
if [ "$ready" != true ]; then
  tail -100 "$root/engine.log"
  cat "$root/info.txt"
  exit 1
fi
"${probe[@]}" version
cat "$root/info.txt"
pulled=false
for image in "$(jq -r .image.reference "$manifest")" "$(jq -r .image.fallback "$manifest")"; do
  for ((attempt=1; attempt<=3; attempt++)); do
    if "${probe[@]}" pull "$image"; then
      "${probe[@]}" tag "$image" "$(jq -r .image.alias "$manifest")"
      pulled=true
      break 2
    fi
    sleep "$((attempt * 5))"
  done
done
[ "$pulled" = true ]
{
  printf 'DOCKER_HOST=%s\n' "$endpoint"
  printf 'PRYX_TEST_ENGINE=%s\n' "$engine"
  printf 'PRYX_TEST_ENGINE_SOCKET=%s\n' "${endpoint#unix://}"
} >> "${GITHUB_ENV:?}"
