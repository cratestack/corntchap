set shell := ["bash", "-euo", "pipefail", "-c"]

# Static musl target (needs `musl-tools` on Debian/Ubuntu: apt install musl-tools).
target := "x86_64-unknown-linux-musl"
image  := "ghcr.io/stephane-segning/corntchap"
pg_url := "postgres://corn:corn@127.0.0.1/postgres"

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
    DATABASE_URL=${DATABASE_URL:-{{pg_url}}} AUTO_MIGRATE=1 cargo run -p corntchap -- serve

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

# Throwaway local Postgres 16 matching the test defaults.
db:
    docker run -d --name corntchap-pg -p 5432:5432 \
      -e POSTGRES_USER=corn -e POSTGRES_PASSWORD=corn postgres:16
