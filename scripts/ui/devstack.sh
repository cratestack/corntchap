#!/usr/bin/env bash
# Dev stack for console work: docker services, the console build and the server, in the background.
# Usage: scripts/ui/devstack.sh            (from the repo root; logs in $LOG_DIR, default /tmp/corntchap-ui)
set -euo pipefail
cd "$(dirname "$0")/../.."
LOG_DIR=${LOG_DIR:-/tmp/corntchap-ui}
mkdir -p "$LOG_DIR"
if ! docker info >/dev/null 2>&1; then
  setsid nohup dockerd >"$LOG_DIR/dockerd.log" 2>&1 < /dev/null &
  for _ in $(seq 1 60); do docker info >/dev/null 2>&1 && break; sleep 1; done
fi
docker compose --env-file .env.dev up -d db rustfs keycloak >/dev/null
set -a; . ./.env.dev; set +a
(cd console && npx --yes @cratestack/cli@0.20.6 generate-typescript --schema ../crates/server/schema/corntchap.cstack \
   --out src/gen --package-name @corntchap/api --base-path /api/v2 --refine >/dev/null && pnpm -s build >"$LOG_DIR/console.log" 2>&1)
# the console is embedded in the binary (rust-embed with debug-embed): re-embed the fresh build
touch crates/server/src/console.rs
cargo build -q -p corntchap
# wait for Keycloak's realm before the server discovers it
for _ in $(seq 1 90); do curl -sf "${OIDC_ISSUER}/.well-known/openid-configuration" >/dev/null && break; sleep 2; done
if [ -f "$LOG_DIR/server.pid" ] && kill -0 "$(cat "$LOG_DIR/server.pid")" 2>/dev/null; then kill "$(cat "$LOG_DIR/server.pid")"; sleep 1; fi
AUTO_MIGRATE=1 setsid nohup ./target/debug/corntchap serve >"$LOG_DIR/server.log" 2>&1 < /dev/null &
echo $! >"$LOG_DIR/server.pid"
for _ in $(seq 1 60); do curl -sf "$PUBLIC_URL/healthz" >/dev/null 2>&1 || curl -sf -o /dev/null "$PUBLIC_URL/" && break; sleep 1; done
echo "server up at $PUBLIC_URL (pid $(cat "$LOG_DIR/server.pid")), logs in $LOG_DIR"
