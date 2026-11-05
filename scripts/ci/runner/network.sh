#!/usr/bin/env bash
# Creates the runners' docker network and walls it off from this machine (run as root, at every
# start of gvp-ci-runners.service; idempotent). Containers on it reach the internet only: not
# the host's own services, not other containers, not private, link-local or tailnet addresses.
set -euo pipefail
net=gvp-ci
bridge=gvpci0
subnet=172.31.240.0/24

if ! docker network inspect "$net" >/dev/null 2>&1; then
  docker network create --driver bridge --subnet "$subnet" \
    -o com.docker.network.bridge.name="$bridge" \
    -o com.docker.network.bridge.enable_icc=false \
    "$net" >/dev/null
fi

# Forwarded traffic from the runners to private ranges is dropped (DOCKER-USER runs before
# Docker's own rules). Replies to connections the runners opened are not affected: they leave
# through the bridge, not into it.
rule_forward() {
  iptables -C DOCKER-USER -i "$bridge" -d "$1" -j DROP 2>/dev/null \
    || iptables -I DOCKER-USER -i "$bridge" -d "$1" -j DROP
}
for cidr in 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 169.254.0.0/16 127.0.0.0/8 224.0.0.0/4; do
  rule_forward "$cidr"
done
# Nothing from the runners reaches a socket on the host itself (the bridge gateway or any other
# host address). DNS goes to public resolvers (the supervisor passes --dns).
iptables -C INPUT -i "$bridge" -j DROP 2>/dev/null || iptables -I INPUT -i "$bridge" -j DROP
# No IPv6 on the bridge.
sysctl -q -w "net.ipv6.conf.$bridge.disable_ipv6=1" 2>/dev/null || true
echo "gvp-ci network ready ($subnet on $bridge)"
