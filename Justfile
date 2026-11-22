set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

# Static musl target (needs `musl-tools` on Debian/Ubuntu: apt install musl-tools).
target := "x86_64-unknown-linux-musl"
image  := "ghcr.io/cratestack/corntchap"

# Local settings: random ports from `just setup` live in .env.dev; optional overrides in .env.
# .env.dev is loaded first, then .env, so values in .env win. Every recipe that needs them starts with {{load}}.
load := "set -a; if [ -f .env.dev ]; then . ./.env.dev; fi; if [ -f .env ]; then . ./.env; fi; set +a;"

default:
    @just --list

build:
    cargo build --workspace

release:
    cargo build --workspace --release

test:
    {{load}} cargo test --workspace

# Needs Postgres reachable at TEST_DATABASE_ADMIN_URL (`just setup` / `just db` provide it).
e2e: build
    {{load}} scripts/e2e.sh

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

run-server:
    {{load}} AUTO_MIGRATE=1 cargo run -p corntchap -- serve

# Fully static binaries (corntchap and corntchap-cli) under target/<target>/release.
static:
    rustup target add {{target}}
    CC_x86_64_unknown_linux_musl=${CC_x86_64_unknown_linux_musl:-musl-gcc} \
      cargo build --release --target {{target}} --workspace
    file target/{{target}}/release/corntchap target/{{target}}/release/corntchap-cli

# Build the server image with podman, falling back to docker.
image tag="dev":
    #!/usr/bin/env bash
    set -euo pipefail
    rt=$(command -v podman || command -v docker)
    "$rt" build -f Containerfile -t {{image}}:{{tag}} .

# Pick random free ports once (writes .env.dev); delete the file to re-roll.
env-init:
    @python3 scripts/devenv.py --ensure .env.dev

# Postgres 18 + Keycloak (dev realm) + RustFS (files) via compose, on random loopback ports from .env.dev.
infra: env-init
    {{load}} docker compose up -d --wait db keycloak rustfs

# Alias kept for muscle memory.
db: infra

# The whole stack: Postgres + server image (server on APP_PORT).
up: env-init
    {{load}} docker compose up -d --build

# ---- Quick start: just setup && just init && just serve (then, in another shell: just seed) ----

# Trust mise.toml and install the toolchain (skipped when mise is not installed).
deps:
    if command -v mise >/dev/null; then mise trust -y && mise install; else echo "mise not found: install Rust 1.100 yourself (rust-toolchain.toml pins it) (https://mise.jdx.dev)"; fi

# Toolchain, Postgres + Keycloak (compose, random ports) and the schema.
setup: deps infra
    {{load}} cargo run -q -p corntchap -- migrate
    @{{load}} echo "Postgres: 127.0.0.1:$PG_PORT   Keycloak: http://localhost:$KC_PORT (users admin/admin, dev/dev)   server: $PUBLIC_URL"

# Create the first admin (an OIDC username; the dev realm has `admin`) and sign the CLI in with its 7-day bootstrap token.
init admin="admin": env-init
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; if [ -f .env.dev ]; then . ./.env.dev; fi; if [ -f .env ]; then . ./.env; fi; set +a
    out=$(cargo run -q -p corntchap -- init --admin "{{admin}}")
    # the bootstrap token is a secret: show it once, never echo the login command
    echo "$out"
    tok=$(printf '%s\n' "$out" | sed -n 's/^bootstrap token[^:]*: //p')
    if [ -n "$tok" ]; then
      cargo run -q -p corntchap-cli -- login "$PUBLIC_URL" --token "$tok" --no-verify
      echo "stored the bootstrap token as your CLI login for $PUBLIC_URL (next: just serve, then just seed)"
    else
      echo "no new bootstrap token was printed (admin already exists?): run \`just login\` to sign in"
    fi

# Run the server on the random APP_PORT (OIDC settings come from .env.dev).
serve: env-init run-server

# Any corntchap-cli command: `just cli task ls PAY`.
cli *args:
    @cargo run -q -p corntchap-cli -- "$@"

# Sign in with the device flow (opens the browser). Pass a token to skip it: `just login ct_...`.
login token="": env-init
    @{{load}} cargo run -q -p corntchap-cli -- login "$PUBLIC_URL" ${1:+--token "$1"}

# Demo data: team acme, project DEMO, one agent (its token is printed once), one task. Re-runnable: steps that
# already exist are skipped (the agent token is only printed the first time).
seed:
    #!/usr/bin/env bash
    set -uo pipefail
    step() {
      local label=$1; shift
      if out=$(cargo run -q -p corntchap-cli -- "$@" 2>&1); then
        printf '%s\n' "$out"
      elif printf '%s' "$out" | grep -qiE '409|already exists|conflict|exists'; then
        echo "skip: $label (already exists)"
      else
        printf '%s\n' "$out" >&2; echo "seed failed at: $label" >&2; exit 1
      fi
    }
    step "team acme" team add acme
    step "project DEMO" project add acme DEMO Demo
    step "agent demo-agent" agent add demo-agent --kind other --runtime local
    step "team membership" team join acme demo-agent
    if cargo run -q -p corntchap-cli -- task ls DEMO 2>/dev/null | grep -q "Try the board"; then
      echo "skip: task 'Try the board' (already exists)"
    else
      step "task" task add DEMO "Try the board" --item "claim me" --item "tick me"
    fi

# Drive a running server with the official MCP Python SDK. `CORNTCHAP_TOKEN=<agent token> just mcp-check`
mcp-check project="DEMO":
    {{load}} [ -d .venv-mcp ] || python3 -m venv .venv-mcp
    .venv-mcp/bin/pip install -q mcp
    {{load}} CORNTCHAP_URL="$PUBLIC_URL" PROJECT={{project}} .venv-mcp/bin/python scripts/mcp_check.py

# Stop the compose stack; the database volume is kept (your tokens stay valid).
down: env-init
    {{load}} docker compose down

# Stop the compose stack, DELETE its data and forget the stored CLI tokens (they die with the database).
reset: env-init
    {{load}} docker compose down -v
    cargo run -q -p corntchap-cli -- logout --all --local
    @echo "reset: database deleted and stored CLI tokens removed. Start over with: just setup && just init && just serve"

# Compose status / logs for the local stack.
ps: env-init
    {{load}} docker compose ps

# e.g. `just logs -f corntchap`
logs *args: env-init
    @{{load}} docker compose logs "$@"

# Same check with the official TypeScript MCP SDK (handshake-era revisions). `CORNTCHAP_TOKEN=<agent token> just mcp-check-ts`
mcp-check-ts project="DEMO":
    {{load}} mkdir -p .venv-mcp-ts && cd .venv-mcp-ts && ([ -d node_modules/@modelcontextprotocol ] || npm i --silent @modelcontextprotocol/sdk@1.32.2) && cd .. && cp scripts/mcp_check_ts.mjs .venv-mcp-ts/check.mjs && CORNTCHAP_URL="$PUBLIC_URL" PROJECT={{project}} node .venv-mcp-ts/check.mjs

# Put corntchap-cli and corntchap on your PATH (~/.cargo/bin).
install:
    cargo install --locked --path crates/cli
    cargo install --locked --path crates/server

# Regenerate the TypeScript client (+ refine resource manifest) for the console from the schema.
# `just gen-client --check` only reports drift (CI guard). Needs node/npx; the cratestack binary
# is fetched by npm at the same version as the Rust crates.
gen-client *flags:
    npx --yes @cratestack/cli@0.20.6 generate-typescript \
      --schema crates/server/schema/corntchap.cstack --out console/src/gen \
      --package-name @corntchap/api --base-path /api/v2 --refine "$@"

# ---- Web console (console/: Vite + React + Refine + Tailwind + shadcn/ui; embedded in the server binary) ----

# The generated client (console/src/gen) is never committed: console-build/-check/-dev run `gen-client` first.
# Install the console's dependencies from the lockfile (pnpm; mise installs node and pnpm).
console-install:
    cd console && pnpm install --frozen-lockfile

# Build console/dist and nudge cargo to re-embed it (`just build` / `just serve` pick it up).
console-build: gen-client console-install
    cd console && pnpm build
    touch crates/server/build.rs

# Typecheck + lint the console (CI runs this before cargo).
console-check: gen-client console-install
    cd console && pnpm typecheck && pnpm lint

# Vite dev server with hot reload, proxying /api, /auth, /logout, /oauth and /device to `just serve`.
# Sign in once at $PUBLIC_URL (cookies are shared across ports on localhost), then open http://localhost:5173.
console-dev: env-init gen-client
    {{load}} cd console && ([ -d node_modules ] || pnpm install --frozen-lockfile) && CORNTCHAP_URL="$PUBLIC_URL" pnpm dev
