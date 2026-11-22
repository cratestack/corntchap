-- Files in S3-compatible storage, per pot (PAY-F12). The bytes never pass through Postgres.
alter table projects add column next_file bigint not null default 1;
create table files (
  id uuid primary key,
  project_id uuid not null references projects(id) on delete cascade,
  number bigint not null,
  name text not null check (length(name) between 1 and 200),
  content_type text not null,
  size_expected bigint,
  size bigint,
  storage_key text not null unique,
  status text not null default 'uploading' check (status in ('uploading', 'ready', 'failed', 'deleted')),
  error text,
  created_by uuid references principals(id) on delete set null,
  task_id uuid references tasks(id) on delete set null,
  question_id uuid references questions(id) on delete set null,
  created_at timestamptz not null default now(),
  ready_at timestamptz,
  upload_expires_at timestamptz not null,
  unique (project_id, number)
);
create index files_pending on files (created_at) where status = 'uploading';
create index files_task on files (task_id) where task_id is not null;
