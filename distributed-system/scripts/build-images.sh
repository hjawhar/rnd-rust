#!/usr/bin/env bash
set -euo pipefail

REGISTRY="${REGISTRY:-localhost:5111}"
TAG="${1:-dev}"
SERVICES="gateway processor enricher aggregator"

for svc in $SERVICES; do
    echo "==> Building ${svc}..."
    docker build \
        -f deploy/docker/Dockerfile \
        --build-arg CRATE_NAME="$svc" \
        -t "${REGISTRY}/distributed-system/${svc}:${TAG}" \
        .

    echo "==> Pushing ${svc}..."
    docker push "${REGISTRY}/distributed-system/${svc}:${TAG}"
done

echo "==> All images built and pushed."
