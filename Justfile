set shell := ["bash", "-euo", "pipefail", "-c"]

# Static musl target (needs `musl-tools` on Debian/Ubuntu: apt install musl-tools).
target := "x86_64-unknown-linux-musl"
image  := "ghcr.io/stephane-segning/corntchap"

# Local settings: random ports from `just setup` live in .env.dev; optional overrides in .env.
# Every recipe that needs them starts with {{load}}.
load := "set -a; [ -f .env ] && . ./.env; [ -f .env.dev ] && . ./.env.dev; set +a;"

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
    [ -f .env.dev ] || python3 scripts/devenv.py > .env.dev

# Postgres 18 + Keycloak (dev realm) via compose, on random loopback ports from .env.dev.
infra: env-init
    {{load}} docker compose up -d --wait db keycloak

# Alias kept for muscle memory.
db: infra

# The whole stack: Postgres + server image (server on APP_PORT).
up: env-init
    {{load}} docker compose up -d --build

# ---- Quick start: just setup && just init && just serve (then, in another shell: just login && just seed) ----

# Trust mise.toml and install the toolchain (skipped when mise is not installed).
deps:
    if command -v mise >/dev/null; then mise trust -y && mise install; else echo "mise not found: install Rust 1.100+ yourself (https://mise.jdx.dev)"; fi

# Toolchain, Postgres + Keycloak (compose, random ports) and the schema.
setup: deps infra
    {{load}} cargo run -q -p corntchap -- migrate
    @{{load}} echo "Postgres: 127.0.0.1:$PG_PORT   Keycloak: http://localhost:$KC_PORT (users admin/admin, dev/dev)   server: $PUBLIC_URL"

# Create the first admin (an OIDC username; the dev realm has `admin`); prints a 7-day bootstrap token.
init admin="admin": env-init
    {{load}} cargo run -q -p corntchap -- init --admin {{admin}}

# Run the server on the random APP_PORT (OIDC settings come from .env.dev).
serve: env-init run-server

# Any corntchap-cli command: `just cli task ls PAY`.
cli *args:
    cargo run -q -p corntchap-cli -- {{args}}

# Sign in with the device flow (opens the browser). Pass a token to skip it: `just login ct_...`.
login token="": env-init
    {{load}} cargo run -q -p corntchap-cli -- login "$PUBLIC_URL" {{ if token == "" { "" } else { "--token " + token } }}

# Demo data: team acme, project DEMO, one agent (its token is printed once), one task.
seed:
    cargo run -q -p corntchap-cli -- team add acme
    cargo run -q -p corntchap-cli -- project add acme DEMO Demo
    cargo run -q -p corntchap-cli -- agent add demo-agent --kind other --runtime local
    cargo run -q -p corntchap-cli -- team join acme demo-agent
    cargo run -q -p corntchap-cli -- task add DEMO "Try the board" --item "claim me" --item "tick me"

# Drive a running server with the official MCP Python SDK. `CORNTCHAP_TOKEN=<agent token> just mcp-check`
mcp-check project="DEMO":
    {{load}} [ -d .venv-mcp ] || python3 -m venv .venv-mcp
    .venv-mcp/bin/pip install -q mcp
    {{load}} CORNTCHAP_URL="$PUBLIC_URL" PROJECT={{project}} .venv-mcp/bin/python scripts/mcp_check.py

# Stop the compose stack and delete its data.
down: env-init
    {{load}} docker compose down -v

# Compose status / logs for the local stack.
ps: env-init
    {{load}} docker compose ps

logs *args: env-init
    {{load}} docker compose logs {{args}}
