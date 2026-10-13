#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
[[ "$(uname -m)" == arm64 && "$(uname -s)" == Darwin ]] || { print -u2 'Apple Silicon macOS is required.'; exit 1; }
toolchain=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$project_root/rust-toolchain.toml")
rust_bin="${NATIVE_RUST_BIN_DIR:-${RUSTUP_HOME:-$HOME/.rustup}/toolchains/$toolchain-aarch64-apple-darwin/bin}"
[[ -x "$rust_bin/rustc" && -x "$rust_bin/cargo" ]] || {
  print -u2 -- "Installed Rust $toolchain is unavailable. Set NATIVE_RUST_BIN_DIR to an existing compatible toolchain's bin directory."
  exit 1
}
export RUSTC="$rust_bin/rustc"
export RUSTDOC="$rust_bin/rustdoc"
export CARGO_HOME="$project_root/.work/cargo"
export CARGO_TARGET_DIR="$project_root/.work/target"
export TMPDIR="$project_root/.work/tmp"
export MACOSX_DEPLOYMENT_TARGET=26.0
mkdir -p "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR"
cd "$project_root"
operation="${1:-build}"
if (( $# > 0 )); then shift; fi
"$rust_bin/cargo" "$operation" -p "${NATIVE_RUST_PACKAGE:-clash-verge-native-core}" --target aarch64-apple-darwin --locked "$@"
