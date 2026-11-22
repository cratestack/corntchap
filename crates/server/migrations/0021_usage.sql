-- What corntchap sends to agents, per day: MCP tool results, tool listings and wake prompts.
-- An estimate of the text only (tokens ~ chars / 4); the agent's own usage is not visible here.
-- Pruned to 90 days by the retention job.
create table usage_daily (
  day date not null,
  principal_id uuid not null references principals(id) on delete cascade,
  project_id uuid references projects(id) on delete cascade,
  -- tool (a tools/call result), list (a tools/list response) or wake (a wake prompt)
  kind text not null check (kind in ('tool', 'list', 'wake')),
  -- the tool name, `tools/list`, the wake reason, or `cloud:<type>`
  name text not null,
  calls bigint not null default 0,
  chars bigint not null default 0,
  constraint usage_daily_key unique nulls not distinct (day, principal_id, project_id, kind, name)
);

create index usage_daily_day on usage_daily (day);
create index usage_daily_project on usage_daily (project_id, day);
