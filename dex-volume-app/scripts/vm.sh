#!/usr/bin/env bash
#
# vm.sh — Unified service management for the monorepo.
#
# Usage: ./scripts/vm.sh <command> [args]
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKER_DIR="$REPO_ROOT/infra/docker"
ENV_FILE="$REPO_ROOT/.env"
ENV_EXAMPLE="$REPO_ROOT/.env.example"

# Compose files in launch order
COMPOSE_FILES=(
  "$DOCKER_DIR/compose.nats.yaml"
  "$DOCKER_DIR/compose.redis.yaml"
  "$DOCKER_DIR/compose.yaml"
  "$DOCKER_DIR/compose.worker-sol.yaml"
  "$DOCKER_DIR/compose.worker-evm.yaml"
)

# Containers managed by this script (order matters for stop: workers → api → infra)
APP_CONTAINERS=(vm_backend worker_sol worker_evm)
INFRA_CONTAINERS=(vm_db vm_redis vm_nats)
ALL_CONTAINERS=(worker_sol worker_evm vm_backend "${INFRA_CONTAINERS[@]}")

# ─── Colors ─────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  C_RESET=$'\033[0m'; C_GREEN=$'\033[32m'; C_RED=$'\033[31m'
  C_YELLOW=$'\033[33m'; C_BLUE=$'\033[34m'; C_DIM=$'\033[2m'
else
  C_RESET=""; C_GREEN=""; C_RED=""; C_YELLOW=""; C_BLUE=""; C_DIM=""
fi

info()  { printf "%b %s%b\n" "${C_BLUE}→${C_RESET}" "$1" ""; }
ok()    { printf "%b %s%b\n" "${C_GREEN}✓${C_RESET}" "$1" ""; }
warn()  { printf "%b %s%b\n" "${C_YELLOW}!${C_RESET}" "$1" "" >&2; }
fail()  { printf "%b %s%b\n" "${C_RED}✗${C_RESET}" "$1" "" >&2; exit 1; }

# ─── Helpers ────────────────────────────────────────────────────────────────
compose() {
  local file=$1; shift
  docker compose --env-file "$ENV_FILE" -f "$file" "$@"
}

require_env() {
  [[ -f "$ENV_FILE" ]] || fail ".env not found. Run: $0 init"
}

require_docker() {
  command -v docker >/dev/null 2>&1 || fail "docker is not installed"
  docker info >/dev/null 2>&1 || fail "docker daemon is not running"
}

truncate_logs() {
  for c in "$@"; do
    if docker inspect "$c" >/dev/null 2>&1; then
      local logpath
      logpath=$(docker inspect --format='{{.LogPath}}' "$c" 2>/dev/null || true)
      if [[ -n "$logpath" && -w "$logpath" ]]; then
        : > "$logpath" 2>/dev/null || true
      fi
    fi
  done
}

# ─── Commands ───────────────────────────────────────────────────────────────
cmd_genkeys() {
  command -v openssl >/dev/null 2>&1 || fail "openssl is required (install: apt install openssl / brew install openssl)"
  local aes ed_hex
  aes=$(openssl rand -hex 32)
  ed_hex=$(openssl genpkey -algorithm ed25519 -outform DER 2>/dev/null | xxd -p | tr -d '\n')
  echo "# Paste into .env:"
  echo "AES256_GCM_KEY=$aes"
  echo "ED25519_KEY=$ed_hex"
}


cmd_init() {
  [[ -f "$ENV_EXAMPLE" ]] || fail ".env.example not found at $ENV_EXAMPLE"
  if [[ -f "$ENV_FILE" ]]; then
    warn ".env already exists at $ENV_FILE (not overwriting)"
    return 0
  fi
  cp "$ENV_EXAMPLE" "$ENV_FILE"
  ok "Created .env from .env.example"
  echo ""
  echo "Required values to fill in:"
  echo "  ${C_YELLOW}POSTGRES_PASSWORD${C_RESET}   — database password"
  echo "  ${C_YELLOW}REDIS_PASSWORD${C_RESET}      — redis password"
  echo "  ${C_YELLOW}AES256_GCM_KEY${C_RESET}      — 64-char hex (wallet encryption)"
  echo "  ${C_YELLOW}ED25519_KEY${C_RESET}         — Ed25519 keypair (JWT + fee payer)"
  echo ""
  echo "Edit: ${C_DIM}$ENV_FILE${C_RESET}"
}

cmd_up() {
  require_docker; require_env
  for f in "${COMPOSE_FILES[@]}"; do
    info "Starting $(basename "$f" .yaml)..."
    compose "$f" up -d
  done
  ok "All services started"
  echo ""
  cmd_status
}

cmd_down() {
  require_docker
  # Reverse order: tear down app-level first, infra last
  local reversed=()
  for ((i=${#COMPOSE_FILES[@]}-1; i>=0; i--)); do
    reversed+=("${COMPOSE_FILES[i]}")
  done
  for f in "${reversed[@]}"; do
    info "Stopping $(basename "$f" .yaml)..."
    compose "$f" down
  done
  ok "All services stopped"
}

cmd_restart() {
  require_docker; require_env
  info "Truncating logs..."
  truncate_logs "${APP_CONTAINERS[@]}"
  info "Restarting app containers..."
  for c in "${APP_CONTAINERS[@]}"; do
    if docker ps -q -f "name=^${c}$" | grep -q .; then
      docker restart "$c" >/dev/null
      ok "Restarted $c"
    else
      warn "$c not running (skipped)"
    fi
  done
}

cmd_stop() {
  require_docker
  info "Stopping containers..."
  for c in "${ALL_CONTAINERS[@]}"; do
    if docker ps -q -f "name=^${c}$" | grep -q .; then
      docker stop "$c" >/dev/null
      ok "Stopped $c"
    fi
  done
}

cmd_remove() {
  require_docker
  cmd_stop
  info "Removing containers..."
  for c in "${ALL_CONTAINERS[@]}"; do
    if docker ps -aq -f "name=^${c}$" | grep -q .; then
      docker rm "$c" >/dev/null
      ok "Removed $c"
    fi
  done
}

cmd_status() {
  require_docker
  local fmt='{{.Names}}\t{{.State}}\t{{.Status}}\t{{.Ports}}'
  local header=$(printf "NAME\tSTATE\tSTATUS\tPORTS")
  local running
  running=$(docker ps -a --filter "name=^vm_" --filter "name=^worker_" --format "$fmt" 2>/dev/null | sort)
  if [[ -z "$running" ]]; then
    warn "No containers found"
    return 0
  fi
  {
    printf "%s\n" "$header"
    printf "%s\n" "$running"
  } | column -t -s $'\t'
}

cmd_logs() {
  require_docker
  local svc="${1:-}"
  if [[ -z "$svc" ]]; then
    fail "Usage: $0 logs <container>  (e.g. vm_backend, worker_sol, worker_evm, vm_db, vm_redis, vm_nats)"
  fi
  shift || true
  docker logs -f --tail=100 "$svc" "$@"
}

cmd_build() {
  require_docker; require_env
  info "Building backend release binaries..."
  for f in "$DOCKER_DIR/compose.yaml" "$DOCKER_DIR/compose.worker-sol.yaml" "$DOCKER_DIR/compose.worker-evm.yaml"; do
    compose "$f" build
  done
  ok "Build complete"
}

# ─── Server Provisioning (Ubuntu) ───────────────────────────────────────────
require_linux() {
  [[ "$(uname)" == "Linux" ]] || fail "setup commands require Ubuntu/Debian Linux"
}

cmd_setup_deps() {
  require_linux
  info "Installing build dependencies..."
  sudo apt update && sudo apt upgrade -y
  sudo apt install -y build-essential libpq-dev libssl-dev pkg-config
  ok "Build dependencies installed"
}

cmd_setup_docker() {
  require_linux
  info "Installing Docker CE..."
  for pkg in docker.io docker-doc docker-compose docker-compose-v2 podman-docker containerd runc; do
    sudo apt-get remove -y "$pkg" 2>/dev/null || true
  done
  sudo apt-get update -y
  sudo apt-get install -y ca-certificates curl
  sudo install -m 0755 -d /etc/apt/keyrings
  sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  sudo chmod a+r /etc/apt/keyrings/docker.asc
  echo \
    "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu \
    $(. /etc/os-release && echo "${UBUNTU_CODENAME:-$VERSION_CODENAME}") stable" | \
    sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
  sudo apt-get update -y
  sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
  ok "Docker installed"
}

cmd_setup_firewall() {
  require_linux
  info "Configuring UFW firewall..."
  sudo tee /etc/docker/daemon.json > /dev/null <<EOF
{
  "iptables": false
}
EOF
  sudo systemctl restart docker
  sudo ufw --force enable
  sudo ufw allow ssh
  sudo ufw allow http
  sudo ufw allow https
  sudo ufw deny 6379/tcp  # Block Redis from public internet
  if [[ -n "${ADMIN_IP:-}" ]]; then
    sudo ufw allow from "$ADMIN_IP" to any port 6379 proto tcp
    ok "Allowed Redis from $ADMIN_IP"
  else
    warn "Set ADMIN_IP env var to whitelist a trusted IP for Redis (6379/tcp)"
  fi
  sudo ufw reload
  ok "Firewall configured"
}

cmd_setup() {
  local target="${1:-}"
  case "$target" in
    deps)     cmd_setup_deps ;;
    docker)   cmd_setup_docker ;;
    firewall) cmd_setup_firewall ;;
    all)      cmd_setup_deps; cmd_setup_docker; cmd_setup_firewall ;;
    "")       fail "Usage: $0 setup {deps|docker|firewall|all}" ;;
    *)        fail "Unknown setup target: $target (expected: deps|docker|firewall|all)" ;;
  esac
}


cmd_ps() { cmd_status; }
cmd_start() { cmd_up; }

usage() {
  cat <<EOF
${C_BLUE}Usage:${C_RESET} $0 <command> [args]

${C_BLUE}Setup:${C_RESET}
  init               Create .env from .env.example (won't overwrite)
  genkeys            Generate fresh AES256_GCM_KEY and ED25519_KEY (prints to stdout)
  setup <target>     Provision Ubuntu server: deps | docker | firewall | all
                     (firewall: set ADMIN_IP env var to whitelist Redis access)

${C_BLUE}Lifecycle:${C_RESET}
  up                 Start all services in order (nats → redis → db → api → workers)
  down               Stop and remove containers via compose
  restart            Restart app containers (truncates logs first)
  stop               Stop all running containers
  remove             Stop and remove all containers

${C_BLUE}Inspection:${C_RESET}
  status | ps        Show container state
  logs <container>   Tail logs (e.g. vm_backend, worker_sol, worker_evm)

EOF
}

case "${1:-}" in
  init)              cmd_init ;;
  genkeys)           cmd_genkeys ;;
  up|start)          cmd_up ;;
  down)              cmd_down ;;
  restart)           cmd_restart ;;
  stop)              cmd_stop ;;
  remove|rm)         cmd_remove ;;
  status|ps)         cmd_status ;;
  logs)              shift; cmd_logs "$@" ;;
  build)             cmd_build ;;
  setup)             shift; cmd_setup "$@" ;;
  ""|help|-h|--help) usage ;;
  *)                 fail "Unknown command: $1 (run '$0 help')" ;;
esac
