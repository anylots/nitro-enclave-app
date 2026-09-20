#!/usr/bin/env bash
set -Eeuo pipefail

# Resolve paths relative to this script so callers may run it from anywhere.
APP_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$APP_ROOT"

# The conversion container reads the application image from the host daemon.
command -v docker >/dev/null || { echo 'Docker is not installed.' >&2; exit 1; }
docker buildx version >/dev/null || { echo 'Docker Buildx is unavailable.' >&2; exit 1; }
[[ -S /var/run/docker.sock ]] || { echo 'Docker socket /var/run/docker.sock is missing.' >&2; exit 1; }
docker info >/dev/null || { echo 'Cannot access the Docker daemon; check that it is running and permissions are correct.' >&2; exit 1; }

# Build and load both images into the local Docker daemon.
docker buildx bake -f docker/docker-bake.hcl --load enclave eif-builder

# Build in a temporary directory. Existing successful output remains intact if
# conversion fails, and the EIF and its measurements are published together.
OUTPUT_DIR="$APP_ROOT/target/enclave-eif"
mkdir -p "$APP_ROOT/target"
BUILD_DIR="$(mktemp -d "$APP_ROOT/target/.enclave-eif.XXXXXX")"
BACKUP_DIR="$BUILD_DIR.previous"
cleanup() {
    if [[ -d "$BACKUP_DIR" && ! -e "$OUTPUT_DIR" ]]; then
        mv -- "$BACKUP_DIR" "$OUTPUT_DIR"
    fi
    rm -rf -- "$BUILD_DIR" "$BACKUP_DIR"
}
trap cleanup EXIT

# pipefail makes a failed nitro-cli conversion visible even though stdout is
# also captured as the PCR measurement document.
docker run --rm \
    --platform linux/amd64 \
    --volume /var/run/docker.sock:/var/run/docker.sock \
    --volume "$BUILD_DIR:/output" \
    nitro-enclave-eif-builder:local \
    build-enclave \
    --docker-uri nitro-enclave-app:local \
    --output-file /output/enclave.eif \
    | tee "$BUILD_DIR/measurements.json"

if [[ -e "$OUTPUT_DIR" ]]; then
    mv -- "$OUTPUT_DIR" "$BACKUP_DIR"
fi
mv -- "$BUILD_DIR" "$OUTPUT_DIR"
printf 'EIF and PCR measurements written to %s\n' "$OUTPUT_DIR"

