# corntchap

A self-hosted coordination server for coding agents. Claude Code (cloud and desktop), Codex and OpenCode share one task list per project without stepping on each other: an agent **claims** a task under a time-limited **lease** with a **fencing epoch**, ticks checklist items as it works, and completes it. A relay on your laptop wakes idle local agents; cloud agents are woken by webhook.

- **Server** (`corntchap`): one static binary + Postgres. MCP at `/mcp` (served by [cratestack](https://cratestack.dev) from `crates/server/schema/corntchap.cstack`, MCP revision `2026-07-28`), REST at `/api/v1`, SSE relay stream, read-only board at `/board`.
- **CLI** (`corntchap-cli`): the board for humans, JSON commands for agents that only have a shell, and `relay`, which turns wake events into local agent runs.
- **Auth**: opaque bearer tokens checked by an [authkestra](https://crates.io/crates/authkestra) guard. Humans sign in with GitHub SSO; CLIs, agents and relays enroll with the RFC 8628 device flow (`corntchap-cli login`), approved in the browser.

## Quick start

Needs `mise`, Docker and a GitHub OAuth app (callback `http://localhost:8080/auth/github/callback`). Everything goes through `just`:

```bash
mise install                                   # rust, just, psql
cp .env.example .env                           # GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET
just setup                                     # Postgres via compose + schema
just init <your-github-login>                  # first admin; prints a 7-day bootstrap token
just serve                                     # server on http://localhost:8080
```

In another shell:

```bash
just login                                     # device flow, approved in the browser (or: just login ct_<bootstrap token>)
just seed                                      # team, project DEMO, an agent (token printed once), a task
just cli task ls DEMO
```

Production: set `DATABASE_URL`, `PUBLIC_URL` (https), `GITHUB_CLIENT_ID/SECRET`, then run `corntchap migrate`, `corntchap init --admin <login>`, `corntchap serve` (or `just up` for the container stack, `deploy/` for Kubernetes + CNPG). The same CLI commands work against any server: `corntchap-cli login <url>`, `team add`, `project add`, `agent add <name> --kind … --runtime …`, `team join`, `task add`.

Point an agent at the server (Claude Code `.mcp.json`; keep the token in an env var):

```json
{ "mcpServers": { "corntchap": { "type": "http", "url": "https://corntchap.example.com/mcp",
    "headers": { "Authorization": "Bearer ${CORNTCHAP_TOKEN}" } } } }
```

### Waking agents

```bash
corntchap-cli relay enroll relay-laptop          # device flow, stores the relay credential
corntchap-cli agent wake add opencode-laptop --type relay
$EDITOR ~/.config/corntchap/config.toml          # [[target]] entries: opencode | exec | webhook adapters
corntchap-cli relay doctor && corntchap-cli relay install
# cloud agents: a webhook (e.g. a Claude routine /fire URL) or a github-comment target
corntchap-cli agent wake add claude-cloud-1 --type webhook --url https://… --token …
```

```toml
# ~/.config/corntchap/config.toml
[[target]]
agent = "opencode-laptop"; adapter = "opencode"; url = "http://127.0.0.1:4096"; directory = "~/code/payments"
[[target]]
agent = "codex-laptop"; adapter = "exec"; command = ["codex", "exec", "resume", "--last", "-"]; cwd = "~/code/payments"; timeout = "45m"
```

## Develop

```bash
mise install                 # rust 1.100, just, postgres client (mise.toml)
mise run db                  # Postgres via compose.yaml
docker compose up -d --build # or the whole stack: Postgres + server (copy .env.example to .env first)
just test      # unit + integration (needs Postgres; TEST_DATABASE_ADMIN_URL, default postgres://corn:corn@127.0.0.1/postgres)
just e2e       # real binaries: server + CLI + relay + a fake agent
```

See `deploy/` for Kubernetes + CloudNativePG, and `Containerfile` for the distroless static image (ghcr.io/stephane-segning/corntchap).

### Deviations from the design doc

- Basic-auth/Argon2 is replaced by bearer tokens (device flow + GitHub SSO), per the later direction.
- MCP is cratestack's binding: tool arguments are flat; list arguments are strings (`labels`/`dependsOn` comma-separated, `items` one per line); resources are exposed as the `get_board` tool instead of `corntchap://` URIs.
- Credentials live in `~/.config/corntchap/credentials.toml` (0600) or `CORNTCHAP_TOKEN`; no OS keyring.
- No generated OpenAPI spec yet.
