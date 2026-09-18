#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$ROOT_DIR"

echo "=== Bambu Homelab Dev Setup ==="

# --- TLS Certificates ---
if [ ! -f .local/certs/ca.pem ]; then
  echo "Generating TLS certificates..."
  mkdir -p .local/certs

  # SAN includes both 'localhost' (for local dev) and 'nats' (Docker service name)
  docker run --rm -v "$(pwd)/.local/certs:/certs" -w /certs \
    alpine sh -c '
      apk add --no-cache openssl >/dev/null 2>&1

      # Generate CA
      openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
        -days 3650 -nodes -keyout ca.key -out ca.pem \
        -subj "/CN=bambu-dev-ca" 2>/dev/null

      # Generate server cert
      openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
        -nodes -keyout server.key -out server.csr \
        -subj "/CN=nats" 2>/dev/null

      # Sign with CA — SAN covers localhost, nats (Docker), and 127.0.0.1
      echo "subjectAltName=DNS:localhost,DNS:nats,IP:127.0.0.1" > ext.cnf
      openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key \
        -CAcreateserial -out server.pem -days 3650 \
        -extfile ext.cnf 2>/dev/null

      rm -f server.csr ca.srl ext.cnf
    '

  echo "  Certs written to .local/certs/"
else
  echo "  TLS certs already exist, skipping."
fi

# --- NKeys ---
if [ ! -f .local/nkeys/gateway.nk ]; then
  echo "Generating NKeys..."
  mkdir -p .local/nkeys

  docker run --rm -v "$(pwd)/.local/nkeys:/nkeys" natsio/nats-box sh -c '
    nsc generate nkey -u > /nkeys/gateway.nk
    nsc generate nkey -u > /nkeys/printer-dev.nk
  '

  echo "  NKeys written to .local/nkeys/"
else
  echo "  NKeys already exist, skipping."
fi

# --- Read keys ---
GW_PUB=$(grep '^U' .local/nkeys/gateway.nk)
PRINTER_PUB=$(grep '^U' .local/nkeys/printer-dev.nk)
GW_SEED=$(grep '^SU' .local/nkeys/gateway.nk)
PRINTER_SEED=$(grep '^SU' .local/nkeys/printer-dev.nk)

echo "  Gateway public key:  $GW_PUB"
echo "  Printer public key:  $PRINTER_PUB"

# --- Resolve NATS config ---
sed \
  -e "s|PLACEHOLDER_GATEWAY_PUBKEY|$GW_PUB|" \
  -e "s|PLACEHOLDER_PRINTER_PUBKEY|$PRINTER_PUB|" \
  config/nats-server.conf > .local/nats-server-resolved.conf

echo "  NATS config written to .local/nats-server-resolved.conf"

# --- Generate JWT secret (stable across runs if .env.docker exists) ---
if [ -f .env.docker ] && grep -q 'JWT_SECRET' .env.docker; then
  JWT_SECRET=$(grep 'BAMBU_API__JWT_SECRET' .env.docker | cut -d= -f2)
else
  JWT_SECRET=$(openssl rand -hex 32)
fi

# --- Write .env.docker (for gateway + API inside Docker network) ---
cat > .env.docker <<EOF
BAMBU_GATEWAY__NATS__URL=tls://nats:4222
BAMBU_GATEWAY__NATS__NKEY_SEED=$GW_SEED
BAMBU_GATEWAY__NATS__CA_CERT=/app/certs/ca.pem
BAMBU_GATEWAY__LISTEN_ADDR=0.0.0.0:8080
BAMBU_API__NATS__URL=tls://nats:4222
BAMBU_API__NATS__NKEY_SEED=$GW_SEED
BAMBU_API__NATS__CA_CERT=/app/certs/ca.pem
BAMBU_API__LISTEN_ADDR=0.0.0.0:8081
BAMBU_API__DATABASE_URL=postgres://bambu:bambu_dev@postgres:5432/bambu
BAMBU_API__JWT_SECRET=$JWT_SECRET
EOF
echo "  .env.docker written (gateway + API)"

# --- Write .env.bridge (bridge runs on host network, uses localhost) ---
cat > .env.bridge <<EOF
BAMBU_BRIDGE__NATS__URL=tls://127.0.0.1:4222
BAMBU_BRIDGE__NATS__NKEY_SEED=$PRINTER_SEED
BAMBU_BRIDGE__NATS__CA_CERT=/app/certs/ca.pem
BAMBU_BRIDGE__DATABASE_URL=postgres://bambu:bambu_dev@127.0.0.1:5432/bambu
EOF
echo "  .env.bridge written (bridge on host network)"

echo ""
echo "=== Setup complete ==="
echo ""
echo "Then run:"
echo "  docker compose up -d          # start everything"
echo "  docker compose logs -f api    # watch API logs (admin password on first run)"
echo "  cd dashboard && npx ng serve  # start Angular dev server"
echo ""
echo "Add printers via the dashboard or API after logging in."