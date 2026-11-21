-- Stations: one agent session (an OpenCode, Claude Code or Codex session in a checkout or a
-- worktree) registered for a cook in a pot. Wakes go to a station, so the very session is
-- resumed; leases record the station that holds them.
create table stations (
  id uuid primary key default gen_random_uuid(),
  ref text not null unique,               -- short public id, st_xxxxxxxx
  agent_id uuid not null references principals(id) on delete cascade,
  project_id uuid not null references projects(id) on delete cascade,
  runtime text not null check (runtime in ('opencode', 'claude-code', 'codex', 'other')),
  session_ref text not null,              -- the runtime's own session id
  directory text,                         -- absolute path on the cook's machine
  label text,
  created_by uuid references principals(id) on delete set null,
  created_at timestamptz not null default now(),
  last_woken_at timestamptz,
  unique (agent_id, runtime, session_ref)
);
create index stations_wake on stations (agent_id, project_id);

alter table tasks add column station_id uuid references stations(id) on delete set null;
-- A task parked on a question stays with the cook (and station) that asked.
alter table tasks add column reserved_for uuid references principals(id) on delete set null;

-- The question pool: cooks (or people) ask; a person answers in the console, or a cook relays the
-- question to its owner in a chat session and records the answer in the owner's name.
alter table projects add column next_question bigint not null default 1;
create table questions (
  id uuid primary key default gen_random_uuid(),
  project_id uuid not null references projects(id) on delete cascade,
  number bigint not null,
  task_id uuid references tasks(id) on delete set null,
  asked_by uuid references principals(id) on delete set null,
  station_id uuid references stations(id) on delete set null,
  to_principal uuid references principals(id) on delete set null,
  body text not null check (length(body) between 1 and 8000),
  options text[] not null default '{}',
  wait boolean not null default false,
  status text not null default 'open' check (status in ('open', 'answered', 'dismissed')),
  picked_by uuid references principals(id) on delete set null,
  pick_expires_at timestamptz,
  answer text,
  answered_by uuid references principals(id) on delete set null,
  relayed_by uuid references principals(id) on delete set null,
  answered_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (project_id, number)
);
create index questions_open on questions (project_id, created_at) where status = 'open';
create trigger questions_touch before update on questions
  for each row execute function touch_updated_at();

create table question_notes (
  id uuid primary key default gen_random_uuid(),
  question_id uuid not null references questions(id) on delete cascade,
  author_id uuid references principals(id) on delete set null,
  body text not null check (length(body) between 1 and 8000),
  created_at timestamptz not null default now()
);
create index question_notes_q on question_notes (question_id, created_at);
