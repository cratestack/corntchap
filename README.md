# corntchap

A self-hosted coordination server for coding agents. Claude Code (cloud and desktop), Codex and OpenCode share one task list per project without stepping on each other: an agent **claims** a task under a time-limited **lease** with a **fencing epoch**, ticks checklist items as it works, and completes it. A relay on your laptop wakes idle local agents; cloud agents are woken by webhook.

- **Server** (`corntchap`): one static binary + Postgres. MCP at `/mcp` (tools served by [cratestack](https://cratestack.dev) from `crates/server/schema/corntchap.cstack`; revision `2026-07-28` natively, plus a compatibility front door for 2025-11-25 / 2025-06-18 / 2025-03-26 / 2024-11-05 clients such as the TypeScript SDK), REST at `/api/v1`, SSE relay stream, read-only board at `/board`.
- **CLI** (`corntchap-cli`): the board for humans, JSON commands for agents that only have a shell, and `relay`, which turns wake events into local agent runs.
- **Auth**: opaque bearer tokens checked by an [authkestra](https://crates.io/crates/authkestra) guard. Humans sign in with OIDC SSO (Keycloak out of the box); CLIs, agents and relays enroll with the RFC 8628 device flow (`corntchap-cli login`), approved in the browser.

## Install

The CLI (`corntchap-cli`) comes from the GitHub releases; the server runs from the container image `ghcr.io/cratestack/corntchap`.

```bash
brew install cratestack/tap/corntchap-cli                                  # macOS and Linux (Homebrew)
curl -fsSL https://raw.githubusercontent.com/cratestack/corntchap/main/install.sh | sh   # macOS and Linux, no package manager
curl -fsSL https://raw.githubusercontent.com/cratestack/corntchap/main/install.sh | VERSION=v0.2.1 sh   # pin a version
```

```powershell
scoop bucket add cratestack https://github.com/cratestack/scoop-bucket     # Windows (Scoop)
scoop install corntchap-cli
```

On Windows `fire install` / `fire uninstall` (a login service) are not available: run `corntchap-cli fire` in a terminal or as a scheduled task.

### Shell completions

Homebrew installs bash, zsh and fish completions with the CLI (zsh: make sure `$(brew --prefix)/share/zsh/site-functions` is in your `fpath` before `compinit`, which Homebrew's shell setup does). Otherwise generate them: `corntchap-cli completions <bash|zsh|fish|powershell|elvish>` prints the script, built from the same definitions as `--help`, so it always matches the binary.

```zsh
corntchap-cli completions zsh > "${fpath[1]}/_corntchap-cli"   # then restart zsh (or: autoload -Uz compinit && compinit)
```

```powershell
corntchap-cli completions powershell | Out-String | Invoke-Expression   # add to $PROFILE to keep it
```

## Quick start

Needs `mise`, Docker and `python3` (random-port picker, e2e and MCP checks), nothing else to configure: compose starts Postgres 18 and Keycloak with a ready-made realm (users `admin`/`admin`, `orch`/`orch` and `dev`/`dev`). Ports are random on purpose: `just setup` picks free ones (saved in `.env.dev`) and prints them. Everything goes through `just`:

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

`just init` creates a local admin that is not linked to any sign-in (it is the CLI's bootstrap identity; `just init chef-one` picks its handle, otherwise it gets a random one such as `saffron-okra-42`). People who sign in through the browser are separate accounts, created on their first OIDC sign-in with a random kitchen handle and linked by the OIDC `sub` only. Change yours with `corntchap-cli me rename <handle>` (or in the console, user menu); `corntchap-cli user role <handle> super-admin|orchestrator|member` sets a role by hand (a manual setting: the IdP roles no longer change it, so a manual demotion sticks; see Roles), `corntchap-cli user disable|enable <handle>` locks a person out (their tokens and sessions stop working), re-running `just init` reuses the init admin and mints another 7-day token (older ones stay valid until they expire), a handle once used is never given to a different account, the dev realm's `admin` user is a super-admin through the `corntchap-admin` realm role and `orch` an orchestrator through `corntchap-orchestrator` (`OIDC_ORCHESTRATOR_ROLE`).

### The CLI

Corntchap is corn and beans cooked slowly in a pot over a fire, and the CLI borrows the kitchen: `kitchen` is the live board, `cook` manages the agents (the cooks), `fire` is the relay that keeps the fire lit and wakes your local cooks, `marabou` (the marabout) checks the pot, `task scoop` takes the next ready portion and `task serve` completes one by hand, and `task toss` discards one (shown as Tossed, with an optional `--reason`; `task restore` brings it back). Every old name (`tui`, `agent`, `relay`, `doctor`, `task claim-next`, `task done`) still works as an alias; REST paths, MCP tools and JSON are unchanged.

Human output is tidy tables and short confirmations (colour only on a TTY, never with `NO_COLOR`); every listing/show command takes `--json`. The agent-mode task commands (`scoop` (alias `claim-next`), `claim`, `check`, `heartbeat`, `complete`, `release`, `block`, `comment`, `changes`) keep emitting JSON for agents.

- `corntchap-cli kitchen`: live board (ratatui). Left: projects; middle: tasks grouped by status with checklist progress and holder; right: the selected task (body, checklist, dependencies, comments). Refreshes every 2.5 s. Keys: arrows or `j`/`k` move, `Tab` switches pane, `Enter` opens, `a` add task (title, then items one per line; a blank line or Ctrl-S saves), `c` comment, `x`/Space tick the highlighted checklist item (no lease needed), `d` done, `u` unblock, `X` toss (asks first), `p` toggle the project's pantry (memories list plus full body, read-only; `j`/`k` scroll in the right pane), `r` refresh, `?` help, `q` quit.
- `corntchap-cli pantry` (alias `memory`): the pot's shared memory, durable knowledge saved under stable keys (kinds: note, convention, gotcha, decision, howto, fact). `pantry list <POT> [-q TEXT] [--kind K] [--tag T] [--limit N]` (alias `ls`; best matches first with `-q`), `pantry search <POT> <words…>`, `pantry show <POT> <KEY>`, `pantry stock <POT> --title T [--key K] [--kind K] [--tag T] [--task PAY-1] [--append] [--revision N] [--body TEXT | --file PATH]` (aliases `add`, `remember`; the body comes from stdin when neither is given; saving the same key updates it, and `--revision` turns a concurrent edit into a conflict instead of an overwrite), `pantry toss <POT> <KEY> [--yes]` (aliases `rm`, `forget`; asks on a terminal), `pantry digest <POT>` (alias `read`; markdown). All but `stock`/`toss`/`digest` take `--json`.
- `corntchap-cli menu` (alias `milestone`): a pot's menus (milestones). `menu ls <POT> [--all] [--json]` (key, name, status, a `███░░░░░ 3/8` progress bar, due date, open/claimed/blocked counts; served menus only with `--all`), `menu add <POT> <NAME> [--key K] [--description D] [--due YYYY-MM-DD] [--position N]`, `menu edit <POT> <KEY> [--name N] [--description D] [--due DATE|""] [--position N]` (`--due ""` clears the date), `menu serve <POT> <KEY>`, `menu reopen <POT> <KEY>`. Tasks join a menu with `task add … --menu KEY` or `task edit <ID> --menu KEY` (`--no-menu` takes it off); `task ls <POT> --menu KEY` filters (`--menu -` lists tasks on no menu) and `task scoop --project POT --menu KEY` takes the next ready task from one menu. Tables, `task show` and the kitchen show each task's menu, and the kitchen's task pane shows the open menus' progress on its bottom border.
- `corntchap-cli marabou`: checks the server, your login, the relay credential and `config.toml`.
- `corntchap-cli logout [--relay | --all]`.

Production: set `DATABASE_URL`, `PUBLIC_URL` (https), `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` (a confidential client whose redirect URI is `$PUBLIC_URL/auth/oidc/callback`), optionally `S3_*` for file uploads (see `deploy/README.md`), then run `corntchap migrate`, `corntchap init` (prints the local admin handle and a bootstrap token), `corntchap serve`, or deploy the image (`ghcr.io/cratestack/corntchap`) with `deploy/` for Kubernetes + CNPG. (`just up` is the *development* stack with the dev Keycloak realm and well-known passwords; never expose it.) The same CLI commands work against any server: `corntchap-cli login <url>`, `team add`, `project add`, `agent add <name> --kind … --runtime …`, `team join`, `task add`.

Point an agent at the server (Claude Code `.mcp.json`; keep the token in an env var):

```json
{ "mcpServers": { "corntchap": { "type": "http", "url": "https://corntchap.example.com/mcp",
    "headers": { "Authorization": "Bearer ${CORNTCHAP_TOKEN}" } } } }
```

### Connecting agents: one URL per pot, no token

An agent's MCP config can be just a URL, safe to commit in each repo. `/mcp/<POT>` is the MCP server for one pot; `/mcp` is the general one (every pot the cook can see).

```jsonc
// OpenCode, opencode.json in the repo
{ "mcp": { "corntchap": { "type": "remote", "url": "https://corntchap.example.com/mcp/ANR" } } }
// Claude Code, .mcp.json in the repo
{ "mcpServers": { "corntchap": { "type": "http", "url": "https://corntchap.example.com/mcp/ANR" } } }
```

```bash
codex mcp add corntchap --url https://corntchap.example.com/mcp/ANR && codex mcp login corntchap
```

On first use the client finds corntchap's OAuth server (RFC 9728 and 8414 metadata), registers itself (RFC 7591) and opens your browser: you sign in as usual and choose the cook it works as, one of yours or a new one (a new cook gets a relay wake target, and can join the pot's team if you own it). The client keeps and refreshes the token itself (authorization code with PKCE; access tokens last 30 days, refresh tokens 180 days and work once). claude.ai connectors work the same way, and since each pot has its own URL, one connector per pot is allowed.

A token issued for `/mcp/ANR` works only there: not on `/mcp`, not on another pot, not on the REST API. On a pot's URL the tools default `project` to the pot (leave it out), refuse another pot or another pot's task (`OPS-3`), and the cross-pot record listings (MCP resources) are not offered. Rotating the cook's token from its card (or `corntchap-cli cook rotate`) cuts every such client off; they sign in again. Bearer tokens (`cook setup`, `Authorization: Bearer ct_…`) keep working on every URL.

### Long tasks: leases

A claim gives the cook a lease for the pot's lease TTL (`lease_ttl_s`, 10 minutes unless set). Every call carrying the lease epoch renews it, and so does every tool use the agent hooks report (`corntchap-cli hook install`). For work that runs longer without a call (a long build, a test suite, a migration):
- ask for more when claiming: `claim_next` / `claim_task` with `ttlS` (seconds, up to a day), or `corntchap-cli task scoop --project PAY --lease 2h`;
- or renew before the long command: `heartbeat` with `ttlS`.

A lease that runs out lets another cook take the task, and the late cook's next call gets `lease_lost`. Whoever takes a task whose last lease lapsed (instead of being released, blocked or completed) gets a `warning` and `previous_lease` (the holder, its station, when it lapsed): that work may still sit uncommitted in the old branch or worktree.

A task a cook **releases** is not handed back to that same cook by `claim_next` (nor offered by `wait_for_work` or a work wake) for `RELEASE_COOLDOWN_MINUTES` (default 15, 0 turns it off): giving a task back means someone else should take it. Cloud cooks are not woken for it again at all until someone else has touched it (see Cloud cooks). Other cooks get it at once; `claim_task` by name takes it back, and a `{none: true}` reply lists it in `released_recently`. A task with an open linked pull request is not handed out by `claim_next` either (its author is waiting on CI or review, and GitHub events wake it): the `none` reply lists it in `open_prs`, and `claim_task` takes it with a warning naming the PR and its branch. A task nobody holds shows no `station`; `last_station` names the session that had it. Leases also stall and cap out, see "Leases, stalls and bounces". A pot's team owner (or a super-admin) changes the pot's default with `set_lease_ttl` or `corntchap-cli pot lease PAY 30m`; leases already held keep their expiry.

### Leases, stalls and bounces

No cook holds a task forever, cloud or local. Besides the lease TTL, the server ends a lease that:
- **stalls:** the holder made no progress for `STALL_MINUTES` (default 120). Progress is a comment, a ticked checklist item, a task edit, a question, a review request or a saved memory on that task by the holder, or any forge event (CI, review, comment) on its pull request. A bare `heartbeat`, an epoch-carrying read and the local `activity` renewals are not progress.
- **holds too long:** `MAX_HOLD_HOURS` (default 8) after the claim, whatever the progress.

Every claim, `heartbeat` and lease renewal is capped at the earlier of the two limits. The lease view carries `stalls_at` (and, in the last 20 minutes, a `warning`: "no progress since …: comment or tick a checklist item, or the task is released"); past a limit the holder's next lease call gets `lease_lost` saying why. The reaper then reopens the task: `task.lease_expired` says `reason` (`lapsed`, `stalled` or `max_hold`), a note from the restaurant lands on the task ("Lease ended: no progress for 2 h; holder X"), and the holder is told.

**The restaurant.** When the backend takes a decision, it shows in the UI and in the tools as an actor of its own: `restaurant`, the server itself (a principal row that is disabled, has no credentials and cannot be messaged, mentioned, assigned or put in a team). Its events and comments are decisions with a `reason` code and, where it helps, a `why` sentence: `task.lease_expired` (`lapsed`, `stalled`, `max_hold`), `task.blocked` after five give-backs, `task.completed` when a linked PR merged (`merged`), `forge.linked` (`auto_link`), `forge.backfill`, `wake`, `wake.delivered`, `wake.failed`. Decisions that have no event of their own are `restaurant.decided` events with `decision`: `wake.held` (`backoff`, `retry_after`, `busy`, `stations_busy`), `wake.capped` (a cook woken three times in a row without a claim), `wake.withheld` (`fire_and_forget`, `uncovered`), `claim.skipping` (the third give-back of a task by one cook), `catch_up`, `merge.kept_open` (`held`, `open_pr`, `subtasks`, `dependencies`, `weak_link`), `run.ended` and `role.kept`. A repeated decision is told once per 30 minutes (`role.kept`: once a day) and then carries `repeated: n`; they are kept 30 days. `what_now` adds a `restaurant` line for a cook whose latest decision holds it back; the cook page in the console has a "Restaurant log".

A task a cook gives back (`release`, or a lapsed lease) 3 times with nobody else touching it in between (a comment, an edit, an unblock, an answer, a forge event) is skipped by that cook's `claim_next` (the `none` reply names it in `bounced`) and refused by its `claim_task`. A task given back 5 times by any cooks since the last person's touch is blocked with "bounced 5×: needs a person". A release made while a healthy pull request is open does not count: that cook waits on CI or review. A parked pull request (failing checks, changes requested, conflicts) makes its task takeable again, and giving it back then counts like any other give-back (3 skips the cook, 5 block the task). A new push clears "changes requested" (the author answered it). The server comment on a blocked task names its owner, who is woken when it is a cook. Only cooks are refused a bounced task: people may claim it. A stalled lease is reported as `stalled` even when a late comment arrives: progress after the lease ended does not count.

### Coming back: catchUp

An agent silent for 30 minutes is back on its next call. Its first `claim_next` / `claim_task` in a pot after that takes no lease and returns `catch_up` instead (never `none`: loops stop on that): how long it was away, what changed in the pot (counts and the latest events), what it still holds, what is parked for it, what waits on it (questions, comments, reviews), unread pass messages, pantry news and the pot's conventions and gotchas, plus `next`: "read this, then claim again". The second claim takes work. Reading `what_now`, `get_board`, `my_work` or `changes_since` with a project counts as the briefing, so a cook that starts with `what_now` (the relay prompts and the cloud contract say so) is never held. People are never gated.

### Memories on claim

Every successful claim lists the pot's `memories` most relevant to the task, at most 5 with a 240-character excerpt: learned on the task, its parent or its dependencies; tagged like its labels or repositories; the pot's conventions and gotchas; sharing words with its title. Ranked deterministically (lineage, tags, text match, kind, recency); cloud work wakes name the conventions and gotchas as "Relevant memories". `complete` requires `learned` (what the next agent on similar work must know, or "nothing new", which writes nothing); `release` and `block` take it optionally. It is saved as the memory `learned-<TASK>` (appended to on a later revision), kind `note`, tagged with the task's labels and repositories, linked to the task. `get_memory` counts as a use; `read_pantry` ends with the memories nobody read for 90 days as candidates to `forget`.

### Waking agents

```bash
corntchap-cli fire enroll relay-laptop          # device flow, stores the relay credential
corntchap-cli fire init                         # writes a commented config.toml (never overwrites; --force)
corntchap-cli cook setup claude-laptop --kind claude-code --runtime local --team acme
#   creates the agent, joins the team, adds a relay wake target, and prints the MCP config
#   (Claude Code .mcp.json / Codex config.toml / OpenCode opencode.json) plus the [[target]] block to paste into config.toml
corntchap-cli fire doctor                       # or `marabou`: checks the server, the credential and each target
corntchap-cli fire install                      # systemd user unit (Linux) / launchd agent (macOS)
corntchap-cli fire uninstall [--purge] [--yes]  # stop, disable and delete that unit; --purge also removes config.toml, relay-state.json and the relay credential (your own login stays)
corntchap-cli cook wake ls                      # all your agents' wake targets
# cloud agents (see "Cloud cooks" below)
corntchap-cli cook wake add claude-web --type claude-routine --url https://api.anthropic.com/v1/claude_code/routines/trig_…/fire --token sk-ant-oat01-…
corntchap-cli cook wake add codex-cloud --type github-comment --repo acme/payments --issue 12 --token ghp_… [--mention @codex]   # default: GITHUB_MENTION (@cook)
corntchap-cli cook wake add hooked --type webhook --url https://… [--token …] [--secret …]
# an agent registered by device flow instead (token printed once): corntchap-cli cook enroll <name> --kind codex --runtime local
```

#### Stations: wake the exact session, in any folder

You normally need **no `[[target]]` and no `directory` at all.** A *station* is one agent session (the runtime's own session id plus the folder it works in) registered for a cook in a pot. When work lands, the server wakes an idle station and the fire resumes that very session, so any number of repos, worktrees and runtimes work side by side:

```sh
cd ~/code/payments                      # the folder you run the agent in; in a git checkout, all its worktrees are covered
corntchap-cli station scan --dry-run    # what would be registered
corntchap-cli station scan --pot PAY    # register it in pot PAY (any folder, git or not; a pot can span many repos)
corntchap-cli station scan              # in a git checkout: the pot whose repo_url matches `origin`
corntchap-cli station ls                # id, cook, pot, runtime, label, holding/waiting, last woken
corntchap-cli station rm st_ab12cd34ef
corntchap-cli station main st_ab12cd34ef  # this session answers the cook's messages in its pot (★)
corntchap-cli fire doctor               # per station: is its runtime reachable from this machine?
```

Messages, mentions and question answers go to a cook, not a session. The server picks one:
1. the station that holds or is parked on the task in question (it has the context);
2. else the cook's **main station** in that pot, chosen with `station main` (or `unpin`), the star in the console's Cooks page, or the `set_main_station` tool;
3. else the station woken last.

Cloud cooks get a different kind of station: one per run, opened by the dispatcher (see "Cloud cooks"). Those never take part in this choice.

A cook has at most one main station per pot. `station scan` keeps it, and moving the session to another pot drops it. While idle, the main station is also the first one offered new work.

`scan` looks for OpenCode sessions (v2 API; the 1.x API as a fallback), Claude Code sessions (`~/.claude/projects/<dir>/<id>.jsonl`) and Codex sessions (`~/.codex/sessions/**/rollout-*.jsonl`), and registers the most recent one per runtime and folder (`--all`: every session). The cook of each runtime is your cook of that `kind` (`--cook` to choose, `--pot` to override the pot). The fire then resumes the session with built-in defaults: OpenCode through its server API, Claude Code with `claude -p --resume <session>` and Codex with `codex exec resume <session> -`, prompt on stdin and the station's folder as the working directory. A `[[target]]` for the cook is only a fallback (wakes that name no station; `adapter = "webhook"`) and may refine a station's runtime (an OpenCode `password_env`, a custom `command` containing `{session}`).

#### Your folder is where you start, not a fence

A session registered in one folder is not limited to its tasks. `what_now` (MCP tool; `corntchap-cli what-now [--pot PAY] [--station st_...]`; REST `GET /api/projects/PAY/what-now?station=st_...`) briefs a cook and station in one pot, short and actionable:

- `holding` and `parked`: what it holds with a live lease, and blocked tasks parked for it;
- `ready_here`: the top 5 ready tasks it may claim that are about this station's repository or about none (the same rules as `claim_next`);
- `ready_elsewhere`: ready tasks about the pot's other repositories, per repository: count, the top 3, and the repository's url and default branch, so the session knows what to check out;
- `waiting_on_you`: open questions addressed to it, and people's comments on its tasks it has not answered;
- `folder_free`: work that needs no checkout: open questions nobody picked, epics with no subtasks, triage and docs tasks, stale claims (a lapsed lease and no pull request), repository-less tasks, the pantry news;
- `guide`: the pot's guide, and `next`: ONE sentence, chosen in a fixed order: finish what you hold, answer what waits on you, claim what is ready for your folder, folder-free work, ready work elsewhere (open a worktree or clone of that repository next to your folder, never inside another session's worktree), else `wait_for_work`.

A session that cannot leave its folder (a sandbox, a harness locked to one directory) registers as **confined**: `corntchap-cli station scan --confined` (also `station add` and `station start`), or `register_station` with `confined: true`. `claim_next` from a confined station with a known repository hands out only tasks about that repository or about none, and the dispatcher wakes it only when such work is ready; other stations get the other repositories' work. Without `confined`, routing is unchanged.

**The pot guide** is "how we work here", written by people for agents (markdown, up to 20 000 characters): conventions, what to do first, where the work is. The pot's team owners and super-admins set it on the pot's About page in the console (editor with preview), with `corntchap-cli pot guide PAY --file guide.md` (`--clear`; without either it prints it), or with the `set_pot_guide` tool. `what_now` returns it whole; `register_station` and `whoami` (with a project, which a pot's MCP URL supplies) carry its first 1 500 characters, and wake prompts say "This pot has a guide: call what_now".

#### Owners and "my work"

A task has one **owner**: the person (or cook) of the pot's team who is accountable for it, whoever does the work. The lease says who holds the task now; the owner answers for it. Set it with `owner` on `create_task` / `update_task` (a handle; an empty string clears it), `corntchap-cli task new --owner HANDLE`, `task edit --owner HANDLE` or `--no-owner`, or the Owner field on the task sheet. Whoever may edit a task's fields may set its owner, and the owner must be on the pot's team. `list_tasks owner=me` (or a handle, or `-` for unowned) filters the list. Tasks that carried a label `owner:<handle>` got that owner when the 0032 migration ran (the label stays).

`my_work` (MCP tool; `corntchap-cli me work`; REST `GET /api/v1/me/work?project=PAY`; the console's My work page) lists what is yours across every pot you can see: `owned` (open, claimed and blocked tasks you own, by pot), `holding` (tasks you hold with a live lease), `waiting_on_me` (questions addressed to you, reviews asked of you or of the team, unanswered comments on your tasks; people also see the open questions nobody picked), `blocked_on_me` (tasks parked on a question you must answer or depending on a task you own, the top 10) and `counts` for a badge.

#### Starting new sessions, and opening them in Claude Desktop

Claude Code sessions can also be started for a cook, on that person's subscription, by their own `claude`:

```sh
cd ~/code/payments
corntchap-cli station start --task PAY-12 "Keep the diff small"   # claude -p in the background; becomes a station
corntchap-cli station start --desktop                             # wait for the first run, then open it in Claude Desktop
corntchap-cli station start --fire --pot PAY --cook clara          # from anywhere: clara's relay starts it on its machine
corntchap-cli station open st_ab12cd34ef                          # claude --desktop --resume <session> in its folder
corntchap-cli station open st_ab12cd34ef --terminal               # claude --resume (codex resume for Codex stations)
```

The console's Cooks page has the same "Start a session" button on Claude Code cooks (the `start_station` tool for agents): the server creates a station with a new session id in a folder where the cook already has one (never another folder) and wakes it at once; the cook's fire sees a session with no transcript on disk yet and starts it with `claude -p --session-id <id>` instead of resuming it. Without a task or prompt the session takes the next ready task. Claude Desktop has no link that sends a prompt on its own, so the dialog's "Draft it in Claude Desktop" link (`claude://code/new?folder=…&q=…`) only prefills one: you confirm the folder and press Send, then `station scan` registers the session. Only Claude Code can be started for now (OpenCode and Codex choose their own session ids).

Config and credentials live in `~/.config/corntchap/` on Linux and `~/Library/Application Support/corntchap/` on macOS; set `CORNTCHAP_CONFIG_DIR` to use another directory. Example `config.toml` (everything in it is optional; the top-level `server = "https://…"` overrides the login's server; `timeout` takes `90`, `90s`, `45m`, `1h30m`):

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

OpenCode v2 runs one shared background service, which the fire finds with `opencode service status` (its URL) and `opencode service get password` (HTTP basic, user `opencode`), then talks to over `/api` (`/api/session`, `/api/session/{id}/prompt` with durable queued delivery); `fire doctor` prints the API version it found. For OpenCode 1.x, whose desktop app and TUI pick a new port on every start, the `opencode` adapter finds the running server itself: `OPENCODE_PORT`, then running `opencode*` processes (their `--port`, or the ports they listen on), then `127.0.0.1:4096`, preferring the one with a session in `directory`. A password-protected server gets its password from `password_env` / `password` on the target, `OPENCODE_SERVER_PASSWORD`, or (Linux) the OpenCode process's own environment. `corntchap-cli fire doctor` shows what it found; `corntchap-cli fire poke <cook> --pot PAY` sends a wake right away to check the wiring.

The fire looks for `claude`, `codex` and `opencode` on its own PATH, then on your login shell's PATH (`$SHELL -l`), then in the usual install folders (`~/.local/bin`, `~/.claude/local`, npm, bun, Homebrew). `fire install` also writes the PATH of the shell you run it from into the service, since systemd and launchd start services with a bare PATH. If a wake says `starting claude: No such file or directory`, run `corntchap-cli fire install` again from a terminal where `claude` works, then restart the service.

A woken `claude -p` has nobody to approve a tool call, so the fire allows corntchap's own tools for it: `--allowedTools mcp__<name>` for every MCP server pointing at your corntchap in the station folder's `.mcp.json` (or a parent's) and in `~/.claude.json`. claude.ai connectors are not in those files: if you reach corntchap through one, allow it once in `~/.claude/settings.json`: `{"permissions": {"allow": ["mcp__corntchap-another"]}}` (the connector's name). Other tools follow your own permission settings.

A cook that is woken but claims nothing (a refused tool, a broken session) is not woken every 30 seconds forever: each wake in a row without a claim doubles the wait, up to 30 minutes, and a claim resets it.

The relay remembers the last event id it saw (`relay-state.json` next to `credentials.toml`) and sends it as `Last-Event-ID` on restart, so wakes sent while it was down are replayed.

### What a cook may do: grants

**Roles.** A person is a `super_admin`, an `orchestrator` or a `member` (cooks and fires are always members and act through their owner). A super-admin runs the instance: people and their roles, enabling and disabling people, the Forges (git hosts), the any-URL cloud targets (webhook, Coder, a Devin or GitHub API at another URL, a Claude routine at another host), and every team, pot, cook and fire. An orchestrator creates and manages their own teams, pots, cooks, fires and wake targets (relay, a Claude routine on Anthropic's host, GitHub comments and Devin on their default hosts), and sees a people directory (handle, display name, role: no contact details) to add people to their teams, and the usage of their own cooks and the pots they own. They cannot touch other people's resources: an orchestrator who owns a team may take a member's cook out of the team, but not rotate, disable, delete or reconfigure it. A member works in the teams they are put in and manages the cooks they own. Owning a team (the `owner` role) and creating teams need the orchestrator role; if an owner is demoted the owner row stays but its powers are dormant until they are an orchestrator again. Super-admins set roles with `corntchap-cli user role <handle> super-admin|orchestrator|member` (MCP `set_role`, console Users page); the IdP roles `corntchap-admin` and `corntchap-orchestrator` set the role at each sign-in for people whose role was never set by hand. The last super-admin who can sign in cannot be demoted. A cook's `admin` grant passes on its owner's role, so an orchestrator's cook is never a super-admin.

Every console action is also an MCP tool (tasks, menus, pantry, teams, pots, cooks, fires, wake targets), with the same rules. A cook works its tasks under leases; beyond that it does what a person grants it, never more than its owner can do: `plan` (edit, toss, restore and unblock any task in its teams, tick items without a lease), `manage` (act as its owner on teams, pots, cooks, fires and wake targets) and `admin` (its owner's role: an orchestrator's cook acts as an orchestrator, never as a super-admin). Only a person changes grants (console: Cooks, "May also"; CLI: `corntchap-cli cook grant <cook> plan manage`, `--none` to take them away; MCP: `set_grants`), so a cook cannot raise its own rights. Events still name the cook as the actor.

### Renaming and deleting cooks, fires, pots and teams

Procedures and MCP tools: `cookRename` / `rename_cook`, `cookDelete` / `delete_cook`, `fireRename` / `rename_fire`, `fireDelete` / `delete_fire`, `potRename` / `rename_project`, `potDelete` / `delete_project`, `teamRename` / `rename_team`, `teamDelete` / `delete_team`. CLI: `corntchap-cli cook|fire rename <old> <new>`, `cook|fire rm <name>`, `pot rename <KEY> <name>`, `pot rm <KEY>`, `team rename <old> <new>`, `team rm <name>`; every `rm` asks on a terminal and refuses without one unless you pass `--yes`. The console has Rename and Delete on cook cards, fires, pots (Pots page and the pot's options menu) and teams; pots and teams ask you to type the key or name.

- **Who:** a cook or fire: its owner or a super-admin (a cook granted `manage` acts as its owner, but never deletes itself). A pot or team: an owner of the team (an orchestrator), or a super-admin.
- **Renaming a cook or fire** keeps everything that points at its id working: tokens, stations, leases, wake targets, team memberships. It does not follow where the old name is stored as text: `@old` mentions in comments and pass messages, the old name inside earlier events' payloads, and the `agent = "old"` of a `[[target]]` in a fire's `config.toml` (edit that file; a fire itself is known by its token, so renaming a fire needs no change on its machine). The new name follows the handle rules (and a name a person ever had is refused); the old one is free again.
- **Deleting a cook** first releases every lease it holds (tasks go back to `open`, with a `task.released` event), then deletes the cook, which cascades to its tokens, stations, wake targets, team memberships, cloud setups, usage and OAuth grants, and to direct pass messages sent to it. What it wrote stays: comments, tasks, checklist ticks, questions, pass messages and files keep their text with no author (migration `0022` makes those foreign keys `on delete set null`), and its events keep its name in `payload.actor_name`.
- **Pots:** the display name changes, the **key never does**: it is part of every task id (`PAY-12`), file reference, saved link and `/mcp/<KEY>` URL, and it is written into comments, messages and events. Deleting a pot needs `confirm` equal to the key and removes the tasks, checklists, comments, events, pass, pantry, views, menus, questions, stations, files' metadata and scoped tokens, then the file objects in S3 (best effort: what could not be removed is reported as `files_left`). A team can be deleted only when it has no pots.

### Agent activity and checkpoints

Claude Code, OpenCode and (best effort) GitHub Copilot can run a hook after every tool use. `corntchap-cli hook` uses it for two things: it
snapshots your work tree into a hidden git ref after each edit or command (a fine-grained, revertable history that
never touches your branch, index or HEAD), and it tells corntchap what happened, so the console shows what each
station is doing (last tool, files, checkpoint) on the cook's page.

```bash
corntchap-cli hook install claude-code            # merges a PostToolUse hook into .claude/settings.json (use --global for ~/.claude)
corntchap-cli hook install opencode               # writes .opencode/plugins/corntchap.js (--global: ~/.config/opencode/plugins)
corntchap-cli hook install copilot                # best effort: writes .github/hooks/corntchap.json (see "Other harnesses")
```

Install is idempotent and keeps your existing hooks. The session must be a registered station
(`corntchap-cli station scan`); activity of unregistered sessions is dropped by the server, after the checkpoint is taken locally.

- **What is sent:** the runtime, session id, tool name, the changed paths (relative to the repo), the checkpoint sha and a
  short summary (the command, cut at 200 characters, for Bash; the paths for edits). Never file contents. Calls within two
  seconds of each other are merged into one entry; the server keeps 14 days of activity. It never wakes anyone.
  A Bash command is sent as typed, so keep secrets out of command lines.
- **Where checkpoints live:** `refs/corntchap/checkpoints/<session>`, a chain of commits made in a temporary index
  (untracked files that are not ignored are included; nothing is committed when the tree did not change). They are local
  to your clone and are not pushed; `git gc` keeps them while the ref exists.
- **Restore:** `corntchap-cli checkpoints` lists the sessions with checkpoints, `--session <id>` lists one session's commits.
  Then `git restore --source <sha> -- <path>` brings back one file, `git checkout <sha> -- .` everything (files created
  later stay). `corntchap-cli checkpoints --restore <sha>` does the latter and refuses while the tree has uncommitted
  changes, unless `--force`.
- **Turn it off:** `corntchap-cli hook uninstall claude-code|opencode|copilot` (same `--global`), or set `CORNTCHAP_HOOKS=off` in the
  environment. Delete the checkpoints with `git for-each-ref --format='delete %(refname)' refs/corntchap | git update-ref --stdin`.
- **Failures:** a hook never blocks the agent and prints nothing; errors go to `hook.log` in the config dir
  (`~/.config/corntchap/`), and the server call times out after 3 seconds.

### Other harnesses (best effort)

**OpenCode is the recommended, first-class harness**: MCP, wakes through its HTTP server, and hooks all work, and it is the one tested end to end. Claude Code and Codex are supported the same way (above). GitHub Copilot (CLI, desktop app, VS Code agent mode) and Google Antigravity (IDE and the `agy` CLI) are best effort: written against their documentation, not exercised on every platform.

| Harness | MCP | Wakes | Hooks |
| --- | --- | --- | --- |
| OpenCode | yes, OAuth or token | yes, through its server | yes (plugin) |
| Claude Code | yes, OAuth or token | yes (`claude -p --resume`) | yes |
| Codex | yes, OAuth or token | yes (`codex exec resume`) | no |
| Copilot CLI and desktop app | yes (`~/.copilot/mcp-config.json`) | yes (`copilot -p … --resume`) | yes (`.github/hooks`) |
| Copilot in VS Code agent mode | yes (`.vscode/mcp.json`, agent mode only) | no (VS Code has no headless resume) | yes (same hook file, preview in VS Code) |
| Antigravity `agy` CLI | yes (`mcp_config.json`) | yes (`agy -p … --conversation`), register the id yourself | not known |
| Antigravity IDE | yes (same file) | not known whether the CLI can resume IDE chats | not known |

**MCP.** Use the pot's URL, so nothing secret is committed; sign in when the client asks (OAuth with dynamic client registration and PKCE). Name the server after the pot (`corntchap-pay`).

```jsonc
// Copilot in VS Code, .vscode/mcp.json: a "Start" / "Auth" CodeLens appears over the server; tools show in agent mode
{ "servers": { "corntchap-pay": { "type": "http", "url": "https://corntchap.example.com/mcp/PAY" } } }
// Copilot CLI and desktop app, ~/.copilot/mcp-config.json (a project's .mcp.json or .github/mcp.json also work)
{ "mcpServers": { "corntchap-pay": { "type": "http", "url": "https://corntchap.example.com/mcp/PAY", "tools": ["*"] } } }
// Antigravity, ~/.gemini/config/mcp_config.json or .agents/mcp_config.json in the repo
{ "mcpServers": { "corntchap-pay": { "serverUrl": "https://corntchap.example.com/mcp/PAY" } } }
```

`corntchap-cli cook setup <name> --kind copilot|antigravity` creates the cook and prints the same files with a bearer token instead (do not commit those). Make the cook's kind `copilot` or `antigravity` (the console offers both); a client that signs in through OAuth gets its kind guessed from its name.

**Wakes.** The relay resumes a registered station headlessly, in the station's folder, with the prompt as an argument:
- Copilot: `copilot -p <wake text> --resume=<session> --allow-all-tools --no-ask-user`. `station scan` finds Copilot CLI sessions in `~/.copilot/session-state/<id>/` (`COPILOT_HOME` moves it) by reading the folder the session was started in; the official documentation names only `events.jsonl` in that folder, so the lookup is lenient. If `scan` does not see your session, register it: `corntchap-cli station add --runtime copilot --session <id> --pot PAY` (the id is in the `copilot --resume=<id>` hint a run ends with, and in the folder name). The desktop app runs its sessions through the CLI.
- Antigravity: `agy -p <wake text> --conversation <id>`. Nothing is scanned: take the id from `agy -p "…" --output-format json` (`conversation_id`) and register it with `station add --runtime antigravity --session <id> --pot PAY`. In headless mode a tool call that needs approval and cannot get it is skipped, so add `permissions.allow` rules to `~/.gemini/antigravity-cli/settings.json`, or, if you trust the folder, give the cook a `[[target]]` whose `command` is `["agy", "-p", "{prompt}", "--conversation={session}", "--dangerously-skip-permissions"]`.
- A `[[target]]` `command` that contains `{session}` replaces the default; `{prompt}` in a command is replaced by the wake text (and then nothing is piped to stdin).
- `station open <id> --terminal` runs `copilot --resume` / `agy --conversation`; `fire doctor` checks that `copilot` / `agy` are on PATH.

**Hooks (Copilot).** `corntchap-cli hook install copilot` writes `.github/hooks/corntchap.json` (`--global`: `~/.copilot/hooks/corntchap.json`, CLI only), a `PostToolUse` command hook. It takes the same checkpoints and reports the same activity as the Claude Code hook; `hook uninstall copilot` deletes the file. A `SessionStart` hook is not installed: there is nothing for corntchap to do at session start. Tool names that count as edits or commands (Copilot CLI `edit`, `create`, `bash`; VS Code `editFiles`, `run_in_terminal` and friends) are matched by name, so an unlisted one is reported without a checkpoint. A VS Code chat session is not a station (it cannot be woken), so its activity is dropped by the server; the checkpoints are still taken.

**Not known.** Whether Antigravity has a hook system, whether `agy` can resume a chat started in the IDE, and the exact tool names and payload details of Copilot's hooks in every host. Test those on your machine; `corntchap-cli hook` logs problems to `hook.log`.

### Comments wake the cook responsible

A comment on a task reaches the cook responsible for it, without anyone having to point it there:
- **Who:** the cook holding the task, parked on it waiting for an answer, or the one that last held it (so a comment on served work reaches whoever served it).
- **Where:** it's woken on the session holding the task, else its main station.
- **Mentions:** `@mention`s in a comment wake those cooks too.
- **Never:** the commenter itself, a fire-and-forget cook (the reply's `note` says it was not woken, and no nudge follows), or anyone on a tossed task.
- **Unanswered:** a person's comment that gets no answer on the task (a comment, or completing, releasing or blocking it) within `COMMENT_NUDGE_MINUTES` (default 10) wakes the cook's **main station** once more. That's the orchestrating session, not the one that stayed silent.

### Questions to a cook, and reviews

**Ask a cook.** `ask` with `to` naming a cook wakes it (reason `question`): the session holding the task in question, else its main station. The prompt carries the title, description and choices and the `answer_question` call to make; a fire-and-forget cook is refused (give it a task). Asking a person works as before: it waits in the pot's question pool.

**Ask for a review.** When the work on a task is ready for a second pair of eyes, the cook that holds or held it (or a person, or a cook granted `plan`) calls `request_review` with the task, a reviewer from the pot's team (a cook or a person, never itself; leave it out for anyone on the team) and a note saying what to look at (the PR address, the branch). CLI: `review request PAY-42 --reviewer bob --note <url>`.
- **A review is not a lane.** The task keeps its status. It carries `review` (the latest round: state, reviewer, requested by, when), which boards and the task sheet show as a chip. GitHub's own reviews arrive through forge events and stay in the task's `pr`.
- **The reviewer** is woken (reason `review`) with the task, the note and the `review` call to make; a fire-and-forget cook can't be named as reviewer; a person reviewer finds it under **Questions > Reviews** (waiting on them) and in the activity feed. They answer with `review` (CLI `review approve|changes|comment PAY-R3 --body ...`, or the task sheet's buttons): `approve`, `changes` (the body says what to fix) or `comment` (not a decision). Nobody reviews its own request, and one open review per task and reviewer.
- **The verdict** wakes the cook that holds or last held the task (reason `review-done`) with the verdict, the body and the next step. After `changes`, it fixes them and calls `request_review` again: that is the next round. `list_reviews` (`review ls PAY --mine`) lists the reviews of a pot; `cancel_review` withdraws one. Events: `review.requested`, `review.done`, `review.cancelled`.

### Cloud cooks

A cook with `runtime: cloud` runs as one-shot cloud sessions: each wake starts a fresh session that remembers nothing.

**Fire and forget.** A cook whose wake targets are all cloud ones (no relay) is *fire and forget*: every wake is a new, paid run that can't stay to chat. `list_cooks` and `list_team_members` (paged: `{items, nextCursor}`) show it as `fireAndForget`; the MCP instructions tell other cooks not to message, ask or assign reviews to it, and to give it a task. Only two things start it:
- **Ready work.** The dispatcher wakes it for a ready task in its pots (narrow it with labels, e.g. only tasks labelled `cloud`; the wake then tells the run to `claim_next` with those labels). Every `CLOUD_SWEEP_SECONDS` (default 60) each pot with a ready task is checked for cooks that could take it: a delivery that failed, a cook that finished one task of several, a target added after the task. A 429's `Retry-After` is honoured (at most an hour). A cook is woken for work while it holds no live lease, at most every 5 minutes (doubling per wake without a claim), and the sweep and the retries of failed deliveries stop after 3 wakes in a row that led to no claim; a claim resets it. A task the cook released or let its lease lapse on is not offered to it again until someone else has updated or commented on it, answered a question on it or sent a forge event for it.
- **A direct message from its own owner** on the pass. The run answers the owner there (to the owner only), then does ready work. Messages that arrive close together are batched into one run.

Nothing else starts it: not other cooks, not other people, not `@mention`s, comments, questions, review requests or verdicts, forge events as such (a PR that fails, gets changes requested or conflicts only makes its task ready work again, and ready work does start it; PR comments and the rest do not), a merge, upstream changes, broadcasts or pantry notifications. Such a call is refused where the caller can be told, and otherwise not delivered:
- a direct message to it from anyone but its owner, `ask` with `to` naming it, and `request_review` naming it are refused ("<cook> is fire-and-forget: it only takes ready work; give it a task");
- a mention on the pass or in a comment wakes nobody, and the reply's `note` says so (no unanswered-comment nudge follows);
- it can't `ask` with `wait=true`: it blocks the task instead.

**What a run does.** One contract for every target (`CLOUD_CONTRACT`: the routine's saved prompt, the first message of a Coder or Devin run, and the work wake): loop `claim_next` (with its labels), `get_task` (comments, `prs`, warnings), read the pantry news, work on a branch `REF-slug`, open a PR titled `REF: …`, `link_pr` it, comment the PR URL and what was and wasn't done, then `release` with the reason "PR open: <url>". A task with an open linked PR is not handed out again unless the PR fails, gets changes requested or conflicts (then it is work again: fix the PR), and other forge events never start the cook; when the PR merges, an unheld task completes by itself (see GitHub). A task that needs no code is completed with its result; one that needs a person is blocked with a precise reason; one someone else could do gets a comment and a release. It never uses `pass` to others, `ask` with `to` or `wait`, or `request_review`, and never waits. It `claim_next`s until none is left, then stops.

**One station per run.** Every wake of a cloud target (a Claude routine run, a Devin or Coder run, a webhook call, a GitHub comment) opens a *cloud-run station* before it is sent, and the wake text says "Your station is st_…: pass station=st_… to what_now, claim_next, claim_task, comment, pass and ask". What the run claims, comments and passes is attributed to it, and when the target gives a session page (a routine's `claude_code_session_url`, a Devin session's `url`) the station carries it as `session_url`, so task comments, the claim line, pass messages and linked pull requests link to the run that did it (`station_url`, `last_station_url` in the task JSON). A run that forgets to pass `station` is attributed anyway: for a fire-and-forget cook an agent call without `station` goes to the cook's most recent cloud-run station in that pot (a live one first). Coder and Devin follow-ups go into the same run, so they reuse its station (and reopen it); a Devin or Coder run that is gone starts a new session and a new station, with the new ref in the text. A delivery that fails or is deferred (HTTP 429) leaves no station behind, and the test wake sends none.

A cloud-run station *ends* (`ended_at`) when its last task is released, completed or blocked, or its last lease is reaped, and no question of it is parked; a run that goes quiet without releasing anything (no claim, comment or pass for the stall time, checked every 20 minutes) is ended too. Naming an ended run's station (or falling back to it) reopens it. Ended cloud-run stations are deleted with the events, after 90 days. They are not sessions: they are never woken, cannot be the main station, and cannot be addressed with `pass to`. `list_stations` takes `kind` (`session` or `cloud-run`), `live` and `q`, and pages (`{items, nextCursor}`; pass `page: {cursor}` for the next); the cook page shows them under **Cloud runs**, apart from **Sessions**.

**What the prompt carries:** who the cook is, the work or the owner's messages, and the pantry news. The session reads the rest through the corntchap MCP tools.

**Target types:**
- **`claude-routine`** (Claude Code on the web):
  - Setup: create a routine at claude.ai/code/routines with the repository and the corntchap MCP server (with the cook's token), add an API trigger, and give corntchap its URL and token.
  - Prompt: Anthropic passes corntchap's text to the run as an untrusted payload, so the routine's own prompt must say to act on it. Adding the target prints that text; the console shows it with a copy button.
  - Each fire starts a new session: it cannot resume a session or reach an existing one (binding the routine to a session does not change that, and routine runs do not get the tools to fire into another session). So each run works from what corntchap sends and keeps: the task with its comments and linked pull requests, the pantry. The routine prompt says so, and tells the run to save what the next one must know.
  - Limits: a routine takes 30 fires an hour, and a retry starts a second run. A `429` is respected (`Retry-After`), not retried blindly.
  - Each run's session link is recorded (`wake.delivered`).
- **`github-comment`** (Codex cloud, Copilot, or any bot that answers mentions): a comment on an issue or PR mentioning the target's `mention` (`@codex`, `@copilot`, …). Without one it mentions the server's `GITHUB_MENTION`, `@cook` by default, so a fresh install never pings a bot that already works on the repository. Comments can be public, so they only point at the messages ("read_pass …, get_task PAY-3"); the content never goes there.
- **`webhook`** (your own endpoint): `POST {text, agent, project, reasons, wakes}`, with `Authorization: Bearer <token>` when set. It is signed `X-Corntchap-Signature: sha256=<HMAC-SHA256 of the body>` when the target has a `secret`. A target `body` replaces this with a fixed body.
- **`coder`** (an agent run on your own [Coder](https://coder.com) deployment, so the cook runs in an environment you know and control). Only a super-admin connects it, because its URL can point anywhere.
  - Which Coder API: Coder Tasks (`/api/v2/tasks`) was deprecated in Coder v2.34 and removed, and Coder Agents (`/api/v2/chats`) replaced it. Current Coder: give `organization` (and optionally `workspace`, to pin the run to a workspace you provisioned from a template you control; otherwise the agent picks a template itself). Older Coder that still has Tasks: give `template` (a name or a version id), optionally `preset` and `owner` (default `me`).
  - What to set up on Coder: a user (a service account is best) and its session token, which corntchap sends as `Coder-Session-Token` and never shows again; the corntchap MCP server (`/mcp` of this server, with the cook's token) reachable from the agent, as an MCP server in Coder Agents, or in the template's agent module for Tasks; and, for a pinned workspace, a workspace whose template has the tools the cook needs. corntchap puts no credential in the prompt.
  - What corntchap sends: the first wake creates a run whose first message is the cloud contract (see above) followed by the usual wake text. The run id is kept per cook and pot, and later wakes for that pot are sent to the same run as follow-up messages (`POST /api/v2/chats/{id}/messages`, queued behind what it is doing; for Tasks `.../send`). If Coder no longer knows the run (404), a new one starts. No run link is recorded: Coder's own pages show the runs.
  - A failed delivery is a `wake.failed` event with Coder's error, like the other targets.
  - Coder's API facts above are marked verified or assumed in `crates/server/src/coder.rs` (read 2026-11-24 from Coder's docs; the Go source was not readable).
- **`devin`** (a [Devin](https://devin.ai) session, through Devin's API v3). Any cook's owner may connect it with Devin's own API host; pointing it at another `url` is a super-admin's call.
  - What to set up in Devin: a service user allowed to use and manage the organization's sessions, and its API key (`cog_…`), which corntchap sends as a bearer token and never shows again; the organization id (`org-…`); and the corntchap MCP server (`/mcp` of this server, with the cook's token) added once in the organization's MCP settings. corntchap puts no credential in the prompt.
  - What corntchap sends: the first wake creates a session (`POST /v3/organizations/{org}/sessions`) whose prompt is the cloud contract (see above) followed by the usual wake text. The session id is kept per cook and pot, and later wakes for that pot are messages to the same session (`.../sessions/{id}/messages`; Devin resumes a suspended session by itself). If Devin answers 404 or 410, or says the session has exited, a new session starts. The session's web link is recorded in `wake.delivered` and shown on the cook page next to the target.
  - Fire and forget like the others: it starts for ready work and its owner's direct message. A failed delivery is a `wake.failed` event with Devin's error, never the key.
  - Devin's API facts are marked verified or assumed in `crates/server/src/devin.rs` (read 2026-11-24 from docs.devin.ai).

#### Connecting a cloud cook from the agent itself

Add the corntchap MCP server to Claude Code (or Codex, Claude Desktop, claude.ai) with the cook's token, then ask the agent to set up its cloud wake, or run the `/mcp__corntchap__connect-cloud` prompt. The agent calls **`connect_cloud`** (`kind`: `claude-routine`, `github-comment`, `webhook`, `coder` or `devin`), which opens a one-hour setup and returns the steps (for a routine: the prompt to paste), the values needed and a setup code. The secret then comes in one of three ways:

1. **The form in the chat.** Clients that render MCP Apps (Claude on the web and desktop, VS Code, ChatGPT, …) show corntchap's form (`ui://corntchap/connect-cloud`) next to the tool call. The values go straight to corntchap through `connect_cloud_save`; the model is only told that the cook is connected.
2. **The link.** Every setup has a console page, `/connect/<code>`, where the cook's owner enters the values signed in. A terminal client such as Claude Code shows this link.
3. **The agent.** If the user pastes the values to the agent, it calls `connect_cloud_save` itself; the secret then stays in the conversation, and the agent says so first.

`connect_cloud` with the `code` shows the setup's state (`waiting`, `connected`, `expired`, `removed`), and with `test: true` sends a test wake through the new target (a routine's run link is shown). Answers never repeat a secret. A cook can connect itself, its owner can connect it, and so can super-admins. Through a setup, a cook's owner may connect a Claude routine (the trigger must be `https://api.anthropic.com/v1/claude_code/routines/…/fire`) or GitHub comments without being a super-admin; webhooks, which can point anywhere, stay a super-admin's call.

### Pots with several repositories

All of this is optional: a pot with no repository, a task that names none and a cook with no scope work as always.

- **Repositories.** A pot has 0..n. `corntchap-cli project repo add PAY https://github.com/acme/backend.git [--name backend] [--branch main]`, `repo rm`, `repo ls`. The name is the short alias (default: the last part of the url). Team owners, super-admins and cooks granted `manage` change them; members read them. `project add --repo URL` still works and becomes the first one; `repo_url` is still returned (the first repository's) for old clients. MCP: `add_repo`, `remove_repo`; `list_projects` shows `repos`.
- **Tasks name repositories.** `task add PAY "title" --repo backend --repo web` (MCP `create_task` / `update_task` `repos`, the console's New task and edit forms). Names are checked against the pot's. No repository means any.
- **Cooks can be scoped.** `cook repos NAME --pot PAY backend web` (a cook may set its own with `set_cook_repos`; none given means all). `claim_next` then skips tasks whose repositories do not overlap the scope; a task with no repository goes to anyone, an unscoped cook takes anything, `claim_task` on a named task is never refused (it warns). Work wakes, `wait_for_work` and the dispatcher follow the same rule.
- **Splitting.** `task split PAY-7` (MCP `split_by_repo`) makes one subtask per repository the task names, titled `[backend] ...`; the task waits for them.
- **Stations.** `station scan` and `station start` match a checkout's `origin` against any repository of any pot and record its name on the station (shown in `list_stations` and the console). Stations do not scope claims unless they are confined (see "Your folder is where you start"); the cook's scope does.
- **Dependencies between repositories.** `project repo link PAY app --on backend [--mode notify|task]` says `app` depends on `backend` (`unlink` removes it; MCP `link_repo`, `unlink_repo`). When a task naming `backend` is served (`complete`), for each dependent repository not named by the task itself: with `notify` (default) the cooks scoped to `app` are woken with the result (unscoped cooks are not) and a comment on the served task and a `task.propagated` event in the activity say who was told; with `task` a follow-up task `[app] Follow up on PAY-12: ...` for `app` is opened on the same menu. One hop: a follow-up does not propagate when served, so cycles never loop. Pass `noPropagate` (`task complete --no-propagate`) when nothing downstream is affected. A fire-and-forget cloud cook is never woken by an upstream wake (it is not listed as told); give it the follow-up task (`--mode task`) and it takes it as ready work.

### GitHub (and other forges)

A git host can tell corntchap about pull requests, checks and reviews. GitHub is built (through a GitHub App); GitLab and Gitea come next on the same interface (`crates/server/src/forge/mod.rs` says how each maps onto it).

**What the app sees.** Read-only permissions (pull requests, checks, statuses, contents, issues, metadata, and actions for workflow runs) and these events: pull requests, reviews, review comments, PR conversation comments, check suites and runs, workflow runs, pushes, and the app being installed or removed. It can write nothing on GitHub.

**What it does.**
- A pull request is followed only when its repository belongs to a pot (`project repo add`). Everything else is ignored.
- It is linked to a task when its branch, title or body names the task (`SHOP-12`, any case, e.g. `feat/shop-12-fix`), or when you link it: `task link-pr SHOP-12 <url>` (also `task unlink-pr`, MCP `link_pr` and `unlink_pr`, the task sheet's "Link a PR"). A link you make wins; an unlinked PR stays unlinked.
- The task sheet shows each PR with its state, checks (and the failing ones), review and conflicts; boards and lists show a small indicator; the activity feed shows `forge.*` events.
- The task's holder (its claimer, else the cook that held it last; never a person) is woken for: CI that failed (once per commit), changes requested, a review or comment by someone other than the cook, and merge conflicts. A fire-and-forget cook is never woken by these.
- **A merge completes the task** when the PR was linked by a person (`link_pr`) or by the task's ref in its branch or title (a body that merely mentions the task never completes it), the task is `open` (nobody holds it, not reserved), no other PR linked to it is open, no subtask is open and every dependency is done: it is done with the result "Merged <url>" (no actor) and a `task.completed` event, so what depends on it becomes ready, and the repositories that depend on the task's repositories hear about it as when a task is served (once). A held task keeps its status and its holder is told ("SHOP-12's status is unchanged, serve it if the work is done"); if it is released or its lease expires after the merge, it is completed then. A second merged PR of a finished task says nothing. A task with no repositories tells the dependents of the PR's repository instead (`link_repo`).

**Telling agents.** The MCP instructions every cook receives say: name the task id in the branch (`anr-123-short-slug`) and in the PR title (`ANR-123: …`) so the PR links itself, or call `link_pr` (`id`, `url`; `unlink_pr` for a wrong link); `get_task` shows `prs` with checks, review and conflicts, so do not poll GitHub: a failed CI run, requested changes, a comment or a conflict wakes the holder (reason `forge`), a merge sends a notice (`forge-merged`) when you still hold the task (`complete` it yourself) and completes a task nobody holds; a task with an open linked PR is not handed out by `claim_next`, and `claim_task` takes it back (continue on the PR's branch).

**Pick a pot's repositories from GitHub.** Once an app is connected and installed, the server lists what each installation can see (it signs a short-lived app token with the stored private key, trades it for an installation token, cached in memory until a minute before it expires and never stored, and reads `GET /installation/repositories`). MCP `list_forge_repos(project?, query?, installation?)` (REST `GET /api/v1/forges/repos?project=&q=&installation=`, CLI `corntchap-cli project repo available PAY [-q TEXT]`) returns them, most recently pushed first, archived last, with `in_pot` marking those already in the pot. MCP `add_repos(project, repos)` (REST `POST /api/v1/projects/{key}/repos/bulk`, CLI `project repo add-many PAY acme/api acme/web`) adds several at once from GitHub full names or urls (newline or comma separated): names default to the repository's name in lower case, `owner-name` when two clash, the default branch comes from GitHub, and repositories already in the pot are skipped and reported. Same people as `add_repo`; without a pot, only super-admins list. The console's About page has an **Add from GitHub** button for it.

**Reading GitHub directly.** `link_pr` reads the pull request from GitHub at once (title, state including merged, draft, branch, author, checks of the head commit, review, conflicts) through the app installation that covers its repository, and fills the row as a delivery would; when no installed app covers the repository the answer carries a `warning` and the row stays empty. `refresh_pr` (`POST /api/v1/tasks/{id}/refresh-pr`) reads a task's pull requests again. When a repository joins a pot (`add_repo`, `add_repos`), its open pull requests that name a task of the pot (branch, title or body) are linked in the background, and a `forge.backfill` event on the pot says how many. A delivery about a repository no pot follows is dropped as before, but the repository is remembered: the **Forges** page lists "Repositories sending events that no pot follows" with **Add to a pot** and **Dismiss** (MCP `dismiss_forge_repo`, super-admins), and `list_forge_repos` marks them `unfollowed_events`.

**What it does not do.** No automatic import: no issue becomes a task, no task is created from GitHub (the one exception is a repository link in mode `task`, which opens its follow-up when a PR merges, as it does when a task is served). **A close never changes a task's status**, and a merge only completes a task nobody holds (or one released after the merge); serve a held task yourself.

**Connect GitHub** (super-admins). The server needs a public https `PUBLIC_URL`, since GitHub has to reach `PUBLIC_URL/webhooks/github`. Open the console's **Forges** page (admin menu), choose **Connect GitHub** (an organization name if the app should belong to one; a GitHub Enterprise host if you use one), and approve the app on GitHub's screen. corntchap stores the app's id, keys and webhook secret (never shown again by any API), then **Install on an org** opens GitHub's install page: pick the account and the repositories. Each delivery is checked against the webhook secret (`X-Hub-Signature-256`), deduplicated by `X-GitHub-Delivery` and logged for a week; the Forges page shows the last one with its status. Removing the connection stops corntchap listening; delete the app on GitHub too if you do not need it.

### The pass: chefs talk

Every pot has a pass where its chefs (people and cooks) talk:
- **On the pass:** every chef of the pot reads it.
- **Direct (`to`):** only its author, the relaying cook (if any) and the recipient read it. Admins don't read other people's direct messages either, in the tools, the `PassMessage` model, the change feed or the live stream.
- **Wakes:** a direct message to a cook, or an `@mention` of one, wakes it with the message (a fire-and-forget cook takes direct messages from its owner only; others are refused, and a mention doesn't wake it). A **broadcast** (`broadcast: true`, or `@all` in the message) wakes every session of every cook in the pot, and isn't limited by lease slots. A cook that passes its own `station` reaches its other sessions too; without it, the sender's sessions are skipped. Each session is told to answer only if it has something to add. A **direct message to one session**: `to: "st_…"` (CLI `pass send --to st_…`, or "To session" when writing to a cook in the console) wakes exactly that session, even another of the sender's own sessions. When the sender passes its own `station`, the recipient is told to reply `to` it, so two sessions can talk to each other. Every wake that targets a session tells it its station id. Sessions pass it to `pass` and `comment`, and their messages show which session wrote them ("opencode-doug · quiet-planet").

`read_pass` returns the conversation, oldest first, plus what is still unread (per conversation), and counts as read.

People write in the console's Pass tab (with unread badges in the sidebar). A cook can also speak for its owner when asked in a chat (`fromOwner: true`, shown as "alice via c1"). The pass is working chatter and is kept for `PASS_RETENTION_DAYS` (default 7); durable knowledge belongs in the pantry.

CLI: `corntchap-cli pass read <POT> [--with NAME|pot] [-f]` and `pass send <POT> "message" [--to NAME] [--task PAY-1] [--from-owner]`.

#### Keeping up with the pantry

Cooks learn about pantry changes in two ways:
- **When they start work (quiet):** `claim_next` and `claim_task` return `pantry` news, the memories saved or changed by others since this cook last read the pantry (`read_pantry`, `recall`, `get_memory` count as reading). Every message wake also says "Pantry: N memories new or changed since you last read it: …".
- **Right away (loud):** `remember` with `notify: true` (CLI `pantry stock --notify`, or "Tell everyone now" in the console) posts the memory on the pass as a broadcast, which wakes every cook's sessions. Keep it for what can't wait, like a new convention or a "don't touch X".
- **Removals:** deleting a memory works the same way. It shows up in pantry news as `deleted: true`, and `forget` with `notify` (CLI `pantry toss --notify`, or the box in the console's delete dialog) broadcasts it.

### Views: SQL over a pot's tasks

A pot starts with its board. Further views are saved per pot by people or by cooks (`save_view`, `run_view`, `list_views`, `delete_view`; CLI `corntchap-cli view ls|run|save|rm`). A view is a SQL condition plus an optional order over the pot's task view, shown as a list or a board tab in the console, e.g. `menu = 'mvp' and 'urgent' = any(labels) and status <> 'done'` ordered by `priority desc, created_at`.

Columns: `id`, `number`, `title`, `body`, `status`, `priority`, `labels`, `menu`, `claimed_by`, `created_by`, `created_at`, `updated_at`, `ready`, `lease_live`, `waiting_on_question`, `open_items`, `total_items`, `parent`.

The SQL is never pasted into a query as text:
- **Parsing:** it must be exactly one expression (or a comma-separated order).
- **Allowlist:** every node is checked: the view's columns, literals, operators, CASE, casts and a few pure functions. No subqueries, other tables or placeholders.
- **Running:** it runs read-only with a 3 s timeout over that pot's rows (500 at most).
- **Saving:** a view that does not run (a type error, say) is refused when saved.

Deleting: people, or the cook that made the view (or one with the `plan` grant). Saved views are also readable as the `PotView` model and the `views` MCP resource.

### The pantry: shared memory per project

Every project has a pantry: what agents and people learned about it (conventions, gotchas, decisions and their reasons, how to build/test/deploy, facts about the system), saved under a stable key so the next agent starts from it instead of rediscovering it. MCP tools: `read_pantry` (the whole pantry as markdown, read once when starting on a project), `recall` (ranked full-text search with kind/tag filters; excerpts), `get_memory`, `remember` (create or update by key; `append`; pass the `revision` you read and a concurrent edit is refused instead of lost) and `forget` (agents only forget what they saved). Overwrites keep the previous version (`memory_revisions`); saves and deletions appear in the change feed (`memory.saved`, `memory.forgotten`). Also at `/api/v1/projects/{key}/pantry`, as the `memories` MCP resource and the read-only `Memory` model at `/api/v2`. Visibility follows the project: whoever sees the project shares its pantry.

### What corntchap costs your agents

Everything corntchap sends into an agent's context is counted, so you can see it instead of guessing:

- **Tool results:** the text of every MCP `tools/call` response, on both MCP revisions, by tool name.
- **Tool listings:** every `tools/list` response (sent once per session start; the biggest single item).
- **Wake prompts:** the text of the wakes corntchap words (answers, messages, mentions) and the text delivered to cloud targets (`cloud:webhook`, `cloud:github-comment`, `cloud:claude-routine`). A bare "new work is ready" wake to a relay carries no text; the relay words its own prompt.

Each send is added to a per-day counter by cook, pot, kind and name (the `usage_daily` table, kept 90 days). **This is an estimate**: tokens are `ceil(characters / 4)` of the text corntchap sends. It is not what the agent uses: corntchap cannot see the agent's own prompts, reasoning or model usage.

See it in the console on the Cooks page (a "Tokens sent (7 days)" line, and a Usage section in each cook card with a per-day bar row and its top three tools), or ask for the `usage_report` tool (`usageReport(project?, cook?, days?)`): totals, per day, per cook, and the top tools and wake reasons. A person sees their own cooks (super-admins all; an orchestrator also the usage in the pots they own), a cook sees itself, and a cook granted `manage` sees its owner's view.

## Mobile app

`apps/mobile` is a [Tauri v2](https://tauri.app) shell for Android and iOS (desktop too). It does not bundle the console: it asks for your server's URL once, then loads the console from it, so sign-in, `/api` and live updates behave exactly as in a browser. It is outside the Cargo workspace, so server builds and CI never compile it. Build steps and limitations are in [`apps/mobile/README.md`](apps/mobile/README.md); getting it onto TestFlight and Google Play is in [`apps/mobile/PUBLISHING.md`](apps/mobile/PUBLISHING.md); CI is `.github/workflows/mobile.yml` (manual or `app-v*` tags).

## Architecture

The server is built on [cratestack](https://cratestack.dev) 0.20 (`crates/server/schema/`: the root `corntchap.cstack` lists its parts under `types/`, `procedures/` and `models/`) and [authkestra](https://crates.io/crates/authkestra) 0.15.

- **Generated REST at `/api/v2`**: every table is a cratestack model (`@@map`/`@map` onto the snake_case tables) with `@@allow`/`@@deny` policies: relays see nothing, super-admins see everything, members see the projects, tasks, items, comments and events of their teams, and an agent may edit only a task it currently holds. Secrets (`oidc_sub`, tokens) are `@server_only` or not modelled. Lease and fencing operations (`claimNext`, `claimTask`, `heartbeat`, `checkItem`, `complete`, `release`, `block`, `unblockTask`, `cancelTask`, ...) are procedures with raw SQL, served at `POST /api/v2/$procs/<name>` and as MCP tools; all procedure outputs are typed (MCP `outputSchema`). Print the route table with `cargo test -p corntchap --test generated_api routes -- --nocapture`.
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

See `deploy/` for Kubernetes + CloudNativePG, and `Containerfile` for the distroless static image (ghcr.io/cratestack/corntchap).

### Deviations from the design doc

- Basic-auth/Argon2 is replaced by bearer tokens (device flow + OIDC SSO via Keycloak), per the later direction.
- MCP is cratestack's binding: tool arguments are flat; list arguments are strings (`labels`/`dependsOn` comma-separated, `items` one per line); every model is also an MCP resource (`cratestack://corntchap/<tasks|projects|items|comments|events|…>[/<id>]`, paged with `?limit=&cursor=`) under the same read policies as REST; `get_board` and `get_task_markdown` render markdown.
- Credentials live in `credentials.toml` in the config dir (see above) (0600) or `CORNTCHAP_TOKEN`; no OS keyring.
- No generated OpenAPI spec yet.
