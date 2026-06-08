#!/bin/bash
# Install Docker on a fresh Debian/Ubuntu host. Idempotent — re-running
# is a no-op once docker + the compose plugin are present.
#
# Called by `make bootstrap` via ssh; doesn't expect to be run locally.

set -euo pipefail

if command -v docker >/dev/null 2>&1 && \
   docker compose version >/dev/null 2>&1; then
    echo "docker $(docker --version | awk '{print $3}' | tr -d ,) + compose plugin already present"
    exit 0
fi

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    ca-certificates curl gnupg

install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg \
    -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc

# Detect ubuntu vs debian and pick the codename. The PPA layout is the
# same for both; only the OS path differs.
. /etc/os-release
case "$ID" in
    ubuntu) repo=ubuntu  ;;
    debian) repo=debian  ;;
    *)      echo "unsupported distro: $ID" >&2; exit 1 ;;
esac
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/${repo} ${VERSION_CODENAME} stable" \
    > /etc/apt/sources.list.d/docker.list

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    docker-ce docker-ce-cli containerd.io \
    docker-buildx-plugin docker-compose-plugin

docker --version
docker compose version
