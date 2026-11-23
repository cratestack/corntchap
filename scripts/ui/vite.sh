#!/usr/bin/env bash
# The console's Vite dev server for this checkout (hot reload), proxying to the server from devstack.sh.
# Usage: scripts/ui/vite.sh <port>   then: CONSOLE_URL=http://localhost:<port> node scripts/ui/shots.mjs ...
# Several worktrees can each run one on their own port against the same server.
set -euo pipefail
cd "$(dirname "$0")/../../console"
PORT=${1:?port}
LOG_DIR=${LOG_DIR:-/tmp/corntchap-ui}
mkdir -p "$LOG_DIR"
[ -d node_modules ] || pnpm install --frozen-lockfile >/dev/null
[ -f src/gen/src/index.ts ] || npx --yes @cratestack/cli@0.20.6 generate-typescript --schema ../crates/server/schema/corntchap.cstack \
  --out src/gen --package-name @corntchap/api --base-path /api/v2 --refine >/dev/null
CORNTCHAP_URL=${PUBLIC_URL:-http://localhost:41011} setsid nohup npx vite --port "$PORT" --strictPort --host 127.0.0.1 \
  >"$LOG_DIR/vite-$PORT.log" 2>&1 < /dev/null &
echo $! >"$LOG_DIR/vite-$PORT.pid"
for _ in $(seq 1 60); do curl -sf -o /dev/null "http://localhost:$PORT/" && break; sleep 1; done
echo "vite on http://localhost:$PORT (pid $(cat "$LOG_DIR/vite-$PORT.pid"))"
