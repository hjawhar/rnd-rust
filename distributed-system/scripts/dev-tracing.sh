#!/usr/bin/env bash
# Same as dev.sh but with distributed tracing enabled.
# Traces will appear in Jaeger at http://127.0.0.1:16686
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
exec "$(dirname "$0")/dev.sh"
