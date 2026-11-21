# corntchap

A self-hosted coordination server for coding agents. Claude Code (cloud and desktop), Codex and OpenCode share one task list per project without stepping on each other: an agent **claims** a task under a time-limited **lease** with a **fencing epoch**, ticks checklist items as it works, and completes it. A relay on your laptop wakes idle local agents; cloud agents are woken by webhook.

- **Server** (`corntchap`): one static binary + Postgres. MCP at `/mcp` (tools served by [cratestack](https://cratestack.dev) from `crates/server/schema/corntchap.cstack`; revision `2026-07-28` natively, plus a compatibility front door for 2025-11-25 / 2025-06-18 / 2025-03-26 / 2024-11-05 clients such as the TypeScript SDK), REST at `/api/v1`, SSE relay stream, read-only board at `/board`.
- **CLI** (`corntchap-cli`): the board for humans, JSON commands for agents that only have a shell, and `relay`, which turns wake events into local agent runs.
- **Auth**: opaque bearer tokens checked by an [authkestra](https://crates.io/crates/authkestra) guard. Humans sign in with OIDC SSO (Keycloak out of the box); CLIs, agents and relays enroll with the RFC 8628 device flow (`corntchap-cli login`), approved in the browser.

## Quick start

Needs `mise`, Docker and `python3` (random-port picker, e2e and MCP checks), nothing else to configure: compose starts Postgres 18 and Keycloak with a ready-made realm (users `admin`/`admin` and `dev`/`dev`). Ports are random on purpose: `just setup` picks free ones (saved in `.env.dev`) and prints them. Everything goes through `just`:

```bash
mise trust && mise install                      # one-time: rust 1.100 (pinned), just (`just setup` also does this)
just setup                                      # Postgres + Keycloak via compose, then the schema
just init                                       # first admin = Keycloak user `admin`; signs the CLI in with the printed 7-day bootstrap token
just serve                                     # server on the random port printed by setup
```

In another shell:

```bash
just seed                                      # team, project DEMO, an agent (token printed once), a task
just install                                   # corntchap-cli and corntchap on your PATH (cargo install --locked; ~/.cargo/bin)
corntchap-cli kitchen                          # the live board; or: corntchap-cli task ls DEMO   (just cli … works without installing)
corntchap-cli marabou                           # server, login, relay credential and config in one check
```

`just init` stores the bootstrap token for you (`just login` is only needed to sign in again, e.g. through the browser device flow as admin/admin; `just login <token>` stores a token you already have, tokens start with `ct_`). `just down` stops the containers and keeps the database; `just reset` also deletes the database and your stored CLI tokens (they die with it), so start over with `just setup && just init`. If a command says the server rejected your stored credential, run `corntchap-cli login <server>`; `corntchap-cli logout` signs out (revokes the token on the server and deletes it locally).

`corntchap-cli user add <login>` is optional pre-provisioning: users are also created on their first OIDC sign-in.

### The CLI

Corntchap is corn and beans cooked slowly in a pot over a fire, and the CLI borrows the kitchen: `kitchen` is the live board, `cook` manages the agents (the cooks), `fire` is the relay that keeps the fire lit and wakes your local cooks, `marabou` (the marabout) checks the pot, `task scoop` takes the next ready portion and `task serve` completes one by hand. Every old name (`tui`, `agent`, `relay`, `doctor`, `task claim-next`, `task done`) still works as an alias; REST paths, MCP tools and JSON are unchanged.

Human output is tidy tables and short confirmations (colour only on a TTY, never with `NO_COLOR`); every listing/show command takes `--json`. The agent-mode task commands (`scoop` (alias `claim-next`), `claim`, `check`, `heartbeat`, `complete`, `release`, `block`, `comment`, `changes`) keep emitting JSON for agents.

- `corntchap-cli kitchen`: live board (ratatui). Left: projects; middle: tasks grouped by status with checklist progress and holder; right: the selected task (body, checklist, dependencies, comments). Refreshes every 2.5 s. Keys: arrows or `j`/`k` move, `Tab` switches pane, `Enter` opens, `a` add task (title, then items one per line; a blank line or Ctrl-S saves), `c` comment, `x`/Space tick the highlighted checklist item (no lease needed), `d` done, `u` unblock, `X` cancel (asks first), `p` toggle the project's pantry (memories list plus full body, read-only; `j`/`k` scroll in the right pane), `r` refresh, `?` help, `q` quit.
- `corntchap-cli pantry` (alias `memory`): the pot's shared memory, durable knowledge saved under stable keys (kinds: note, convention, gotcha, decision, howto, fact). `pantry list <POT> [-q TEXT] [--kind K] [--tag T] [--limit N]` (alias `ls`; best matches first with `-q`), `pantry search <POT> <words…>`, `pantry show <POT> <KEY>`, `pantry stock <POT> --title T [--key K] [--kind K] [--tag T] [--task PAY-1] [--append] [--revision N] [--body TEXT | --file PATH]` (aliases `add`, `remember`; the body comes from stdin when neither is given; saving the same key updates it, and `--revision` turns a concurrent edit into a conflict instead of an overwrite), `pantry toss <POT> <KEY> [--yes]` (aliases `rm`, `forget`; asks on a terminal), `pantry digest <POT>` (alias `read`; markdown). All but `stock`/`toss`/`digest` take `--json`.
- `corntchap-cli marabou`: checks the server, your login, the relay credential and `config.toml`.
- `corntchap-cli logout [--relay | --all]`.

Production: set `DATABASE_URL`, `PUBLIC_URL` (https), `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` (a confidential client whose redirect URI is `$PUBLIC_URL/auth/oidc/callback`), then run `corntchap migrate`, `corntchap init --admin <login>`, `corntchap serve`, or deploy the image (`ghcr.io/stephane-segning/corntchap`) with `deploy/` for Kubernetes + CNPG. (`just up` is the *development* stack with the dev Keycloak realm and well-known passwords; never expose it.) The same CLI commands work against any server: `corntchap-cli login <url>`, `team add`, `project add`, `agent add <name> --kind … --runtime …`, `team join`, `task add`.

Point an agent at the server (Claude Code `.mcp.json`; keep the token in an env var):

```json
{ "mcpServers": { "corntchap": { "type": "http", "url": "https://corntchap.example.com/mcp",
    "headers": { "Authorization": "Bearer ${CORNTCHAP_TOKEN}" } } } }
```

### Waking agents

```bash
corntchap-cli fire enroll relay-laptop          # device flow, stores the relay credential
corntchap-cli fire init                         # writes a commented config.toml (never overwrites; --force)
corntchap-cli cook setup claude-laptop --kind claude-code --runtime local --team acme
#   creates the agent, joins the team, adds a relay wake target, and prints the MCP config
#   (Claude Code .mcp.json / Codex config.toml / OpenCode opencode.json) plus the [[target]] block to paste into config.toml
corntchap-cli fire doctor                       # or `marabou`: checks the server, the credential and each target
corntchap-cli fire install                      # systemd user unit (Linux) / launchd agent (macOS)
corntchap-cli cook wake ls                      # all your agents' wake targets
# cloud agents: a webhook (e.g. a Claude routine /fire URL) or a github-comment target
corntchap-cli cook wake add claude-cloud-1 --type webhook --url https://… --token …
# an agent registered by device flow instead (token printed once): corntchap-cli cook enroll <name> --kind codex --runtime local
```

Config and credentials live in `~/.config/corntchap/` on Linux and `~/Library/Application Support/corntchap/` on macOS; set `CORNTCHAP_CONFIG_DIR` to use another directory. Example `config.toml` (the optional top-level `server = "https://…"` overrides the login's server; `timeout` takes `90`, `90s`, `45m`, `1h30m`):

```toml
[[target]]
agent = "opencode-laptop"
adapter = "opencode"
directory = "~/code/payments"   # wakes the newest OpenCode session in this directory
# url = "http://127.0.0.1:4096" # optional: the running OpenCode (desktop, TUI or `opencode serve`) is found on its own

[[target]]
agent = "codex-laptop"
adapter = "exec"
command = ["codex", "exec", "resume", "--last", "-"]
cwd = "~/code/payments"
timeout = "45m"

[[target]]
agent = "claude-laptop"
adapter = "exec"
command = ["claude", "-p", "--continue"]
cwd = "~/code/payments"
```

OpenCode's desktop app and TUI pick a new port on every start, so the `opencode` adapter finds the running server itself: `OPENCODE_PORT`, then running `opencode*` processes (their `--port`, or the ports they listen on), then `127.0.0.1:4096`, preferring the one with a session in `directory`. A password-protected server gets its password from `password_env` / `password` on the target, `OPENCODE_SERVER_PASSWORD`, or (Linux) the OpenCode process's own environment. `corntchap-cli fire doctor` shows what it found; `corntchap-cli fire poke <cook> --pot PAY` sends a wake right away to check the wiring. (OpenCode 1.x server API; the 2.x API is not supported yet.)

The relay remembers the last event id it saw (`relay-state.json` next to `credentials.toml`) and sends it as `Last-Event-ID` on restart, so wakes sent while it was down are replayed.

### The pantry: shared memory per project

Every project has a pantry: what agents and people learned about it (conventions, gotchas, decisions and their reasons, how to build/test/deploy, facts about the system), saved under a stable key so the next agent starts from it instead of rediscovering it. MCP tools: `read_pantry` (the whole pantry as markdown, read once when starting on a project), `recall` (ranked full-text search with kind/tag filters; excerpts), `get_memory`, `remember` (create or update by key; `append`; pass the `revision` you read and a concurrent edit is refused instead of lost) and `forget` (agents only forget what they saved). Overwrites keep the previous version (`memory_revisions`); saves and deletions appear in the change feed (`memory.saved`, `memory.forgotten`). Also at `/api/v1/projects/{key}/pantry`, as the `memories` MCP resource and the read-only `Memory` model at `/api/v2`. Visibility follows the project: whoever sees the project shares its pantry.

## Architecture

The server is built on [cratestack](https://cratestack.dev) 0.20 (`crates/server/schema/`: the root `corntchap.cstack` lists its parts under `types/`, `procedures/` and `models/`) and [authkestra](https://crates.io/crates/authkestra) 0.15.

- **Generated REST at `/api/v2`**: every table is a cratestack model (`@@map`/`@map` onto the snake_case tables) with `@@allow`/`@@deny` policies: relays see nothing, admins see everything, members see the projects, tasks, items, comments and events of their teams, and an agent may edit only a task it currently holds. Secrets (`oidc_sub`, tokens) are `@server_only` or not modelled. Lease and fencing operations (`claimNext`, `claimTask`, `heartbeat`, `checkItem`, `complete`, `release`, `block`, `unblockTask`, `cancelTask`, ...) are procedures with raw SQL, served at `POST /api/v2/$procs/<name>` and as MCP tools; all procedure outputs are typed (MCP `outputSchema`). Print the route table with `cargo test -p corntchap --test generated_api routes -- --nocapture`.
- **TypeScript client**: `just gen-client` writes `console/src/gen` (client, types and the `@cratestack/refine` resource manifest) from the schema; `just gen-client --check` reports drift.
- **Migrations**: sqlx (`crates/server/migrations`, `corntchap migrate`, idempotent). Migrations are append-only: never edit one that has shipped (CI rejects it), add a new file. cratestack's `migrate baseline|diff` and `apply_pending` were evaluated and not adopted: they track a second history table (`cratestack_migrations`) and diff from the models, but our schema carries things the models do not describe (triggers, `notify_event`, check constraints, generated mirror columns, columns deliberately left out of models), so the generated DDL would not reproduce it. Revisit once models cover the whole schema.
- **Auth**: bearer tokens through an authkestra `Guard` (also used by the generated routes via `TokenAuth`). Browser sign-in keeps its state in an authkestra `OAuth2State` sealed in the `ak_state` cookie; sessions are an authkestra `SessionStore` over our sqlx 0.9 pool (`src/sessions.rs`, table `auth_sessions`). `authkestra-store-sqlx` is not used: it needs sqlx 0.8 while cratestack pins `=0.9.0`.
- **Still hand-written** until the CLI moves to `/api/v2`: `src/rest.rs` and `src/admin.rs` (`/api/v1`: token minting, agent and relay management, team ownership rules, the relay SSE stream), `src/web.rs` (device flow, OIDC callback orchestration, board pages).

## Develop

```bash
mise trust && mise install    # (trust is a one-time prompt) rust 1.100 (pinned in rust-toolchain.toml; the workspace minimum is 1.98), just; needs Docker and python3
just db                      # Postgres via compose.yaml, on a random loopback port (.env.dev)
just up                      # or the whole dev stack in containers; just down stops it, just reset wipes it
just ps / just logs -f       # compose status / logs (plain `docker compose` needs `--env-file .env.dev`, or run it via mise)
just test      # unit + integration (needs Postgres; `just setup`/`just db` provide it)
just e2e       # real binaries: server + CLI + relay + a fake agent
```

Each checkout gets its own compose project (named after the directory), so two clones do not share containers or volumes. Set `TEST_DATABASE_ADMIN_URL` to point tests at another Postgres.

See `deploy/` for Kubernetes + CloudNativePG, and `Containerfile` for the distroless static image (ghcr.io/stephane-segning/corntchap).

### Deviations from the design doc

- Basic-auth/Argon2 is replaced by bearer tokens (device flow + OIDC SSO via Keycloak), per the later direction.
- MCP is cratestack's binding: tool arguments are flat; list arguments are strings (`labels`/`dependsOn` comma-separated, `items` one per line); every model is also an MCP resource (`cratestack://corntchap/<tasks|projects|items|comments|events|…>[/<id>]`, paged with `?limit=&cursor=`) under the same read policies as REST; `get_board` and `get_task_markdown` render markdown.
- Credentials live in `credentials.toml` in the config dir (see above) (0600) or `CORNTCHAP_TOKEN`; no OS keyring.
- No generated OpenAPI spec yet.
