-- Comments from people that woke a cook and wait for its answer. If the cook does not answer on
-- the task (a comment, or completing / releasing / blocking it) in time, its main station is
-- woken once more with the comment (after COMMENT_NUDGE_MINUTES, default 10).
create table comment_replies (
  comment_id uuid not null references comments(id) on delete cascade,
  agent_id uuid not null references principals(id) on delete cascade,
  task_id uuid not null references tasks(id) on delete cascade,
  project_id uuid not null references projects(id) on delete cascade,
  author_id uuid references principals(id) on delete set null,
  since_seq bigint not null,
  created_at timestamptz not null default now(),
  nudged_at timestamptz,
  answered_at timestamptz,
  primary key (comment_id, agent_id)
);
create index comment_replies_pending on comment_replies (created_at) where nudged_at is null and answered_at is null;
