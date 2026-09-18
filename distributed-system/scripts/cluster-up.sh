#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="distributed-system"
REGISTRY_NAME="dsystem-registry"
REGISTRY_PORT="5111"

echo "==> Creating local registry..."
if ! docker inspect "$REGISTRY_NAME" &>/dev/null; then
    docker run -d --restart=always -p "${REGISTRY_PORT}:5000" --name "$REGISTRY_NAME" registry:2
fi

echo "==> Creating k3d cluster..."
k3d cluster create "$CLUSTER_NAME" \
    --registry-use "k3d-${REGISTRY_NAME}:${REGISTRY_PORT}" \
    --port "30000-30010:30000-30010@server:0" \
    --agents 2

echo "==> Cluster ready. Context set to k3d-${CLUSTER_NAME}"
kubectl cluster-info
