#!/usr/bin/env bash
set -euo pipefail

NET="st-link-$$"
SRV="st-server-$$"
CLI="st-client-$$"
WORK="$(mktemp -d)"
mkdir -p "$WORK/state" "$WORK/share" "$WORK/data"

STATUS="FAIL"
cleanup() {
    if [[ "$STATUS" == "FAIL" ]]; then
        docker logs "$SRV" 2>&1 || true
    fi
    docker rm -f "$SRV" "$CLI" >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
    rm -rf "$WORK"
    if [[ "$STATUS" == "FAIL" ]]; then
        echo "TWO-HOST: FAIL"
    else
        echo "TWO-HOST: PASS"
    fi
}
trap cleanup EXIT

if ! docker image inspect ubuntu:24.04 >/dev/null 2>&1; then
    echo "FAIL: required Docker image ubuntu:24.04 is not present locally" >&2
    exit 1
fi

cargo build --release -p simply-transfer-link --example link_node
BIN="$(pwd)/target/release/examples/link_node"

UID_GID="$(id -u):$(id -g)"

docker network create "$NET" >/dev/null

docker run --rm \
    --user "$UID_GID" \
    -v "$BIN:/usr/local/bin/link_node:ro" \
    -v "$WORK/state:/state" \
    ubuntu:24.04 \
    /usr/local/bin/link_node keygen --out /state

docker run -d \
    --name "$SRV" \
    --user "$UID_GID" \
    --network "$NET" \
    --network-alias st-server \
    -v "$BIN:/usr/local/bin/link_node:ro" \
    -v "$WORK/state:/state" \
    -v "$WORK/share:/share" \
    ubuntu:24.04 \
    /usr/local/bin/link_node serve \
        --bind 0.0.0.0:2222 \
        --state /state \
        --share /share \
        --client-pub /state/client.pub >/dev/null

READY=0
for _ in $(seq 1 40); do
    if [[ -f "$WORK/state/ready" ]]; then
        READY=1
        break
    fi
    sleep 0.5
done

if [[ "$READY" -ne 1 ]]; then
    echo "FAIL: server did not become ready within 20s" >&2
    exit 1
fi

if docker run --rm \
    --name "$CLI" \
    --user "$UID_GID" \
    --network "$NET" \
    -v "$BIN:/usr/local/bin/link_node:ro" \
    -v "$WORK/state:/state:ro" \
    -v "$WORK/data:/data" \
    ubuntu:24.04 \
    /usr/local/bin/link_node client \
        --server st-server:2222 \
        --state /state \
        --data /data; then
    rc=0
else
    rc=$?
fi

if [[ "$rc" -ne 0 ]]; then
    exit 1
fi

if [[ ! -f "$WORK/data/payload.bin" || ! -f "$WORK/share/inbox/payload.bin" || ! -f "$WORK/share/inbox/resume.bin" ]]; then
    echo "FAIL: expected output files missing after client run" >&2
    exit 1
fi

SRC_SHA="$(sha256sum "$WORK/data/payload.bin" | awk '{print $1}')"
UP_SHA="$(sha256sum "$WORK/share/inbox/payload.bin" | awk '{print $1}')"
RES_SHA="$(sha256sum "$WORK/share/inbox/resume.bin" | awk '{print $1}')"

if [[ "$SRC_SHA" != "$UP_SHA" || "$SRC_SHA" != "$RES_SHA" ]]; then
    echo "FAIL: host-side sha256 mismatch (src=$SRC_SHA upload=$UP_SHA resume=$RES_SHA)" >&2
    exit 1
fi

STATUS="PASS"
exit 0
