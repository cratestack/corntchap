-- Reviews: a cook (or a person) asks a cook or a person to look at a task's work. A review is not a
-- lane: the task keeps its status. One row per request; a task reviewed again after changes gets a
-- new row with the next round.
alter table projects add column next_review bigint not null default 1;

create table reviews (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id) on delete cascade,
  number bigint not null,
  task_id uuid not null references tasks(id) on delete cascade,
  requested_by uuid references principals(id) on delete set null,
  -- the one asked; null: anyone in the pot's team except the requester
  reviewer uuid references principals(id) on delete set null,
  state text not null default 'requested'
    check (state in ('requested', 'approved', 'changes_requested', 'commented', 'cancelled')),
  -- what to look at: a pull request address, a branch, a question
  note text not null default '',
  -- the verdict's text
  verdict text,
  -- who gave the verdict (the named reviewer, or whoever of the team took an open request)
  reviewed_by uuid references principals(id) on delete set null,
  round integer not null default 1,
  created_at timestamptz not null default now(),
  decided_at timestamptz,
  updated_at timestamptz not null default now(),
  unique (project_id, number)
);
-- one open review per task and reviewer (null: one open request to anyone)
create unique index reviews_one_open on reviews
  (task_id, coalesce(reviewer, '00000000-0000-0000-0000-000000000000'::uuid)) where state = 'requested';
create index reviews_task on reviews (task_id, round desc, created_at desc);
create index reviews_open on reviews (project_id, created_at) where state = 'requested';
create trigger reviews_touch before update on reviews
  for each row execute function touch_updated_at();
