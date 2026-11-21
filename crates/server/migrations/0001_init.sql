create table principals (
  id uuid primary key default gen_random_uuid(),
  kind text not null check (kind in ('user','agent','relay')),
  name text unique not null,
  owner_id uuid references principals(id),
  github_id text unique,
  github_login text unique,
  agent_kind text,
  runtime text,
  max_leases int not null default 1,
  is_admin bool not null default false,
  disabled_at timestamptz,
  last_seen_at timestamptz,
  created_at timestamptz not null default now()
);
create index principals_owner on principals(owner_id);

create table teams (id uuid primary key default gen_random_uuid(), name text unique not null);
create table team_members (
  team_id uuid not null references teams(id) on delete cascade,
  principal_id uuid not null references principals(id) on delete cascade,
  role text not null check (role in ('owner','member')),
  primary key (team_id, principal_id)
);
create index team_members_principal on team_members(principal_id);

create table projects (
  id uuid primary key default gen_random_uuid(),
  team_id uuid not null references teams(id),
  key text unique not null check (key ~ '^[A-Z][A-Z0-9]{0,9}$'),
  name text not null,
  repo_url text,
  lease_ttl interval not null default '10 minutes',
  next_number int not null default 1
);

create table tasks (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id),
  number int not null,
  title text not null,
  body text not null default '',
  priority int not null default 0,
  labels text[] not null default '{}',
  parent_id uuid references tasks(id),
  status text not null check (status in ('open','claimed','blocked','done','cancelled')),
  claimed_by uuid references principals(id),
  lease_epoch bigint not null default 0,
  lease_expires_at timestamptz,
  result text,
  created_by uuid references principals(id),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (project_id, number)
);
create index tasks_ready on tasks (project_id, priority desc, created_at) where status = 'open';
create index tasks_leases on tasks (lease_expires_at) where status = 'claimed';
create index tasks_claimed_by on tasks (claimed_by) where status = 'claimed';
create index tasks_parent on tasks (parent_id);

create table task_deps (
  task_id uuid not null references tasks(id) on delete cascade,
  depends_on uuid not null references tasks(id),
  primary key (task_id, depends_on)
);
create index task_deps_on on task_deps(depends_on);

create table items (
  task_id uuid not null references tasks(id) on delete cascade,
  position int not null,
  text text not null,
  done bool not null default false,
  done_by uuid references principals(id),
  primary key (task_id, position)
);

create table comments (
  id uuid primary key default gen_random_uuid(),
  task_id uuid not null references tasks(id) on delete cascade,
  author_id uuid references principals(id),
  body text not null,
  created_at timestamptz not null default now()
);
create index comments_task on comments(task_id, created_at);

create table wake_targets (
  id uuid primary key default gen_random_uuid(),
  agent_id uuid not null references principals(id) on delete cascade,
  type text not null check (type in ('relay','webhook','github-comment')),
  projects text[],
  labels text[],
  config jsonb not null default '{}'
);
create index wake_targets_agent on wake_targets(agent_id);

create table events (
  seq bigserial primary key,
  project_id uuid,
  task_id uuid,
  kind text not null,
  actor_id uuid,
  payload jsonb not null default '{}',
  at timestamptz not null default now()
);
create index events_project on events(project_id, seq);
create index events_at on events(at);

create table event_daily_counts (
  day date not null,
  project_id uuid not null,
  kind text not null,
  count bigint not null,
  primary key (day, project_id, kind)
);

create function notify_event() returns trigger language plpgsql as $$
begin
  perform pg_notify('events', new.seq::text);
  return new;
end $$;
create trigger events_notify after insert on events
  for each row execute function notify_event();

-- Bearer credentials. Only a SHA-256 of the token is stored (32 random bytes: a slow hash adds nothing).
create table tokens (
  id uuid primary key default gen_random_uuid(),
  principal_id uuid not null references principals(id) on delete cascade,
  token_hash bytea unique not null,
  label text not null default '',
  created_at timestamptz not null default now(),
  last_used_at timestamptz,
  expires_at timestamptz,
  revoked_at timestamptz
);
create index tokens_principal on tokens(principal_id);

-- RFC 8628 device authorization grant. A human approves in the browser (GitHub SSO);
-- the device then exchanges its device_code for a token.
create table device_requests (
  id uuid primary key default gen_random_uuid(),
  device_hash bytea unique not null,
  user_code text unique not null,
  kind text not null check (kind in ('user','agent','relay')),
  name text,
  runtime text,
  agent_kind text,
  status text not null default 'pending' check (status in ('pending','approved','denied','consumed')),
  approved_by uuid references principals(id),
  interval_s int not null default 5,
  last_polled_at timestamptz,
  created_at timestamptz not null default now(),
  expires_at timestamptz not null
);

-- OAuth2 (GitHub) login state between redirect and callback.
create table oauth_states (
  state text primary key,
  data jsonb not null,
  next text not null default '/',
  expires_at timestamptz not null
);

-- Browser sessions for the approval page and the board.
create table web_sessions (
  id_hash bytea primary key,
  user_id uuid not null references principals(id) on delete cascade,
  csrf text not null,
  created_at timestamptz not null default now(),
  expires_at timestamptz not null
);
