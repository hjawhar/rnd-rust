#!/usr/bin/env bash
set -euo pipefail

REGISTRY="${REGISTRY:-localhost:5111}"
TAG="${1:-dev}"

echo "==> Applying namespace..."
kubectl apply -f deploy/k8s/namespace.yaml

echo "==> Deploying infrastructure (NATS + PostgreSQL)..."
kubectl apply -f deploy/k8s/nats/
kubectl apply -f deploy/k8s/postgres/

echo "==> Waiting for NATS and PostgreSQL to be ready..."
kubectl -n distributed-system wait --for=condition=available deployment/nats --timeout=60s
kubectl -n distributed-system wait --for=condition=available deployment/postgres --timeout=60s

echo "==> Deploying observability stack..."
kubectl apply -f deploy/k8s/observability/otel-collector.yaml
kubectl apply -f deploy/k8s/observability/jaeger.yaml
kubectl apply -f deploy/k8s/observability/grafana-dashboard.yaml

if ! helm list -n distributed-system | grep -q prometheus; then
    helm repo add prometheus-community https://prometheus-community.github.io/helm-charts 2>/dev/null || true
    helm repo update
    helm install prometheus prometheus-community/kube-prometheus-stack \
        -n distributed-system \
        -f deploy/k8s/observability/prometheus-values.yaml \
        --wait --timeout 5m
fi

echo "==> Updating image references..."
for svc in gateway processor enricher aggregator; do
    sed "s|image: distributed-system/${svc}:dev|image: k3d-dsystem-registry:5111/distributed-system/${svc}:${TAG}|" \
        "deploy/k8s/${svc}/deployment.yaml" | kubectl apply -f -
done

echo "==> Waiting for services to be ready..."
for svc in gateway processor enricher aggregator; do
    kubectl -n distributed-system wait --for=condition=available "deployment/${svc}" --timeout=120s
done

echo "==> All services deployed."
echo "    Gateway:    http://localhost:30000"
echo "    Aggregator: http://localhost:30001"
echo "    Jaeger:     http://localhost:30002"
echo "    Grafana:    http://localhost:30003"
