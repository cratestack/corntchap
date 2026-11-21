#!/usr/bin/env bash
# End-to-end with the real binaries: server + CLI + relay (exec adapter) + a fake agent.
# Needs a Postgres superuser URL in TEST_DATABASE_ADMIN_URL (default: postgres://corn:corn@127.0.0.1/postgres).
set -euo pipefail
cd "$(dirname "$0")/.."
ADMIN_URL=${TEST_DATABASE_ADMIN_URL:-postgres://corn:corn@127.0.0.1/postgres}
BIN=${BIN_DIR:-target/debug}
DB=ct_e2e_$$
PORT=${PORT:-$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')}
WORK=$(mktemp -d)
cleanup() { kill ${SERVER_PID:-} ${RELAY_PID:-} 2>/dev/null || true; psqlc "drop database if exists $DB" >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

# Use a local psql if there is one, else the compose Postgres container (nothing to install).
psqlc() { if command -v psql >/dev/null; then psql "$ADMIN_URL" -qc "$1"; else docker compose exec -T db psql -U corn -d postgres -qc "$1"; fi; }
psqlc "create database $DB"
export DATABASE_URL="${ADMIN_URL%/*}/$DB" BIND=127.0.0.1:$PORT PUBLIC_URL=http://127.0.0.1:$PORT RUST_LOG=warn
export CORNTCHAP_CONFIG_DIR=$WORK/cfg CORNTCHAP_NO_BROWSER=1
SERVER=http://127.0.0.1:$PORT
CLI=$(cd "$BIN" && pwd)/corntchap-cli

$BIN/corntchap migrate
BOOT=$($BIN/corntchap init --admin octo | sed -n 's/^bootstrap token[^:]*: //p')
$BIN/corntchap serve >"$WORK/server.log" 2>&1 & SERVER_PID=$!
for i in $(seq 50); do curl -sf $SERVER/healthz >/dev/null && break; sleep 0.2; done

step() { printf '\n== %s\n' "$*"; }
step "login"; $CLI login $SERVER --token "$BOOT"
step "team, project, agent"
$CLI team add acme; $CLI project add acme PAY Payments
AGENT_TOKEN=$($CLI agent add fake-agent --kind other --runtime local | awk '/^  ct_/{print $1}')
$CLI team join acme fake-agent
$CLI agent wake add fake-agent --type relay
RELAY_TOKEN=$(curl -sf -H "Authorization: Bearer $BOOT" -H 'content-type: application/json' -d '{"name":"relay-test"}' $SERVER/api/v1/relays | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')

step "relay config"
cat >"$WORK/agent.sh" <<AGENT
#!/usr/bin/env bash
# stand-in for a coding agent: the prompt arrives on stdin, work goes through corntchap-cli
export CORNTCHAP_TOKEN=$AGENT_TOKEN CORNTCHAP_SERVER=$SERVER
cat >"$WORK/prompt.txt"
while :; do
  R=\$($CLI task claim-next --project PAY --json)
  [ "\$(echo "\$R" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("none",False))')" = True ] && break
  ID=\$(echo "\$R" | python3 -c 'import sys,json;print(json.load(sys.stdin)["task"]["id"])')
  EP=\$(echo "\$R" | python3 -c 'import sys,json;print(json.load(sys.stdin)["lease"]["epoch"])')
  $CLI task check \$ID --epoch \$EP --item 1
  $CLI task complete \$ID --epoch \$EP --result "done by fake agent" >/dev/null
done
AGENT
chmod +x "$WORK/agent.sh"
mkdir -p "$WORK/cfg"
cat >"$WORK/cfg/config.toml" <<TOML
[[target]]
agent   = "fake-agent"
adapter = "exec"
command = ["$WORK/agent.sh"]
cwd     = "$WORK"
timeout = "1m"
TOML
CORNTCHAP_RELAY_TOKEN=$RELAY_TOKEN $CLI relay doctor
CORNTCHAP_RELAY_TOKEN=$RELAY_TOKEN $CLI relay >"$WORK/relay.log" 2>&1 & RELAY_PID=$!
sleep 1.5

step "create work; the relay should wake the agent, which drains the queue"
$CLI task add PAY "First" --item "step one"
$CLI task add PAY "Second" --item "step one" --depends PAY-1 --priority 3
for i in $(seq 60); do
  [ "$($CLI task ls PAY --status done --json | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')" = 2 ] && break
  sleep 0.5
done
$CLI task ls PAY
DONE=$($CLI task ls PAY --status done --json | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')
[ "$DONE" = 2 ] || { echo "FAIL: expected 2 done tasks, got $DONE"; cat "$WORK/relay.log" "$WORK/server.log"; exit 1; }
grep -q "New work is ready in PAY" "$WORK/prompt.txt" || { echo "FAIL: wake prompt missing"; exit 1; }
$CLI task show PAY-2
echo; echo "E2E OK"
