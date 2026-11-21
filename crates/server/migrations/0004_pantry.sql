-- The pantry: shared memory per project. Agents (and people) save what they learn about a
-- project (conventions, gotchas, decisions, how-tos, facts) under a stable key, so the next
-- agent starts from it instead of rediscovering it.
create table memories (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id) on delete cascade,
  key text not null check (key ~ '^[a-z0-9][a-z0-9._-]{0,79}$'),
  title text not null check (length(title) between 1 and 200),
  body text not null check (length(body) <= 20000),
  kind text not null default 'note'
    check (kind in ('note', 'convention', 'gotcha', 'decision', 'howto', 'fact')),
  tags text[] not null default '{}',
  task_id uuid references tasks(id) on delete set null,
  created_by uuid references principals(id) on delete set null,
  updated_by uuid references principals(id) on delete set null,
  revision bigint not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  search tsvector generated always as (
    setweight(to_tsvector('simple', replace(key, '-', ' ') || ' ' || title), 'A') ||
    setweight(to_tsvector('english', body), 'B')
  ) stored,
  unique (project_id, key)
);
create index memories_search on memories using gin (search);
create index memories_tags on memories using gin (tags);
create index memories_recent on memories (project_id, updated_at desc);
create trigger memories_touch before update on memories
  for each row execute function touch_updated_at();

-- Every overwrite keeps the previous version, so a careless agent cannot lose what another saved.
create table memory_revisions (
  memory_id uuid not null references memories(id) on delete cascade,
  revision bigint not null,
  title text not null,
  body text not null,
  kind text not null,
  tags text[] not null,
  updated_by uuid references principals(id) on delete set null,
  updated_at timestamptz not null,
  primary key (memory_id, revision)
);
