#!/usr/bin/env bash
# Run the end-to-end test in a throwaway container with a real nginx.
#   cargo build --release --target x86_64-unknown-linux-musl
#   tests/e2e/docker.sh [image]     # archlinux:latest (default), debian:stable, ubuntu:24.04
set -euo pipefail
cd "$(dirname "$0")/../.."
image=${1:-archlinux:latest}
bin=${NGX_BIN:-target/x86_64-unknown-linux-musl/release/ngx}
[ -x "$bin" ] || { echo "build first: cargo build --release --target x86_64-unknown-linux-musl" >&2; exit 2; }
exec docker run --rm -v "$PWD/$bin:/usr/local/bin/ngx:ro" -v "$PWD/tests/e2e/run.sh:/e2e.sh:ro" "$image" bash /e2e.sh
