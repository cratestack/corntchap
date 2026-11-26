-- A question can be linked to tasks other than the one it was asked on: whoever links it is told
-- when it is answered or dismissed, and a link that `blocks` keeps the task from being ready until
-- then. `questions.task_id` (the task it was asked on) stays and counts as an implicit link.
create table question_links (
  question_id uuid not null references questions(id) on delete cascade,
  task_id uuid not null references tasks(id) on delete cascade,
  linked_by uuid references principals(id) on delete set null,
  blocks boolean not null default false,
  created_at timestamptz not null default now(),
  primary key (question_id, task_id)
);
create index question_links_task on question_links (task_id) where blocks;
create index question_links_task_all on question_links (task_id);
