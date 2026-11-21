set shell := ["bash", "-euo", "pipefail", "-c"]

# Static musl target (needs `musl-tools` on Debian/Ubuntu: apt install musl-tools).
target := "x86_64-unknown-linux-musl"
image  := "ghcr.io/stephane-segning/corntchap"
pg_url  := "postgres://corn:corn@127.0.0.1/postgres"
dev_url := "postgres://corn:corn@127.0.0.1:5432/corntchap"
server  := "http://localhost:8080"

default:
    @just --list

build:
    cargo build --workspace

release:
    cargo build --workspace --release

test:
    cargo test --workspace

# Needs Postgres reachable at TEST_DATABASE_ADMIN_URL (see `just db`).
e2e: build
    TEST_DATABASE_ADMIN_URL=${TEST_DATABASE_ADMIN_URL:-{{pg_url}}} scripts/e2e.sh

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

run-server:
    DATABASE_URL=${DATABASE_URL:-{{dev_url}}} AUTO_MIGRATE=1 cargo run -p corntchap -- serve

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

# Local Postgres 16 via compose (matches the test defaults on 127.0.0.1:5432).
db:
    docker compose up -d --wait db

# The whole stack: Postgres + server image.
up:
    docker compose up -d --build

# ---- Quick start: just setup && just init <github-login> && just serve (then, in another shell: just login && just seed) ----

# Start Postgres (compose) and apply the schema.
setup: db
    DATABASE_URL=${DATABASE_URL:-{{dev_url}}} cargo run -q -p corntchap -- migrate

# Create the first admin (a GitHub login); prints a 7-day bootstrap token.
init admin:
    DATABASE_URL=${DATABASE_URL:-{{dev_url}}} cargo run -q -p corntchap -- init --admin {{admin}}

# Run the server on http://localhost:8080 (set GITHUB_CLIENT_ID/GITHUB_CLIENT_SECRET for browser sign-in).
serve: run-server

# Any corntchap-cli command: `just cli task ls PAY`.
cli *args:
    cargo run -q -p corntchap-cli -- {{args}}

# Sign in with the device flow (opens the browser). Pass a token to skip it: `just login ct_...`.
login token="":
    cargo run -q -p corntchap-cli -- login {{server}} {{ if token == "" { "" } else { "--token " + token } }}

# Demo data: team acme, project DEMO, one agent (its token is printed once), one task.
seed:
    cargo run -q -p corntchap-cli -- team add acme
    cargo run -q -p corntchap-cli -- project add acme DEMO Demo
    cargo run -q -p corntchap-cli -- agent add demo-agent --kind other --runtime local
    cargo run -q -p corntchap-cli -- team join acme demo-agent
    cargo run -q -p corntchap-cli -- task add DEMO "Try the board" --item "claim me" --item "tick me"
