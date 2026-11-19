#!/bin/sh
# SPDX-License-Identifier: FSL-1.1-ALv2
# Tests pull-rustfs.sh with a stand-in docker that refuses the registries a case names.
set -eu

script="$(cd "$(dirname "$0")" && pwd)/pull-rustfs.sh"
digest=$(sed -n 's|^FROM ghcr.io/rustfs/rustfs:1.0.0@\(sha256:[0-9a-f]\{64\}\) .*|\1|p' "$(dirname "$script")/../Dockerfile")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat > "$work/docker" <<'EOF'
#!/bin/sh
echo "docker $*" >> "$PULL_LOG"
for refused in $PULL_REFUSED; do
  case "$1 $3" in "pull $refused"*) exit 1 ;; esac
done
EOF
cat > "$work/sleep" <<'EOF'
#!/bin/sh
echo "sleep $*" >> "$PULL_LOG"
EOF
chmod +x "$work/docker" "$work/sleep"

pull() {
  : > "$work/log"
  PULL_LOG="$work/log" PULL_REFUSED="$1" PATH="$work:$PATH" sh "$script"
}
expect() {
  if [ "$(cat "$work/log")" != "$1" ]; then
    printf 'Expected:\n%s\nGot:\n%s\n' "$1" "$(cat "$work/log")" >&2
    exit 1
  fi
}

pull ''
expect "docker pull -q ghcr.io/rustfs/rustfs@$digest
docker tag ghcr.io/rustfs/rustfs@$digest ghcr.io/rustfs/rustfs:1.0.0"

pull 'ghcr.io/'
expect "docker pull -q ghcr.io/rustfs/rustfs@$digest
docker pull -q mirror.gcr.io/rustfs/rustfs@$digest
docker tag mirror.gcr.io/rustfs/rustfs@$digest ghcr.io/rustfs/rustfs:1.0.0"

if pull 'ghcr.io/ mirror.gcr.io/'; then
  echo 'Both registries refused, yet the pull passed' >&2
  exit 1
fi
if [ "$(grep -c '^docker pull' "$work/log")" != 12 ] || [ "$(grep -c '^sleep' "$work/log")" != 5 ] ||
  grep -q '^docker tag' "$work/log"; then
  cat "$work/log" >&2
  exit 1
fi
echo 'pull-rustfs.sh: GHCR first, the same digest from the mirror next, and no image on refusal'
