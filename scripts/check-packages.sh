#!/usr/bin/env bash
# Clippy, with warnings denied, on every feature set a build uses (docs/Packaging.md), so a feature gate that only one
# package sees still compiles: the workspace on its defaults (the Maximal package, `cargo lint`), the Default and
# Enterprise packages of the binary, the host with no features (`cargo lint-min`), and the framework with none (what
# the browser build links).
#   scripts/check-packages.sh [extra clippy args...]
set -euo pipefail
cd "$(dirname "$0")/.."
run() {
    echo "== cargo clippy $*" >&2
    cargo clippy "$@" -- -D warnings
}
run --workspace --all-targets "$@"
run -p moqui --no-default-features --features lite --all-targets "$@"
run -p moqui --no-default-features --features enterprise --all-targets "$@"
run -p moqui-host --no-default-features --all-targets "$@"
run -p moqui-core --no-default-features --all-targets "$@"
echo "All package feature sets pass" >&2
