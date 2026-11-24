-- Coder (coder.com) as a cloud wake target: a wake starts a Coder agent run, a later wake for the
-- same cook and pot goes to that run as a follow-up message. `coder_runs` remembers which run.
alter table wake_targets drop constraint wake_targets_type_check;
alter table wake_targets add constraint wake_targets_type_check
  check (type in ('relay', 'webhook', 'github-comment', 'claude-routine', 'coder'));
alter table cloud_setups drop constraint cloud_setups_type_check;
alter table cloud_setups add constraint cloud_setups_type_check
  check (type in ('claude-routine', 'webhook', 'github-comment', 'coder'));

create table coder_runs (
  target_id uuid not null references wake_targets(id) on delete cascade,
  -- the pot's key
  project text not null,
  -- 'chats' (POST /api/v2/chats) or 'tasks' (the removed Tasks API, older deployments)
  api text not null,
  -- the Coder chat id or task id
  remote_id text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  primary key (target_id, project)
);
