#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="distributed-system"
REGISTRY_NAME="dsystem-registry"

echo "==> Deleting k3d cluster..."
k3d cluster delete "$CLUSTER_NAME" 2>/dev/null || true

echo "==> Removing local registry..."
docker rm -f "$REGISTRY_NAME" 2>/dev/null || true

echo "==> Cluster and registry removed."
