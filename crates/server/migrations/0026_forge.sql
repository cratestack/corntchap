-- Forge events (src/forge): a git host (GitHub first, through a GitHub App) tells corntchap about pull
-- requests, checks and reviews. Nothing is imported: a PR only attaches to a task that exists.

-- One row per connected app. The app's secrets (client secret, private key, webhook secret) are
-- stored here, as wake-target secrets are, and never returned by any API; the webhook secret must be
-- readable to check signatures.
create table forge_connections (
  id uuid primary key default gen_random_uuid(),
  provider text not null check (provider in ('github','gitlab','gitea')),
  name text not null,
  -- the REST API (https://api.github.com, or https://ghe.example/api/v3) and the web host
  api_url text not null,
  web_url text not null,
  app_id bigint,
  slug text,
  html_url text,
  client_id text,
  client_secret text,
  private_key text,
  webhook_secret text not null,
  owner_login text,
  created_by uuid references principals(id) on delete set null,
  created_at timestamptz not null default now()
);

-- A manifest flow in progress: the `state` the callback must bring back.
create table forge_states (
  state text primary key,
  provider text not null,
  org text,
  api_url text not null,
  web_url text not null,
  created_by uuid references principals(id) on delete cascade,
  expires_at timestamptz not null
);

-- Where an app is installed (learned from `installation` events and from the events themselves).
create table forge_installations (
  connection_id uuid not null references forge_connections(id) on delete cascade,
  installation_id bigint not null,
  account text,
  account_type text,
  repo_selection text,
  suspended boolean not null default false,
  created_at timestamptz not null default now(),
  primary key (connection_id, installation_id)
);

-- Every delivery received, kept 7 days (the retention job in jobs.rs): dedupe by delivery id, and
-- "the last delivery" on the admin page.
create table forge_deliveries (
  id bigint generated always as identity primary key,
  provider text not null,
  delivery_id text not null,
  connection_id uuid references forge_connections(id) on delete cascade,
  event text not null,
  action text,
  status text not null check (status in ('received','ok','ignored','error')),
  detail text,
  at timestamptz not null default now(),
  unique (provider, delivery_id)
);
create index forge_deliveries_at on forge_deliveries (at);

-- The pull requests corntchap follows: only those of a repository that belongs to a pot (or that a
-- person linked by hand). `task_id` is the task it is linked to, if any.
create table forge_prs (
  id uuid primary key default gen_random_uuid(),
  provider text not null,
  repo text not null,
  norm_url text not null,
  number bigint not null,
  url text not null default '',
  title text not null default '',
  author text,
  state text not null default 'open' check (state in ('open','closed','merged')),
  draft boolean not null default false,
  head_branch text not null default '',
  head_sha text not null default '',
  base_branch text,
  checks text not null default 'none' check (checks in ('pending','success','failure','none')),
  failing text[] not null default '{}',
  failing_sha text not null default '',
  checks_url text,
  review text not null default 'none' check (review in ('approved','changes_requested','commented','none')),
  mergeable text not null default 'unknown' check (mergeable in ('true','false','unknown')),
  task_id uuid references tasks(id) on delete set null,
  -- a person linked it (or unlinked it): the branch and title rules leave it alone
  explicit boolean not null default false,
  unlinked boolean not null default false,
  -- what was already told, so a burst of events wakes a cook once: "ci:<sha>", "conflict:<sha>", "merged"
  notified text[] not null default '{}',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create unique index forge_prs_key on forge_prs (provider, lower(repo), number);
create index forge_prs_task on forge_prs (task_id) where task_id is not null;
create index forge_prs_norm on forge_prs (norm_url);
