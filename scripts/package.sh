#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
builder=${2:-native}
sdk_rev=21c0711a6693372869475d1bec3037ef4b08525f
# Multi-platform official image; pinned so rebuilding does not silently change glibc/toolchain.
image=rust:1.97.0-bookworm@sha256:8fa55b2f3ddf97471ab6a767bfa3f37e6bad0986ba823e75fea57e2a2a5c3073

case "$target" in
  x86_64-unknown-linux-gnu) platform=linux/amd64 ;;
  aarch64-unknown-linux-gnu) platform=linux/arm64 ;;
  aarch64-apple-darwin) platform= ;;
  *) echo "Unsupported target: $target" >&2; exit 2 ;;
esac
case "$builder" in
  native)
    cargo test --release --locked --target "$target"
    cargo build --release --locked --target "$target"
    ;;
  --docker)
    if [[ -z "$platform" ]]; then
      echo "Docker builds support Linux targets only" >&2; exit 2
    fi
    mkdir -p "$root/target/docker-cache/$target"
    docker run --rm --platform "$platform" \
      -v "$root:/work" -w /work \
      -v "$root/target/docker-cache/$target:/cargo-cache" \
      -e CARGO_HOME=/cargo-cache -e RUSTUP_TOOLCHAIN=1.97.0 \
      "$image" bash -euc '
        cargo test --release --locked --target "$1"
        cargo build --release --locked --target "$1"
      ' build "$target"
    ;;
  --docker-cross)
    if [[ "$target" != x86_64-unknown-linux-gnu ]]; then
      echo "--docker-cross builds Linux AMD64 using an ARM64 compiler" >&2; exit 2
    fi
    mkdir -p "$root/target/docker-cache/aarch64-unknown-linux-gnu"
    docker run --rm --platform linux/arm64 \
      -v "$root:/work" -w /work \
      -v "$root/target/docker-cache/aarch64-unknown-linux-gnu:/cargo-cache" \
      -e CARGO_HOME=/cargo-cache -e RUSTUP_TOOLCHAIN=1.97.0 \
      -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
      "$image" bash -euc '
        apt-get update -qq
        apt-get install -y -qq --no-install-recommends gcc-x86-64-linux-gnu libc6-dev-amd64-cross > /tmp/cross-install.log 2>&1 || { cat /tmp/cross-install.log; exit 1; }
        rustup target add x86_64-unknown-linux-gnu
        cargo test --release --locked --target x86_64-unknown-linux-gnu --no-run --message-format=json > target/x86_64-tests.json
        cargo build --release --locked --target x86_64-unknown-linux-gnu
      '
    python3 scripts/run-container-tests.py "$root" "$image"
    ;;
  *) echo "Usage: scripts/package.sh [target-triple] [native|--docker|--docker-cross]" >&2; exit 2 ;;
esac

# Use the same pinned author-manifest contract as the SDK. No global cargo install.
cli=${CPR_PLUGIN:-"$root/.tools/$sdk_rev/bin/cpr-plugin"}
if [[ -z "${CPR_PLUGIN:-}" && ! -x "$cli" ]]; then
  cargo install --git https://github.com/zyycn/codex-proxy-rs.git \
    --rev "$sdk_rev" --locked --root "$root/.tools/$sdk_rev" codex-proxy-plugin-cli
fi
"$cli" package --manifest "$root/plugin.json" \
  --binary "$root/target/$target/release/cps-block-ua" \
  --target "$target" --output-dir "$root/dist"
