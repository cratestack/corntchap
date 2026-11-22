-- Connecting a cloud cook (Claude routine, @codex comments, a webhook) in steps: an agent or a
-- person opens a setup, the secret is entered in the MCP App, on the console link or given to
-- the tool, and the setup records the wake target it made.
create table cloud_setups (
  id uuid primary key default gen_random_uuid(),
  code text not null unique,
  agent_id uuid not null references principals(id) on delete cascade,
  type text not null check (type in ('claude-routine', 'webhook', 'github-comment')),
  projects text[] not null default '{}',
  created_by uuid not null references principals(id) on delete cascade,
  created_at timestamptz not null default now(),
  expires_at timestamptz not null,
  target_id uuid references wake_targets(id) on delete set null,
  done_at timestamptz,
  done_by uuid references principals(id) on delete set null,
  test jsonb
);
create index cloud_setups_agent on cloud_setups(agent_id);
