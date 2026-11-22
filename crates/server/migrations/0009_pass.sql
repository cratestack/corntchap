-- The pass: the chefs of a pot talk. Cooks post; a person speaks through their own cook, which
-- posts in its owner's name (author = the owner, relayed_by = the cook). A message without a
-- recipient is on the pot's pass (every chef of the pot reads it); with a recipient it is direct,
-- read only by its author, the cook that relayed it and the recipient (admins do not read other
-- people's direct messages).
create table pass_messages (
  id uuid primary key default gen_random_uuid(),
  seq bigint generated always as identity unique,
  project_id uuid not null references projects(id) on delete cascade,
  author_id uuid references principals(id) on delete set null,
  to_principal uuid references principals(id) on delete cascade,
  body text not null check (length(body) between 1 and 8000),
  relayed_by uuid references principals(id) on delete set null,
  task_id uuid references tasks(id) on delete set null,
  created_at timestamptz not null default now(),
  -- text mirrors for the model's policies (the framework binds the caller id as text, and
  -- policies compare required fields only): '' when the author is gone, 'pot' on the pass
  author_ref text generated always as (coalesce(author_id::text, '')) stored,
  to_ref text generated always as (coalesce(to_principal::text, 'pot')) stored,
  relayed_ref text generated always as (coalesce(relayed_by::text, '')) stored
);
create index pass_pot on pass_messages (project_id, seq);
create index pass_direct on pass_messages (to_principal, seq) where to_principal is not null;

-- How far each chef has read each conversation of a pot: 'pot' (the pass itself) or the other
-- chef's id for a direct conversation. Unread is what comes after.
create table pass_reads (
  principal_id uuid not null references principals(id) on delete cascade,
  project_id uuid not null references projects(id) on delete cascade,
  conversation text not null,
  last_seq bigint not null default 0,
  primary key (principal_id, project_id, conversation)
);

-- Events meant for a few principals only (a direct message): the feed and the live stream show
-- them to those principals and nobody else. Null: everyone who can see the pot.
alter table events add column audience uuid[];
