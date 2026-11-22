-- Saved views of a pot's tasks: a SQL condition (and order) over the pot's task view, written by a
-- person or by a cook for them. The condition is parsed and checked before it is stored or run
-- (one expression over known columns, no subqueries, a short list of functions) and runs read-only.
create table pot_views (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id) on delete cascade,
  key text not null check (key ~ '^[a-z0-9][a-z0-9._-]{0,47}$'),
  name text not null check (length(name) between 1 and 80),
  description text,
  layout text not null default 'list' check (layout in ('list', 'board')),
  where_sql text not null default 'true' check (length(where_sql) <= 2000),
  order_sql text check (length(order_sql) <= 500),
  position bigint not null default 0,
  created_by uuid references principals(id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (project_id, key)
);
