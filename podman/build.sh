#!/usr/bin/env bash
# Build the localhost/moqui:latest image from the Maximal package.
#   podman/build.sh            ./moq build --release, then podman build
#   podman/build.sh --nobuild  image only, using the existing target/release/moqui
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
nobuild=0
for arg in "$@"; do
    case "$arg" in
        --nobuild) nobuild=1 ;;
        *)
            echo "Unknown argument: $arg" >&2
            echo "Usage: podman/build.sh [--nobuild]" >&2
            exit 1
            ;;
    esac
done

if [ "$nobuild" -eq 0 ]; then
    (cd "$root" && ./moq build --release)
fi

binary=$root/target/release/moqui
if [ ! -x "$binary" ]; then
    echo "No release binary at $binary. Run ./moq build --release, or drop --nobuild." >&2
    exit 1
fi
if ! command -v podman >/dev/null 2>&1; then
    echo "podman is not on PATH" >&2
    exit 1
fi

copy_tree() {
    local src=$1 dest=$2
    mkdir -p "$dest"
    tar -C "$src" --exclude=.git --exclude=target --exclude=node_modules -cf - . | tar -C "$dest" -xf -
}

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

cp -a "$binary" "$stage/moqui"
chmod 755 "$stage/moqui"
cp -a "$root/podman/entrypoint.sh" "$stage/entrypoint.sh"
chmod 755 "$stage/entrypoint.sh"
cp -a "$root/LICENSE.md" "$stage/LICENSE.md"
cp -a "$root/runtime/conf/MoquiProductionConf.xml" "$stage/MoquiConf.xml"
mkdir -p "$stage/conf"
cp -a "$root/runtime/conf/." "$stage/conf/"
copy_tree "$root/runtime/base-component" "$stage/base-component"
mkdir -p "$stage/component"
if [ -d "$root/runtime/component" ] && [ -n "$(ls -A "$root/runtime/component")" ]; then
    copy_tree "$root/runtime/component" "$stage/component"
fi

# docker format keeps the image HEALTHCHECK. Podman's default OCI format drops it.
podman build --format docker -f "$root/podman/Containerfile" -t localhost/moqui:latest "$stage"
echo "Built localhost/moqui:latest"
